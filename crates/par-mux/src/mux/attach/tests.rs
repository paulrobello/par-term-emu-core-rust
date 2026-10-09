use super::*;
use crate::mux::{emit_block, MuxServer};
// The Listener trait supplies `.accept()` on unix; the Windows named-pipe
// listener accepts via its own inherent impl, so this import is unused there.
#[cfg(unix)]
use interprocess::local_socket::traits::Listener as _;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

/// A temp-dir socket path unique to this test process.
fn test_socket(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("par-mux-attach-{}-{tag}.sock", std::process::id()));
    path
}

/// A hand-rolled daemon substitute: accepts one connection, records
/// every control line it receives (in order), and answers each with a
/// scripted reply chosen by the command name. Replies use the real
/// wire framing (`emit_block`) so the client parser sees the protocol
/// exactly as the daemon writes it. The listener is consumed by the
/// accept thread; the socket path is what the test connects to.
struct FakeDaemon {
    /// (command name, full command line) per control line, in order.
    received: Receiver<(String, String)>,
}

impl FakeDaemon {
    fn bind(tag: &str) -> (Self, PathBuf) {
        let path = test_socket(tag);
        let _ = std::fs::remove_file(&path);
        let listener = crate::mux::bind_local_listener(&path).expect("bind fake daemon");
        let (tx, received) = channel();
        std::thread::spawn(move || {
            if let Ok(stream) = listener.accept() {
                serve_one(stream, tx);
            }
        });
        (Self { received }, path)
    }
}

/// Serve one connection: record lines, answer per the scripted table.
fn serve_one(stream: crate::mux::LocalStream, tx: std::sync::mpsc::Sender<(String, String)>) {
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
        let reply = match name.as_str() {
            "version" => "9.9.9+deadbeef".to_string(),
            "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
            "list-panes" => "%0\n".to_string(),
            // The resync's scripted restore body: absolute row
            // addressing like the real encoder's, ending with the
            // pane's real cursor CUP (row 2, col 9 — right after
            // "prompt> "). No trailing newline, like the real stream.
            "refresh-client" if trimmed.contains("-t %0") => {
                "\x1b[1;1Hfirst\x1b[2;1Hprompt> \x1b[2;9H".to_string()
            }
            _ => String::new(),
        };
        number += 1;
        tx.send((name, trimmed.to_owned())).ok();
        writer
            .write_all(emit_block(number, &reply, true).as_bytes())
            .ok();
        writer.flush().ok();
    }
}

/// Waits for the recorded lines, tolerating the reader thread's delay.
fn recorded(rx: &Receiver<(String, String)>, n: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for _ in 0..n {
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(line) => out.push(line),
            Err(_) => break,
        }
    }
    out
}

/// The next recorded line whose command name is `name`, draining any
/// interleaved status re-queries first — the robust needle when the
/// chord's resync burst has a variable tail.
fn wait_for_line(rx: &Receiver<(String, String)>, name: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::time::Instant::now() > deadline {
            panic!("no {name} line arrived within the bound");
        }
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok((n, line)) if n == name => return line,
            Ok(_) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("no {name} line: the daemon side is gone");
            }
        }
    }
}

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

/// A real server answers the whole handshake; attach resolves the
/// newest session's newest pane, pumps, and exits cleanly when the
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

/// AttachOptions resolves --socket over NAME and falls back to the env
/// var in the documented precedence.
#[test]
fn socket_path_precedence() {
    let explicit = AttachOptions {
        socket: Some(PathBuf::from("/tmp/explicit.sock")),
        name: Some("work".to_string()),
        target: None,
        prefix: None,
        reload: None,
        mode: None,
    };
    assert_eq!(explicit.socket_path(), PathBuf::from("/tmp/explicit.sock"));
    let named = AttachOptions {
        socket: None,
        name: Some("work".to_string()),
        target: None,
        prefix: None,
        reload: None,
        mode: None,
    };
    assert_eq!(named.socket_path(), crate::mux::default_socket_path("work"));
    let fallback = AttachOptions {
        socket: None,
        name: None,
        target: None,
        prefix: None,
        reload: None,
        mode: None,
    };
    // No env var in the test harness -> the unnamed default.
    if std::env::var_os("PAR_MUX_SOCKET").is_none() {
        assert_eq!(
            fallback.socket_path(),
            crate::mux::default_socket_path("default")
        );
    }
}

/// The `--prefix` grammar: control spellings and a literal single
/// character; anything else refuses rather than silently misrouting.
#[test]
fn prefix_parsing_covers_tmux_spellings_and_literals() {
    assert_eq!(parse_prefix("C-b"), Some(0x02));
    assert_eq!(parse_prefix("C-a"), Some(0x01));
    assert_eq!(parse_prefix("C-Z"), Some(0x1a));
    assert_eq!(parse_prefix("C-Space"), Some(0x00));
    assert_eq!(parse_prefix("q"), Some(b'q'));
    assert_eq!(parse_prefix(""), None);
    assert_eq!(parse_prefix("C-"), None);
    assert_eq!(parse_prefix("C-1"), None);
    assert_eq!(parse_prefix("C-bb"), None);
}

/// The default target: the newest session's newest pane — the highest
/// pane id in the global roster. A real tree over the default factory
/// pins the pick with one pane.
#[test]
fn default_target_is_the_newest_sessions_newest_pane() {
    let (daemon, path) = FakeDaemon::bind("resolve");
    {
        let mut conn = conn::AttachConn::connect(&path).expect("connect");
        let pane = resolve_target(&mut conn, None).expect("resolve");
        assert_eq!(pane, "%0");
        // The fake saw list-panes after the handshake's four lines.
        let lines = recorded(&daemon.received, 5);
        assert_eq!(
            lines[4].0, "list-panes",
            "handshake then list-panes: {lines:?}"
        );
    }
}

/// The `list-sessions` / bare `list-windows` wire shape `$N: name`
/// parses to a colon-less id plus the name — the shape the daemon's
/// id parser (SessionId::from_str) accepts, and the reason a plain
/// whitespace split (`$0:`) breaks every downstream `-t $N:` query.
#[test]
fn parse_session_line_strips_the_sigil_colon() {
    assert_eq!(
        parse_session_line("$0: work"),
        Some(("$0".to_string(), "work".to_string()))
    );
    assert_eq!(
        parse_session_line("$12: spaced name"),
        Some(("$12".to_string(), "spaced name".to_string()))
    );
    assert_eq!(parse_session_line("$12"), None, "no colon-space, no parse");
    assert_eq!(parse_session_line(""), None);
}

/// One `list-workspaces` line parses into id, name, and the active
/// marker; a line without the `+N:` shape does not parse.
#[test]
fn parse_workspace_line_splits_the_id_name_and_active_marker() {
    assert_eq!(
        parse_workspace_line("+0: main active"),
        Some(("+0".to_string(), "main".to_string(), true))
    );
    assert_eq!(
        parse_workspace_line("+1: lab"),
        Some(("+1".to_string(), "lab".to_string(), false))
    );
    assert_eq!(
        parse_workspace_line("+2: spaced name"),
        Some(("+2".to_string(), "spaced name".to_string(), false))
    );
    assert_eq!(parse_workspace_line("0: x"), None, "no + sigil, no parse");
    assert_eq!(parse_workspace_line("+2"), None, "no colon-space, no parse");
    assert_eq!(parse_workspace_line(""), None);
}

/// `-t` with a pane id passes through; a window target resolves to the
/// window's marked pane via the targeted list-panes query.
#[test]
fn explicit_targets_resolve_through_the_daemon() {
    // Scripted daemon: pane-info %5 ok; list-panes -t @1 replies a
    // two-pane roster with pane %7 marked active.
    let path = test_socket("targets");
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        use interprocess::TryClone as _;
        let Ok(stream) = listener.accept() else {
            return;
        };
        let mut writer = stream.try_clone().expect("clone");
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
            let reply = match trimmed {
                "version" => "9.9.9+deadbeef".to_string(),
                "list-commands" => String::new(),
                "pane-info -t %5" => "%5 @0 80x24".to_string(),
                "list-panes -t @1" => "%6 0 -\n%7 1 *".to_string(),
                _ => String::new(),
            };
            tx.send((name, trimmed.to_owned())).ok();
            writer
                .write_all(crate::mux::emit_block(number, &reply, true).as_bytes())
                .ok();
            writer.flush().ok();
        }
    });
    let mut conn = conn::AttachConn::connect(&path).expect("connect");
    drop(rx);
    assert_eq!(
        resolve_target(&mut conn, Some("%5")).expect("pane id"),
        "%5"
    );
    assert_eq!(
        resolve_target(&mut conn, Some("@1")).expect("window"),
        "%7",
        "the marked active pane wins over the first row"
    );
    assert!(
        resolve_target(&mut conn, Some("@nope")).is_err(),
        "an unresolvable target errors"
    );
}

