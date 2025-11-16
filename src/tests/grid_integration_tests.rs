// Integration tests for grid operations through the Terminal API
//
// These tests verify grid manipulation operations (erase, insert, delete, scroll)
// by using only the public Terminal API and checking terminal state afterwards.

// ========== Erase Operations ==========

#[test]
fn test_erase_in_display_from_cursor() {
    let mut term = Terminal::new(80, 24);

    // Fill screen with 'X'
    for _ in 0..24 {
        for _ in 0..80 {
            term.process(b"X");
        }
    }

    // Move to position (10, 5) - row 5, col 10 (1-indexed)
    term.process(b"\x1b[6;11H");

    // ED 0 - Erase from cursor to end of display
    term.process(b"\x1b[J");

    // Check cursor position unchanged
    assert_eq!(term.cursor.row, 5); // 0-indexed
    assert_eq!(term.cursor.col, 10);

    // Check cells before cursor are preserved
    let grid = term.active_grid();
    assert_eq!(grid.get(0, 0).map(|c| c.c), Some('X'));
    assert_eq!(grid.get(0, 5).map(|c| c.c), Some('X'));
    assert_eq!(grid.get(9, 5).map(|c| c.c), Some('X'));

    // Check cells at and after cursor are erased
    assert_eq!(grid.get(10, 5).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(79, 5).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(0, 6).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(0, 23).map(|c| c.c), Some(' '));
}

#[test]
fn test_erase_in_display_to_cursor() {
    let mut term = Terminal::new(80, 24);

    // Fill screen with 'Y'
    for _ in 0..24 * 80 {
        term.process(b"Y");
    }

    // Move to position (10, 5)
    term.process(b"\x1b[6;11H");

    // ED 1 - Erase from start to cursor
    term.process(b"\x1b[1J");

    let grid = term.active_grid();

    // Check cells before and at cursor are erased
    assert_eq!(grid.get(0, 0).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(79, 4).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(10, 5).map(|c| c.c), Some(' '));

    // Check cells after cursor are preserved
    assert_eq!(grid.get(11, 5).map(|c| c.c), Some('Y'));
    assert_eq!(grid.get(79, 23).map(|c| c.c), Some('Y'));
}

#[test]
fn test_erase_in_display_all() {
    let mut term = Terminal::new(80, 24);

    // Fill screen with 'Z'
    for _ in 0..24 * 80 {
        term.process(b"Z");
    }

    // Move to middle
    term.process(b"\x1b[12;40H");

    // ED 2 - Erase entire display
    term.process(b"\x1b[2J");

    let grid = term.active_grid();

    // All cells should be erased
    assert_eq!(grid.get(0, 0).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(39, 11).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(79, 23).map(|c| c.c), Some(' '));

    // Cursor should stay in place
    assert_eq!(term.cursor.row, 11);
    assert_eq!(term.cursor.col, 39);
}

