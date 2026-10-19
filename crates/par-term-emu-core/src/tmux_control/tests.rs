use super::*;

#[test]
fn test_parser_new() {
    let parser = TmuxControlParser::new(true);
    assert!(parser.is_control_mode());
    assert_eq!(parser.buffer_len(), 0);

    let parser = TmuxControlParser::new(false);
    assert!(!parser.is_control_mode());
}

#[test]
fn test_set_control_mode() {
    let mut parser = TmuxControlParser::new(false);
    assert!(!parser.is_control_mode());

    parser.set_control_mode(true);
    assert!(parser.is_control_mode());

    parser.set_control_mode(false);
    assert!(!parser.is_control_mode());
}

#[test]
fn test_parse_begin() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%begin 1234567890 42 some-flag\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::Begin {
            timestamp,
            command_number,
            flags,
        } => {
            assert_eq!(*timestamp, 1234567890);
            assert_eq!(*command_number, 42);
            assert_eq!(flags, "some-flag");
        }
        _ => panic!("Expected Begin notification"),
    }
}

#[test]
fn test_parse_end() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%end 1234567890 42\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::End {
            timestamp,
            command_number,
            ..
        } => {
            assert_eq!(*timestamp, 1234567890);
            assert_eq!(*command_number, 42);
        }
        _ => panic!("Expected End notification"),
    }
}

#[test]
fn test_parse_output() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%output %1 Hello World\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::Output { pane_id, data } => {
            assert_eq!(pane_id, "%1");
            assert_eq!(data, b"Hello World");
        }
        _ => panic!("Expected Output notification"),
    }
}

#[test]
fn test_parse_output_single_space() {
    // Regression test: single space character should not be trimmed away
    let mut parser = TmuxControlParser::new(true);
    // Note: "%output %1  \n" has two spaces - one after %1 (separator) and one as data
    let notifications = parser.parse(b"%output %1  \n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::Output { pane_id, data } => {
            assert_eq!(pane_id, "%1");
            assert_eq!(data, b" ", "Space character should be preserved in output");
        }
        _ => panic!("Expected Output notification"),
    }
}

#[test]
fn test_parse_output_trailing_spaces() {
    // Ensure trailing spaces in output data are preserved
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%output %1 text   \n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::Output { pane_id, data } => {
            assert_eq!(pane_id, "%1");
            assert_eq!(data, b"text   ", "Trailing spaces should be preserved");
        }
        _ => panic!("Expected Output notification"),
    }
}

#[test]
fn test_parse_output_crlf_line_ending() {
    // tmux sends \r\n line endings - ensure \r is stripped, not included in data
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%output %1 a\r\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::Output { pane_id, data } => {
            assert_eq!(pane_id, "%1");
            assert_eq!(data, b"a", "CR should be stripped from line ending");
        }
        _ => panic!("Expected Output notification"),
    }
}

#[test]
fn test_parse_output_single_space_crlf() {
    // Regression test: single space with \r\n line ending
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%output %1  \r\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::Output { pane_id, data } => {
            assert_eq!(pane_id, "%1");
            assert_eq!(data, b" ", "Space should be preserved with CRLF ending");
        }
        _ => panic!("Expected Output notification"),
    }
}

#[test]
fn test_unescape_output() {
    // Test newline escape
    assert_eq!(
        TmuxControlParser::unescape_output("Hello\\012World"),
        b"Hello\nWorld"
    );

    // Test backslash escape
    assert_eq!(
        TmuxControlParser::unescape_output("Path\\134file"),
        b"Path\\file"
    );

    // Test tab escape
    assert_eq!(TmuxControlParser::unescape_output("A\\011B"), b"A\tB");

    // Test multiple escapes
    assert_eq!(
        TmuxControlParser::unescape_output("\\033[32mGreen\\033[0m"),
        b"\x1b[32mGreen\x1b[0m"
    );

    // Test no escapes
    assert_eq!(
        TmuxControlParser::unescape_output("Plain text"),
        b"Plain text"
    );
}

#[test]
fn test_parse_window_renamed() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%window-renamed @1 new-window-name\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::WindowRenamed { window_id, name } => {
            assert_eq!(window_id, "@1");
            assert_eq!(name, "new-window-name");
        }
        _ => panic!("Expected WindowRenamed notification"),
    }
}