/// Prefix routing: d detaches; a plain byte goes to the pane as hex;
/// prefix prefix sends the prefix byte itself.
#[test]
fn prefix_router_detaches_forwards_and_sends_literal_prefix() {
    let mut session = Session {
        conn: test_dead_conn(),
        socket_path: PathBuf::from("/nonexistent"),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    // Plain bytes forward when the pane is live.
    assert!(!session.route_bytes(b"hello"));
    assert!(session.route_bytes(&[0x02, b'd']), "prefix d detaches");
    assert!(
        !session.route_bytes(&[0x02, 0x02]),
        "prefix prefix consumes without detaching"
    );
}

/// A held-dead focused pane takes no bytes: with `exited` set, plain
/// stdin bytes are dropped, so the daemon's NotStartedError `%error`
/// reply to a dead-pane send-keys never has a chance to (a) hit the
/// wire at all or (b) the bytes themselves never echo into the pane's
/// frozen screen. Prefix chords still route.
#[test]
fn dead_pane_takes_no_stdin_bytes_but_prefix_keys_still_route() {
    let mut session = Session {
        conn: test_dead_conn(),
        socket_path: PathBuf::from("/nonexistent"),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: Some(Some(0)),
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    assert!(
        !session.route_bytes(b"typed while dead"),
        "typing into a dead pane never detaches"
    );
    // Prefix keys still route on a dead pane.
    assert!(
        session.route_bytes(&[0x02, b'd']),
        "prefix d still detaches"
    );
    assert!(
        !session.route_bytes(&[0x02, 0x02]),
        "prefix prefix still consumes"
    );
}

/// The status line carries the respawn hint beside the exit code —
/// the cue that tells a user why their typing does nothing.
#[test]
fn status_line_names_the_respawn_chord_when_the_pane_is_dead() {
    let mut session = Session {
        conn: test_dead_conn(),
        socket_path: PathBuf::from("/nonexistent"),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: "work".to_string(),
        workspaces: vec![
            ("+0".to_string(), "main".to_string()),
            ("+1".to_string(), "lab".to_string()),
        ],
        active_workspace: Some("+0".to_string()),
        pane_title: "bash".to_string(),
        agents: 0,
        exited: Some(Some(7)),
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    let line = session.status_line();
    assert!(
        line.contains("(exited 7 — C-b r respawns)"),
        "the held-dead cue names the respawn chord: {line}"
    );
    session.exited = Some(None);
    assert!(
        session
            .status_line()
            .contains("(exited ? — C-b r respawns)"),
        "the unknown-code form carries the hint too"
    );
    session.exited = None;
    assert!(
        !session.status_line().contains("respawns"),
        "a live pane's line has no exited cue"
    );
}

/// The status draw's cursor placement tracks the shadow emulator: feed
/// it a replay that parks the cursor at a known cell, then a %output
/// The resync seeds the shadow with the daemon's screen-restore byte
/// stream VERBATIM: the stream ends with the pane's real cursor CUP,
/// so the shadow's tracked cursor is the pane's truth. A trailing
/// newline appended after that CUP parks the shadow — and with it
/// every status draw's absolute placement — one row below the prompt,
/// and only the shell's next repaint papers over it (the manual-pass
/// cursor bug).
#[test]
fn resync_feeds_the_shadow_the_restore_stream_verbatim() {
    let (_daemon, path) = FakeDaemon::bind("resync-cursor");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    session.resync();
    let cursor = session.emulator.terminal().cursor();
    assert_eq!(
        (cursor.col, cursor.row),
        (8, 1),
        "the shadow cursor must sit exactly where the restore stream's final CUP put it \
         (right after `prompt> `), not one row below"
    );
}

/// Every pane-show re-fits the target window BEFORE the replay: the
/// handshake's target-less -C sizes only the newest session's active
/// window, so a window restored at another size never resized and its
/// child ran at the stale height (the manual-pass htop report; render
/// mode's switch path has always reported). The report precedes the
/// replay — the replay must encode the post-resize screen. Unix-only:
/// the assert pins `terminal_grid`'s non-tty (80, 24) fallback (an
/// interactive Windows console session reports its real grid —
/// measured on the Windows VM, 2026-10-04).
#[cfg(unix)]
#[test]
fn resync_size_reports_the_target_window_before_the_replay() {
    let (_daemon, path) = FakeDaemon::bind("resync-size");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    session.resync();
    // The connect-time handshake sends its own target-less size report
    // (refresh-client -C WxH -p); the resync's targeted report is the
    // first refresh-client naming the pane.
    let report = loop {
        match _daemon
            .received
            .recv_timeout(std::time::Duration::from_secs(5))
        {
            Ok((name, line)) if name == "refresh-client" && line.contains("-t %0") => {
                break line;
            }
            Ok(_) => continue,
            Err(_) => panic!("no targeted size report arrived"),
        }
    };
    assert!(
        report.contains("-C 80x23"),
        "the size report names the pane at the content region: {report}"
    );
    let second = wait_for_line(&_daemon.received, "refresh-client");
    assert!(
        second.contains("-t %0") && !second.contains("-C"),
        "the replay follows without a size: {second}"
    );
}

/// chunk containing a CUP and a line feed; the bytes `draw_status`
/// emits must end with an absolute CUP at the EMULATOR's tracked cell,
/// not a bare ESC8 restore. A pane scroll landing between the draw and
/// the placement cannot make a fresh absolute position wrong — that is
/// the manual-pass race this kills (ESC7..ESC8 alone restored a
/// position one line off after a scroll).
#[test]
fn status_draw_places_the_cursor_at_the_tracked_cell() {
    let (_daemon, path) = FakeDaemon::bind("tracked-cursor");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };

    // Replay that parks the cursor (CUP row 3 col 8), then a %output
    // chunk that homes, writes "line", and line feeds: the emulator
    // must end at col 4, row 1 (LF moves down, column preserved).
    session.emulator.feed(b"\x1b[3;8H");
    session.emulator.feed(b"\x1b[1;1Hline\n");
    let cursor = session.emulator.terminal().cursor();
    assert_eq!(
        (cursor.col, cursor.row),
        (4, 1),
        "the emulator's tracked cursor must follow the scripted stream"
    );

    // The draw's emission: the placement CUP is the LAST sequence in
    // the draw and names the tracked cell.
    let bytes = session.status_draw_bytes(24, 80);
    let expected = b"\x1b[2;5H"; // row+1 ; col+1, 1-indexed
    let pos = bytes
        .windows(expected.len())
        .rposition(|w| w == expected)
        .unwrap_or_else(|| {
            panic!(
                "the draw must end with an absolute CUP at the tracked cell \
                 (row 4, col 8): {:?}",
                String::from_utf8_lossy(&bytes)
            )
        });
    // It is the draw's LAST sequence: everything after it is nothing.
    assert_eq!(
        pos + expected.len(),
        bytes.len(),
        "the tracked-cell CUP must close the draw: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    // The wrap survives (protects the draw against interleaved output
    // within itself) but the restore no longer closes it.
    assert!(
        bytes.windows(2).any(|w| w == b"\x1b7") && bytes.windows(2).any(|w| w == b"\x1b8"),
        "the ESC7/ESC8 wrap stays inside the draw: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    // The DECSTBM reserve semantics are unchanged.
    assert!(
        bytes.windows(7).any(|w| w == b"\x1b[1;23r"),
        "the draw still reserves the content region (rows 1..23): {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

/// Card 01a11d62 (passthrough "drops" the rich demo's paragraphs): the
/// pane's PTY is the host grid minus the status row, so a pane scrolling
/// at its bottom keeps its cursor on content row 23 (1-based). The shadow
/// emulator must share that geometry: a full-grid shadow tracked the
/// cursor one row lower — on the status row, OUTSIDE the `1;23` scroll
/// region — and every draw parked the host cursor there, where line feeds
/// never scroll, so all later output overwrote one line and never reached
/// the host's scrollback.
#[test]
fn status_draw_keeps_the_cursor_inside_the_content_region_after_a_scroll() {
    let (_daemon, path) = FakeDaemon::bind("tracked-scroll");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    // First draw fits the shadow to the host grid's pane geometry.
    let _ = session.status_draw_bytes(24, 80);
    // The pane scrolls: far more line feeds than it has rows.
    session.emulator.feed(&b"line\r\n".repeat(60));
    let bytes = session.status_draw_bytes(24, 80);
    assert!(
        bytes.ends_with(b"\x1b[23;1H"),
        "a scrolled pane's cursor sits on its last row — content row 23, \
         inside the 1;23 scroll region, never the status row 24: {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

/// Dead pane: the tracked cell is the frozen screen's cell — feeding
/// the emulator the frozen stream is what `resync` does, so the same
/// shape holds with `exited` set; the placement does not move to a
/// default position when the pane dies.
#[test]
fn status_draw_tracks_the_frozen_cell_when_the_pane_is_dead() {
    let (_daemon, path) = FakeDaemon::bind("dead-tracked");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: Some(Some(0)),
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    session.emulator.feed(b"\x1b[5;3H");
    let bytes = session.status_draw_bytes(24, 80);
    let expected = b"\x1b[5;3H";
    let pos = bytes
        .windows(expected.len())
        .rposition(|w| w == expected)
        .unwrap_or_else(|| {
            panic!(
                "the dead pane's placement CUP names the frozen cell: {:?}",
                String::from_utf8_lossy(&bytes)
            )
        });
    assert_eq!(
        pos + expected.len(),
        bytes.len(),
        "the frozen-cell CUP closes the draw: {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

/// Prefix r on a held-dead pane respawns it daemon-side and resumes
/// forwarding; the plain bytes typed afterwards go to the pane.
#[test]
fn respawn_chord_resumes_forwarding_after_revival() {
    let (daemon, path) = FakeDaemon::bind("respawn");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: Some(Some(0)),
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    // The fake daemon answers every unknown command with an ok empty
    // block, so respawn-pane succeeds and clears the dead flag.
    assert!(
        !session.route_bytes(&[0x02, b'r']),
        "prefix r never detaches"
    );
    assert!(session.exited.is_none(), "a successful respawn clears it");
    // Forwarding resumed: the next plain byte is on the wire.
    assert!(!session.route_bytes(b"x"));
    // Handshake lines first (version, list-commands, set-client-colors,
    // refresh-client -C), then the chord and respawn's resync
    // (refresh-client -t) — the wire evidence forwarding is live again.
    let lines = recorded(&daemon.received, 6);
    assert_eq!(lines[4].0, "respawn-pane", "the chord respawned: {lines:?}");
    assert_eq!(
        lines[5].0, "refresh-client",
        "the respawn resync rode the revived connection: {lines:?}"
    );
}

/// The scripted-reply daemon the management-chord tests share: like
/// [`serve_one`] but with a table covering split-window (reply = new
/// pane id), list-panes under both windows, select-pane,
/// new-window (reply = window id), and select-window.
fn serve_management(
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
        let reply = match trimmed {
            "version" => "9.9.9+deadbeef".to_string(),
            "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
            "list-panes" => "%0\n".to_string(),
            // The focused pane %0 splits: the new pane %1 arrives.
            "split-window -t %0 -h" => "%1\n".to_string(),
            "split-window -t %0" => "%1\n".to_string(),
            // After the split lands the pump on %1, the window @0
            // holds both panes with %1 active (the daemon focuses the
            // fresh split).
            "list-panes -t @0" => "%0 0 -\n%1 1 *".to_string(),
            "select-pane -t %1" => String::new(),
            "select-pane -t %0" => String::new(),
            // new-window in session $0: the fresh @1.
            "new-window -t $0" => "@1\n".to_string(),
            "select-window -t @1" => String::new(),
            "list-windows -t $0" => "@0 0 -\n@1 1 *".to_string(),
            // The new window's active pane: %1 (a distinct id keeps
            // the landing assertion honest — %0 is @0's pane).
            "list-panes -t @1" => "%1 0 *".to_string(),
            _ => String::new(),
        };
        number += 1;
        tx.send((name, trimmed.to_owned())).ok();
        writer
            .write_all(emit_block(number, &reply, true).as_bytes())
            .ok();
        writer.flush().ok();
    }
}

/// Bind the management fake and build a Session over it, focused on
/// %0 in window @0 of session $0.
fn management_session(tag: &str) -> (std::sync::mpsc::Receiver<(String, String)>, Session) {
    let path = test_socket(tag);
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let (tx, rx) = channel();
    let sender = tx.clone();
    std::thread::spawn(move || {
        if let Ok(stream) = listener.accept() {
            serve_management(stream, sender);
        }
    });
    let conn = conn::AttachConn::connect(&path).expect("connect");
    drop(tx);
    let mut session = Session {
        conn,
        socket_path: path,
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: "@0".to_string(),
        session_id: Some("$0".to_string()),
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    // A status draw writes to the process stdout; in tests that is
    // the captured harness. Idle both flags so route_bytes's tail
    // has nothing to repaint and the wire sees only the chord.
    session.drawn_size = Some((80, 24));
    (rx, session)
}

/// prefix %: `split-window -t %0 -h` rides the wire, and the pump
/// lands on the new pane — select-pane %1 plus the switch contract's
/// refresh-client resync, with the fresh pane now the Session's
/// target (subsequent typing forwards to it).
#[test]
fn split_right_chord_sends_split_window_and_lands_on_the_new_pane() {
    let (rx, mut session) = management_session("split-h");
    assert!(!session.route_bytes(&[0x02, b'%']));
    let lines = recorded(&rx, 6);
    let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "version",
            "list-commands",
            "set-client-colors",
            "refresh-client",
            "split-window",
            "select-pane",
        ],
        "the chord then the landing: {lines:?}"
    );
    assert_eq!(
        lines[4].1, "split-window -t %0 -h",
        "the split targets the FOCUSED pane with -h: {lines:?}"
    );
    assert_eq!(
        lines[5].1, "select-pane -t %1",
        "the pump followed the new pane from the reply: {lines:?}"
    );
    // Forwarding follows the landing: the switch contract's resync
    // burst (refresh-client, the status re-queries) rides first; the
    // next plain byte lands on the fresh pane %1.
    assert!(!session.route_bytes(b"q"));
    let keys = wait_for_line(&rx, "send-keys");
    assert_eq!(
        keys, "send-keys -t %1 -H 71",
        "typing forwards to the landed pane"
    );
}

/// prefix ": the vertical spelling (`split-window -t %0` with no -h)
/// and the same land-on-the-new-pane contract.
#[test]
fn split_down_chord_sends_the_vertical_split_spelling() {
    let (rx, mut session) = management_session("split-v");
    assert!(!session.route_bytes(&[0x02, b'"']));
    let lines = recorded(&rx, 6);
    assert_eq!(
        lines[4].1, "split-window -t %0",
        "the default direction rides bare (below): {lines:?}"
    );
    assert_eq!(lines[5].1, "select-pane -t %1", "lands on the new pane");
}

/// prefix x: `kill-pane -t %0` rides the wire and the pump follows
/// the window's surviving active pane.
#[test]
fn kill_chord_sends_kill_pane_and_lands_on_the_survivor() {
    let (rx, mut session) = management_session("kill");
    assert!(!session.route_bytes(&[0x02, b'x']));
    let lines = recorded(&rx, 7);
    let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "version",
            "list-commands",
            "set-client-colors",
            "refresh-client",
            "kill-pane",
            "list-panes",
            "select-pane",
        ],
        "kill, survivor lookup, landing: {lines:?}"
    );
    assert_eq!(lines[4].1, "kill-pane -t %0");
    assert_eq!(
        lines[5].1, "list-panes -t @0",
        "the survivor query targets the focused pane's window: {lines:?}"
    );
    assert_eq!(
        lines[6].1, "select-pane -t %1",
        "the *-marked survivor wins the landing: {lines:?}"
    );
}

/// prefix x on the window's LAST pane: no survivor — the dead guard
/// engages (the exited cue shows; typing is dropped, prefix chords
/// keep routing) instead of the pump forwarding into a corpse.
#[test]
fn kill_of_the_last_pane_engages_the_dead_guard() {
    let path = test_socket("kill-last");
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let (tx, rx) = channel();
    let sender = tx.clone();
    std::thread::spawn(move || {
        if let Ok(stream) = listener.accept() {
            use interprocess::TryClone as _;
            let mut writer = stream.try_clone().expect("clone stream");
            let mut reader = BufReader::new(stream);
            let mut number = 0u32;
            let mut line = String::new();
            let tx = sender;
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
                let reply = match name.as_str() {
                    "version" => "9.9.9+deadbeef".to_string(),
                    "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
                    "kill-pane -t %0" => String::new(),
                    _ => String::new(),
                };
                // The survivor query (list-panes -t @0) is the one
                // command this script errors on: an %error block, no
                // body, so `marked_pane` fails and the guard engages.
                let ok = name != "list-panes";
                if !ok {
                    number += 1;
                    tx.send((name.clone(), trimmed.to_owned())).ok();
                    writer
                        .write_all(emit_block(number, "can't identify a pane", false).as_bytes())
                        .ok();
                    writer.flush().ok();
                    continue;
                }
                number += 1;
                tx.send((name, trimmed.to_owned())).ok();
                writer
                    .write_all(emit_block(number, &reply, true).as_bytes())
                    .ok();
                writer.flush().ok();
            }
        }
    });
    let conn = conn::AttachConn::connect(&path).expect("connect");
    drop(tx);
    let mut session = Session {
        conn,
        socket_path: path,
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: "@0".to_string(),
        session_id: Some("$0".to_string()),
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    session.drawn_size = Some((80, 24));
    assert!(!session.route_bytes(&[0x02, b'x']));
    assert!(
        session.exited.is_some(),
        "no survivor -> the dead guard engages"
    );
    // The dead guard drops typing but keeps prefix chords live.
    assert!(!session.route_bytes(b"typing"), "no detach, bytes dropped");
    assert!(
        session.route_bytes(&[0x02, b'd']),
        "prefix d still detaches"
    );
    drop(rx);
}

/// prefix c: `new-window -t $0` rides the wire, the fresh window is
/// selected, and the pump attaches to its active pane — the same
/// follow the window-switch chords make.
#[test]
fn new_window_chord_follows_the_new_windows_active_pane() {
    let (rx, mut session) = management_session("newwin");
    assert!(!session.route_bytes(&[0x02, b'c']));
    let lines = recorded(&rx, 7);
    let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "version",
            "list-commands",
            "set-client-colors",
            "refresh-client",
            "new-window",
            "select-window",
            "list-panes",
        ],
        "new-window, select, active-pane lookup: {lines:?}"
    );
    assert_eq!(lines[4].1, "new-window -t $0");
    assert_eq!(
        lines[5].1, "select-window -t @1",
        "the fresh window: {lines:?}"
    );
    // The active-pane lookup ran against the NEW window (its marked
    // pane %1 from the scripted table), and the Session's window
    // tracks it — the next switch_window list-windows query targets
    // @1 through session_id.
    assert_eq!(lines[6].1, "list-panes -t @1");
    assert_eq!(session.window, "@1");
    // And the pump landed on the new window's active pane.
    assert_eq!(session.pane, "%1");
}

/// Config: a chord override parses and routes — `%` remapped to `s`
/// splits, and the default `%` no longer intercepts (it forwards as
/// an ordinary byte through the dead-conn fallback).
#[test]
fn chord_override_remaps_the_split_chord() {
    let file: crate::mux::config::ConfigFile = toml::from_str(
        "[client]\nsplit-right = \"s\"\nsplit-down = \"v\"\nkill-pane = \"K\"\nnew-window = \"w\"\n",
    )
    .expect("parse");
    let (rx, mut session) = management_session("remap");
    let chords = crate::mux::config::reload_client_chords(
        &file,
        &crate::mux::config::Chords {
            prefix: session.prefix,
            reload: session.reload_key,
            management: session.management,
            ..crate::mux::config::Chords::with_defaults()
        },
    )
    .expect("chords parse");
    session.management = chords.management;
    assert!(
        !session.route_bytes(&[0x02, b's']),
        "the REMAPPED key splits"
    );
    let split = wait_for_line(&rx, "split-window");
    assert_eq!(
        split, "split-window -t %0 -h",
        "the remap targets %0 with -h"
    );
    // The landing burst ends with refresh_status's roster query;
    // after it drains, the wire is quiet.
    let _ = wait_for_line(&rx, "list-agents");
    // The OLD default key `%` no longer intercepts — and being
    // unbound now, the fixed table consumes it silently (the
    // unknown-chord rule: ignored), so nothing rides the wire.
    assert!(!session.route_bytes(&[0x02, b'%']));
    match rx.recv_timeout(std::time::Duration::from_millis(300)) {
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        other => panic!("the old key must be consumed, not acted on: {other:?}"),
    }
}

/// prefix R (the resize chord) enters the sticky resize mode; each
/// arrow sends the wire's relative resize-pane for the FOCUSED pane
/// at the configured step; q exits and normal forwarding resumes.
#[test]
fn resize_chord_arrows_send_resize_pane_and_q_exits() {
    let (rx, mut session) = management_session("resize-chord");
    assert!(
        !session.route_bytes(&[0x02, b'R']),
        "the resize chord enters the mode"
    );
    assert!(session.resize_mode, "the mode is sticky");
    // Right arrow: the wire's relative form at the default step 1.
    assert!(!session.route_bytes(b"\x1b[C"));
    assert_eq!(
        wait_for_line(&rx, "resize-pane"),
        "resize-pane -t %0 -R 1",
        "the right arrow resizes the focused pane's right edge"
    );
    // Up arrow: -U at the same step.
    assert!(!session.route_bytes(b"\x1b[A"));
    assert_eq!(
        wait_for_line(&rx, "resize-pane"),
        "resize-pane -t %0 -U 1",
        "the up arrow is -U"
    );
    // q exits; typing forwards again afterward.
    assert!(!session.route_bytes(b"q"));
    assert!(!session.resize_mode);
    assert!(!session.route_bytes(b"z"));
    assert_eq!(
        wait_for_line(&rx, "send-keys"),
        "send-keys -t %0 -H 7a",
        "typing forwards after the mode exits"
    );
}

/// In resize mode, a non-arrow key exits the mode and is REPROCESSED
/// by the normal router (the cancelling key still does its job — as
/// plain typing, since chords always need the prefix).
#[test]
fn resize_mode_cancelling_key_reprocesses_normally() {
    let (rx, mut session) = management_session("resize-cancel");
    assert!(!session.route_bytes(&[0x02, b'R']));
    // 'x' exits and reprocesses: a plain byte forwards to the pane.
    assert!(!session.route_bytes(b"x"));
    assert!(!session.resize_mode);
    assert_eq!(
        wait_for_line(&rx, "send-keys"),
        "send-keys -t %0 -H 78",
        "the cancelling key routed through the normal router"
    );
}

/// Config overrides: a resize-step of 3 rides each arrow, and a
/// remapped resize chord (Z) enters the mode while the old default
/// key is consumed unbound.
#[test]
fn resize_chord_and_step_follow_the_config() {
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nresize = \"Z\"\nresize-step = 3\n").expect("parse");
    let (rx, mut session) = management_session("resize-cfg");
    let chords = crate::mux::config::reload_client_chords(
        &file,
        &crate::mux::config::Chords {
            prefix: session.prefix,
            reload: session.reload_key,
            management: session.management,
            ..crate::mux::config::Chords::with_defaults()
        },
    )
    .expect("chords parse");
    session.management = chords.management;
    session.resize_step = chords.resize_step;
    assert!(!session.route_bytes(&[0x02, b'Z']));
    assert!(session.resize_mode, "the REMAPPED key enters the mode");
    assert!(!session.route_bytes(b"\x1b[C"));
    assert_eq!(
        wait_for_line(&rx, "resize-pane"),
        "resize-pane -t %0 -R 3",
        "the configured step rides the arrow"
    );
    // The old default key R is consumed unbound (nothing rides the
    // wire); drain the burst first, then require quiet.
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}
    assert!(!session.route_bytes(&[0x02, b'R']));
    match rx.recv_timeout(std::time::Duration::from_millis(300)) {
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        other => panic!("the old key must be consumed, not acted on: {other:?}"),
    }
}

/// prefix {: swap with the layout-order neighbor — `swap-pane -s
/// <focused> -t <prev>` — from the same list-panes roster cycle_pane
/// walks.
#[test]
fn swap_prev_chord_sends_swap_pane() {
    let (rx, mut session) = management_session("swap-prev");
    assert!(!session.route_bytes(&[0x02, b'{']));
    let line = wait_for_line(&rx, "swap-pane");
    assert_eq!(
        line, "swap-pane -s %0 -t %1",
        "focused %0 swaps with its next-roster neighbor (wraps to %1)"
    );
}

/// prefix ?: the bindings panel prints as plain text with the LIVE
/// chords — a remapped split chord shows its remapped key, and the
/// category headers are present.
#[test]
fn help_rows_carry_effective_bindings_and_categories() {
    use crate::mux::config::Management;
    let rows = super::help_rows(
        0x02,
        0x12,
        Management {
            split_right: b's',
            ..Management::default()
        },
        2,
    );
    let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    let joined = text.join("\n");
    for category in [
        "global",
        "panes",
        "tabs / windows / sessions",
        "navigation",
        "mouse",
    ] {
        assert!(
            text.iter().any(|t| t.contains(category)),
            "the category header {category} is present: {joined}"
        );
    }
    // Effective bindings: the remapped split shows s, not %.
    assert!(
        text.iter()
            .any(|t| t.starts_with(" C-b s") && t.contains("split right")),
        "the remapped chord shows its remapped key: {joined}"
    );
    assert!(
        !text.iter().any(|t| t.contains("C-b %")),
        "the old default spelling is gone: {joined}"
    );
    // The resize step surfaces in the resize row.
    assert!(
        text.iter().any(|t| t.contains("edge moves by 2")),
        "the step reflects the live config: {joined}"
    );
}

/// The passthrough dump leads with a blank line (the first header used
/// to print on the cursor's current line — the prompt's) and bolds the
/// category headers; plain rows stay unstyled.
#[test]
fn passthrough_help_dump_leads_with_a_blank_line_and_bolds_headers() {
    let rows = super::help_rows(0x02, 0x12, Default::default(), 1);
    let dump = super::help_dump_text(&rows);
    assert!(
        dump.starts_with("\r\n"),
        "the dump must start on a fresh line: {dump:?}"
    );
    let accent_count = rows.iter().filter(|r| r.accent).count();
    assert_eq!(
        dump.matches("\x1b[1m").count(),
        accent_count,
        "every category header is bold, nothing else: {dump:?}"
    );
    assert!(
        dump.contains("\x1b[1m global \x1b[0m\r\n"),
        "the global header is bold: {dump:?}"
    );
    // The first entry row is plain: no SGR anywhere outside headers.
    assert!(
        dump.contains("\r\n C-b C-b  type a literal prefix\r\n"),
        "entry rows stay plain: {dump:?}"
    );
}

/// The passthrough help chord advances the pane past the dump — empty
/// Enters (one command line + fresh prompt each, ~2 rows) land the
/// shell below the text so later keystrokes do not paint over it.
#[test]
fn help_chord_advances_the_pane_past_the_dump() {
    let (_daemon, path) = FakeDaemon::bind("help-advance");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    let expected = super::help_rows(0x02, 0x12, Default::default(), 1).len() / 2 + 1;
    session.show_help();
    let line = wait_for_line(&_daemon.received, "send-keys");
    let sent = line.matches("0d").count();
    assert_eq!(
        sent, expected,
        "one Enter per ~2 dump rows advances the pane past the help: {line}"
    );
}

/// The sidebar compose after round 6: NO section header rows (the
/// title moved to the tab strip's lead segment), workspace rows
/// start at composed row 0, and the footer row pins ` new ` to the
/// panel's bottom-left and ` menu ` to its bottom-right — each chip
/// its own clickable span and id.
#[test]
fn compose_sidebar_lists_rows_and_pins_the_footer_chips() {
    let sections = vec![super::SidebarSection {
        rows: vec![
            ("+0".to_string(), "main".to_string(), true),
            ("+1".to_string(), "dev".to_string(), false),
            (
                "+2".to_string(),
                "a very long workspace name".to_string(),
                false,
            ),
        ],
    }];
    let lines = super::compose_sidebar(&sections, 20, 12);
    // Row 0 is the first workspace — no header row above it.
    assert_eq!(lines[0].text, " ▸ main");
    assert_eq!(lines[0].id.as_deref(), Some("+0"));
    assert!(lines[0].active, "the active row is flagged");
    assert_eq!(lines[1].text, "   dev");
    assert_eq!(lines[1].id.as_deref(), Some("+1"));
    assert_eq!(lines[2].text, "   a very long work", "clipped to width - 1");
    // The footer chips pin to the panel's LAST row (footer_y = 11 of
    // 12 rows), never at a body row's position.
    let new_chip = lines
        .iter()
        .find(|l| l.id.as_deref() == Some(super::SIDEBAR_NEW_ID))
        .expect("the new chip composes");
    assert_eq!(new_chip.y, 11);
    assert_eq!(new_chip.text, " new ");
    assert_eq!((new_chip.x, new_chip.x_end), (0, 5));
    let menu_chip = lines
        .iter()
        .find(|l| l.id.as_deref() == Some(super::SIDEBAR_MENU_ID))
        .expect("the menu chip composes");
    assert_eq!(menu_chip.text, " menu ");
    assert_eq!(
        (menu_chip.x, menu_chip.x_end),
        (13, 19),
        "bottom-right of the 19-col content"
    );
    // A strip too narrow for both chips drops the one that does not
    // fit; a strip too narrow for either drops both.
    let narrow = super::compose_sidebar(&sections, 7, 4);
    assert!(
        narrow
            .iter()
            .any(|l| l.id.as_deref() == Some(super::SIDEBAR_NEW_ID)),
        "the new chip fits a 6-col content: {narrow:?}"
    );
    assert!(
        !narrow
            .iter()
            .any(|l| l.id.as_deref() == Some(super::SIDEBAR_MENU_ID)),
        "the menu chip does not fit: {narrow:?}"
    );
    let tiny = super::compose_sidebar(&sections, 4, 3);
    assert!(
        !tiny
            .iter()
            .any(|l| l.id.as_deref().is_some_and(|id| id.starts_with("panel:"))),
        "no chips fit a 3-col content: {tiny:?}"
    );
}

/// The new-workspace prompt's default bumps past conflicts: one past
/// the highest workspace ordinal, and past any workspace NAME that
/// already claims the number (the same rule next_window_name runs).
#[test]
fn next_workspace_name_bumps_past_conflicts() {
    assert_eq!(super::next_workspace_name(&[]), "1");
    let roster = vec![
        ("+1".to_string(), "1".to_string()),
        ("+2".to_string(), "3".to_string()),
    ];
    assert_eq!(
        super::next_workspace_name(&roster),
        "4",
        "one past the max ordinal, bumped past the claimed name '3'"
    );
    let claimed = vec![("+1".to_string(), "2".to_string())];
    assert_eq!(
        super::next_workspace_name(&claimed),
        "3",
        "a claimed default bumps again"
    );
}

/// The help panel compose: the filter line is ALWAYS present (the
/// placeholder when inactive — the round-3 no-feedback fix), the
/// filter narrows rows live (headers hide when nothing beneath them
/// matches), the window scrolls, and the footer line names the
/// controls. The border ring/title/badge are the renderer's
/// paint_overlay job, not the compose's.
#[test]
fn compose_help_panel_filters_scrolls_and_chromes() {
    let rows = super::help_rows(0x02, 0x12, Default::default(), 1);
    // Full panel: content + footer — no idle placeholder (the footer
    // already advertises `search /`), and entering filter mode swaps
    // in the cursor-input line immediately — `/` must give visible
    // feedback before any typing.
    let panel = super::compose_help_panel(&rows, "", false, 100, 0);
    let text: Vec<&str> = panel.iter().map(|r| r.text.as_str()).collect();
    assert!(
        !text.iter().any(|t| t.contains("press / to filter")),
        "no idle placeholder row: {text:?}"
    );
    assert_eq!(
        text[0], " global ",
        "the content leads the idle panel: {}",
        text[0]
    );
    let active = super::compose_help_panel(&rows, "", true, 100, 0);
    assert_eq!(
        active[0].text, " /▌",
        "the active filter line shows the input cursor: {}",
        active[0].text
    );
    assert!(
        text.last()
            .copied()
            .unwrap_or("")
            .contains("close esc/enter"),
        "the footer names the controls"
    );
    // Filter: "swap" keeps only the swap rows (and their header).
    let filtered = super::compose_help_panel(&rows, "swap", false, 100, 0);
    let ftext: String = filtered
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(ftext.contains("swap"), "the matching rows survive: {ftext}");
    assert!(
        ftext.contains("panes"),
        "the matching rows' header survives"
    );
    assert!(!ftext.contains("detach"), "non-matching rows drop: {ftext}");
    assert!(
        !ftext.contains(" global "),
        "a category with no matching rows hides: {ftext}"
    );
    // Filter box drawn while active.
    assert!(
        ftext.contains("/swap▌"),
        "the filter box shows the live filter: {ftext}"
    );
    // Scroll: a tiny window shows a slice, and scrolling past the end
    // clamps.
    let small = super::compose_help_panel(&rows, "", false, 3, 0);
    let big = super::compose_help_panel(&rows, "", false, 3, 10_000);
    assert_ne!(small[1..4], big[1..4], "scrolling moves the window");
    assert_eq!(
        super::compose_help_panel(&rows, "", false, 3, 10_000).len(),
        small.len(),
        "the panel shape is stable under scroll"
    );
}

/// The picker's rows: one accent header per session with its windows
/// nested beneath, the current session and the shown window
/// `>`-marked, the session's active window `*`-suffixed, and the
/// parallel refs mapping each row to its selection target.
#[test]
fn picker_rows_nest_windows_and_mark_current() {
    let entries = vec![
        super::PickerEntry {
            session_id: "$0".to_string(),
            session_name: "work".to_string(),
            windows: vec![
                ("@0".to_string(), "main".to_string()),
                ("@1".to_string(), "vim".to_string()),
            ],
            active_window: Some("@1".to_string()),
            current: true,
        },
        super::PickerEntry {
            session_id: "$1".to_string(),
            session_name: "build".to_string(),
            windows: vec![("@2".to_string(), "logs".to_string())],
            active_window: Some("@2".to_string()),
            current: false,
        },
    ];
    let (rows, refs) = super::picker_rows(&entries, Some("@0"));
    let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text.len(), 5, "two headers + three windows: {text:?}");
    assert_eq!(
        text[0], " >$0: work",
        "the current session's header is >-marked"
    );
    assert!(rows[0].accent, "session headers are accent rows");
    assert_eq!(text[1], " >  @0: main", "the shown window is >-marked");
    assert_eq!(text[2], "    @1: vim *", "the active window carries *");
    assert_eq!(text[3], "  $1: build", "a non-current header is unmarked");
    assert!(rows[3].accent, "every session header is an accent row");
    assert!(!rows[1].accent, "window rows are not accent rows");
    assert_eq!(refs[0], super::PickerRef::Session(0));
    assert_eq!(refs[1], super::PickerRef::Window(0, 0));
    assert_eq!(refs[2], super::PickerRef::Window(0, 1));
    assert_eq!(refs[3], super::PickerRef::Session(1));
    assert_eq!(refs[4], super::PickerRef::Window(1, 0));
}

/// The picker compose: the filter line, the filtered content with
/// headers hiding when nothing beneath them matches, the `▸` cursor
/// on the selected row, the footer, and the refs/start return that
/// maps a click at composed row `i` to content row `start + i - 1`.
#[test]
fn compose_picker_panel_filters_marks_and_maps_clicks() {
    let entries = vec![
        super::PickerEntry {
            session_id: "$0".to_string(),
            session_name: "work".to_string(),
            windows: vec![("@0".to_string(), "main".to_string())],
            active_window: Some("@0".to_string()),
            current: true,
        },
        super::PickerEntry {
            session_id: "$1".to_string(),
            session_name: "build".to_string(),
            windows: vec![("@1".to_string(), "vim".to_string())],
            active_window: Some("@1".to_string()),
            current: false,
        },
    ];
    let (rows, refs) = super::picker_rows(&entries, Some("@0"));
    // Full panel: 4 content rows + footer — no idle placeholder (the
    // footer already advertises `filter /`).
    let (panel, refs2, start) = super::compose_picker_panel(&rows, &refs, "", false, 0, 100, 0);
    assert_eq!(panel.len(), 5, "content + footer: {panel:?}");
    assert_eq!(start, 0);
    assert_eq!(refs2.len(), 4);
    assert_eq!(
        panel.last().map(|r| r.text.as_str()),
        Some(super::PICKER_FOOTER)
    );
    // Filter "vim" keeps only that window row and its header; the
    // other session's header hides with no matching children.
    let (filtered, frefs, fstart) =
        super::compose_picker_panel(&rows, &refs, "vim", false, 0, 100, 0);
    let ftext = filtered
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(ftext.contains("vim"), "the matching row survives: {ftext}");
    assert!(
        ftext.contains("build"),
        "the matching row's header survives"
    );
    assert!(
        !ftext.contains("work"),
        "an emptied session's header hides: {ftext}"
    );
    assert_eq!(
        frefs,
        vec![super::PickerRef::Session(1), super::PickerRef::Window(1, 0)]
    );
    assert_eq!(fstart, 0);
    // The selection cursor: selected=2 marks the third content row.
    let (marked, _, _) = super::compose_picker_panel(&rows, &refs, "", false, 2, 100, 0);
    assert!(
        marked[2].text.starts_with('▸'),
        "the selected row carries the cursor glyph: {}",
        marked[2].text
    );
    assert!(
        !marked[1].text.starts_with('▸'),
        "only the selected row carries it"
    );
    // Click mapping: composed row 1 (content row 1) maps to content
    // index start + 1.
    let (small, _, small_start) = super::compose_picker_panel(&rows, &refs, "", false, 0, 2, 0);
    assert_eq!(small.len(), 3, "2 visible + footer");
    assert_eq!(small_start, 0);
    // Panning: a selected row below the window moves `start`.
    let (_panned, _, panned_start) = super::compose_picker_panel(&rows, &refs, "", false, 3, 2, 0);
    assert_eq!(
        panned_start, 2,
        "the window pans to keep the cursor visible"
    );
}

/// The listbox panning rule: `start` moves only when `selected`
/// leaves the visible window.
#[test]
fn listbox_scroll_pans_only_at_the_edges() {
    assert_eq!(super::listbox_scroll(0, 0, 3), 0);
    assert_eq!(super::listbox_scroll(0, 2, 3), 0, "inside the window");
    assert_eq!(super::listbox_scroll(0, 3, 3), 1, "one past the end");
    assert_eq!(super::listbox_scroll(1, 0, 3), 0, "above the window");
    assert_eq!(super::listbox_scroll(0, 0, 0), 0, "an empty window is safe");
}

/// A management chord spelling that does not parse is a reload error
/// (the flash), not a silent keep-the-old.
#[test]
fn management_chord_override_errors_on_malformed_spelling() {
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nsplit-right = \"C-\"\n").expect("parse");
    assert!(
        crate::mux::config::reload_client_chords(
            &file,
            &crate::mux::config::Chords {
                prefix: 0x02,
                reload: 0x12,
                management: crate::mux::config::Management::default(),
                resize_step: 1,
                ..crate::mux::config::Chords::with_defaults()
            },
        )
        .is_err(),
        "a malformed chord is an error"
    );
}

