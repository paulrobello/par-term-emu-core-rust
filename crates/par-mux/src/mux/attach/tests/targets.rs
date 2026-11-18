//! Socket path, prefix parsing, and target resolution.

use super::*;

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

/// The default target: the only session's active window's marked pane,
/// via the target-less rule (list-sessions picks the single session — no
/// create, no refusal). A fake daemon scripts the roster.
#[test]
fn default_target_is_the_only_sessions_active_pane() {
    let (daemon, path) = FakeDaemon::bind("resolve");
    {
        let mut conn = conn::AttachConn::connect(&path).expect("connect");
        let pane = resolve_target(&mut conn, None).expect("resolve");
        assert_eq!(pane, "%0");
        // The fake saw list-sessions, list-windows -t $0, and
        // list-panes -t @0 after the handshake's four lines — and no
        // new-workspace, because a session already existed.
        let lines = recorded(&daemon.received, 7);
        let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            !names.contains(&"new-workspace"),
            "one existing session must attach without a create: {lines:?}"
        );
        let windows_line = &lines[5].1;
        assert_eq!(
            windows_line, "list-windows -t $0",
            "the session narrows through its active window: {lines:?}"
        );
        let panes_line = &lines[6].1;
        assert_eq!(
            panes_line, "list-panes -t @0",
            "the window narrows to its marked pane: {lines:?}"
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
            // A window-shaped target is NOT a session: the real daemon
            // errors `list-windows -t @N`, and the resolver's
            // session-before-window probe order depends on that.
            if trimmed == "list-windows -t @1" {
                number += 1;
                tx.send(("list-windows".to_owned(), trimmed.to_owned()))
                    .ok();
                writer
                    .write_all(
                        crate::mux::emit_block(number, "no such session: @1", false).as_bytes(),
                    )
                    .ok();
                writer.flush().ok();
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
