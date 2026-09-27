// Integration tests for trigger system (Feature 18)
use par_term_emu_core_rust::terminal::trigger::TriggerAction;
use par_term_emu_core_rust::terminal::Terminal;
use par_term_emu_core_rust::terminal::TriggerEngine;

#[test]
fn test_trigger_add_remove() {
    let mut term = Terminal::new(80, 24);
    let id = TriggerEngine::add_trigger(&mut term, "test".into(), "ERROR".into(), vec![]).unwrap();
    assert_eq!(TriggerEngine::list_triggers(&term,).len(), 1);
    assert!(TriggerEngine::get_trigger(&term, id).is_some());
    assert!(TriggerEngine::remove_trigger(&mut term, id));
    assert!(TriggerEngine::get_trigger(&term, id).is_none());
    assert_eq!(TriggerEngine::list_triggers(&term,).len(), 0);
}

#[test]
fn test_trigger_enable_disable() {
    let mut term = Terminal::new(80, 24);
    let id = TriggerEngine::add_trigger(&mut term, "test".into(), "MATCH".into(), vec![]).unwrap();

    // Process text with trigger enabled
    term.process(b"MATCH here\n");
    TriggerEngine::process_trigger_scans(&mut term);
    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(matches.len(), 1);

    // Disable trigger
    assert!(TriggerEngine::set_trigger_enabled(&mut term, id, false));

    term.process(b"MATCH again\n");
    TriggerEngine::process_trigger_scans(&mut term);
    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(matches.len(), 0);

    // Re-enable
    assert!(TriggerEngine::set_trigger_enabled(&mut term, id, true));
    term.process(b"MATCH once more\n");
    TriggerEngine::process_trigger_scans(&mut term);
    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(matches.len(), 1);
}

#[test]
fn test_trigger_scan_match() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(&mut term, "error".into(), r"ERROR:\s+(.+)".into(), vec![]).unwrap();

    term.process(b"prefix ERROR: something went wrong\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].row, 0);
    assert!(matches[0].text.contains("ERROR:"));
    assert_eq!(matches[0].captures.len(), 2); // group 0 + group 1
}

#[test]
fn test_trigger_capture_groups() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(
        &mut term,
        "ip".into(),
        r"(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})".into(),
        vec![],
    )
    .unwrap();

    term.process(b"IP: 192.168.1.100\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].captures.len(), 5);
    assert_eq!(matches[0].captures[0], "192.168.1.100");
    assert_eq!(matches[0].captures[1], "192");
    assert_eq!(matches[0].captures[4], "100");
}

#[test]
fn test_trigger_multi_pattern() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(&mut term, "error".into(), "ERROR".into(), vec![]).unwrap();
    TriggerEngine::add_trigger(&mut term, "warn".into(), "WARN".into(), vec![]).unwrap();

    term.process(b"ERROR and WARN on same line\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(matches.len(), 2);
}

#[test]
fn test_trigger_invalid_regex() {
    let mut term = Terminal::new(80, 24);
    let result = TriggerEngine::add_trigger(&mut term, "bad".into(), "[invalid".into(), vec![]);
    assert!(result.is_err());
}

#[test]
fn test_trigger_event_generation() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(&mut term, "test".into(), "FOUND".into(), vec![]).unwrap();

    term.process(b"FOUND it\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let events = term.poll_events();
    let trigger_events: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                par_term_emu_core_rust::terminal::TerminalEvent::TriggerMatched(_)
            )
        })
        .collect();
    assert_eq!(trigger_events.len(), 1);
}

#[test]
fn test_trigger_action_highlight() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(
        &mut term,
        "test".into(),
        "HIGHLIGHT".into(),
        vec![TriggerAction::Highlight {
            fg: None,
            bg: Some((255, 0, 0)),
            duration_ms: 0, // permanent
        }],
    )
    .unwrap();

    term.process(b"HIGHLIGHT this\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let highlights = TriggerEngine::get_trigger_highlights(&term);
    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].row, 0);
    assert_eq!(highlights[0].bg, Some((255, 0, 0)));
    assert_eq!(highlights[0].fg, None);
}