/// A Session for routing tests only: its connection is a closed
/// channel pair, so sends fail silently (routing logic does not care).
fn test_dead_conn() -> conn::AttachConn {
    let path = test_socket("deadconn");
    let _ = std::fs::remove_file(&path);
    // connect() fails without a listener, which is exactly the "dead"
    // connection a routing test wants — but Session needs a value.
    // Build one over a listener the test immediately drops: every
    // send fails, which is the behavior route_bytes tolerates.
    match conn::AttachConn::connect(&path) {
        Ok(conn) => conn,
        Err(_) => {
            // Fall back: bind-accept-drop. One bound listener that
            // answers nothing, then closes.
            let listener = crate::mux::bind_local_listener(&path).expect("bind");
            std::thread::spawn(move || {
                // The Listener trait supplies `.accept()` on unix only;
                // the Windows named-pipe listener accepts inherently.
                #[cfg(unix)]
                use interprocess::local_socket::traits::Listener as _;
                let _ = listener.accept();
            });
            conn::AttachConn::connect(&path).expect("connect to the accepting listener")
        }
    }
}

/// The reload chord: a written config with a new prefix rebinds the
/// running session's chords LIVE — the prefix typed after the reload
/// detaches under the NEW prefix, the old one forwards — and the
/// `reload-config` control command rides the same chord to the
/// daemon.
#[test]
fn reload_chord_rebinds_prefix_and_sends_reload_config() {
    let (_daemon, path) = FakeDaemon::bind("reload");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    // A config naming a new prefix (C-a) and a moved reload chord
    // (C-a C-s): the rebind is pure over the parsed file, so the test
    // feeds it directly (the env-pin path is covered end to end by
    // the PTY-level attach tests, where the child's env is injectable
    // — QA-196 forbids env mutation in this suite).
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nprefix = \"C-a\"\nreload = \"C-a C-s\"\n").unwrap();
    let new_chords = crate::mux::config::reload_client_chords(
        &file,
        &crate::mux::config::Chords {
            prefix: session.prefix,
            reload: session.reload_key,
            management: session.management,
            ..crate::mux::config::Chords::with_defaults()
        },
    )
    .expect("chords parse");
    session.prefix = new_chords.prefix;
    session.reload_key = new_chords.reload;
    session.management = new_chords.management;
    // The reloaded session routes: prefix d (0x01 'd') detaches under
    // the NEW prefix; the old prefix byte no longer intercepts.
    assert!(
        session.route_bytes(&[0x01, b'd']),
        "the NEW prefix detaches"
    );
    assert!(
        !session.route_bytes(&[0x02, b'x']),
        "the OLD prefix no longer intercepts"
    );
}

