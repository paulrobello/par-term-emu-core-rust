// Unit tests for Terminal capture_snapshot / restore_from_snapshot
//
// These tests have access to private fields and methods of Terminal.
// Included via include!() macro in terminal/mod.rs test module.

#[test]
fn test_capture_basic_snapshot() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello, world!");

    let snap = term.capture_snapshot();
    assert_eq!(snap.cols, 80);
    assert_eq!(snap.rows, 24);
    assert!(snap.timestamp > 0);
    assert!(snap.estimated_size_bytes > 0);
    assert!(!snap.alt_screen_active);

    // Check the grid captured the text
    assert_eq!(snap.grid.cells[0].c, 'H');
    assert_eq!(snap.grid.cells[1].c, 'e');
    assert_eq!(snap.grid.cells[4].c, 'o');
}

#[test]
fn test_roundtrip_restore_cells() {
    let mut term = Terminal::new(20, 5);
    term.process(b"ABCDE");

    let snap = term.capture_snapshot();

    // Verify initial state
    assert_eq!(term.grid().get(0, 0).unwrap().c, 'A');
    assert_eq!(term.grid().get(4, 0).unwrap().c, 'E');

    // Overwrite the terminal content
    term.process(b"\x1b[H"); // Move cursor to home
    term.process(b"ZZZZZ");

    assert_eq!(term.grid().get(0, 0).unwrap().c, 'Z');

    // Restore from snapshot
    term.restore_from_snapshot(&snap);

    assert_eq!(term.grid().get(0, 0).unwrap().c, 'A');
    assert_eq!(term.grid().get(1, 0).unwrap().c, 'B');
    assert_eq!(term.grid().get(2, 0).unwrap().c, 'C');
    assert_eq!(term.grid().get(3, 0).unwrap().c, 'D');
    assert_eq!(term.grid().get(4, 0).unwrap().c, 'E');
}

#[test]
fn test_snapshot_color_preservation() {
    let mut term = Terminal::new(40, 10);

    // Write red text: SGR 31 sets foreground red
    term.process(b"\x1b[31mRed\x1b[0m");
    // Write with RGB color: SGR 38;2;0;255;0 sets foreground green RGB
    term.process(b"\x1b[38;2;0;255;0mGrn\x1b[0m");

    let snap = term.capture_snapshot();

    // 'R' should have Named(Red) foreground
    assert_eq!(snap.grid.cells[0].c, 'R');
    assert_eq!(snap.grid.cells[0].fg, Color::Named(NamedColor::Red));

    // 'G' should have RGB(0,255,0) foreground
    assert_eq!(snap.grid.cells[3].c, 'G');
    assert_eq!(snap.grid.cells[3].fg, Color::Rgb(0, 255, 0));

    // Overwrite and restore
    term.process(b"\x1b[H\x1b[0mXXXXXX");
    term.restore_from_snapshot(&snap);

    assert_eq!(term.grid().get(0, 0).unwrap().c, 'R');
    assert_eq!(
        term.grid().get(0, 0).unwrap().fg,
        Color::Named(NamedColor::Red)
    );
    assert_eq!(term.grid().get(3, 0).unwrap().c, 'G');
    assert_eq!(
        term.grid().get(3, 0).unwrap().fg,
        Color::Rgb(0, 255, 0)
    );
}

#[test]
fn test_snapshot_scrollback() {
    let mut term = Terminal::with_scrollback(20, 5, 100);

    // Fill more than 5 lines to push content into scrollback
    for i in 0..10 {
        term.process(format!("Line {}\r\n", i).as_bytes());
    }

    let sb_before = term.grid().scrollback_len();
    assert!(sb_before > 0, "should have scrollback");

    let snap = term.capture_snapshot();

    // Write more to change scrollback
    for i in 10..20 {
        term.process(format!("Extra {}\r\n", i).as_bytes());
    }

    let sb_after = term.grid().scrollback_len();
    assert!(sb_after > sb_before, "scrollback should have grown");

    // Restore
    term.restore_from_snapshot(&snap);
    assert_eq!(
        term.grid().scrollback_len(),
        sb_before,
        "scrollback should be restored"
    );
}

