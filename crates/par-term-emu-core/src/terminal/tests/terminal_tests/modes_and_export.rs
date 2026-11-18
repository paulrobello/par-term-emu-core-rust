//! Insert/LNM modes, title stack, and text export.

use super::*;

#[test]
fn test_insert_mode() {
    let mut term = Terminal::new(80, 24);

    // Write some initial text
    term.process(b"Hello");
    assert_eq!(term.content().trim_end(), "Hello");

    // Move cursor to column 2 (after 'H')
    term.process(b"\x1b[1;2H");

    // Enable insert mode (IRM)
    term.process(b"\x1b[4h");
    assert!(term.insert_mode());

    // Write "XX" - should insert, not replace
    term.process(b"XX");

    // Should be "HXXello" (inserted XX after H)
    let content = term.content();
    let content = content.trim_end();
    assert!(content.starts_with("HXXello"), "Got: {}", content);

    // Disable insert mode
    term.process(b"\x1b[4l");
    assert!(!term.insert_mode());

    // Move to column 4
    term.process(b"\x1b[1;4H");

    // Write "YY" - should replace, not insert
    term.process(b"YY");

    // Should be "HXXYYlo" (replaced XX with YY)
    let content = term.content();
    let content = content.trim_end();
    assert!(content.starts_with("HXXYYlo"), "Got: {}", content);
}

#[test]
fn test_line_feed_new_line_mode() {
    let mut term = Terminal::new(80, 24);

    // Write text and move cursor to column 5
    term.process(b"Hello");
    assert_eq!(term.cursor.col, 5);
    assert_eq!(term.cursor.row, 0);

    // LF without LNM - should move down but stay in same column
    term.process(b"\n");
    assert_eq!(term.cursor.col, 5); // Same column
    assert_eq!(term.cursor.row, 1); // Next row

    // Reset and test with LNM enabled
    term.reset();
    term.process(b"Hello");
    assert_eq!(term.cursor.col, 5);
    assert_eq!(term.cursor.row, 0);

    // Enable LNM
    term.process(b"\x1b[20h");
    assert!(term.line_feed_new_line_mode());

    // LF with LNM - should move down AND to column 0 (CR+LF)
    term.process(b"\n");
    assert_eq!(term.cursor.col, 0); // Column 0 (CR happened)
    assert_eq!(term.cursor.row, 1); // Next row (LF happened)

    // Disable LNM
    term.process(b"\x1b[20l");
    assert!(!term.line_feed_new_line_mode());
}

#[test]
fn test_xtwinops_title_stack() {
    let mut term = Terminal::new(80, 24);

    // Set initial title
    term.process(b"\x1b]0;Title1\x07");
    assert_eq!(term.title(), "Title1");

    // Push title to stack (XTWINOPS 22)
    term.process(b"\x1b[22t");

    // Change title
    term.process(b"\x1b]0;Title2\x07");
    assert_eq!(term.title(), "Title2");

    // Push again
    term.process(b"\x1b[22t");

    // Change title again
    term.process(b"\x1b]0;Title3\x07");
    assert_eq!(term.title(), "Title3");

    // Pop title (XTWINOPS 23) - should restore Title2
    term.process(b"\x1b[23t");
    assert_eq!(term.title(), "Title2");

    // Pop again - should restore Title1
    term.process(b"\x1b[23t");
    assert_eq!(term.title(), "Title1");

    // Pop from empty stack - title should remain Title1
    term.process(b"\x1b[23t");
    assert_eq!(term.title(), "Title1");
}

#[test]
fn test_insert_mode_with_wide_chars() {
    let mut term = Terminal::new(80, 24);

    // Write some text
    term.process(b"Hello");

    // Move to column 2
    term.process(b"\x1b[1;2H");

    // Enable insert mode
    term.process(b"\x1b[4h");

    // Insert a wide character (emoji)
    term.process("🦀".as_bytes());

    // Should insert the wide char (2 cells), shifting "ello" to the right by 2 columns
    // This results in "H🦀 ello" with an extra space
    let content = term.content();
    let content = content.trim_end();
    assert!(content.starts_with("H🦀 ello"), "Got: {}", content);
}

#[test]
fn test_export_text_basic() {
    let mut term = Terminal::new(20, 3);
    term.process(b"Hello\r\nWorld\r\nTest");

    let text = term.export_text();
    let lines: Vec<&str> = text.lines().collect();

    assert_eq!(lines[0], "Hello");
    assert_eq!(lines[1], "World");
    assert_eq!(lines[2], "Test");
}

#[test]
fn test_export_text_with_colors() {
    let mut term = Terminal::new(20, 2);
    // Text with color codes
    term.process(b"\x1b[31mRed\x1b[0m \x1b[32mGreen\x1b[0m");

    let text = term.export_text();

    // Should export plain text without escape codes
    assert!(text.starts_with("Red Green"));
    // Should NOT contain escape sequences
    assert!(!text.contains("\x1b["));
}

#[test]
fn test_export_text_with_scrollback() {
    let mut term = Terminal::with_scrollback(20, 2, 1000);

    // Fill screen
    term.process(b"Line1\r\n");
    term.process(b"Line2\r\n");
    term.process(b"Line3");

    let text = term.export_text();

    // Should have scrollback + current screen
    assert!(text.contains("Line1"));
    assert!(text.contains("Line2"));
    assert!(text.contains("Line3"));
}

#[test]
fn test_export_text_trims_trailing_spaces() {
    let mut term = Terminal::new(20, 2);
    term.process(b"Hello   ");

    let text = term.export_text();
    let lines: Vec<&str> = text.lines().collect();

    // Should trim trailing spaces
    assert_eq!(lines[0], "Hello");
}

#[test]
fn test_export_text_alternate_screen() {
    let mut term = Terminal::new(20, 3);

    // Write to primary screen
    term.process(b"Primary");

    // Switch to alternate screen
    term.process(b"\x1b[?1049h");
    term.process(b"Alternate");

    let text = term.export_text();

    // Should export alternate screen (current active screen)
    assert!(text.contains("Alternate"));
    // Alternate screen has no scrollback, so should not contain primary
    assert!(!text.contains("Primary"));
}
