//! XTMODKEYS modifyOtherKeys mode handling.

use super::*;

// === modifyOtherKeys Protocol Tests ===

#[test]
fn test_modify_other_keys_initial_state() {
    let term = Terminal::new(80, 24);
    // Initial state should be 0 (disabled)
    assert_eq!(term.modify_other_keys_mode(), 0);
}

#[test]
fn test_modify_other_keys_mode_setting() {
    let mut term = Terminal::new(80, 24);

    // Set mode 1: CSI > 4 ; 1 m
    term.process(b"\x1b[>4;1m");
    assert_eq!(term.modify_other_keys_mode(), 1);

    // Set mode 2: CSI > 4 ; 2 m
    term.process(b"\x1b[>4;2m");
    assert_eq!(term.modify_other_keys_mode(), 2);

    // Set mode 0 (disable): CSI > 4 ; 0 m
    term.process(b"\x1b[>4;0m");
    assert_eq!(term.modify_other_keys_mode(), 0);
}

#[test]
fn test_modify_other_keys_mode_clamping() {
    let mut term = Terminal::new(80, 24);

    // Mode values > 2 should be clamped to 2
    term.process(b"\x1b[>4;5m");
    assert_eq!(term.modify_other_keys_mode(), 2);

    term.process(b"\x1b[>4;255m");
    assert_eq!(term.modify_other_keys_mode(), 2);
}

#[test]
fn test_modify_other_keys_query() {
    let mut term = Terminal::new(80, 24);

    // Query when mode is 0
    term.process(b"\x1b[?4m");
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[>4;0m");

    // Set mode 1 and query
    term.process(b"\x1b[>4;1m");
    term.process(b"\x1b[?4m");
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[>4;1m");

    // Set mode 2 and query
    term.process(b"\x1b[>4;2m");
    term.process(b"\x1b[?4m");
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[>4;2m");
}

#[test]
fn test_modify_other_keys_reset() {
    let mut term = Terminal::new(80, 24);

    // Enable mode 2
    term.process(b"\x1b[>4;2m");
    assert_eq!(term.modify_other_keys_mode(), 2);

    // Reset terminal
    term.reset();
    assert_eq!(term.modify_other_keys_mode(), 0);
}

#[test]
fn test_modify_other_keys_alt_screen_reset() {
    let mut term = Terminal::new(80, 24);

    // Enable mode 2
    term.process(b"\x1b[>4;2m");
    assert_eq!(term.modify_other_keys_mode(), 2);

    // Enter alternate screen
    term.process(b"\x1b[?1049h");
    assert!(term.is_alt_screen_active());
    // Mode should persist when entering alt screen
    assert_eq!(term.modify_other_keys_mode(), 2);

    // Exit alternate screen - mode should be reset
    term.process(b"\x1b[?1049l");
    assert!(!term.is_alt_screen_active());
    assert_eq!(term.modify_other_keys_mode(), 0);
}

#[test]
fn test_modify_other_keys_direct_setter() {
    let mut term = Terminal::new(80, 24);

    // Test direct setter method
    term.set_modify_other_keys_mode(1);
    assert_eq!(term.modify_other_keys_mode(), 1);

    term.set_modify_other_keys_mode(2);
    assert_eq!(term.modify_other_keys_mode(), 2);

    // Direct setter should also clamp
    term.set_modify_other_keys_mode(10);
    assert_eq!(term.modify_other_keys_mode(), 2);
}

#[test]
fn test_modify_other_keys_sgr_unaffected() {
    let mut term = Terminal::new(80, 24);

    // Enable modifyOtherKeys mode 2
    term.process(b"\x1b[>4;2m");
    assert_eq!(term.modify_other_keys_mode(), 2);

    // Regular SGR should still work (no '>' intermediate)
    term.process(b"\x1b[31m"); // Set red foreground
                               // Write a character to see the color
    term.process(b"X");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.fg(), Color::Named(NamedColor::Red));

    // modifyOtherKeys should still be mode 2
    assert_eq!(term.modify_other_keys_mode(), 2);
}

#[test]
fn test_modify_other_keys_ignored_for_other_params() {
    let mut term = Terminal::new(80, 24);

    // CSI > with param other than 4 should be ignored (for this feature)
    // This tests that we don't accidentally set mode for other params
    term.process(b"\x1b[>5;2m");
    assert_eq!(term.modify_other_keys_mode(), 0);

    term.process(b"\x1b[>0;1m");
    assert_eq!(term.modify_other_keys_mode(), 0);
}