#[test]
fn test_parse_session_changed() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%session-changed $1 my-session\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::SessionChanged { session_id, name } => {
            assert_eq!(session_id, "$1");
            assert_eq!(name, "my-session");
        }
        _ => panic!("Expected SessionChanged notification"),
    }
}

#[test]
fn test_parse_sessions_changed() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%sessions-changed\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::SessionsChanged => {}
        _ => panic!("Expected SessionsChanged notification"),
    }
}

#[test]
fn test_parse_exit() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%exit\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::Exit => {}
        _ => panic!("Expected Exit notification"),
    }
}

#[test]
fn test_parse_multiple_lines() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%sessions-changed\n%exit\n");
    assert_eq!(notifications.len(), 2);

    assert!(matches!(
        &notifications[0],
        TmuxNotification::SessionsChanged
    ));
    assert!(matches!(&notifications[1], TmuxNotification::Exit));
}

#[test]
fn test_parse_incomplete_line() {
    let mut parser = TmuxControlParser::new(true);

    // Parse partial data
    let notifications = parser.parse(b"%sessions");
    assert_eq!(notifications.len(), 0);
    assert_eq!(parser.buffer_len(), 9); // "%sessions" buffered

    // Complete the line
    let notifications = parser.parse(b"-changed\n");
    assert_eq!(notifications.len(), 1);
    assert!(matches!(
        &notifications[0],
        TmuxNotification::SessionsChanged
    ));
    assert_eq!(parser.buffer_len(), 0);
}

#[test]
fn test_parse_unknown_notification() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%unknown-notification some args\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::Unknown { line } => {
            assert_eq!(line, "%unknown-notification some args");
        }
        _ => panic!("Expected Unknown notification"),
    }
}

#[test]
fn test_parse_empty_line() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"\n");
    assert_eq!(notifications.len(), 0);
}

#[test]
fn test_parse_terminal_output_in_control_mode() {
    let mut parser = TmuxControlParser::new(true);
    // Non-% prefixed line in control mode (shouldn't normally happen)
    let notifications = parser.parse(b"regular output\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::TerminalOutput { data } => {
            assert_eq!(data, b"regular output");
        }
        _ => panic!("Expected TerminalOutput notification"),
    }
}

#[test]
fn test_parse_non_control_mode() {
    let mut parser = TmuxControlParser::new(false);
    let notifications = parser.parse(b"This is regular terminal output");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::TerminalOutput { data } => {
            assert_eq!(data, b"This is regular terminal output");
        }
        _ => panic!("Expected TerminalOutput notification"),
    }
}

#[test]
fn test_clear_buffer() {
    let mut parser = TmuxControlParser::new(true);
    parser.parse(b"%incomplete");
    assert!(parser.buffer_len() > 0);

    parser.clear_buffer();
    assert_eq!(parser.buffer_len(), 0);
}

#[test]
fn test_parse_pane_mode_changed() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%pane-mode-changed %1\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::PaneModeChanged { pane_id } => {
            assert_eq!(pane_id, "%1");
        }
        _ => panic!("Expected PaneModeChanged notification"),
    }
}

#[test]
fn test_parse_window_pane_changed() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%window-pane-changed @5 %12\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::WindowPaneChanged { window_id, pane_id } => {
            assert_eq!(window_id, "@5");
            assert_eq!(pane_id, "%12");
        }
        _ => panic!("Expected WindowPaneChanged notification"),
    }
}

#[test]
fn test_parse_layout_change() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%layout-change @1 abc123,80x24,0,0 def456,80x24,0,0 *Z\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::LayoutChange {
            window_id,
            window_layout,
            window_visible_layout,
            window_raw_flags,
        } => {
            assert_eq!(window_id, "@1");
            assert_eq!(window_layout, "abc123,80x24,0,0");
            assert_eq!(window_visible_layout, "def456,80x24,0,0");
            assert_eq!(window_raw_flags, "*Z");
        }
        _ => panic!("Expected LayoutChange notification"),
    }
}

