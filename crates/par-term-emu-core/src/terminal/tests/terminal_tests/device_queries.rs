//! Device attribute, status, DECSACE, DECRQM, and answerback replies.

use super::*;

// Device query response tests
#[test]
fn test_da_primary() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[62\"p"); // Force VT220 for predictable DA response
    term.process(b"\x1b[c"); // Primary DA

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[?62;1;4;6;9;15;22;52c");
}

#[test]
fn test_da_primary_with_param() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[62\"p");
    term.process(b"\x1b[0c"); // Primary DA with param 0

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[?62;1;4;6;9;15;22;52c");
}

#[test]
fn test_da_secondary() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[>c"); // Secondary DA

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[>82;10000;0c");
}

#[test]
fn test_da_secondary_with_param() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[>0c"); // Secondary DA with param 0

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[>82;10000;0c");
}

#[test]
fn test_dsr_operating_status() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5n"); // Operating status report

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[0n");
}

#[test]
fn test_dsr_cursor_position() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;20H"); // Move to row 10, col 20
    term.process(b"\x1b[6n"); // Cursor position report

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[10;20R");
}

#[test]
fn test_dsr_cursor_position_origin() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1;1H"); // Move to origin
    term.process(b"\x1b[6n"); // Cursor position report

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[1;1R");
}

#[test]
fn test_dsr_cursor_position_various() {
    let mut term = Terminal::new(80, 24);

    // Test position 5, 10
    term.process(b"\x1b[5;10H\x1b[6n");
    assert_eq!(term.drain_responses(), b"\x1b[5;10R");

    // Test position 1, 1
    term.process(b"\x1b[1;1H\x1b[6n");
    assert_eq!(term.drain_responses(), b"\x1b[1;1R");

    // Test position 24, 80
    term.process(b"\x1b[24;80H\x1b[6n");
    assert_eq!(term.drain_responses(), b"\x1b[24;80R");
}

#[test]
fn test_decreqtparm_solicited() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[0x"); // Solicited DECREQTPARM

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[2;1;1;120;120;1;0x");
}

#[test]
fn test_decreqtparm_unsolicited() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1x"); // Unsolicited DECREQTPARM

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[3;1;1;120;120;1;0x");
}

#[test]
fn test_decsace_selects_extent_and_never_replies() {
    // DECSACE (CSI Ps * x) selects the DECCARA/DECRARA extent. It must never
    // fall through to DECREQTPARM and emit a spurious reply (QA-010).
    let mut term = Terminal::new(80, 24);
    assert_eq!(
        term.modes.attribute_change_extent,
        crate::terminal::AttributeChangeExtent::Rectangle
    );

    term.process(b"\x1b[1*x");
    assert_eq!(
        term.modes.attribute_change_extent,
        crate::terminal::AttributeChangeExtent::Stream
    );

    term.process(b"\x1b[2*x");
    assert_eq!(
        term.modes.attribute_change_extent,
        crate::terminal::AttributeChangeExtent::Rectangle
    );

    assert!(
        term.drain_responses().is_empty(),
        "DECSACE must not emit a reply"
    );

    // 0 behaves as 1 (stream); invalid values are ignored
    term.process(b"\x1b[0*x");
    assert_eq!(
        term.modes.attribute_change_extent,
        crate::terminal::AttributeChangeExtent::Stream
    );
    term.process(b"\x1b[9*x");
    assert_eq!(
        term.modes.attribute_change_extent,
        crate::terminal::AttributeChangeExtent::Stream
    );
}

#[test]
fn test_deccara_respects_decsace_extent() {
    // Rectangle (default): only the addressed area changes
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[2;2;4;4;1$r"); // bold rows 2-4, cols 2-4 (1-based)
    assert!(
        term.active_grid().get(1, 1).unwrap().flags.bold(),
        "inside rect"
    );
    assert!(
        !term.active_grid().get(5, 1).unwrap().flags.bold(),
        "same row, right of rect"
    );
    assert!(
        !term.active_grid().get(0, 0).unwrap().flags.bold(),
        "above rect"
    );

    // Stream: reading order from (2,2) to (4,4) — tail of row 2, all of
    // row 3, head of row 4
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1*x");
    term.process(b"\x1b[2;2;4;4;1$r");
    assert!(
        term.active_grid().get(5, 1).unwrap().flags.bold(),
        "row 2 tail is in the stream"
    );
    assert!(
        term.active_grid().get(0, 2).unwrap().flags.bold(),
        "row 3 is fully in the stream"
    );
    assert!(
        term.active_grid().get(3, 3).unwrap().flags.bold(),
        "bottom-right corner is in both extents"
    );
    assert!(
        !term.active_grid().get(0, 0).unwrap().flags.bold(),
        "before the stream start"
    );
    assert!(
        !term.active_grid().get(4, 3).unwrap().flags.bold(),
        "row 4 right of the stream end"
    );
}

#[test]
fn test_decrara_respects_decsace_extent() {
    // Reverse in stream mode flips cells outside the rectangle too
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1*x");
    term.process(b"\x1b[2;2;4;4;1$t"); // reverse bold rows 2-4, cols 2-4
    assert!(
        term.active_grid().get(5, 1).unwrap().flags.bold(),
        "row 2 tail reversed in stream mode"
    );
}

#[test]
fn test_decsace_decstr_restores_rectangle() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1*x");
    term.process(b"\x1b[!p"); // DECSTR
    assert_eq!(
        term.modes.attribute_change_extent,
        crate::terminal::AttributeChangeExtent::Rectangle
    );
}

#[test]
fn test_decreqtparm_bare_x_still_replies() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[x");
    assert_eq!(term.drain_responses(), b"\x1b[2;1;1;120;120;1;0x");
}

#[test]
fn test_decrqm_application_cursor() {
    let mut term = Terminal::new(80, 24);

    // Query when not set
    term.process(b"\x1b[?1$p");
    assert_eq!(term.drain_responses(), b"\x1b[?1;2$y");

    // Enable application cursor
    term.process(b"\x1b[?1h");

    // Query when set
    term.process(b"\x1b[?1$p");
    assert_eq!(term.drain_responses(), b"\x1b[?1;1$y");
}

#[test]
fn test_decrqm_cursor_visibility() {
    let mut term = Terminal::new(80, 24);

    // Cursor visible by default
    term.process(b"\x1b[?25$p");
    assert_eq!(term.drain_responses(), b"\x1b[?25;1$y");

    // Hide cursor
    term.process(b"\x1b[?25l");

    // Query when hidden
    term.process(b"\x1b[?25$p");
    assert_eq!(term.drain_responses(), b"\x1b[?25;2$y");
}

#[test]
fn test_decrqm_mouse_modes() {
    let mut term = Terminal::new(80, 24);

    // Query mouse mode 1000 (off by default)
    term.process(b"\x1b[?1000$p");
    assert_eq!(term.drain_responses(), b"\x1b[?1000;2$y");

    // Enable normal mouse tracking
    term.process(b"\x1b[?1000h");
    term.process(b"\x1b[?1000$p");
    assert_eq!(term.drain_responses(), b"\x1b[?1000;1$y");

    // Test button event mode
    term.process(b"\x1b[?1002h");
    term.process(b"\x1b[?1002$p");
    assert_eq!(term.drain_responses(), b"\x1b[?1002;1$y");

    // Test any event mode
    term.process(b"\x1b[?1003h");
    term.process(b"\x1b[?1003$p");
    assert_eq!(term.drain_responses(), b"\x1b[?1003;1$y");
}

#[test]
fn test_decrqm_bracketed_paste() {
    let mut term = Terminal::new(80, 24);

    // Query when not set
    term.process(b"\x1b[?2004$p");
    assert_eq!(term.drain_responses(), b"\x1b[?2004;2$y");

    // Enable bracketed paste
    term.process(b"\x1b[?2004h");
    term.process(b"\x1b[?2004$p");
    assert_eq!(term.drain_responses(), b"\x1b[?2004;1$y");
}

#[test]
fn test_decrqm_synchronized_updates() {
    let mut term = Terminal::new(80, 24);

    // Query when not set
    term.process(b"\x1b[?2026$p");
    assert_eq!(term.drain_responses(), b"\x1b[?2026;2$y");

    // Enable synchronized updates
    term.process(b"\x1b[?2026h");
    // Note: Can't query while synchronized mode is active because
    // the query itself gets buffered. This is expected behavior.

    // Disable synchronized updates, then query
    term.process(b"\x1b[?2026l");
    term.process(b"\x1b[?2026$p");
    assert_eq!(term.drain_responses(), b"\x1b[?2026;2$y");
}

#[test]
fn test_decrqm_unrecognized_mode() {
    let mut term = Terminal::new(80, 24);

    // Query unrecognized mode
    term.process(b"\x1b[?9999$p");
    assert_eq!(term.drain_responses(), b"\x1b[?9999;0$y");
}

#[test]
fn test_enq_answerback_string() {
    let mut term = Terminal::new(80, 24);

    // Default: disabled for security, emit nothing
    term.process(b"\x05");
    assert_eq!(term.drain_responses(), b"");
    assert_eq!(term.answerback_string(), None);

    // Configure a custom answerback string
    term.set_answerback_string(Some("par-term".to_string()));
    assert_eq!(term.answerback_string(), Some("par-term"));

    // ENQ should now return the configured answerback payload
    term.process(b"\x05");
    assert_eq!(term.drain_responses(), b"par-term");

    // Disabling clears the response again
    term.set_answerback_string(None);
    term.process(b"\x05");
    assert_eq!(term.drain_responses(), b"");
}

#[test]
fn test_multiple_queries() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[62\"p");

    // Send multiple queries
    term.process(b"\x1b[5n"); // Operating status
    term.process(b"\x1b[6n"); // Cursor position
    term.process(b"\x1b[c"); // Primary DA

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[0n\x1b[1;1R\x1b[?62;1;4;6;9;15;22;52c");

    // After draining, buffer should be empty
    assert!(!term.has_pending_responses());
}

#[test]
fn test_response_buffer_operations() {
    let mut term = Terminal::new(80, 24);

    // No responses initially
    assert!(!term.has_pending_responses());

    // Generate a response
    term.process(b"\x1b[5n");
    assert!(term.has_pending_responses());

    // Drain responses
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[0n");
    assert!(!term.has_pending_responses());

    // Draining again should return empty
    let response = term.drain_responses();
    assert_eq!(response, b"");
}
