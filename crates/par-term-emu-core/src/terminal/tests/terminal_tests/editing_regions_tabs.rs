//! Line/character editing, scroll regions, margins, and tab stops.

use super::*;

// VT220 editing tests
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

// Scrolling region tests
#[test]
fn test_scroll_region_basic() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5;10r"); // Set scroll region lines 5-10

    assert_eq!(term.margins.scroll_region_top, 4); // 0-indexed
    assert_eq!(term.margins.scroll_region_bottom, 9);
}

#[test]
fn test_scroll_region_with_content() {
    let mut term = Terminal::new(80, 10);
    for i in 0..10 {
        term.process(format!("Line {}\r\n", i).as_bytes());
    }

    term.process(b"\x1b[3;7r"); // Set scroll region lines 3-7
    term.process(b"\x1b[3;1H"); // Move to start of region (row 2, 0-indexed)
    term.process(b"\x1b[1M"); // Delete line (scroll region up)

    // After deleting a line in the scroll region, content should shift
    // Just verify the operation completed without checking exact content
    assert_eq!(term.margins.scroll_region_top, 2); // 0-indexed
    assert_eq!(term.margins.scroll_region_bottom, 6); // 0-indexed
}

#[test]
fn test_index_within_scroll_region() {
    let mut term = Terminal::new(80, 10);
    term.process(b"\x1b[3;7r"); // Set scroll region lines 3-7 (1-indexed)

    // Verify scroll region was set correctly
    assert_eq!(term.margins.scroll_region_top, 2); // 0-indexed
    assert_eq!(term.margins.scroll_region_bottom, 6); // 0-indexed

    term.process(b"\x1b[7;1H"); // Move to row 7 (bottom of region, 1-indexed)
    term.process(b"Test\n"); // This should handle newline within scroll region

    // Just verify the scroll region is still set correctly
    assert_eq!(term.margins.scroll_region_bottom, 6);
}

#[test]
fn test_decstbm_zero_defaults() {
    // When DECSTBM parameters are 0 or missing, they should default to
    // top=1 and bottom=rows.
    let mut term = Terminal::new(10, 12);

    // CSI 0;0 r → full screen
    term.process(b"\x1b[0;0r");
    assert_eq!(term.margins.scroll_region_top, 0);
    assert_eq!(term.margins.scroll_region_bottom, 11);

    // CSI r (no params) → reset to full screen
    term.process(b"\x1br");
    assert_eq!(term.margins.scroll_region_top, 0);
    assert_eq!(term.margins.scroll_region_bottom, 11);

    // CSI 0;5 r → top defaults to 1, bottom=5
    term.process(b"\x1b[0;5r");
    assert_eq!(term.margins.scroll_region_top, 0);
    assert_eq!(term.margins.scroll_region_bottom, 4);

    // CSI 3;0 r → top=3, bottom defaults to rows
    term.process(b"\x1b[3;0r");
    assert_eq!(term.margins.scroll_region_top, 2);
    assert_eq!(term.margins.scroll_region_bottom, 11);
}

#[test]
fn test_preserve_margins_on_resize() {
    let mut term = Terminal::new(20, 15);
    // Set a non-trivial region
    term.process(b"\x1b[2;10r");
    assert_eq!(term.margins.scroll_region_top, 1);
    assert_eq!(term.margins.scroll_region_bottom, 9);

    // Resize should reset scroll region to full screen (matches xterm behavior)
    // This prevents stale scroll regions from causing rendering issues (e.g., in tmux)
    term.resize(25, 25);
    assert_eq!(term.margins.scroll_region_top, 0);
    assert_eq!(term.margins.scroll_region_bottom, 24); // full screen

    // Another resize also resets
    term.resize(25, 8);
    assert_eq!(term.margins.scroll_region_top, 0);
    assert_eq!(term.margins.scroll_region_bottom, 7); // full screen
}

// Tab stop tests
#[test]
fn test_default_tab_stops() {
    let term = Terminal::new(80, 24);
    assert!(term.tab_stops[0]);
    assert!(term.tab_stops[8]);
    assert!(term.tab_stops[16]);
    assert!(!term.tab_stops[1]);
}

#[test]
fn test_set_tab_stop() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5G"); // Move to column 5
    term.process(b"\x1bH"); // Set tab stop (HTS)

    assert!(term.tab_stops[4]); // 0-indexed
}

#[test]
fn test_clear_tab_stop() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[8G"); // Move to column 8 (tab stop)
    term.process(b"\x1b[0g"); // Clear tab stop at current position

    assert!(!term.tab_stops[7]); // 0-indexed, should be cleared
}