#[test]
fn test_parse_paste_buffer_changed() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%paste-buffer-changed buffer0\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::PasteBufferChanged { name } => {
            assert_eq!(name, "buffer0");
        }
        _ => panic!("Expected PasteBufferChanged notification"),
    }
}

#[test]
fn test_parse_paste_buffer_deleted() {
    let mut parser = TmuxControlParser::new(true);
    let notifications = parser.parse(b"%paste-buffer-deleted buffer0\n");
    assert_eq!(notifications.len(), 1);

    match &notifications[0] {
        TmuxNotification::PasteBufferDeleted { name } => {
            assert_eq!(name, "buffer0");
        }
        _ => panic!("Expected PasteBufferDeleted notification"),
    }
}

#[test]
fn test_notification_type() {
    assert_eq!(
        TmuxNotification::SessionsChanged.notification_type(),
        "sessions-changed"
    );
    assert_eq!(TmuxNotification::Exit.notification_type(), "exit");
    assert_eq!(
        TmuxNotification::AgentTelemetryChanged {
            pane_id: "%1".to_string(),
            agent: "claude".to_string(),
        }
        .notification_type(),
        "agent-telemetry-changed"
    );
    assert_eq!(
        TmuxNotification::Output {
            pane_id: "%1".to_string(),
            data: vec![]
        }
        .notification_type(),
        "output"
    );
    assert_eq!(
        TmuxNotification::LayoutChange {
            window_id: "@1".to_string(),
            window_layout: "layout".to_string(),
            window_visible_layout: "visible".to_string(),
            window_raw_flags: "flags".to_string(),
        }
        .notification_type(),
        "layout-change"
    );
    assert_eq!(
        TmuxNotification::PasteBufferChanged {
            name: "buffer0".to_string(),
        }
        .notification_type(),
        "paste-buffer-changed"
    );
    assert_eq!(
        TmuxNotification::PasteBufferDeleted {
            name: "buffer0".to_string(),
        }
        .notification_type(),
        "paste-buffer-deleted"
    );
}

#[test]
fn test_auto_detect_default() {
    let parser = TmuxControlParser::new(false);
    assert!(!parser.is_auto_detect());

    let parser = TmuxControlParser::new(true);
    assert!(!parser.is_auto_detect());
}

#[test]
fn test_set_auto_detect() {
    let mut parser = TmuxControlParser::new(false);
    assert!(!parser.is_auto_detect());

    parser.set_auto_detect(true);
    assert!(parser.is_auto_detect());

    parser.set_auto_detect(false);
    assert!(!parser.is_auto_detect());
}

#[test]
fn test_set_control_mode_enables_auto_detect() {
    let mut parser = TmuxControlParser::new(false);
    assert!(!parser.is_auto_detect());

    // Enabling control mode should also enable auto-detect
    parser.set_control_mode(true);
    assert!(parser.is_control_mode());
    assert!(parser.is_auto_detect());
}

#[test]
fn test_auto_detect_finds_begin_at_start() {
    let mut parser = TmuxControlParser::new(false);
    parser.set_auto_detect(true);
    assert!(!parser.is_control_mode());

    // Data starting with %begin should auto-enable control mode
    let notifications = parser.parse(b"%begin 1234567890 1\n");
    assert_eq!(notifications.len(), 1);
    assert!(parser.is_control_mode()); // Should have auto-enabled

    match &notifications[0] {
        TmuxNotification::Begin {
            timestamp,
            command_number,
            ..
        } => {
            assert_eq!(*timestamp, 1234567890);
            assert_eq!(*command_number, 1);
        }
        _ => panic!("Expected Begin notification"),
    }
}

#[test]
fn test_auto_detect_finds_begin_after_terminal_output() {
    let mut parser = TmuxControlParser::new(false);
    parser.set_auto_detect(true);
    assert!(!parser.is_control_mode());

    // Data with terminal output before %begin
    let data = b"$ tmux -CC\n%begin 1234567890 1\n";
    let notifications = parser.parse(data);

    // Should have terminal output + begin notification
    assert_eq!(notifications.len(), 2);
    assert!(parser.is_control_mode()); // Should have auto-enabled

    // First notification should be terminal output
    match &notifications[0] {
        TmuxNotification::TerminalOutput { data } => {
            assert_eq!(data, b"$ tmux -CC\n");
        }
        _ => panic!("Expected TerminalOutput notification first"),
    }

    // Second notification should be Begin
    match &notifications[1] {
        TmuxNotification::Begin { .. } => {}
        _ => panic!("Expected Begin notification second"),
    }
}