/// The pure chord rebind errors on a malformed chord (the error is
/// what the status flash shows) instead of silently keeping the old
/// chords.
#[test]
fn reload_client_chords_errors_on_malformed_chords() {
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nprefix = \"C-\"\n").unwrap();
    assert!(
        crate::mux::config::reload_client_chords(
            &file,
            &crate::mux::config::Chords {
                prefix: 0x02,
                reload: 0x12,
                management: crate::mux::config::Management::default(),
                ..crate::mux::config::Chords::with_defaults()
            }
        )
        .is_err(),
        "a malformed prefix is an error"
    );
    // An absent tier keeps the current chords.
    let partial: crate::mux::config::ConfigFile =
        toml::from_str("[daemon]\nsocket = \"work\"\n").unwrap();
    let kept = crate::mux::config::reload_client_chords(
        &partial,
        &crate::mux::config::Chords {
            prefix: 0x02,
            reload: 0x12,
            management: crate::mux::config::Management {
                split_right: b'&',
                split_down: b'*',
                kill_pane: b'X',
                new_window: b'C',
                resize: b'R',
                swap_prev: b'{',
                swap_next: b'}',
                workspace_next: b'N',
                workspace_prev: b'P',
                help: b'?',
                picker: b'w',
                zoom: b'z',
                rename_window: b',',
                rename_pane: b'$',
                border_cycle: b'B',
                label_toggle: b'l',
                workspace_picker: b'g',
                sidebar: b's',
                status_bar: b'S',
            },
            resize_step: 1,
            ..crate::mux::config::Chords::with_defaults()
        },
    )
    .expect("partial file");
    assert_eq!(
        kept,
        crate::mux::config::Chords {
            prefix: 0x02,
            reload: 0x12,
            management: crate::mux::config::Management {
                split_right: b'&',
                split_down: b'*',
                kill_pane: b'X',
                new_window: b'C',
                resize: b'R',
                swap_prev: b'{',
                swap_next: b'}',
                workspace_next: b'N',
                workspace_prev: b'P',
                help: b'?',
                picker: b'w',
                zoom: b'z',
                rename_window: b',',
                rename_pane: b'$',
                border_cycle: b'B',
                label_toggle: b'l',
                workspace_picker: b'g',
                sidebar: b's',
                status_bar: b'S',
            },
            resize_step: 1,
            ..crate::mux::config::Chords::with_defaults()
        }
    );
}

