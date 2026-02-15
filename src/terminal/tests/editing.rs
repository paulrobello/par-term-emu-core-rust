// Editing sequences (VT220)
use crate::terminal::*;

#[test]
fn test_insert_lines() {
    let mut term = Terminal::new(80, 24);
    // Write some lines with \r\n to ensure proper line breaks
    term.process(b"Line 0\r\nLine 1\r\nLine 2\r\nLine 3\r\nLine 4");
    term.process(b"\x1b[2;1H"); // Move to row 2, col 1 (1-indexed)
    term.process(b"\x1b[2L"); // Insert 2 lines at current position

    let line1 = term.grid().row(1).unwrap();
    let line1_str: String = line1.iter().map(|c| c.c).collect();
    assert!(line1_str.trim().is_empty()); // Line 1 should now be blank

    // Check that content was pushed down
    let mut found_line1 = false;
    for i in 2..10 {
        if let Some(row) = term.grid().row(i) {
            let text: String = row.iter().map(|c| c.c).collect();
            if text.contains("Line 1") {
                found_line1 = true;
                break;
            }
        }
    }
    assert!(found_line1, "Line 1 should have been pushed down");
}

#[test]
fn test_delete_lines() {
    let mut term = Terminal::new(80, 24);
    // Write some lines with \r\n to ensure proper line breaks
    term.process(b"Line 0\r\nLine 1\r\nLine 2\r\nLine 3\r\nLine 4");
    term.process(b"\x1b[2;1H"); // Move to row 2, col 1 (1-indexed)
    term.process(b"\x1b[2M"); // Delete 2 lines at current position

    // Check that lines below moved up
    let mut found_line3 = false;
    for i in 0..5 {
        if let Some(row) = term.grid().row(i) {
            let text: String = row.iter().map(|c| c.c).collect();
            if text.contains("Line 3") {
                found_line3 = true;
                break;
            }
        }
    }
    assert!(found_line3, "Line 3 should have moved up");
}

#[test]
fn test_insert_characters() {
    let mut term = Terminal::new(80, 24);
    term.process(b"ABCDEFGH");
    term.process(b"\x1b[1;4H"); // Move to col 4 (after C)
    term.process(b"\x1b[3@"); // Insert 3 characters

    let line0 = term.grid().row(0).unwrap();
    let text: String = line0.iter().take(11).map(|c| c.c).collect();
    assert_eq!(text.trim(), "ABC   DEFGH");
}

#[test]
fn test_delete_characters() {
    let mut term = Terminal::new(80, 24);
    term.process(b"ABCDEFGH");
    term.process(b"\x1b[1;3H"); // Move to col 3 (C)
    term.process(b"\x1b[2P"); // Delete 2 characters

    let line0 = term.grid().row(0).unwrap();
    let text: String = line0.iter().take(6).map(|c| c.c).collect();
    assert_eq!(text.trim(), "ABEFGH");
}

#[test]
fn test_erase_characters() {
    let mut term = Terminal::new(80, 24);
    term.process(b"ABCDEFGH");
    term.process(b"\x1b[1;3H"); // Move to col 3 (C)
    term.process(b"\x1b[3X"); // Erase 3 characters

    let line0 = term.grid().row(0).unwrap();
    let text: String = line0.iter().take(8).map(|c| c.c).collect();
    assert!(text.starts_with("AB   FGH"));
}