#[test]
fn test_auto_detect_without_begin() {
    let mut parser = TmuxControlParser::new(false);
    parser.set_auto_detect(true);
    assert!(!parser.is_control_mode());

    // Data without %begin should remain as terminal output
    let notifications = parser.parse(b"regular terminal output\n");
    assert_eq!(notifications.len(), 1);
    assert!(!parser.is_control_mode()); // Should NOT have auto-enabled

    match &notifications[0] {
        TmuxNotification::TerminalOutput { data } => {
            assert_eq!(data, b"regular terminal output\n");
        }
        _ => panic!("Expected TerminalOutput notification"),
    }
}

#[test]
fn test_auto_detect_disabled() {
    let mut parser = TmuxControlParser::new(false);
    // Auto-detect is disabled (default)
    assert!(!parser.is_auto_detect());

    // Data with %begin should NOT auto-enable control mode
    let notifications = parser.parse(b"%begin 1234567890 1\n");
    assert_eq!(notifications.len(), 1);
    assert!(!parser.is_control_mode()); // Should NOT have auto-enabled

    // Should be treated as terminal output
    match &notifications[0] {
        TmuxNotification::TerminalOutput { data } => {
            assert_eq!(data, b"%begin 1234567890 1\n");
        }
        _ => panic!("Expected TerminalOutput notification"),
    }
}

#[test]
fn test_auto_detect_handles_race_condition() {
    // This test simulates the race condition where:
    // 1. User enables control mode (which also enables auto-detect)
    // 2. But data arrives BEFORE the control mode flag is checked
    //
    // In this case, auto-detect should catch the %begin and switch modes

    let mut parser = TmuxControlParser::new(false);

    // Simulate: user calls set_control_mode(true) which enables auto-detect
    // but let's pretend control_mode is still false due to a race
    parser.set_auto_detect(true);
    // parser.control_mode is still false here

    // Now data arrives with %begin
    let notifications = parser.parse(b"shell prompt $ \n%begin 1234567890 1\n%output %1 Hello\n");

    // Should have:
    // 1. TerminalOutput for "shell prompt $ \n"
    // 2. Begin notification
    // 3. Output notification
    assert!(notifications.len() >= 2);
    assert!(parser.is_control_mode()); // Should have auto-enabled

    // Verify first notification is terminal output
    match &notifications[0] {
        TmuxNotification::TerminalOutput { data } => {
            assert_eq!(data, b"shell prompt $ \n");
        }
        _ => panic!("Expected TerminalOutput first"),
    }

    // Verify second notification is Begin
    match &notifications[1] {
        TmuxNotification::Begin { .. } => {}
        _ => panic!("Expected Begin second"),
    }
}

#[test]
fn test_find_control_mode_start_at_beginning() {
    let parser = TmuxControlParser::new(false);
    assert_eq!(parser.find_control_mode_start(b"%begin 123 1\n"), Some(0));
    assert_eq!(parser.find_control_mode_start(b"%begin\n"), Some(0));
}

#[test]
fn test_find_control_mode_start_after_newline() {
    let parser = TmuxControlParser::new(false);
    assert_eq!(
        parser.find_control_mode_start(b"prompt $ \n%begin 123 1\n"),
        Some(10) // Position after the newline
    );
}

#[test]
fn test_find_control_mode_start_not_found() {
    let parser = TmuxControlParser::new(false);
    assert_eq!(parser.find_control_mode_start(b"no begin here\n"), None);
    // %begin in middle of line shouldn't match
    assert_eq!(parser.find_control_mode_start(b"text %begin more\n"), None);
}

fn parse_one(line: &str) -> Vec<TmuxNotification> {
    let mut parser = TmuxControlParser::new(true);
    parser.parse(format!("{line}\n").as_bytes())
}