/// A passthrough Session over a fake daemon answering each exact
/// command line from `replies` (everything else an ok empty reply),
/// focused on %0 in window @0 of session $0; the handshake's lines
/// are drained before it returns, so the receiver holds only what
/// the test drives.
fn scripted_passthrough(
    tag: &str,
    replies: &[(&str, &str)],
) -> (std::sync::mpsc::Receiver<String>, Session) {
    let path = test_socket(tag);
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let table: std::collections::HashMap<String, String> = replies
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let Ok(stream) = listener.accept() else {
            return;
        };
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
            let reply = match trimmed {
                "version" => "9.9.9+deadbeef".to_string(),
                "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
                other => table.get(other).cloned().unwrap_or_default(),
            };
            number += 1;
            tx.send(trimmed.to_owned()).ok();
            writer
                .write_all(emit_block(number, &reply, true).as_bytes())
                .ok();
            writer.flush().ok();
        }
    });
    let conn = conn::AttachConn::connect(&path).expect("connect");
    let session = Session {
        conn,
        socket_path: path,
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(0, 80, 24),
        window: "@0".to_string(),
        session_id: Some("$0".to_string()),
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: Some((80, 24)),
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    let _: Vec<String> = rx.try_iter().collect();
    (rx, session)
}

/// The wire lines a passthrough action sent that select or land
/// (the status re-query burst filtered out), in order.
fn switch_lines(rx: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
    rx.try_iter()
        .filter(|l| {
            l.starts_with("select-")
                || l.starts_with("list-panes -t")
                || l.starts_with("list-windows -t")
                || l.starts_with("list-sessions -t")
        })
        .collect()
}

