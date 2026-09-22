// Mode-related terminal tests
use crate::mouse::{MouseEncoding, MouseMode};
use crate::terminal::*;

#[test]
fn test_alt_screen() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Primary");

    // Switch to alt screen
    term.process(b"\x1b[?1049h");
    assert!(term.is_alt_screen_active());

    term.process(b"Alternate");
    let content = term.content();
    assert!(content.contains("Alternate"));
    assert!(!content.contains("Primary"));

    // Switch back
    term.process(b"\x1b[?1049l");
    assert!(!term.is_alt_screen_active());

    let content = term.content();
    assert!(content.contains("Primary"));
}

#[test]
fn test_mouse_modes() {
    let mut term = Terminal::new(80, 24);

    // Enable normal mouse tracking
    term.process(b"\x1b[?1000h");
    assert_eq!(term.mouse_mode(), MouseMode::Normal);

    // Enable SGR encoding
    term.process(b"\x1b[?1006h");
    assert_eq!(term.mouse_encoding(), MouseEncoding::Sgr);

    // Disable mouse
    term.process(b"\x1b[?1000l");
    assert_eq!(term.mouse_mode(), MouseMode::Off);
}

#[test]
fn test_bracketed_paste() {
    let mut term = Terminal::new(80, 24);

    assert!(!term.bracketed_paste());

    // Enable bracketed paste
    term.process(b"\x1b[?2004h");
    assert!(term.bracketed_paste());

    // Disable
    term.process(b"\x1b[?2004l");
    assert!(!term.bracketed_paste());
}

#[test]
fn test_focus_tracking() {
    let mut term = Terminal::new(80, 24);

    assert!(!term.focus_tracking());

    // Enable focus tracking
    term.process(b"\x1b[?1004h");
    assert!(term.focus_tracking());

    // Test focus events
    let focus_in = term.report_focus_in();
    assert_eq!(focus_in, b"\x1b[I");

    let focus_out = term.report_focus_out();
    assert_eq!(focus_out, b"\x1b[O");
}

#[test]
fn test_synchronized_updates() {
    let mut term = Terminal::new(80, 24);

    // Initially disabled
    assert!(!term.synchronized_updates());

    // Enable synchronized updates
    term.process(b"\x1b[?2026h");
    assert!(term.synchronized_updates());

    // Process some content - it should be buffered
    term.process(b"Buffered");
    let content = term.content();
    // Content should be empty because it's buffered
    assert!(!content.contains("Buffered"));

    // Disable synchronized updates - this should flush the buffer
    term.process(b"\x1b[?2026l");
    assert!(!term.synchronized_updates());

    // Now content should appear
    let content = term.content();
    assert!(content.contains("Buffered"));
}

#[test]
fn test_synchronized_updates_multiple_updates() {
    let mut term = Terminal::new(80, 24);

    // Enable synchronized updates
    term.process(b"\x1b[?2026h");

    // Send multiple updates
    term.process(b"Line1\r\n");
    term.process(b"Line2\r\n");
    term.process(b"Line3");

    // All should be buffered
    let content = term.content();
    assert!(!content.contains("Line1"));
    assert!(!content.contains("Line2"));
    assert!(!content.contains("Line3"));

    // Disable and flush
    term.process(b"\x1b[?2026l");

    // All lines should appear
    let content = term.content();
    assert!(content.contains("Line1"));
    assert!(content.contains("Line2"));
    assert!(content.contains("Line3"));
}

#[test]
fn test_synchronized_updates_manual_flush() {
    let mut term = Terminal::new(80, 24);

    // Enable synchronized updates
    term.process(b"\x1b[?2026h");
    term.process(b"Test");

    // Content buffered
    assert!(!term.content().contains("Test"));

    // Manual flush
    term.flush_synchronized_updates();

    // Content should appear, mode still enabled
    assert!(term.content().contains("Test"));
    assert!(term.synchronized_updates());
}

#[test]
fn test_mouse_event_encoding() {
    let mut term = Terminal::new(80, 24);
    term.set_mouse_mode(MouseMode::Normal);
    term.set_mouse_encoding(MouseEncoding::Sgr);

    let event = MouseEvent::new(0, 10, 5, true, 0);
    let encoded = term.report_mouse(event);

    assert_eq!(encoded, b"\x1b[<0;11;6M");
}