/// A known keyword whose arguments are too few or malformed is dropped
/// outright — not surfaced as `Unknown`, which a client would treat as
/// an unrecognised-but-valid line — and the parser keeps going: the
/// next well-formed line still parses.
#[test]
fn malformed_known_notifications_are_dropped_and_parsing_continues() {
    for line in [
        "%begin",
        "%begin 1",
        "%begin x 1 1",
        "%begin 1 y 1",
        "%end 1",
        "%end 1 x",
        "%error",
        "%error x 1",
        "%window-pane-changed @1",
        "%window-renamed @1",
        "%unlinked-window-renamed @1",
        "%session-changed $1",
        "%client-session-changed c $1",
        "%session-renamed $1",
        "%session-window-changed $1",
        "%client-attached",
        "%client-left",
        "%extended-output %1 5",
        "%extended-output %1 soon : x",
        "%layout-change @1 a b",
        "%agent-state-changed %1 claude",
        "%agent-released %1",
        "%agent-released %1 claude extra",
        "%agent-telemetry-changed %1",
        "%pane-title-changed",
        "%pane-title-changed  title",
        "%pane-exited",
        "%pane-respawned",
    ] {
        let mut parser = TmuxControlParser::new(true);
        let parsed = parser.parse(format!("{line}\n%exit\n").as_bytes());
        assert_eq!(
            parsed,
            vec![TmuxNotification::Exit],
            "`{line}` must be dropped and the next line still parse"
        );
    }
}

/// The parse-only notifications (the daemon never emits them, real
/// tmux does) decode into their typed fields.
#[test]
fn parse_only_notifications_decode_their_fields() {
    use TmuxNotification as N;
    let cases: Vec<(&str, N)> = vec![
        (
            "%pause %4",
            N::Pause {
                pane_id: "%4".into(),
            },
        ),
        (
            "%continue %4",
            N::Continue {
                pane_id: "%4".into(),
            },
        ),
        (
            "%subscription-changed sub $1 @2 0 %3 - : the value",
            N::SubscriptionChanged {
                name: "sub".into(),
                value: "the value".into(),
            },
        ),
        (
            "%subscription-changed lonely",
            N::SubscriptionChanged {
                name: "lonely".into(),
                value: String::new(),
            },
        ),
        (
            "%unlinked-window-add @7",
            N::UnlinkedWindowAdd {
                window_id: "@7".into(),
            },
        ),
        (
            "%unlinked-window-close @7",
            N::UnlinkedWindowClose {
                window_id: "@7".into(),
            },
        ),
        (
            "%unlinked-window-renamed @7 build logs",
            N::UnlinkedWindowRenamed {
                window_id: "@7".into(),
                name: "build logs".into(),
            },
        ),
        (
            "%client-detached /dev/ttys003",
            N::ClientDetached {
                client: "/dev/ttys003".into(),
            },
        ),
        (
            "%client-session-changed /dev/ttys003 $2 my session",
            N::ClientSessionChanged {
                client: "/dev/ttys003".into(),
                session_id: "$2".into(),
                name: "my session".into(),
            },
        ),
        (
            "%session-renamed $2 new name",
            N::SessionRenamed {
                session_id: "$2".into(),
                name: "new name".into(),
            },
        ),
        (
            "%session-window-changed $2 @9",
            N::SessionWindowChanged {
                session_id: "$2".into(),
                window_id: "@9".into(),
            },
        ),
        (
            "%error 1700000000 3 1",
            N::Error {
                timestamp: 1_700_000_000,
                command_number: 3,
                flags: "1".into(),
            },
        ),
        (
            "%end 1700000000 3",
            N::End {
                timestamp: 1_700_000_000,
                command_number: 3,
                flags: String::new(),
            },
        ),
        (
            "%output %1",
            N::Output {
                pane_id: "%1".into(),
                data: Vec::new(),
            },
        ),
        ("%workspaces-changed", N::WorkspacesChanged),
        ("%config-changed", N::ConfigChanged),
    ];
    for (line, expected) in cases {
        assert_eq!(parse_one(line), vec![expected], "`{line}`");
    }
}