/// Passthrough prefix n / p: the neighbor window (wrapping) is
/// selected and the pump attaches its `*`-marked pane; an unlisted
/// shown window moves nowhere.
#[test]
fn passthrough_window_switch_wraps_and_attaches_the_marked_pane() {
    let (rx, mut session) = scripted_passthrough(
        "pt-win",
        &[
            ("list-sessions", "$0: work"),
            ("list-windows -t $0", "@0 * a\n@1 - b\n@2 - c"),
            ("list-panes -t @2", "%4 0 -\n%5 1 *"),
            ("pane-info -t %5", "%5 @2 80x24"),
        ],
    );
    assert!(!session.route_bytes(&[0x02, b'p']));
    assert_eq!(
        switch_lines(&rx)[..4].to_vec(),
        vec![
            "list-windows -t $0",
            "select-window -t @2",
            "list-panes -t @2",
            "select-pane -t %5"
        ],
        "p from the first window wraps to the last"
    );
    assert_eq!(session.window, "@2");
    assert_eq!(session.pane, "%5", "the pump follows the marked pane");

    assert_eq!(session.session_id.as_deref(), Some("$0"));
    session.window = "@9".to_string();
    assert!(!session.route_bytes(&[0x02, b'n']));
    assert_eq!(switch_lines(&rx), vec!["list-windows -t $0".to_string()]);
    assert_eq!(session.pane, "%5", "an unlisted window moves nowhere");
}