#[test]
fn test_erase_in_line_from_cursor() {
    let mut term = Terminal::new(80, 24);

    // Fill a line with 'A'
    term.process(b"\x1b[10H"); // Row 10
    for _ in 0..80 {
        term.process(b"A");
    }

    // Move to column 30
    term.process(b"\x1b[10;31H");

    // EL 0 - Erase from cursor to end of line
    term.process(b"\x1b[K");

    let grid = term.active_grid();

    // Check before cursor preserved
    assert_eq!(grid.get(0, 9).map(|c| c.c), Some('A'));
    assert_eq!(grid.get(29, 9).map(|c| c.c), Some('A'));

    // Check at and after cursor erased
    assert_eq!(grid.get(30, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(79, 9).map(|c| c.c), Some(' '));
}

#[test]
fn test_erase_in_line_to_cursor() {
    let mut term = Terminal::new(80, 24);

    // Fill a line with 'B'
    term.process(b"\x1b[10H");
    for _ in 0..80 {
        term.process(b"B");
    }

    // Move to column 30
    term.process(b"\x1b[10;31H");

    // EL 1 - Erase from start to cursor
    term.process(b"\x1b[1K");

    let grid = term.active_grid();

    // Check up to and including cursor erased
    assert_eq!(grid.get(0, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(30, 9).map(|c| c.c), Some(' '));

    // Check after cursor preserved
    assert_eq!(grid.get(31, 9).map(|c| c.c), Some('B'));
    assert_eq!(grid.get(79, 9).map(|c| c.c), Some('B'));
}

#[test]
fn test_erase_in_line_all() {
    let mut term = Terminal::new(80, 24);

    // Fill a line with 'C'
    term.process(b"\x1b[10H");
    for _ in 0..80 {
        term.process(b"C");
    }

    // Move to middle
    term.process(b"\x1b[10;40H");

    // EL 2 - Erase entire line
    term.process(b"\x1b[2K");

    let grid = term.active_grid();

    // Entire line should be erased
    assert_eq!(grid.get(0, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(39, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(79, 9).map(|c| c.c), Some(' '));

    // Other lines should be unaffected
    if let Some(cell) = grid.get(0, 8) {
        assert_eq!(cell.c, ' '); // Never written to
    }
}

// ========== Insert/Delete Character Operations ==========

#[test]
fn test_insert_characters_grid() {
    let mut term = Terminal::new(80, 24);

    // Write "ABCDEF" on row 10
    term.process(b"\x1b[10H");
    term.process(b"ABCDEF");

    // Move cursor to 'C' (column 3, 1-indexed column 4)
    term.process(b"\x1b[10;4H");

    // Insert 3 spaces (ICH)
    term.process(b"\x1b[3@");

    let grid = term.active_grid();

    // Check result: "ABC   DEF"
    assert_eq!(grid.get(0, 9).map(|c| c.c), Some('A'));
    assert_eq!(grid.get(1, 9).map(|c| c.c), Some('B'));
    assert_eq!(grid.get(2, 9).map(|c| c.c), Some('C'));
    assert_eq!(grid.get(3, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(4, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(5, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(6, 9).map(|c| c.c), Some('D'));
    assert_eq!(grid.get(7, 9).map(|c| c.c), Some('E'));
    assert_eq!(grid.get(8, 9).map(|c| c.c), Some('F'));
}

#[test]
fn test_delete_characters_grid() {
    let mut term = Terminal::new(80, 24);

    // Write "ABCDEFGH" on row 10
    term.process(b"\x1b[10H");
    term.process(b"ABCDEFGH");

    // Move cursor to 'E' (column 4, 1-indexed column 5)
    term.process(b"\x1b[10;5H");

    // Delete 2 characters (DCH) - deletes 'E' and 'F'
    term.process(b"\x1b[2P");

    let grid = term.active_grid();

    // Check result: "ABCDGH  " (E and F deleted, G and H shifted left)
    assert_eq!(grid.get(0, 9).map(|c| c.c), Some('A'));
    assert_eq!(grid.get(1, 9).map(|c| c.c), Some('B'));
    assert_eq!(grid.get(2, 9).map(|c| c.c), Some('C'));
    assert_eq!(grid.get(3, 9).map(|c| c.c), Some('D'));
    assert_eq!(grid.get(4, 9).map(|c| c.c), Some('G')); // 'E' and 'F' deleted
    assert_eq!(grid.get(5, 9).map(|c| c.c), Some('H'));
    assert_eq!(grid.get(6, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(7, 9).map(|c| c.c), Some(' '));
}

// ========== Insert/Delete Line Operations ==========

#[test]
fn test_insert_lines_grid() {
    let mut term = Terminal::new(80, 24);

    // Write "Line 1" through "Line 5"
    for i in 1..=5 {
        term.process(format!("\x1b[{}HLine {}", i, i).as_bytes());
    }

    // Move to row 3
    term.process(b"\x1b[3H");

    // Insert 2 blank lines (IL)
    term.process(b"\x1b[2L");

    let grid = term.active_grid();

    // Row 1 and 2 should be unchanged
    assert!(grid.row_text(0).trim_end().starts_with("Line 1"));
    assert!(grid.row_text(1).trim_end().starts_with("Line 2"));

    // Rows 3 and 4 should be blank
    assert_eq!(grid.row_text(2).trim(), "");
    assert_eq!(grid.row_text(3).trim(), "");

    // Original row 3, 4, 5 should be pushed down
    assert!(grid.row_text(4).trim_end().starts_with("Line 3"));
    assert!(grid.row_text(5).trim_end().starts_with("Line 4"));
    assert!(grid.row_text(6).trim_end().starts_with("Line 5"));
}

#[test]
fn test_delete_lines_grid() {
    let mut term = Terminal::new(80, 24);

    // Write "Line 1" through "Line 8"
    for i in 1..=8 {
        term.process(format!("\x1b[{}HLine {}", i, i).as_bytes());
    }

    // Move to row 3
    term.process(b"\x1b[3H");

    // Delete 2 lines (DL)
    term.process(b"\x1b[2M");

    let grid = term.active_grid();

    // Rows 1 and 2 unchanged
    assert!(grid.row_text(0).trim_end().starts_with("Line 1"));
    assert!(grid.row_text(1).trim_end().starts_with("Line 2"));

    // Row 3 should now have Line 5 (Line 3 and 4 deleted)
    assert!(grid.row_text(2).trim_end().starts_with("Line 5"));
    assert!(grid.row_text(3).trim_end().starts_with("Line 6"));
    assert!(grid.row_text(4).trim_end().starts_with("Line 7"));
    assert!(grid.row_text(5).trim_end().starts_with("Line 8"));
}

// ========== Character Writing and Wrap ==========

#[test]
fn test_character_wrap_enabled() {
    let mut term = Terminal::new(10, 5); // Small terminal

    // Autowrap is enabled by default
    assert!(term.auto_wrap);

    // Write 15 characters (should wrap to next line)
    term.process(b"ABCDEFGHIJKLMNO");

    let grid = term.active_grid();

    // First 10 chars on row 0
    assert_eq!(grid.row_text(0).trim_end(), "ABCDEFGHIJ");

    // Next 5 chars on row 1
    assert!(grid.row_text(1).trim_end().starts_with("KLMNO"));

    // Cursor should be at column 5 (0-indexed) of row 1
    assert_eq!(term.cursor.row, 1);
    assert_eq!(term.cursor.col, 5);
}

#[test]
fn test_character_wrap_disabled() {
    let mut term = Terminal::new(10, 5);

    // Disable autowrap
    term.process(b"\x1b[?7l");
    assert!(!term.auto_wrap);

    // Move to start
    term.process(b"\x1b[H");

    // Write 15 characters (should NOT wrap)
    term.process(b"ABCDEFGHIJKLMNO");

    let grid = term.active_grid();

    // Only last 10 characters visible on row 0 (overwriting)
    // The last character 'O' should be at the last column
    assert_eq!(grid.get(9, 0).map(|c| c.c), Some('O'));

    // Row 1 should be empty
    assert_eq!(grid.row_text(1).trim(), "");

    // Cursor stays at last column
    assert_eq!(term.cursor.row, 0);
    assert_eq!(term.cursor.col, 9); // Last column (0-indexed)
}

// ========== Tab Operations ==========

#[test]
fn test_tab_forward_grid() {
    let mut term = Terminal::new(80, 24);

    // Move to start
    term.process(b"\x1b[H");

    // Write "A", tab, "B"
    term.process(b"A\tB");

    let grid = term.active_grid();

    // 'A' at column 0
    assert_eq!(grid.get(0, 0).map(|c| c.c), Some('A'));

    // 'B' should be at next tab stop (default every 8 columns)
    assert_eq!(grid.get(8, 0).map(|c| c.c), Some('B'));

    // Cursor should be after 'B'
    assert_eq!(term.cursor.col, 9);
}

#[test]
fn test_backtab() {
    let mut term = Terminal::new(80, 24);

    // Move to column 20
    term.process(b"\x1b[1;21H");

    // Backtab (CBT) - move back 1 tab stop
    term.process(b"\x1b[Z");

    // Should move to previous tab stop at column 16 (0-indexed)
    assert_eq!(term.cursor.col, 16);
}

// ========== Scroll Operations ==========

#[test]
fn test_scroll_up_full_screen() {
    let mut term = Terminal::new(80, 5);

    // Fill screen with numbered lines
    for i in 1..=5 {
        term.process(format!("\x1b[{}HLine {}", i, i).as_bytes());
    }

    // Move to bottom
    term.process(b"\x1b[5H");

    // Scroll up 2 lines (SU)
    term.process(b"\x1b[2S");

    let grid = term.active_grid();

    // First 2 lines should be gone, replaced by blank lines at bottom
    assert!(grid.row_text(0).trim_end().starts_with("Line 3"));
    assert!(grid.row_text(1).trim_end().starts_with("Line 4"));
    assert!(grid.row_text(2).trim_end().starts_with("Line 5"));
    assert_eq!(grid.row_text(3).trim(), "");
    assert_eq!(grid.row_text(4).trim(), "");
}

#[test]
fn test_scroll_down_full_screen() {
    let mut term = Terminal::new(80, 5);

    // Fill screen
    for i in 1..=5 {
        term.process(format!("\x1b[{}HLine {}", i, i).as_bytes());
    }

    // Scroll down 2 lines (SD)
    term.process(b"\x1b[2T");

    let grid = term.active_grid();

    // First 2 lines should be blank
    assert_eq!(grid.row_text(0).trim(), "");
    assert_eq!(grid.row_text(1).trim(), "");

    // Original lines shifted down
    assert!(grid.row_text(2).trim_end().starts_with("Line 1"));
    assert!(grid.row_text(3).trim_end().starts_with("Line 2"));
    assert!(grid.row_text(4).trim_end().starts_with("Line 3"));
}

// ========== Erase Character Operation ==========

#[test]
fn test_erase_characters_grid() {
    let mut term = Terminal::new(80, 24);

    // Write "ABCDEFGH"
    term.process(b"\x1b[10H");
    term.process(b"ABCDEFGH");

    // Move to 'D' (column 4, 1-indexed column 5)
    term.process(b"\x1b[10;5H");

    // Erase 3 characters (ECH) - replaces with spaces, doesn't shift
    term.process(b"\x1b[3X");

    let grid = term.active_grid();

    // Check result: "ABCD   H" (DEF replaced with spaces)
    assert_eq!(grid.get(0, 9).map(|c| c.c), Some('A'));
    assert_eq!(grid.get(1, 9).map(|c| c.c), Some('B'));
    assert_eq!(grid.get(2, 9).map(|c| c.c), Some('C'));
    assert_eq!(grid.get(3, 9).map(|c| c.c), Some('D'));
    assert_eq!(grid.get(4, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(5, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(6, 9).map(|c| c.c), Some(' '));
    assert_eq!(grid.get(7, 9).map(|c| c.c), Some('H'));
}
