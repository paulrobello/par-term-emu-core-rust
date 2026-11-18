//! Handshake, feature negotiation, and the no-daemon path.

use super::*;

/// The daemon answers version and list-commands; the handshake must
/// send version, then list-commands, then set-client-colors, then
/// refresh-client with -C and -p — in that order.
#[test]
fn handshake_sends_version_list_commands_colors_refresh_in_order() {
    let (daemon, path) = FakeDaemon::bind("order");
    {
        let conn = conn::AttachConn::connect(&path).expect("connect");
        assert_eq!(conn.daemon_stamp(), Some("9.9.9+deadbeef"));
        assert!(conn.has_feature("replay-held-state"));
    }
    let lines = recorded(&daemon.received, 4);
    let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "version",
            "list-commands",
            "set-client-colors",
            "refresh-client"
        ],
        "handshake order: {lines:?}"
    );
    let colors = &lines[2].1;
    assert!(colors.starts_with("set-client-colors -f "), "got: {colors}");
    assert!(colors.contains(" -b "), "got: {colors}");
    let refresh = &lines[3].1;
    assert!(refresh.contains(" -C "), "got: {refresh}");
    assert!(refresh.contains(" -p "), "got: {refresh}");
}

/// A pre-feature daemon rejects list-commands; the handshake proceeds
/// with no features and still completes steps 3-4.
#[test]
fn pre_feature_daemon_leads_to_empty_features_and_completed_handshake() {
    let path = test_socket("prefeature");
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        if let Ok(stream) = listener.accept() {
            serve_pre_feature(stream, tx);
        }
    });
    let conn = conn::AttachConn::connect(&path).expect("connect");
    assert!(conn.daemon_features().is_empty());
    assert!(!conn.has_feature("replay-held-state"));
    let lines = recorded(&rx, 4);
    let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "version",
            "list-commands",
            "set-client-colors",
            "refresh-client"
        ]
    );
}

/// A daemon from before `list-commands`: %error on it, everything else
/// answered.
fn serve_pre_feature(
    stream: crate::mux::LocalStream,
    tx: std::sync::mpsc::Sender<(String, String)>,
) {
    use interprocess::TryClone as _;
    let mut writer = stream.try_clone().expect("clone stream");
    let mut reader = BufReader::new(stream);
    let mut number = 0u32;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
        number += 1;
        tx.send((name.clone(), trimmed.to_owned())).ok();
        let reply = match name.as_str() {
            "version" => "9.9.9+deadbeef".to_string(),
            _ => String::new(),
        };
        let ok = name == "version" || name == "set-client-colors" || name == "refresh-client";
        writer
            .write_all(emit_block(number, &reply, ok).as_bytes())
            .ok();
        writer.flush().ok();
    }
}

/// The stamp quiet rule: equal stamps are quiet; a same-version
/// `+unknown` pair is quiet (unprovable); anything else warns.
#[test]
fn stamp_quiet_rule() {
    use super::conn::stamp_mismatch_is_quiet as quiet;
    assert!(quiet("1.2.3+abc", "1.2.3+abc"));
    assert!(quiet("1.2.3+unknown", "1.2.3+whatever"));
    assert!(quiet("1.2.3+abc", "1.2.3+unknown"));
    assert!(!quiet("1.2.3+abc", "1.2.4+abc"));
    assert!(!quiet("1.2.3+abc", "1.2.3+def"));
}

/// `list-commands` reply parsing: the `features` line's tokens, unknown
/// lines ignored.
#[test]
fn feature_token_parsing() {
    use super::conn::parse_feature_tokens;
    let body = vec![
        "capture-pane escape".to_string(),
        "features replay-held-state".to_string(),
        "something else".to_string(),
    ];
    assert_eq!(
        parse_feature_tokens(&body),
        vec!["replay-held-state".to_string()]
    );
    assert!(parse_feature_tokens(&[]).is_empty());
}

/// Per-command feature tokens come from the command rows (the daemon's
/// REAL `list-commands` body), not the daemon-level `features` line — the
/// `refresh-client chrome` token the size report's `-I` gates on.
#[test]
fn command_feature_parsing_reads_the_real_list_commands_body() {
    use super::conn::{parse_command_features, parse_feature_tokens};
    let body: Vec<String> = crate::mux::command::list_commands_body()
        .lines()
        .map(str::to_owned)
        .collect();
    let pairs = parse_command_features(&body);
    assert!(pairs.contains(&("refresh-client".to_string(), "chrome".to_string())));
    assert!(pairs.contains(&("refresh-client".to_string(), "cell-pixels".to_string())));
    assert!(
        !pairs.iter().any(|(c, _)| c == "features"),
        "the daemon-level line is not a command row"
    );
    assert!(!parse_feature_tokens(&body).contains(&"chrome".to_string()));
}

/// attach with no daemon on the path reports the no-daemon error
/// instead of spawning one.
#[test]
fn no_daemon_path_reports_no_daemon_without_spawning() {
    let path = test_socket("nodaemon");
    let _ = std::fs::remove_file(&path);
    let options = AttachOptions {
        socket: Some(path.clone()),
        name: None,
        target: None,
        prefix: None,
        reload: None,
        mode: None,
    };
    assert_eq!(run(&options), ExitCode::FAILURE);
    // And nothing appeared on the path: no auto-spawn.
    assert!(!path.exists(), "attach must not spawn a daemon");
}

/// A real server answers the whole handshake; attach resolves the only
/// session's active pane, pumps, and exits cleanly when the
/// daemon shuts down (`%exit`). The headless test ends through that
/// shutdown — stdin never delivers bytes here, so `%exit` is the
/// deterministic exit path.
#[test]
fn end_to_end_with_real_server() {
    let path = test_socket("real");
    let _ = std::fs::remove_file(&path);
    let server = MuxServer::bind(&path).expect("bind server");
    let handle = Arc::new(Mutex::new(Some(server.shutdown_handle())));
    let server_thread = std::thread::spawn(move || server.run());
    // Give the accept loop a moment; `run()` returns only at shutdown.
    std::thread::sleep(std::time::Duration::from_millis(100));

    // A pane to attach to, created by an ordinary client.
    let mut seeder = crate::mux::MuxClient::connect(&path).expect("seeder connect");
    seeder
        .send("new-session -s attach-e2e")
        .expect("new-session");

    // Shut the daemon down shortly after attach starts: %exit is the
    // pump's deterministic exit, and the headless harness never types.
    let killer_path = path.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Ok(mut client) = crate::mux::MuxClient::connect(&killer_path) {
            let _ = client.send_checked("kill-server");
        }
    });

    let options = AttachOptions {
        socket: Some(path.clone()),
        name: None,
        target: None,
        prefix: None,
        reload: None,
        mode: None,
    };
    assert_eq!(run(&options), ExitCode::SUCCESS);

    handle
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    server_thread.join().expect("server thread");
}