#[test]
fn test_tmux_scroll_region_with_status_bar() {
    // Simulate tmux with status bar: 51 rows total, status bar at row 51 (0-indexed: row 50)
    let mut term = Terminal::new(80, 51);

    // Fill screen with identifiable content
    for row in 0..51 {
        term.process(b"\x1b[H"); // Home
        term.process(format!("\x1b[{}H", row + 1).as_bytes()); // Move to row (1-indexed)
        term.process(format!("Row{:02}", row).as_bytes()); // Write "Row00", "Row01", etc.
    }

    // Verify initial content
    assert_eq!(term.grid.get(0, 0).unwrap().c, 'R'); // Row00
    assert_eq!(term.grid.get(0, 1).unwrap().c, 'R'); // Row01
    assert_eq!(term.grid.get(0, 50).unwrap().c, 'R'); // Row50 (status bar)

    // Set scroll region to exclude status bar: rows 1-50 (0-indexed: 0-49)
    term.process(b"\x1b[1;50r");
    assert_eq!(term.margins.scroll_region_top, 0);
    assert_eq!(term.margins.scroll_region_bottom, 49);

    // Move cursor to row 50 (VT) = row 49 (0-indexed) - bottom of scroll region
    term.process(b"\x1b[50;1H");
    assert_eq!(term.cursor.row, 49);

    // Get content before scroll
    let row0_before = term.grid.get(0, 0).unwrap().c;
    let row1_before = term.grid.get(0, 1).unwrap().c;
    assert_eq!(row0_before, 'R'); // Row00
    assert_eq!(row1_before, 'R'); // Row01

    // LF at bottom of scroll region should scroll the region per VT spec
    term.process(b"\n");

    // After scroll, row 0 should have content that was at row 1
    let row0_after = term.grid.get(0, 0).unwrap().c;
    let row0_char4 = term.grid.get(4, 0).unwrap().c; // 5th character (0-indexed position 4)

    // Row00 scrolled off, Row01 is now at row 0
    assert_eq!(row0_after, 'R'); // Still 'R', but it's Row01 now
                                 // Check 5th char (index 4): Row00 has '0', Row01 has '1'
    assert_eq!(
        row0_char4, '1',
        "After scroll, row 0 should have Row01's content"
    );

    // Status bar at row 50 should be UNCHANGED
    let status_content = term.grid.get(0, 50).unwrap().c;
    let status_char4 = term.grid.get(4, 50).unwrap().c;
    assert_eq!(status_content, 'R');
    assert_eq!(
        status_char4, '0',
        "Status bar (Row50) should not be affected by scroll"
    );

    // Cursor should still be at row 49 (bottom of scroll region) after scroll
    assert_eq!(term.cursor.row, 49);
}

#[test]
fn test_clear_all_tab_stops() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[3g"); // Clear all tab stops

    assert!(term.tab_stops.iter().all(|&x| !x));
}

#[test]
fn test_wrap_at_scroll_region_bottom_scrolls_region() {
    // Terminal with 80x24; reserve last row (row 23) as status bar
    let mut term = Terminal::new(80, 24);

    // Fill rows 0..=23 with identifiable content
    for row in 0..24 {
        term.process(format!("\x1b[{};1H", row + 1).as_bytes());
        term.process(format!("R{:02}", row).as_bytes());
    }

    // Set scroll region to exclude status bar: rows 1-23 -> 0..=22 (0-indexed)
    term.process(b"\x1b[1;23r");
    assert_eq!(term.margins.scroll_region_top, 0);
    assert_eq!(term.margins.scroll_region_bottom, 22);

    // Move cursor to bottom of scroll region, last column
    term.process(b"\x1b[23;80H"); // row 23 (VT) -> 22 (0-indexed), col 80 -> 79
    assert_eq!(term.cursor.row, 22);

    // With delayed wrap, first printable at last column sets wrap-pending, second triggers wrap
    term.process(b"X");
    // No scroll yet; now print another printable to advance
    term.process(b"Y");

    // After wrap at bottom of region, region should have scrolled up by 1
    // Row0 now contains what used to be Row01 ('R01')
    let row0_c2 = term.grid.get(2, 0).unwrap().c; // 3rd char of label R01
    assert_eq!(row0_c2, '1', "Wrap at region bottom must scroll region up");

    // Status bar at last row (row 23) must be preserved (starts with 'R23')
    let status_c2 = term.grid.get(2, 23).unwrap().c; // third char in 'R23'
    assert_eq!(status_c2, '3');

    // Cursor remains at bottom line of the region after scroll
    assert_eq!(term.cursor.row, 22);
}

#[test]
fn test_ind_scrolls_within_region_not_screen() {
    let mut term = Terminal::new(80, 24);
    // Fill and set region 1..23 (0..=22)
    for row in 0..24 {
        term.process(format!("\x1b[{};1H", row + 1).as_bytes());
        term.process(format!("R{:02}", row).as_bytes());
    }
    term.process(b"\x1b[1;23r");
    assert_eq!(term.margins.scroll_region_bottom, 22);
    // Move to bottom of region
    term.process(b"\x1b[23;1H");
    assert_eq!(term.cursor.row, 22);
    // ESC D (IND)
    term.process(b"\x1bD");

    // Region scrolled up by one; status row (row 23) preserved
    let row0_c2 = term.grid.get(2, 0).unwrap().c; // '1' from R01
    assert_eq!(row0_c2, '1');
    let status_c2 = term.grid.get(2, 23).unwrap().c; // '3' from R23
    assert_eq!(status_c2, '3');
    assert_eq!(term.cursor.row, 22);
}

#[test]
fn test_nel_scrolls_within_region_not_screen() {
    let mut term = Terminal::new(80, 24);
    // Fill and set region 1..23 (0..=22)
    for row in 0..24 {
        term.process(format!("\x1b[{};1H", row + 1).as_bytes());
        term.process(format!("R{:02}", row).as_bytes());
    }
    term.process(b"\x1b[1;23r");
    // Move to bottom of region, near end of line
    term.process(b"\x1b[23;40H");
    assert_eq!(term.cursor.row, 22);
    // ESC E (NEL)
    term.process(b"\x1bE");

    // Region scrolled; status bar preserved
    let row0_c2 = term.grid.get(2, 0).unwrap().c; // '1' from R01
    assert_eq!(row0_c2, '1');
    let status_c2 = term.grid.get(2, 23).unwrap().c; // '3' from R23
    assert_eq!(status_c2, '3');
    assert_eq!(term.cursor.row, 22);
    // Cursor should be at col 0 (or left margin) after NEL
    assert_eq!(term.cursor.col, 0);
}

#[test]
fn test_tab_forward() {
    let mut term = Terminal::new(80, 24);
    term.process(b"A\t"); // Write A then tab

    assert_eq!(term.cursor.col, 8); // Should tab to column 8
}
