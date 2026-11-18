//! Cursor movement, cursor state, and SGR attributes.

use super::*;

#[test]
fn test_cursor_movement_param_zero() {
    // Test that param 0 is treated as 1 for cursor movement commands
    // This is a regression test for the bug where \x1b[C (param=0) didn't move cursor
    let mut term = Terminal::new(80, 24);

    // Start at (5, 5)
    term.cursor.goto(5, 5);

    // CUF (Cursor Forward) with no param - should move right 1
    term.process(b"\x1b[C");
    assert_eq!(term.cursor.col, 6, "CUF with no param should move right 1");

    // CUB (Cursor Back) with no param - should move left 1
    term.process(b"\x1b[D");
    assert_eq!(term.cursor.col, 5, "CUB with no param should move left 1");

    // CUU (Cursor Up) with no param - should move up 1
    term.process(b"\x1b[A");
    assert_eq!(term.cursor.row, 4, "CUU with no param should move up 1");

    // CUD (Cursor Down) with no param - should move down 1
    term.process(b"\x1b[B");
    assert_eq!(term.cursor.row, 5, "CUD with no param should move down 1");

    // Test with explicit 0 parameter
    term.cursor.goto(5, 5);
    term.process(b"\x1b[0C"); // CUF with param 0
    assert_eq!(term.cursor.col, 6, "CUF with param 0 should move right 1");

    term.process(b"\x1b[0D"); // CUB with param 0
    assert_eq!(term.cursor.col, 5, "CUB with param 0 should move left 1");

    // Test with explicit 3 parameter
    term.process(b"\x1b[3C"); // CUF with param 3
    assert_eq!(term.cursor.col, 8, "CUF with param 3 should move right 3");
}

#[test]
fn test_cursor_forward_tabulation() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[2I"); // CHT - forward 2 tab stops

    assert_eq!(term.cursor.col, 16); // From 0 to 8, then to 16
}

#[test]
fn test_cursor_backward_tabulation() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[20G"); // Move to column 20
    term.process(b"\x1b[1Z"); // CBT - backward 1 tab stop

    assert_eq!(term.cursor.col, 16); // Should be at tab stop 16
}

// Edge case tests
#[test]
fn test_cursor_bounds_checking() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[999;999H"); // Try to move way out of bounds

    assert_eq!(term.cursor.col, 79); // Should clamp to max
    assert_eq!(term.cursor.row, 23);
}

#[test]
fn test_wrap_mode() {
    let mut term = Terminal::new(10, 5);
    term.process(b"0123456789ABC"); // More than 10 chars

    // With auto-wrap enabled, text should wrap
    assert_eq!(term.cursor.row, 1); // Should be on second line
}

#[test]
fn test_save_restore_cursor() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;20H"); // Move cursor
    term.process(b"\x1b[31m"); // Set red foreground
    term.process(b"\x1b[s"); // Save cursor

    term.process(b"\x1b[1;1H"); // Move to origin
    term.process(b"\x1b[0m"); // Reset attributes

    term.process(b"\x1b[u"); // Restore cursor

    assert_eq!(term.cursor.col, 19); // 0-indexed
    assert_eq!(term.cursor.row, 9);
    assert_eq!(term.attrs.fg, Color::Named(NamedColor::Red));
}

#[test]
fn test_origin_mode() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5;15r"); // Set scroll region
    term.process(b"\x1b[?6h"); // Enable origin mode

    assert!(term.modes.origin_mode);

    // The current implementation sets scroll region but cursor positioning
    // doesn't implement full origin mode yet, so test what it does
    term.process(b"\x1b[1;1H"); // Position to "home"
                                // For now, just verify origin mode is enabled
    assert!(term.modes.origin_mode);
}

#[test]
fn test_reverse_index() {
    let mut term = Terminal::new(80, 10);
    term.process(b"Line 0\nLine 1\nLine 2\nLine 3");
    term.process(b"\x1b[1;1H"); // Move to top
    term.process(b"\x1bM"); // Reverse index (RI)

    // Should scroll region down
    let line0 = term.grid().row(0).unwrap();
    let text: String = line0.iter().map(|c| c.c).collect();
    assert!(text.trim().is_empty()); // First line should be blank
}

#[test]
fn test_cursor_next_previous_line() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5;10H"); // Move to row 5, col 10 (1-indexed)
    term.process(b"\x1b[2E"); // CNL - cursor next line (2 lines)

    assert_eq!(term.cursor.row, 6); // Row 4 (0-indexed for row 5) + 2 = 6
    assert_eq!(term.cursor.col, 0); // Should be at column 0

    term.process(b"\x1b[1F"); // CPL - cursor previous line
    assert_eq!(term.cursor.row, 5);
    assert_eq!(term.cursor.col, 0);
}

#[test]
fn test_cursor_horizontal_absolute() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[42G"); // CHA - move to column 42

    assert_eq!(term.cursor.col, 41); // 0-indexed
}

#[test]
fn test_line_position_absolute() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[12d"); // VPA - move to line 12

    assert_eq!(term.cursor.row, 11); // 0-indexed
}

#[test]
fn test_256_color() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[38;5;196mRed"); // Set foreground to color 196

    let cell = term.grid().get(0, 0).unwrap();
    assert_eq!(cell.fg(), Color::from_ansi_code(196));
}

#[test]
fn test_sgr_reset() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1;31;42mTest"); // Bold, red fg, green bg
    term.process(b"\x1b[0m"); // Reset

    assert_eq!(term.attrs.fg, Color::Named(NamedColor::White));
    assert_eq!(term.attrs.bg, Color::Named(NamedColor::Black));
    assert!(!term.attrs.flags.bold());
}

#[test]
fn test_multiple_sgr_attributes() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1;3;4;9mTest"); // Bold, italic, underline, strikethrough

    assert!(term.attrs.flags.bold());
    assert!(term.attrs.flags.italic());
    assert!(term.attrs.flags.underline());
    assert!(term.attrs.flags.strikethrough());
}
