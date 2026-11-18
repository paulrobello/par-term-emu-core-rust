//! DECFRA, DECCRA, DECSERA and rectangle bounds.

use super::*;

#[test]
fn test_decfra_fill_rectangle() {
    let mut term = Terminal::new(80, 24);

    // Fill a 3x3 rectangle with 'X' at position (2,2)
    // DECFRA: CSI Pc ; Pt ; Pl ; Pb ; Pr $ x
    // Pc = 88 (ASCII 'X'), Pt=3, Pl=3, Pb=5, Pr=5 (1-indexed)
    term.process(b"\x1b[88;3;3;5;5$x");

    // Check that the rectangle is filled
    for row in 2..=4 {
        for col in 2..=4 {
            if let Some(cell) = term.grid().get(col, row) {
                assert_eq!(cell.c, 'X', "Cell at ({},{}) should be 'X'", col, row);
            }
        }
    }

    // Check that cells outside the rectangle are not filled
    assert_ne!(term.grid().get(1, 1).unwrap().c, 'X');
    assert_ne!(term.grid().get(5, 5).unwrap().c, 'X');
}

#[test]
fn test_deccra_copy_rectangle() {
    let mut term = Terminal::new(80, 24);

    // Write some text in source area
    term.cursor.row = 1;
    term.cursor.col = 1;
    term.process(b"ABC");

    // Copy 1x3 rectangle from (2,2) to (2,10)
    // DECCRA: CSI Pts ; Pls ; Pbs ; Prs ; Pps ; Ptd ; Pld ; Ppd $ v
    // Source: row 2, cols 2-4 (1-indexed)
    // Dest: row 2, col 10
    term.process(b"\x1b[2;2;2;4;1;2;10;1$v");

    // Check that text was copied to destination
    assert_eq!(term.grid().get(9, 1).unwrap().c, 'A');
    assert_eq!(term.grid().get(10, 1).unwrap().c, 'B');
    assert_eq!(term.grid().get(11, 1).unwrap().c, 'C');

    // Source should still have the text
    assert_eq!(term.grid().get(1, 1).unwrap().c, 'A');
    assert_eq!(term.grid().get(2, 1).unwrap().c, 'B');
    assert_eq!(term.grid().get(3, 1).unwrap().c, 'C');
}

#[test]
fn test_decsera_erase_rectangle() {
    let mut term = Terminal::new(80, 24);

    // Fill area with text starting at (0,0)
    term.cursor.row = 0;
    term.cursor.col = 0;
    term.process(b"XXXXXX\r\nXXXXXX\r\nXXXXXX");

    // Erase a 2x2 rectangle at (2,2) in 1-indexed coords
    // DECSERA: CSI Pt ; Pl ; Pb ; Pr $ {
    // Top=2, Left=2, Bottom=3, Right=3 (1-indexed = 1,1 to 2,2 in 0-indexed)
    term.process(b"\x1b[2;2;3;3${");

    // Check that the rectangle is erased (should be spaces)
    assert_eq!(term.grid().get(1, 1).unwrap().c, ' ');
    assert_eq!(term.grid().get(2, 1).unwrap().c, ' ');
    assert_eq!(term.grid().get(1, 2).unwrap().c, ' ');
    assert_eq!(term.grid().get(2, 2).unwrap().c, ' ');

    // Check that cells outside the erased rectangle are not erased
    assert_eq!(term.grid().get(0, 0).unwrap().c, 'X');
    assert_eq!(term.grid().get(3, 0).unwrap().c, 'X'); // Row 0, col 3 should still be 'X'
    assert_eq!(term.grid().get(0, 1).unwrap().c, 'X'); // Row 1, col 0 should still be 'X'
}

#[test]
fn test_decfra_with_current_attributes() {
    let mut term = Terminal::new(80, 24);

    // Set red foreground color
    term.process(b"\x1b[31m");

    // Fill rectangle with '*'
    // ASCII 42 = '*'
    term.process(b"\x1b[42;2;2;4;4$x");

    // Check that cells have the red color attribute
    for row in 1..=3 {
        for col in 1..=3 {
            if let Some(cell) = term.grid().get(col, row) {
                assert_eq!(cell.c, '*');
                // Foreground should be red (Named color 1)
                assert!(matches!(cell.fg(), Color::Named(NamedColor::Red)));
            }
        }
    }
}

#[test]
fn test_deccra_overlapping_copy() {
    let mut term = Terminal::new(80, 24);

    // Write text in source
    term.cursor.row = 1;
    term.cursor.col = 1;
    term.process(b"TEST");

    // Copy overlapping: source (2,2-5) to dest (2,4-7)
    // This should work correctly with buffering
    term.process(b"\x1b[2;2;2;5;1;2;4;1$v");

    // Destination should have copied text
    assert_eq!(term.grid().get(3, 1).unwrap().c, 'T');
    assert_eq!(term.grid().get(4, 1).unwrap().c, 'E');
    assert_eq!(term.grid().get(5, 1).unwrap().c, 'S');
    assert_eq!(term.grid().get(6, 1).unwrap().c, 'T');
}

#[test]
fn test_rectangle_bounds_clamping() {
    let mut term = Terminal::new(10, 5);

    // Try to fill rectangle that exceeds grid bounds
    // Bottom=100, Right=100 should be clamped to grid size
    term.process(b"\x1b[88;1;1;100;100$x");

    // Entire grid should be filled with 'X'
    for row in 0..5 {
        for col in 0..10 {
            assert_eq!(term.grid().get(col, row).unwrap().c, 'X');
        }
    }
}

#[test]
fn test_rectangle_operations_on_alternate_screen() {
    let mut term = Terminal::new(80, 24);

    // Switch to alternate screen
    term.process(b"\x1b[?1049h");

    // Fill rectangle with 'A' on alternate screen
    term.process(b"\x1b[65;2;2;4;4$x");

    // Check alternate screen has the fill
    for row in 1..=3 {
        for col in 1..=3 {
            assert_eq!(term.active_grid().get(col, row).unwrap().c, 'A');
        }
    }

    // Switch back to primary screen
    term.process(b"\x1b[?1049l");

    // Primary screen should not have the fill
    assert_ne!(term.active_grid().get(1, 1).unwrap().c, 'A');
}