/// The par-mux extensions decode their optional tails: a client-left
/// with and without its view pair, a pane-exited with a code, without
/// one, and with an unparsable one (None, not a misread), and an
/// agent-state-changed whose state spans words, with and without the
/// trailing provenance token.
#[test]
fn par_mux_extension_optional_fields_decode() {
    use TmuxNotification as N;
    assert_eq!(
        parse_one("%client-left 12 $1 @3"),
        vec![N::ClientLeft {
            client: "12".into(),
            session_id: Some("$1".into()),
            window_id: Some("@3".into()),
        }]
    );
    assert_eq!(
        parse_one("%client-left 12"),
        vec![N::ClientLeft {
            client: "12".into(),
            session_id: None,
            window_id: None,
        }]
    );
    assert_eq!(
        parse_one("%client-attached 12 extra"),
        vec![N::ClientAttached {
            client: "12".into()
        }]
    );
    for (line, code) in [
        ("%pane-exited %5 3", Some(3)),
        ("%pane-exited %5 -1", Some(-1)),
        ("%pane-exited %5", None),
        ("%pane-exited %5 ?", None),
    ] {
        assert_eq!(
            parse_one(line),
            vec![N::PaneExited {
                pane_id: "%5".into(),
                exit_code: code,
            }],
            "`{line}`"
        );
    }
    assert_eq!(
        parse_one("%pane-respawned %5"),
        vec![N::PaneRespawned {
            pane_id: "%5".into()
        }]
    );
    assert_eq!(
        parse_one("%pane-title-changed %5 my build pane"),
        vec![N::PaneTitleChanged {
            pane_id: "%5".into(),
            title: "my build pane".into(),
        }]
    );
    assert_eq!(
        parse_one("%pane-title-changed %5"),
        vec![N::PaneTitleChanged {
            pane_id: "%5".into(),
            title: String::new(),
        }],
        "a missing title is the clear"
    );
    assert_eq!(
        parse_one("%agent-state-changed %1 claude waiting for input source=hook"),
        vec![N::AgentStateChanged {
            pane_id: "%1".into(),
            agent: "claude".into(),
            state: "waiting for input".into(),
            source: "hook".into(),
            ws: String::new(),
            msg: String::new(),
        }]
    );
    assert_eq!(
        parse_one("%agent-state-changed %1 claude busy"),
        vec![N::AgentStateChanged {
            pane_id: "%1".into(),
            agent: "claude".into(),
            state: "busy".into(),
            source: String::new(),
            ws: String::new(),
            msg: String::new(),
        }]
    );
    assert_eq!(
        parse_one("%agent-state-changed %1 claude source=hook"),
        vec![N::AgentStateChanged {
            pane_id: "%1".into(),
            agent: "claude".into(),
            state: "source=hook".into(),
            source: String::new(),
            ws: String::new(),
            msg: String::new(),
        }],
        "with only three tokens the third is the state, never provenance"
    );
    // ENH-061: trailing key=value tokens are metadata; unknown keys are
    // ignored and ws rides after source.
    assert_eq!(
        parse_one("%agent-state-changed %1 claude working source=hook ws=+2"),
        vec![N::AgentStateChanged {
            pane_id: "%1".into(),
            agent: "claude".into(),
            state: "working".into(),
            source: "hook".into(),
            ws: "+2".into(),
            msg: String::new(),
        }]
    );
    assert_eq!(
        parse_one("%agent-state-changed %1 claude waiting for input source=scrape ws=+0"),
        vec![N::AgentStateChanged {
            pane_id: "%1".into(),
            agent: "claude".into(),
            state: "waiting for input".into(),
            source: "scrape".into(),
            ws: "+0".into(),
            msg: String::new(),
        }]
    );
    assert_eq!(
        parse_one("%agent-state-changed %1 claude working source=hook future=x"),
        vec![N::AgentStateChanged {
            pane_id: "%1".into(),
            agent: "claude".into(),
            state: "working".into(),
            source: "hook".into(),
            ws: String::new(),
            msg: String::new(),
        }],
        "an unknown key is ignored, never folded into the state"
    );
    assert_eq!(
        parse_one(
            "%agent-state-changed %1 claude blocked source=hook ws=+2 msg=bmVlZHMgYXBwcm92YWw="
        ),
        vec![N::AgentStateChanged {
            pane_id: "%1".into(),
            agent: "claude".into(),
            state: "blocked".into(),
            source: "hook".into(),
            ws: "+2".into(),
            msg: "needs approval".into(),
        }],
        "the msg token decodes the base64 reason"
    );
    assert_eq!(
        parse_one("%agent-state-changed %1 claude blocked source=hook ws=+2 msg=!!!"),
        vec![N::AgentStateChanged {
            pane_id: "%1".into(),
            agent: "claude".into(),
            state: "blocked".into(),
            source: "hook".into(),
            ws: "+2".into(),
            msg: String::new(),
        }],
        "malformed base64 stays empty: no message, never garbage"
    );
    assert_eq!(
        parse_one("%agent-state-changed %1 claude working ws=+2"),
        vec![N::AgentStateChanged {
            pane_id: "%1".into(),
            agent: "claude".into(),
            state: "working".into(),
            source: String::new(),
            ws: "+2".into(),
            msg: String::new(),
        }],
        "ws rides without a source token"
    );
}