/// Passthrough prefix ( / ): the neighbor session in roster order
/// (wrapping), its `*` window, that window's marked pane; a session
/// absent from the roster (or an empty roster) moves nowhere.
#[test]
fn passthrough_session_switch_lands_on_the_neighbors_active_pane() {
    let (rx, mut session) = scripted_passthrough(
        "pt-sess",
        &[
            ("list-sessions", "$0: work\n$1: lab"),
            ("list-windows -t $1", "@3 - x\n@4 * y"),
            ("list-panes -t @4", "%8 0 *"),
        ],
    );
    assert!(!session.route_bytes(&[0x02, b'(']));
    let lines = switch_lines(&rx);
    assert_eq!(
        lines[..3].to_vec(),
        vec![
            "list-windows -t $1",
            "select-window -t @4",
            "list-panes -t @4"
        ],
        "( from the head wraps to $1: {lines:?}"
    );
    assert_eq!(session.session_id.as_deref(), Some("$1"));
    assert_eq!(session.window, "@4");
    assert_eq!(session.pane, "%8");

    session.session_id = Some("$7".to_string());
    assert!(!session.route_bytes(&[0x02, b')']));
    assert!(switch_lines(&rx).is_empty(), "an unknown session: no move");
    assert_eq!(session.pane, "%8");
}