#[test]
fn test_snapshot_alt_screen() {
    let mut term = Terminal::new(20, 5);
    term.process(b"Primary");

    // Switch to alt screen
    term.process(b"\x1b[?1049h");
    assert!(term.alt_screen_active);
    term.process(b"Alt");

    let snap = term.capture_snapshot();
    assert!(snap.alt_screen_active);

    // self.grid is always the primary grid, self.alt_grid is always the alt grid
    assert_eq!(snap.grid.cells[0].c, 'P'); // primary grid preserved
    assert_eq!(snap.alt_grid.cells[0].c, 'A'); // alt grid has 'Alt'

    // Switch back to primary and overwrite
    term.process(b"\x1b[?1049l");
    term.process(b"\x1b[HXXXXX");

    assert!(!term.alt_screen_active);

    // Restore snapshot (which was in alt screen mode)
    term.restore_from_snapshot(&snap);

    assert!(term.alt_screen_active);
    // Primary grid (self.grid) should have 'P'
    assert_eq!(term.grid().get(0, 0).unwrap().c, 'P');
    // Alt grid (self.alt_grid) should have 'A', accessed via active_grid()
    assert_eq!(term.active_grid().get(0, 0).unwrap().c, 'A');
    assert_eq!(term.alt_grid().get(0, 0).unwrap().c, 'A');
}

#[test]
fn test_snapshot_mode_flags() {
    let mut term = Terminal::new(20, 5);

    // Set various modes
    term.process(b"\x1b[?7l"); // Disable auto-wrap (DECAWM)
    term.process(b"\x1b[?6h"); // Enable origin mode (DECOM)
    term.process(b"\x1b[4h"); // Enable insert mode (IRM)
    term.process(b"\x1b[?2004h"); // Enable bracketed paste
    term.process(b"\x1b[?1004h"); // Enable focus tracking
    term.process(b"\x1b[?1h"); // Enable application cursor keys

    let snap = term.capture_snapshot();
    assert!(!snap.auto_wrap);
    assert!(snap.origin_mode);
    assert!(snap.insert_mode);
    assert!(snap.bracketed_paste);
    assert!(snap.focus_tracking);
    assert!(snap.application_cursor);

    // Reset all modes
    term.process(b"\x1b[?7h"); // Re-enable auto-wrap
    term.process(b"\x1b[?6l"); // Disable origin mode
    term.process(b"\x1b[4l"); // Disable insert mode
    term.process(b"\x1b[?2004l"); // Disable bracketed paste
    term.process(b"\x1b[?1004l"); // Disable focus tracking
    term.process(b"\x1b[?1l"); // Disable application cursor keys

    assert!(term.auto_wrap);
    assert!(!term.origin_mode);
    assert!(!term.insert_mode);
    assert!(!term.bracketed_paste);
    assert!(!term.focus_tracking);
    assert!(!term.application_cursor);

    // Restore from snapshot
    term.restore_from_snapshot(&snap);

    assert!(!term.auto_wrap);
    assert!(term.origin_mode);
    assert!(term.insert_mode);
    assert!(term.bracketed_paste);
    assert!(term.focus_tracking);
    assert!(term.application_cursor);
}

#[test]
fn test_snapshot_cursor_state() {
    let mut term = Terminal::new(40, 10);
    term.process(b"\x1b[5;10H"); // Move cursor to row 5, col 10 (1-indexed)

    let snap = term.capture_snapshot();
    assert_eq!(snap.cursor.row, 4); // 0-indexed
    assert_eq!(snap.cursor.col, 9); // 0-indexed

    // Move cursor elsewhere
    term.process(b"\x1b[1;1H");
    assert_eq!(term.cursor.row, 0);
    assert_eq!(term.cursor.col, 0);

    // Restore
    term.restore_from_snapshot(&snap);
    assert_eq!(term.cursor.row, 4);
    assert_eq!(term.cursor.col, 9);
}

#[test]
fn test_snapshot_title() {
    let mut term = Terminal::new(40, 10);
    term.process(b"\x1b]0;My Title\x07"); // OSC 0: set title

    let snap = term.capture_snapshot();
    assert_eq!(snap.title, "My Title");

    // Change title
    term.process(b"\x1b]0;Changed\x07");
    assert_eq!(term.title, "Changed");

    // Restore
    term.restore_from_snapshot(&snap);
    assert_eq!(term.title, "My Title");
}