#[test]
fn test_trigger_action_notify() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(
        &mut term,
        "test".into(),
        r"ERROR: (\S+)".into(),
        vec![TriggerAction::Notify {
            title: "Error Alert".into(),
            message: "Found: $1".into(),
        }],
    )
    .unwrap();

    term.process(b"ERROR: diskfull\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let notifications = term.notifications();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].title, "Error Alert");
    assert_eq!(notifications[0].message, "Found: diskfull");
}

#[test]
fn test_trigger_action_mark_line() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(
        &mut term,
        "test".into(),
        "BOOKMARK".into(),
        vec![TriggerAction::MarkLine {
            label: Some("Auto-bookmark".into()),
            color: None,
        }],
    )
    .unwrap();

    term.process(b"BOOKMARK here\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let bookmarks = term.get_bookmarks();
    assert_eq!(bookmarks.len(), 1);
    assert_eq!(bookmarks[0].label, "Auto-bookmark");
}

#[test]
fn test_trigger_action_set_variable() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(
        &mut term,
        "test".into(),
        r"STATUS: (\w+)".into(),
        vec![TriggerAction::SetVariable {
            name: "last_status".into(),
            value: "$1".into(),
        }],
    )
    .unwrap();

    term.process(b"STATUS: RUNNING\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let vars = term.session_variables();
    assert_eq!(vars.custom.get("last_status"), Some(&"RUNNING".to_string()));
}

#[test]
fn test_trigger_action_stop_propagation() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(
        &mut term,
        "test".into(),
        "STOP".into(),
        vec![
            TriggerAction::MarkLine {
                label: Some("Before stop".into()),
                color: None,
            },
            TriggerAction::StopPropagation,
            TriggerAction::MarkLine {
                label: Some("After stop".into()),
                color: None,
            },
        ],
    )
    .unwrap();

    term.process(b"STOP here\n");
    TriggerEngine::process_trigger_scans(&mut term);

    // Only the first action before StopPropagation should have executed
    let bookmarks = term.get_bookmarks();
    assert_eq!(bookmarks.len(), 1);
    assert_eq!(bookmarks[0].label, "Before stop");
}

#[test]
fn test_trigger_capture_substitution() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(
        &mut term,
        "test".into(),
        r"USER:(\w+) CMD:(\w+)".into(),
        vec![TriggerAction::SetVariable {
            name: "user_$1".into(),
            value: "ran_$2".into(),
        }],
    )
    .unwrap();

    term.process(b"USER:alice CMD:deploy\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let vars = term.session_variables();
    assert_eq!(
        vars.custom.get("user_alice"),
        Some(&"ran_deploy".to_string())
    );
}

#[test]
fn test_highlight_expiry() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(
        &mut term,
        "test".into(),
        "BRIEF".into(),
        vec![TriggerAction::Highlight {
            fg: None,
            bg: Some((0, 255, 0)),
            duration_ms: 1, // 1ms - will expire almost immediately
        }],
    )
    .unwrap();

    term.process(b"BRIEF flash\n");
    TriggerEngine::process_trigger_scans(&mut term);

    // Wait for expiry
    std::thread::sleep(std::time::Duration::from_millis(10));

    let highlights = TriggerEngine::get_trigger_highlights(&term);
    assert_eq!(highlights.len(), 0, "Expired highlights should be filtered");

    // Also test clear_expired_highlights
    TriggerEngine::clear_expired_highlights(&mut term);
}

#[test]
fn test_trigger_frontend_actions() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(
        &mut term,
        "test".into(),
        "RUN".into(),
        vec![
            TriggerAction::RunCommand {
                command: "echo".into(),
                args: vec!["hello".into()],
            },
            TriggerAction::PlaySound {
                sound_id: "alert".into(),
                volume: 80,
            },
            TriggerAction::SendText {
                text: "response\n".into(),
                delay_ms: 100,
            },
        ],
    )
    .unwrap();

    term.process(b"RUN now\n");
    TriggerEngine::process_trigger_scans(&mut term);

    let results = TriggerEngine::poll_action_results(&mut term);
    assert_eq!(results.len(), 3);
}