/// Passthrough prefix o: the next pane of the window (wrapping) by
/// the daemon's list; a single-pane window does nothing.
#[test]
fn passthrough_cycle_moves_to_the_next_listed_pane() {
    let (rx, mut session) =
        scripted_passthrough("pt-cycle", &[("list-panes -t @0", "%0 0 *\n%1 1 -")]);
    assert!(!session.route_bytes(&[0x02, b'o']));
    assert_eq!(
        switch_lines(&rx)[..2].to_vec(),
        vec!["list-panes -t @0", "select-pane -t %1"]
    );
    assert_eq!(session.pane, "%1");
    assert!(!session.route_bytes(&[0x02, b'o']));
    assert_eq!(switch_lines(&rx)[1], "select-pane -t %0", "wraps");

    let (rx, mut session) = scripted_passthrough("pt-cycle1", &[("list-panes -t @0", "%0 0 *")]);
    assert!(!session.route_bytes(&[0x02, b'o']));
    assert_eq!(switch_lines(&rx), vec!["list-panes -t @0".to_string()]);
    assert_eq!(session.pane, "%0");
}

/// Passthrough prefix W: the next workspace is selected and the pump
/// lands in its first session's active window's marked pane; a
/// workspace with no sessions only moves the selection.
#[test]
fn passthrough_workspace_switch_lands_in_the_next_workspace() {
    let (rx, mut session) = scripted_passthrough(
        "pt-ws",
        &[
            ("list-workspaces", "+0: main active\n+1: lab"),
            ("list-sessions -t +1", "+1: lab: $2: build"),
            ("list-sessions", "+1: lab: $2: build"),
            ("list-windows -t $2", "@6 * logs"),
            ("list-panes -t @6", "%9 0 *"),
        ],
    );
    assert!(!session.route_bytes(&[0x02, b'W']));
    assert_eq!(
        switch_lines(&rx)[..5].to_vec(),
        vec![
            "select-workspace -t +1",
            "list-sessions -t +1",
            "list-windows -t $2",
            "select-window -t @6",
            "list-panes -t @6"
        ]
    );
    assert_eq!(session.session_id.as_deref(), Some("$2"));
    assert_eq!(session.window, "@6");
    assert_eq!(session.pane, "%9");

    let (rx, mut session) = scripted_passthrough(
        "pt-ws-empty",
        &[("list-workspaces", "+0: main active\n+1: lab")],
    );
    assert!(!session.route_bytes(&[0x02, 0x17]));
    let lines = switch_lines(&rx);
    assert_eq!(
        lines[..2].to_vec(),
        vec!["select-workspace -t +1", "list-sessions -t +1"],
        "C-w from +0 wraps to +1: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with("select-window")),
        "an empty workspace lands nowhere: {lines:?}"
    );
    assert_eq!(session.pane, "%0");
}

/// SEC-208 (d): passthrough tracks bracketed paste over the raw stream —
/// a paste split across two bursts forwards whole (markers included, the
/// pane's own framing) and its embedded prefix + `x` never kills a pane.
#[test]
fn passthrough_paste_body_skips_the_prefix_scan() {
    let (rx, mut session) = management_session("paste-pt");
    assert!(!session.route_bytes(b"\x1b[200~ab"));
    assert!(!session.route_bytes(b"\x02xcd\x1b[201~"));
    assert!(
        !session.prefix_pending,
        "a pasted prefix byte never arms the prefix"
    );
    // After the paste closes, the prefix routes again.
    assert!(
        session.route_bytes(&[0x02, b'd']),
        "prefix d detaches after the paste"
    );
    // `send_checked` returns after the reply, and the fake records each
    // line before replying, so every line is already queued.
    let lines: Vec<String> = rx.try_iter().map(|(_, line)| line).collect();
    assert!(
        !lines.iter().any(|l| l.starts_with("kill-pane")),
        "the pasted chord did not fire: {lines:?}"
    );
    let sends: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("send-keys"))
        .collect();
    assert_eq!(
        sends,
        vec![
            "send-keys -t %0 -H 1b 5b 32 30 30 7e 61 62",
            "send-keys -t %0 -H 02 78 63 64 1b 5b 32 30 31 7e",
        ],
        "{lines:?}"
    );
}

/// SEC-208 (d'): a marker split across bursts still flips the paste
/// state (the five-byte carry), and the carry stays bounded.
#[test]
fn passthrough_paste_markers_split_across_bursts_are_recognized() {
    let mut tracker = PasteTracker::default();
    for &b in b"\x1b[20" {
        tracker.observe(b);
    }
    assert!(!tracker.in_paste);
    for &b in b"0~" {
        tracker.observe(b);
    }
    assert!(tracker.in_paste, "split opener recognized");
    for &b in b"body\x1b[2" {
        tracker.observe(b);
    }
    assert!(tracker.in_paste);
    for &b in b"01~" {
        tracker.observe(b);
    }
    assert!(!tracker.in_paste, "split terminator recognized");
    assert!(
        tracker.carry.len() < PASTE_START.len(),
        "the carry stays bounded"
    );
}

/// SEC-209 defence in depth: an OSC 0/2 pane title (pane output, not
/// sanitized at the daemon) and names carrying controls never reach the
/// host through the passthrough status row.
#[test]
fn status_line_strips_control_characters() {
    let (_rx, mut session) = management_session("status-strip");
    session.session_name = "wo\x1b[2Jrk".to_string();
    session.workspaces = vec![("+0".to_string(), "ma\x07in".to_string())];
    session.active_workspace = Some("+0".to_string());
    session.pane_title = "t\x1b]52;c;aGk=\x07\u{9b}x".to_string();
    let line = session.status_line();
    assert!(!line.chars().any(char::is_control), "{line:?}");
    assert!(
        line.contains("[main]") && line.contains("wo[2Jrk") && line.contains("t]52;c;aGk=x"),
        "{line:?}"
    );
}

/// ARC-123: the attach options with only the given CLI fields set.
fn cli_options(prefix: Option<&str>, reload: Option<&str>) -> AttachOptions {
    AttachOptions {
        socket: None,
        name: None,
        target: None,
        prefix: prefix.map(str::to_string),
        reload: reload.map(str::to_string),
        mode: None,
    }
}

/// ARC-123: `resolve_client` layers CLI over file over defaults: no
/// file and no flag is the built-in set; `--prefix C-a` sets the
/// prefix byte 0x01.
#[test]
fn resolve_client_cli_prefix_overrides_the_default() {
    let empty = crate::mux::config::ConfigFile::default();
    let base = resolve_client_with(&cli_options(None, None), empty.clone()).expect("defaults");
    assert_eq!(base.chords, crate::mux::config::Chords::with_defaults());
    let resolved =
        resolve_client_with(&cli_options(Some("C-a"), None), empty).expect("C-a resolves");
    assert_eq!(resolved.chords.prefix, 0x01);
}

/// ARC-123: the CLI tier wins over the file's `[client]` chords, and
/// the file still supplies everything the CLI does not name.
#[test]
fn resolve_client_cli_wins_over_the_file() {
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nprefix = \"C-a\"\nreload = \"C-a C-s\"\nsplit-right = \"|\"\n")
            .unwrap();
    let from_file = resolve_client_with(&cli_options(None, None), file.clone()).expect("file");
    assert_eq!(from_file.chords.prefix, 0x01);
    assert_eq!(from_file.chords.reload, 0x13);
    assert_eq!(from_file.chords.management.split_right, b'|');
    let cli = resolve_client_with(&cli_options(Some("C-x"), Some("C-x C-e")), file).expect("cli");
    assert_eq!(cli.chords.prefix, 0x18, "--prefix wins over the file");
    assert_eq!(cli.chords.reload, 0x05, "the CLI reload chord wins");
    assert_eq!(
        cli.chords.management.split_right, b'|',
        "the file still fills the rest"
    );
}

/// ARC-123: a malformed `--prefix` is one `Handshake` error with the
/// passthrough spelling, whichever mode is attaching.
#[test]
fn resolve_client_rejects_a_malformed_prefix() {
    let err = resolve_client_with(
        &cli_options(Some("bogus-key"), None),
        crate::mux::config::ConfigFile::default(),
    )
    .expect_err("a bad prefix fails");
    match err {
        AttachError::Handshake(err) => {
            assert!(err.to_string().contains("invalid --prefix"), "{err}")
        }
        other => panic!("expected a handshake error, got {other:?}"),
    }
}