#[test]
fn test_x10_mouse_mode_reports_press_only() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[?9h");
    assert_eq!(term.mouse_mode(), MouseMode::X10);

    // Press reports with the legacy CSI M encoding, modifiers stripped
    let press = term.report_mouse(MouseEvent::new(0, 10, 5, true, 4));
    assert_eq!(&press[0..3], b"\x1b[M");
    assert_eq!(press[3] as i32 - 32, 0, "X10 encodes the raw button");

    // Release and motion produce nothing
    let release = term.report_mouse(MouseEvent::new(0, 10, 5, false, 0));
    assert!(release.is_empty());
    let motion = term.report_mouse(MouseEvent::new(3, 11, 6, true, 0));
    assert!(motion.is_empty());

    // DECRST 9 stops reporting entirely
    term.process(b"\x1b[?9l");
    assert_eq!(term.mouse_mode(), MouseMode::Off);
    assert!(term
        .report_mouse(MouseEvent::new(0, 10, 5, true, 0))
        .is_empty());
}

#[test]
fn test_x10_coexists_with_1000_series() {
    let mut term = Terminal::new(80, 24);

    // 1000 after 9: release is reported again
    term.process(b"\x1b[?9h");
    term.process(b"\x1b[?1000h");
    assert_eq!(term.mouse_mode(), MouseMode::Normal);
    let release = term.report_mouse(MouseEvent::new(0, 10, 5, false, 0));
    assert!(!release.is_empty());

    // 9 after 1000: the most recent set wins
    term.process(b"\x1b[?9h");
    assert_eq!(term.mouse_mode(), MouseMode::X10);
    assert!(term
        .report_mouse(MouseEvent::new(0, 10, 5, false, 0))
        .is_empty());
}

#[test]
fn test_decrqm_reports_mode_9() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[?9$p");
    assert_eq!(term.drain_responses(), b"\x1b[?9;2$y");

    term.process(b"\x1b[?9h");
    term.process(b"\x1b[?9$p");
    assert_eq!(term.drain_responses(), b"\x1b[?9;1$y");
}

/// ARC-009: every DEC private mode the table implements can be set, reset,
/// and reported by DECRQM — no arm exists on one side only. 1048 is a
/// save/restore action rather than a persistent mode, so its label is None
/// and DECRQM reports the pending-restore bit instead of the set/reset
/// status.
#[test]
fn dec_private_modes_set_reset_and_report_symmetrically() {
    use crate::terminal::sequences::csi::mode::DEC_PRIVATE_MODES;

    let mut term = Terminal::new(80, 24);
    assert_eq!(
        term.dec_mode_label(9999),
        None,
        "an unknown mode labels None"
    );
    for &param in DEC_PRIVATE_MODES {
        if param == 1048 {
            assert_eq!(
                term.dec_mode_label(param),
                None,
                "1048 is an action, not a persistent mode"
            );
            continue;
        }
        assert!(
            term.dec_mode_label(param).is_some(),
            "mode {param} must have a label"
        );
        // 2026 (synchronized updates) buffers every following sequence —
        // including the DECRQM reply — until the reset flushes, so its
        // symmetry is asserted through the label instead of the wire.
        if param == 2026 {
            term.process(b"\x1b[?2026h");
            assert_eq!(
                term.dec_mode_label(2026).as_deref(),
                Some("sync_updates:true")
            );
            term.process(b"\x1b[?2026l");
            assert_eq!(
                term.dec_mode_label(2026).as_deref(),
                Some("sync_updates:false")
            );
            term.drain_responses();
            continue;
        }
        term.process(format!("\x1b[?{param}h").as_bytes());
        term.process(format!("\x1b[?{param}$p").as_bytes());
        let reply = term.drain_responses();
        let expected_set = format!("\x1b[?{param};1$y").into_bytes();
        assert_eq!(
            reply, expected_set,
            "DECRQM must report mode {param} as set after DECSET"
        );
        term.process(format!("\x1b[?{param}l").as_bytes());
        term.process(format!("\x1b[?{param}$p").as_bytes());
        let reply = term.drain_responses();
        let expected_reset = format!("\x1b[?{param};2$y").into_bytes();
        assert_eq!(
            reply, expected_reset,
            "DECRQM must report mode {param} as reset after DECRST"
        );
    }
}