/// Octal escapes decode only as three digits forming a byte; anything
/// else (an out-of-range value, a non-octal digit, a short or trailing
/// backslash) stays literal, so a stray backslash in pane output is
/// never eaten.
#[test]
fn unescape_output_keeps_invalid_escapes_literal() {
    let decode = |s: &str| TmuxControlParser::unescape_output(s);
    assert_eq!(decode("\\101\\102"), b"AB");
    assert_eq!(decode("\\400"), b"\\400", "past 0o377 is not a byte");
    assert_eq!(decode("\\189"), b"\\189", "8 and 9 are not octal digits");
    assert_eq!(decode("\\12"), b"\\12", "two digits are not an escape");
    assert_eq!(decode("end\\"), b"end\\", "a trailing backslash stays");
    assert_eq!(decode("\\\\"), b"\\\\", "an escaped backslash is \\134");
}

/// An unrecognised `%` keyword reaches the caller verbatim as
/// `Unknown` (leading whitespace trimmed), and the parser's line
/// buffer drains completely.
#[test]
fn unknown_keywords_surface_verbatim_and_drain_the_buffer() {
    let mut parser = TmuxControlParser::new(true);
    let parsed = parser.parse(b"  %config-error bad line here\r\n%sessions-changed\n");
    assert_eq!(
        parsed,
        vec![
            TmuxNotification::Unknown {
                line: "%config-error bad line here".into()
            },
            TmuxNotification::SessionsChanged,
        ]
    );
    assert_eq!(parser.buffer_len(), 0);
    assert_eq!(
        TmuxNotification::Unknown {
            line: String::new()
        }
        .notification_type(),
        "unknown"
    );
}

/// tmux(1) CONTROL MODE: `%extended-output pane-id age ... : value` —
/// the value follows a lone `:` and its separating space, and any
/// arguments between age and the `:` are for future use and ignored.
#[test]
fn extended_output_value_follows_the_lone_colon_separator() {
    let mut parser = TmuxControlParser::new(true);
    let parsed = parser.parse(b"%extended-output %3 125 : hello world\n");
    assert_eq!(
        parsed,
        vec![TmuxNotification::ExtendedOutput {
            pane_id: "%3".to_string(),
            delay_ms: 125,
            data: b"hello world".to_vec(),
        }]
    );
    let parsed = parser.parse(b"%extended-output %3 7 future args : \\033[0m\n");
    assert_eq!(
        parsed,
        vec![TmuxNotification::ExtendedOutput {
            pane_id: "%3".to_string(),
            delay_ms: 7,
            data: b"\x1b[0m".to_vec(),
        }],
        "future-use arguments before the colon are ignored"
    );
    // A value that itself contains " : " keeps it: only the first lone
    // colon separates.
    let parsed = parser.parse(b"%extended-output %3 7 : a : b\n");
    assert_eq!(
        parsed,
        vec![TmuxNotification::ExtendedOutput {
            pane_id: "%3".to_string(),
            delay_ms: 7,
            data: b"a : b".to_vec(),
        }]
    );
}
