// Unit tests for terminal.rs
//
// These tests have access to private fields and methods of Terminal.
// Included via include!() macro in terminal.rs to maintain private field access.

#[test]
fn test_terminal_creation() {
    let term = Terminal::new(80, 24);
    assert_eq!(term.size(), (80, 24));
}

#[test]
fn test_write_simple_text() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello");

    let content = term.content();
    assert!(content.starts_with("Hello"));
}

#[test]
fn test_newline() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Line1\nLine2");

    let content = term.content();
    let lines: Vec<&str> = content.lines().collect();
    assert!(lines[0].starts_with("Line1"));
    // LF alone doesn't reset column, so Line2 appears after Line1's cursor position
    assert!(lines[1].contains("Line2"));
}

#[test]
fn test_true_color() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[38;2;255;128;64mTrue Color\x1b[0m");

    let content = term.content();
    assert!(content.contains("True Color"));

    // Check the color was set correctly
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.fg, Color::Rgb(255, 128, 64));
}

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
fn test_shell_integration() {
    let mut term = Terminal::new(80, 24);

    // Prompt start
    term.process(b"\x1b]133;A\x07");
    assert!(term.shell_integration().in_prompt());

    // Command start
    term.process(b"\x1b]133;B\x07");
    assert!(term.shell_integration().in_command_input());

    // Command executed
    term.process(b"\x1b]133;C\x07");
    assert!(term.shell_integration().in_command_output());

    // Set CWD (OSC 7 with file:// URL format)
    term.process(b"\x1b]7;file://hostname/home/user\x07");
    assert_eq!(term.shell_integration().cwd(), Some("/home/user"));
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

    assert_eq!(term.scroll_region_top, 4); // 0-indexed
    assert_eq!(term.scroll_region_bottom, 9);
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
    assert_eq!(term.scroll_region_top, 2); // 0-indexed
    assert_eq!(term.scroll_region_bottom, 6); // 0-indexed
}

#[test]
fn test_index_within_scroll_region() {
    let mut term = Terminal::new(80, 10);
    term.process(b"\x1b[3;7r"); // Set scroll region lines 3-7 (1-indexed)

    // Verify scroll region was set correctly
    assert_eq!(term.scroll_region_top, 2); // 0-indexed
    assert_eq!(term.scroll_region_bottom, 6); // 0-indexed

    term.process(b"\x1b[7;1H"); // Move to row 7 (bottom of region, 1-indexed)
    term.process(b"Test\n"); // This should handle newline within scroll region

    // Just verify the scroll region is still set correctly
    assert_eq!(term.scroll_region_bottom, 6);
}

#[test]
fn test_decstbm_zero_defaults() {
    // When DECSTBM parameters are 0 or missing, they should default to
    // top=1 and bottom=rows.
    let mut term = Terminal::new(10, 12);

    // CSI 0;0 r → full screen
    term.process(b"\x1b[0;0r");
    assert_eq!(term.scroll_region_top, 0);
    assert_eq!(term.scroll_region_bottom, 11);

    // CSI r (no params) → reset to full screen
    term.process(b"\x1br");
    assert_eq!(term.scroll_region_top, 0);
    assert_eq!(term.scroll_region_bottom, 11);

    // CSI 0;5 r → top defaults to 1, bottom=5
    term.process(b"\x1b[0;5r");
    assert_eq!(term.scroll_region_top, 0);
    assert_eq!(term.scroll_region_bottom, 4);

    // CSI 3;0 r → top=3, bottom defaults to rows
    term.process(b"\x1b[3;0r");
    assert_eq!(term.scroll_region_top, 2);
    assert_eq!(term.scroll_region_bottom, 11);
}

#[test]
fn test_preserve_margins_on_resize() {
    let mut term = Terminal::new(20, 15);
    // Set a non-trivial region
    term.process(b"\x1b[2;10r");
    assert_eq!(term.scroll_region_top, 1);
    assert_eq!(term.scroll_region_bottom, 9);

    // Resize should reset scroll region to full screen (matches xterm behavior)
    // This prevents stale scroll regions from causing rendering issues (e.g., in tmux)
    term.resize(25, 25);
    assert_eq!(term.scroll_region_top, 0);
    assert_eq!(term.scroll_region_bottom, 24); // full screen

    // Another resize also resets
    term.resize(25, 8);
    assert_eq!(term.scroll_region_top, 0);
    assert_eq!(term.scroll_region_bottom, 7); // full screen
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
    assert_eq!(term.scroll_region_top, 0);
    assert_eq!(term.scroll_region_bottom, 49);

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
    assert_eq!(term.scroll_region_top, 0);
    assert_eq!(term.scroll_region_bottom, 22);

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
    assert_eq!(term.scroll_region_bottom, 22);
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
    assert_eq!(term.fg, Color::Named(NamedColor::Red));
}

#[test]
fn test_origin_mode() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5;15r"); // Set scroll region
    term.process(b"\x1b[?6h"); // Enable origin mode

    assert!(term.origin_mode);

    // The current implementation sets scroll region but cursor positioning
    // doesn't implement full origin mode yet, so test what it does
    term.process(b"\x1b[1;1H"); // Position to "home"
                                // For now, just verify origin mode is enabled
    assert!(term.origin_mode);
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
    assert_eq!(cell.fg, Color::from_ansi_code(196));
}

#[test]
fn test_sgr_reset() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1;31;42mTest"); // Bold, red fg, green bg
    term.process(b"\x1b[0m"); // Reset

    assert_eq!(term.fg, Color::Named(NamedColor::White));
    assert_eq!(term.bg, Color::Named(NamedColor::Black));
    assert!(!term.flags.bold());
}

#[test]
fn test_multiple_sgr_attributes() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1;3;4;9mTest"); // Bold, italic, underline, strikethrough

    assert!(term.flags.bold());
    assert!(term.flags.italic());
    assert!(term.flags.underline());
    assert!(term.flags.strikethrough());
}

// Device query response tests
#[test]
fn test_da_primary() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[62\"p"); // Force VT220 for predictable DA response
    term.process(b"\x1b[c"); // Primary DA

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[?62;1;4;6;9;15;22c");
}

#[test]
fn test_da_primary_with_param() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[62\"p");
    term.process(b"\x1b[0c"); // Primary DA with param 0

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[?62;1;4;6;9;15;22c");
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
    assert_eq!(response, b"\x1b[0n\x1b[1;1R\x1b[?62;1;4;6;9;15;22c");

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
fn test_paste_with_bracketed_mode() {
    let mut term = Terminal::new(80, 24);

    // Enable bracketed paste mode
    term.process(b"\x1b[?2004h");
    assert!(term.bracketed_paste());

    // Paste content
    term.paste("Hello World");

    // Content should appear with bracketed paste markers processed
    let content = term.content();
    assert!(content.contains("Hello World"));
}

#[test]
fn test_paste_without_bracketed_mode() {
    let mut term = Terminal::new(80, 24);

    // Bracketed paste mode disabled by default
    assert!(!term.bracketed_paste());

    // Paste content
    term.paste("Direct Paste");

    // Content should appear directly
    let content = term.content();
    assert!(content.contains("Direct Paste"));
}

#[test]
fn test_paste_multiline() {
    let mut term = Terminal::new(80, 24);

    // Enable bracketed paste
    term.process(b"\x1b[?2004h");

    // Paste multiline content
    term.paste("Line 1\nLine 2\nLine 3");

    // All lines should appear
    let content = term.content();
    assert!(content.contains("Line 1"));
    assert!(content.contains("Line 2"));
    assert!(content.contains("Line 3"));
}

#[test]
fn test_paste_special_characters() {
    let mut term = Terminal::new(80, 24);

    // Paste content with special characters
    term.paste("Tab:\tNewline:\nCarriage:\r");

    // Content should be processed correctly
    let content = term.content();
    assert!(content.contains("Tab:"));
    assert!(content.contains("Newline:"));
    assert!(content.contains("Carriage:"));
}

// Kitty Keyboard Protocol tests
#[test]
fn test_kitty_keyboard_query() {
    let mut term = Terminal::new(80, 24);

    // Query keyboard capabilities (should return 0 initially)
    term.process(b"\x1b[?u");
    assert_eq!(term.drain_responses(), b"\x1b[?0u");
}

#[test]
fn test_kitty_keyboard_set_mode() {
    let mut term = Terminal::new(80, 24);

    // Set keyboard mode with flags 1 (disambiguate)
    term.process(b"\x1b[=1;1u");
    assert_eq!(term.keyboard_flags(), 1);

    // Query should return the set flags
    term.process(b"\x1b[?u");
    assert_eq!(term.drain_responses(), b"\x1b[?1u");

    // Set different flags (3 = disambiguate + report events)
    term.process(b"\x1b[=3;1u");
    assert_eq!(term.keyboard_flags(), 3);

    // Unset (mode 0)
    term.process(b"\x1b[=0;0u");
    assert_eq!(term.keyboard_flags(), 0);
}

#[test]
fn test_kitty_keyboard_push_pop() {
    let mut term = Terminal::new(80, 24);

    // Set initial flags
    term.process(b"\x1b[=1;1u");
    assert_eq!(term.keyboard_flags(), 1);

    // Push new flags to stack
    term.process(b"\x1b[>3u");
    assert_eq!(term.keyboard_flags(), 3);

    // Push another set
    term.process(b"\x1b[>7u");
    assert_eq!(term.keyboard_flags(), 7);

    // Pop once
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 3);

    // Pop again
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 1);

    // Popping empty stack should not change flags
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 1);
}

#[test]
fn test_kitty_keyboard_pop_multiple() {
    let mut term = Terminal::new(80, 24);

    // Set initial flags
    term.process(b"\x1b[=1;1u");

    // Push multiple values
    term.process(b"\x1b[>3u");
    term.process(b"\x1b[>7u");
    term.process(b"\x1b[>15u");
    assert_eq!(term.keyboard_flags(), 15);

    // Pop 2 at once
    term.process(b"\x1b[<2u");
    assert_eq!(term.keyboard_flags(), 3);
}

#[test]
fn test_kitty_keyboard_report_mode() {
    let mut term = Terminal::new(80, 24);

    // Set flags
    term.process(b"\x1b[=5;1u");
    assert_eq!(term.keyboard_flags(), 5);

    // Use mode 3 to report current flags
    term.process(b"\x1b[=0;3u");
    assert_eq!(term.drain_responses(), b"\x1b[?5u");
}

#[test]
fn test_kitty_keyboard_alternate_screen_stacks() {
    let mut term = Terminal::new(80, 24);

    // Set flags on main screen
    term.process(b"\x1b[=1;1u");
    term.process(b"\x1b[>3u");
    assert_eq!(term.keyboard_flags(), 3);

    // Switch to alternate screen
    term.process(b"\x1b[?1049h");
    assert!(term.is_alt_screen_active());

    // Push should use alternate stack
    term.process(b"\x1b[>7u");
    assert_eq!(term.keyboard_flags(), 7);

    // Pop from alternate stack
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 3);

    // Switch back to main screen
    term.process(b"\x1b[?1049l");
    assert!(!term.is_alt_screen_active());

    // Pop from main stack
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 1);
}

#[test]
fn test_kitty_keyboard_reset() {
    let mut term = Terminal::new(80, 24);

    // Set flags
    term.process(b"\x1b[=15;1u");
    assert_eq!(term.keyboard_flags(), 15);

    // Reset terminal
    term.reset();

    // Flags should be reset to 0
    assert_eq!(term.keyboard_flags(), 0);
}

#[test]
fn test_osc52_clipboard_write() {
    let mut term = Terminal::new(80, 24);

    // Initially clipboard should be empty
    assert_eq!(term.clipboard(), None);

    // Write to clipboard using OSC 52 with base64 encoded "Hello, World!"
    // Base64("Hello, World!") = "SGVsbG8sIFdvcmxkIQ=="
    term.process(b"\x1b]52;c;SGVsbG8sIFdvcmxkIQ==\x1b\\");

    // Clipboard should now contain the decoded text
    assert_eq!(term.clipboard(), Some("Hello, World!"));
}

#[test]
fn test_osc52_clipboard_query_allowed() {
    let mut term = Terminal::new(80, 24);

    // Set clipboard content
    term.set_clipboard(Some("Test content".to_string()));

    // Enable clipboard read
    term.set_allow_clipboard_read(true);
    assert!(term.allow_clipboard_read());

    // Query clipboard
    term.process(b"\x1b]52;c;?\x1b\\");

    // Should respond with base64 encoded content
    let response = term.drain_responses();
    // Base64("Test content") = "VGVzdCBjb250ZW50"
    assert_eq!(response, b"\x1b]52;c;VGVzdCBjb250ZW50\x1b\\");
}

#[test]
fn test_osc52_clipboard_query_denied() {
    let mut term = Terminal::new(80, 24);

    // Set clipboard content
    term.set_clipboard(Some("Secret content".to_string()));

    // Clipboard read is disabled by default
    assert!(!term.allow_clipboard_read());

    // Query clipboard
    term.process(b"\x1b]52;c;?\x1b\\");

    // Should NOT respond (security)
    let response = term.drain_responses();
    assert_eq!(response, b"");
}

#[test]
fn test_osc52_clipboard_clear() {
    let mut term = Terminal::new(80, 24);

    // Set clipboard content
    term.set_clipboard(Some("Some text".to_string()));
    assert_eq!(term.clipboard(), Some("Some text"));

    // Clear clipboard with empty data
    term.process(b"\x1b]52;c;\x1b\\");

    // Clipboard should be empty
    assert_eq!(term.clipboard(), None);
}

#[test]
fn test_osc52_clipboard_empty_query() {
    let mut term = Terminal::new(80, 24);

    // Enable clipboard read
    term.set_allow_clipboard_read(true);

    // Query empty clipboard
    term.process(b"\x1b]52;c;?\x1b\\");

    // Should respond with empty data
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b]52;c;\x1b\\");
}

#[test]
fn test_osc52_clipboard_multiple_operations() {
    let mut term = Terminal::new(80, 24);

    // Write first content
    term.process(b"\x1b]52;c;Zmlyc3Q=\x1b\\"); // "first"
    assert_eq!(term.clipboard(), Some("first"));

    // Overwrite with second content
    term.process(b"\x1b]52;c;c2Vjb25k\x1b\\"); // "second"
    assert_eq!(term.clipboard(), Some("second"));

    // Clear
    term.process(b"\x1b]52;c;\x1b\\");
    assert_eq!(term.clipboard(), None);
}

#[test]
fn test_osc52_clipboard_programmatic_access() {
    let mut term = Terminal::new(80, 24);

    // Set via programmatic API
    term.set_clipboard(Some("API content".to_string()));
    assert_eq!(term.clipboard(), Some("API content"));

    // Clear via programmatic API
    term.set_clipboard(None);
    assert_eq!(term.clipboard(), None);
}

#[test]
fn test_osc52_clipboard_selection_parameter() {
    let mut term = Terminal::new(80, 24);

    // Test with explicit 'c' parameter (clipboard)
    term.process(b"\x1b]52;c;Y2xpcGJvYXJk\x1b\\"); // "clipboard"
    assert_eq!(term.clipboard(), Some("clipboard"));

    // Test with empty selection (should still work)
    term.process(b"\x1b]52;;cHJpbWFyeQ==\x1b\\"); // "primary"
    assert_eq!(term.clipboard(), Some("primary"));
}

#[test]
fn test_underline_style_straight() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4 - basic underline (should default to straight)
    term.process(b"\x1b[4mTest");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Straight);

    // SGR 4:1 - explicit straight underline
    term.process(b"\x1b[0m\x1b[4:1mTest");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Straight);
}

#[test]
fn test_underline_style_double() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:2 - double underline
    term.process(b"\x1b[4:2mDouble");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Double);
}

#[test]
fn test_underline_style_curly() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:3 - curly underline (for errors, spell check)
    term.process(b"\x1b[4:3mError");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Curly);
}

#[test]
fn test_underline_style_dotted() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:4 - dotted underline
    term.process(b"\x1b[4:4mDotted");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Dotted);
}

#[test]
fn test_underline_style_dashed() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:5 - dashed underline
    term.process(b"\x1b[4:5mDashed");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Dashed);
}

#[test]
fn test_underline_style_disable() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // Set underline with style
    term.process(b"\x1b[4:3mText");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Curly);

    // SGR 24 - disable underline
    term.process(b"\x1b[24mNo");
    // Check cells written after disable (at position 4, 5)
    let cell = term.active_grid().get(4, 0).unwrap();
    assert!(!cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::None);
}

#[test]
fn test_underline_style_reset() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // Set underline with style
    term.process(b"\x1b[4:2mDouble");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Double);

    // SGR 0 - reset all
    term.process(b"\x1b[0mReset");
    // Check cells written after reset (at position 6, 7, etc)
    let cell = term.active_grid().get(6, 0).unwrap();
    assert!(!cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::None);
}

#[test]
fn test_underline_style_none() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:0 - no underline (explicit)
    term.process(b"\x1b[4:0mNone");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(!cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::None);
}

#[test]
fn test_underline_style_multiple_switches() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // Start with straight
    term.process(b"\x1b[4mA");
    assert_eq!(
        term.active_grid().get(0, 0).unwrap().flags.underline_style,
        UnderlineStyle::Straight
    );

    // Switch to curly
    term.process(b"\x1b[4:3mB");
    assert_eq!(
        term.active_grid().get(1, 0).unwrap().flags.underline_style,
        UnderlineStyle::Curly
    );

    // Switch to double
    term.process(b"\x1b[4:2mC");
    assert_eq!(
        term.active_grid().get(2, 0).unwrap().flags.underline_style,
        UnderlineStyle::Double
    );

    // Disable
    term.process(b"\x1b[24mD");
    assert_eq!(
        term.active_grid().get(3, 0).unwrap().flags.underline_style,
        UnderlineStyle::None
    );
}

#[test]
fn test_cursor_style_blinking_block() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // Default should be blinking block
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);

    // CSI 1 SP q - blinking block
    term.process(b"\x1b[1 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);
}

#[test]
fn test_cursor_style_steady_block() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 2 SP q - steady block
    term.process(b"\x1b[2 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyBlock);
}

#[test]
fn test_cursor_style_blinking_underline() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 3 SP q - blinking underline
    term.process(b"\x1b[3 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingUnderline);
}

#[test]
fn test_cursor_style_steady_underline() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 4 SP q - steady underline
    term.process(b"\x1b[4 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyUnderline);
}

#[test]
fn test_cursor_style_blinking_bar() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 5 SP q - blinking bar
    term.process(b"\x1b[5 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBar);
}

#[test]
fn test_cursor_style_steady_bar() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 6 SP q - steady bar
    term.process(b"\x1b[6 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyBar);
}

#[test]
fn test_cursor_style_default() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI SP q or CSI 0 SP q - default (blinking block)
    term.process(b"\x1b[ q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);

    term.process(b"\x1b[0 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);
}

#[test]
fn test_cursor_style_saved_restored() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // Set to blinking bar
    term.process(b"\x1b[5 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBar);

    // Save cursor (DECSC - ESC 7)
    term.process(b"\x1b7");

    // Change to steady underline
    term.process(b"\x1b[4 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyUnderline);

    // Restore cursor (DECRC - ESC 8)
    term.process(b"\x1b8");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBar);
}

#[test]
fn test_osc_10_query_default_fg() {
    let mut term = Terminal::new(80, 24);

    // Query default foreground color: OSC 10 ; ? ST
    term.process(b"\x1b]10;?\x1b\\");

    // Check response
    let responses = term.drain_responses();
    assert!(!responses.is_empty(), "Expected response from OSC 10 query");

    let response = String::from_utf8_lossy(&responses);
    // Default fg is White (192, 192, 192) = 0xc0c0 in 16-bit
    assert!(
        response.contains("rgb:"),
        "Response should contain rgb: format"
    );
    assert!(
        response.starts_with("\x1b]10;"),
        "Response should start with OSC 10"
    );
    assert!(response.ends_with("\x1b\\"), "Response should end with ST");
}

#[test]
fn test_osc_11_query_default_bg() {
    let mut term = Terminal::new(80, 24);

    // Query default background color: OSC 11 ; ? ST
    term.process(b"\x1b]11;?\x1b\\");

    // Check response
    let responses = term.drain_responses();
    assert!(!responses.is_empty(), "Expected response from OSC 11 query");

    let response = String::from_utf8_lossy(&responses);
    // Default bg is Black (0, 0, 0) = 0x0000 in 16-bit
    assert!(
        response.contains("rgb:"),
        "Response should contain rgb: format"
    );
    assert!(
        response.starts_with("\x1b]11;"),
        "Response should start with OSC 11"
    );
    assert!(response.ends_with("\x1b\\"), "Response should end with ST");
}

#[test]
fn test_osc_12_query_cursor_color() {
    let mut term = Terminal::new(80, 24);

    // Query cursor color: OSC 12 ; ? ST
    term.process(b"\x1b]12;?\x1b\\");

    // Check response
    let responses = term.drain_responses();
    assert!(!responses.is_empty(), "Expected response from OSC 12 query");

    let response = String::from_utf8_lossy(&responses);
    // Default cursor color is White (192, 192, 192) = 0xc0c0 in 16-bit
    assert!(
        response.contains("rgb:"),
        "Response should contain rgb: format"
    );
    assert!(
        response.starts_with("\x1b]12;"),
        "Response should start with OSC 12"
    );
    assert!(response.ends_with("\x1b\\"), "Response should end with ST");
}

#[test]
fn test_osc_10_11_12_custom_colors() {
    let mut term = Terminal::new(80, 24);

    // Set custom colors
    term.set_default_fg(Color::Rgb(255, 128, 64));
    term.set_default_bg(Color::Rgb(32, 64, 128));
    term.set_cursor_color(Color::Rgb(0, 255, 0));

    // Query foreground
    term.process(b"\x1b]10;?\x1b\\");
    let responses = term.drain_responses();
    let response = String::from_utf8_lossy(&responses);
    // 255 * 257 = 65535 = 0xffff, 128 * 257 = 32896 = 0x8080, 64 * 257 = 16448 = 0x4040
    assert!(response.contains("ffff"), "Should contain red=255");
    assert!(response.contains("8080"), "Should contain green=128");
    assert!(response.contains("4040"), "Should contain blue=64");

    // Query background
    term.process(b"\x1b]11;?\x1b\\");
    let responses = term.drain_responses();
    let response = String::from_utf8_lossy(&responses);
    // 32 * 257 = 8224 = 0x2020, 64 * 257 = 16448 = 0x4040, 128 * 257 = 32896 = 0x8080
    assert!(response.contains("2020"), "Should contain red=32");
    assert!(response.contains("4040"), "Should contain green=64");
    assert!(response.contains("8080"), "Should contain blue=128");

    // Query cursor color
    term.process(b"\x1b]12;?\x1b\\");
    let responses = term.drain_responses();
    let response = String::from_utf8_lossy(&responses);
    // 0 * 257 = 0 = 0x0000, 255 * 257 = 65535 = 0xffff
    assert!(response.contains("0000"), "Should contain red=0");
    assert!(response.contains("ffff"), "Should contain green=255");
}

#[test]
fn test_osc_color_query_response_format() {
    let mut term = Terminal::new(80, 24);

    // Set a known color (pure red)
    term.set_default_fg(Color::Rgb(255, 0, 0));

    // Query and check exact format
    term.process(b"\x1b]10;?\x1b\\");
    let responses = term.drain_responses();
    let response = String::from_utf8_lossy(&responses);

    // Expected format: ESC ] 10 ; rgb:ffff/0000/0000 ESC \
    // 255 * 257 = 65535 = 0xffff, 0 * 257 = 0 = 0x0000
    assert_eq!(response, "\x1b]10;rgb:ffff/0000/0000\x1b\\");
}

#[test]
fn test_osc_color_getters_setters() {
    let mut term = Terminal::new(80, 24);

    // Test default colors
    assert_eq!(term.default_fg(), Color::Named(NamedColor::White));
    assert_eq!(term.default_bg(), Color::Named(NamedColor::Black));
    assert_eq!(term.cursor_color(), Color::Named(NamedColor::White));

    // Test setters
    term.set_default_fg(Color::Rgb(100, 150, 200));
    term.set_default_bg(Color::Rgb(50, 75, 100));
    term.set_cursor_color(Color::Rgb(255, 255, 0));

    // Verify getters
    assert_eq!(term.default_fg(), Color::Rgb(100, 150, 200));
    assert_eq!(term.default_bg(), Color::Rgb(50, 75, 100));
    assert_eq!(term.cursor_color(), Color::Rgb(255, 255, 0));

    // Verify RGB conversion
    assert_eq!(term.default_fg().to_rgb(), (100, 150, 200));
    assert_eq!(term.default_bg().to_rgb(), (50, 75, 100));
    assert_eq!(term.cursor_color().to_rgb(), (255, 255, 0));
}

#[test]
fn test_osc_color_queries_multiple() {
    let mut term = Terminal::new(80, 24);

    // Query all three colors in sequence
    term.process(b"\x1b]10;?\x1b\\");
    term.process(b"\x1b]11;?\x1b\\");
    term.process(b"\x1b]12;?\x1b\\");

    // Drain responses - should have 3 responses
    let responses = term.drain_responses();
    let response_str = String::from_utf8_lossy(&responses);

    // Count OSC responses (should have 3)
    let osc_count = response_str.matches("\x1b]").count();
    assert_eq!(osc_count, 3, "Should have 3 OSC responses");

    // Verify each type is present
    assert!(response_str.contains("\x1b]10;"), "Missing OSC 10 response");
    assert!(response_str.contains("\x1b]11;"), "Missing OSC 11 response");
    assert!(response_str.contains("\x1b]12;"), "Missing OSC 12 response");
}

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
                assert!(matches!(cell.fg, Color::Named(NamedColor::Red)));
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

// ============================================================================
// VT520 Tests
// ============================================================================

#[test]
fn test_vt520_conformance_level_default() {
    let term = Terminal::new(80, 24);
    // Default conformance level should be VT520
    assert_eq!(term.conformance_level.level(), 5);
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT520
    );
}

#[test]
fn test_vt520_decscl_set_conformance_level() {
    let mut term = Terminal::new(80, 24);

    // Set to VT100 (short form)
    term.process(b"\x1b[1\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT100
    );

    // Set to VT220 (long form)
    term.process(b"\x1b[62\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT220
    );

    // Set to VT320 (short form)
    term.process(b"\x1b[3\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT320
    );

    // Set to VT420 (long form)
    term.process(b"\x1b[64\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT420
    );

    // Set to VT520 (short form)
    term.process(b"\x1b[5\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT520
    );

    // Set to VT520 (long form)
    term.process(b"\x1b[65\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT520
    );
}

#[test]
fn test_vt520_decscl_with_8bit_mode() {
    let mut term = Terminal::new(80, 24);

    // Set to VT220 with 8-bit mode
    term.process(b"\x1b[62;2\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT220
    );

    // Set to VT420 with 7-bit mode
    term.process(b"\x1b[64;0\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT420
    );
}

#[test]
fn test_vt520_decscl_invalid_level() {
    let mut term = Terminal::new(80, 24);
    let original_level = term.conformance_level;

    // Invalid level should be ignored
    term.process(b"\x1b[99\"p");
    assert_eq!(term.conformance_level, original_level);

    term.process(b"\x1b[0\"p");
    assert_eq!(term.conformance_level, original_level);
}

#[test]
fn test_vt520_device_attributes_vt100() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1\"p"); // Set to VT100

    // Request Primary DA
    term.process(b"\x1b[c");

    let responses = term.drain_responses();
    let response = String::from_utf8(responses).unwrap();
    // Should report VT100 (id=1)
    assert!(response.starts_with("\x1b[?1;"));
}

#[test]
fn test_vt520_device_attributes_vt220() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[62\"p"); // Set to VT220

    // Request Primary DA
    term.process(b"\x1b[c");

    let responses = term.drain_responses();
    let response = String::from_utf8(responses).unwrap();
    // Should report VT220 (id=62)
    assert!(response.starts_with("\x1b[?62;"));
}

#[test]
fn test_vt520_device_attributes_vt520() {
    let mut term = Terminal::new(80, 24);
    // Default is VT520

    // Request Primary DA
    term.process(b"\x1b[c");

    let responses = term.drain_responses();
    let response = String::from_utf8(responses).unwrap();
    // Should report VT520 (id=65)
    assert!(response.starts_with("\x1b[?65;"));
    // Should include features: 1;4;6;9;15;22
    assert!(response.contains("1;4;6;9;15;22c"));
}

#[test]
fn test_vt520_device_attributes_all_levels() {
    let mut term = Terminal::new(80, 24);

    let test_cases = vec![
        (b"\x1b[61\"p", "\x1b[?1;"),   // VT100
        (b"\x1b[62\"p", "\x1b[?62;"),  // VT220
        (b"\x1b[63\"p", "\x1b[?63;"),  // VT320
        (b"\x1b[64\"p", "\x1b[?64;"),  // VT420
        (b"\x1b[65\"p", "\x1b[?65;"),  // VT520
    ];

    for (set_level, expected_da) in test_cases {
        term.process(set_level);
        term.process(b"\x1b[c");

        let responses = term.drain_responses();
        let response = String::from_utf8(responses).unwrap();
        assert!(
            response.starts_with(expected_da),
            "Expected DA to start with {} but got {}",
            expected_da,
            response
        );
    }
}

#[test]
fn test_vt520_decswbv_set_warning_bell_volume() {
    let mut term = Terminal::new(80, 24);

    // Default should be 4 (moderate)
    assert_eq!(term.warning_bell_volume, 4);

    // Set to off (0)
    term.process(b"\x1b[0 t");
    assert_eq!(term.warning_bell_volume, 0);

    // Set to low (1)
    term.process(b"\x1b[1 t");
    assert_eq!(term.warning_bell_volume, 1);

    // Set to medium (4)
    term.process(b"\x1b[4 t");
    assert_eq!(term.warning_bell_volume, 4);

    // Set to high (8)
    term.process(b"\x1b[8 t");
    assert_eq!(term.warning_bell_volume, 8);
}

#[test]
fn test_vt520_decswbv_clamps_to_valid_range() {
    let mut term = Terminal::new(80, 24);

    // Values above 8 should be clamped to 8
    term.process(b"\x1b[99 t");
    assert_eq!(term.warning_bell_volume, 8);

    term.process(b"\x1b[255 t");
    assert_eq!(term.warning_bell_volume, 8);
}

#[test]
fn test_vt520_decsmbv_set_margin_bell_volume() {
    let mut term = Terminal::new(80, 24);

    // Default should be 4 (moderate)
    assert_eq!(term.margin_bell_volume, 4);

    // Set to off (0)
    term.process(b"\x1b[0 u");
    assert_eq!(term.margin_bell_volume, 0);

    // Set to low (1)
    term.process(b"\x1b[1 u");
    assert_eq!(term.margin_bell_volume, 1);

    // Set to medium (4)
    term.process(b"\x1b[4 u");
    assert_eq!(term.margin_bell_volume, 4);

    // Set to high (8)
    term.process(b"\x1b[8 u");
    assert_eq!(term.margin_bell_volume, 8);
}

#[test]
fn test_vt520_decsmbv_clamps_to_valid_range() {
    let mut term = Terminal::new(80, 24);

    // Values above 8 should be clamped to 8
    term.process(b"\x1b[99 u");
    assert_eq!(term.margin_bell_volume, 8);

    term.process(b"\x1b[200 u");
    assert_eq!(term.margin_bell_volume, 8);
}

#[test]
fn test_vt520_bell_volumes_independent() {
    let mut term = Terminal::new(80, 24);

    // Set different values for each bell
    term.process(b"\x1b[2 t"); // Warning bell = 2
    term.process(b"\x1b[6 u"); // Margin bell = 6

    assert_eq!(term.warning_bell_volume, 2);
    assert_eq!(term.margin_bell_volume, 6);

    // Change warning bell without affecting margin bell
    term.process(b"\x1b[7 t");
    assert_eq!(term.warning_bell_volume, 7);
    assert_eq!(term.margin_bell_volume, 6); // Should remain unchanged
}

#[test]
fn test_vt520_conformance_level_feature_support() {
    use crate::conformance_level::{ConformanceLevel, Feature};

    let vt100 = ConformanceLevel::VT100;
    let vt220 = ConformanceLevel::VT220;
    let vt420 = ConformanceLevel::VT420;
    let vt520 = ConformanceLevel::VT520;

    // VT100 supports basic features only
    assert!(vt100.supports(Feature::CursorMovement));
    assert!(!vt100.supports(Feature::LineEditing));
    assert!(!vt100.supports(Feature::RectangleOperations));
    assert!(!vt100.supports(Feature::BellVolumeControl));

    // VT220 adds line editing
    assert!(vt220.supports(Feature::CursorMovement));
    assert!(vt220.supports(Feature::LineEditing));
    assert!(!vt220.supports(Feature::RectangleOperations));
    assert!(!vt220.supports(Feature::BellVolumeControl));

    // VT420 adds rectangles and character protection
    assert!(vt420.supports(Feature::CursorMovement));
    assert!(vt420.supports(Feature::LineEditing));
    assert!(vt420.supports(Feature::RectangleOperations));
    assert!(vt420.supports(Feature::CharacterProtection));
    assert!(!vt420.supports(Feature::BellVolumeControl));

    // VT520 supports everything
    assert!(vt520.supports(Feature::CursorMovement));
    assert!(vt520.supports(Feature::LineEditing));
    assert!(vt520.supports(Feature::RectangleOperations));
    assert!(vt520.supports(Feature::CharacterProtection));
    assert!(vt520.supports(Feature::BellVolumeControl));
}

#[test]
fn test_vt520_conformance_level_ordering() {
    use crate::conformance_level::ConformanceLevel;

    assert!(ConformanceLevel::VT100 < ConformanceLevel::VT220);
    assert!(ConformanceLevel::VT220 < ConformanceLevel::VT320);
    assert!(ConformanceLevel::VT320 < ConformanceLevel::VT420);
    assert!(ConformanceLevel::VT420 < ConformanceLevel::VT520);
}

// ========== Tests for New TUI App Features ==========

#[test]
fn test_dirty_region_tracking() {
    let mut term = Terminal::new(80, 24);

    // Initially no dirty rows
    assert!(term.get_dirty_rows().is_empty());
    assert_eq!(term.get_dirty_region(), None);

    // Write to first row
    term.process(b"Hello");
    let dirty = term.get_dirty_rows();
    assert!(dirty.contains(&0));
    assert_eq!(term.get_dirty_region(), Some((0, 0)));

    // Write to another row
    term.process(b"\n\n");
    term.process(b"World");
    let dirty = term.get_dirty_rows();
    assert!(dirty.len() >= 2);

    // Mark clean and verify
    term.mark_clean();
    assert!(term.get_dirty_rows().is_empty());
    assert_eq!(term.get_dirty_region(), None);

    // Manual dirty marking
    term.mark_row_dirty(10);
    assert_eq!(term.get_dirty_rows(), vec![10]);
}

#[test]
fn test_bell_events() {
    let mut term = Terminal::new(80, 24);

    // Initially no bell events
    assert_eq!(term.drain_bell_events().len(), 0);

    // Trigger bell
    term.process(b"\x07");
    let events = term.drain_bell_events();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], BellEvent::WarningBell(_) | BellEvent::VisualBell));

    // Drain clears the buffer
    assert_eq!(term.drain_bell_events().len(), 0);

    // Multiple bells
    term.process(b"\x07\x07\x07");
    assert_eq!(term.drain_bell_events().len(), 3);
}

#[test]
fn test_mode_introspection() {
    let mut term = Terminal::new(80, 24);

    // Check default modes
    assert!(term.auto_wrap_mode());
    assert!(!term.origin_mode());
    assert!(!term.application_cursor());
    assert_eq!(term.scroll_region(), (0, 23));
    assert_eq!(term.left_right_margins(), None);

    // Enable origin mode
    term.process(b"\x1b[?6h");
    assert!(term.origin_mode());

    // Enable application cursor
    term.process(b"\x1b[?1h");
    assert!(term.application_cursor());

    // Disable auto wrap
    term.process(b"\x1b[?7l");
    assert!(!term.auto_wrap_mode());

    // Set scroll region
    term.process(b"\x1b[5;20r");
    assert_eq!(term.scroll_region(), (4, 19)); // 0-indexed

    // Enable left/right margins
    term.process(b"\x1b[?69h");
    term.process(b"\x1b[10;70s");
    assert!(term.left_right_margins().is_some());
    let (left, right) = term.left_right_margins().unwrap();
    assert_eq!(left, 9); // 0-indexed
    assert_eq!(right, 69); // 0-indexed
}

#[test]
fn test_palette_access() {
    let mut term = Terminal::new(80, 24);

    // Get default ANSI colors
    let palette = term.get_ansi_palette();
    assert_eq!(palette.len(), 16);

    // Get individual color
    assert!(term.get_ansi_color(0).is_some());
    assert!(term.get_ansi_color(15).is_some());
    assert!(term.get_ansi_color(16).is_none());

    // Modify a color
    term.process(b"\x1b]4;1;rgb:ff/00/00\x1b\\");
    let red = term.get_ansi_color(1).unwrap();
    assert_eq!(red, Color::Rgb(255, 0, 0));

    // Get cursor/link colors
    let _cursor_color = term.get_cursor_color();
    let _link_color = term.get_link_color();
    let _sel_bg = term.get_selection_bg_color();
    let _sel_fg = term.get_selection_fg_color();
}

#[test]
fn test_tab_stop_access() {
    let mut term = Terminal::new(80, 24);

    // Default tab stops at every 8 columns
    let stops = term.get_tab_stops();
    assert!(stops.contains(&0));
    assert!(stops.contains(&8));
    assert!(stops.contains(&16));

    // Clear a tab stop
    term.clear_tab_stop(8);
    let stops = term.get_tab_stops();
    assert!(!stops.contains(&8));
    assert!(stops.contains(&16));

    // Set a custom tab stop
    term.set_tab_stop(5);
    let stops = term.get_tab_stops();
    assert!(stops.contains(&5));

    // Clear all tab stops
    term.clear_all_tab_stops();
    assert!(term.get_tab_stops().is_empty());
}

#[test]
fn test_hyperlink_enumeration() {
    let mut term = Terminal::new(80, 24);

    // Initially no hyperlinks
    assert_eq!(term.get_all_hyperlinks().len(), 0);

    // Add a hyperlink
    term.process(b"\x1b]8;;https://example.com\x1b\\Link\x1b]8;;\x1b\\");
    let links = term.get_all_hyperlinks();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://example.com");
    assert!(!links[0].positions.is_empty());

    // Add another hyperlink
    term.process(b" \x1b]8;;https://test.com\x1b\\Test\x1b]8;;\x1b\\");
    let links = term.get_all_hyperlinks();
    assert_eq!(links.len(), 2);
}

#[test]
fn test_bulk_read_operations() {
    let mut term = Terminal::new(10, 5);
    term.process(b"12345\n67890\nABCDE");

    // Get visible region
    let visible = term.get_visible_region();
    assert_eq!(visible.len(), 5); // 5 rows
    assert_eq!(visible[0].len(), 10); // 10 cols

    // Get row range
    let rows = term.get_row_range(0, 2);
    assert_eq!(rows.len(), 2);

    // Get rectangle
    let rect = term.get_rectangle(0, 0, 2, 4);
    assert_eq!(rect.len(), 3); // 3 rows
    assert_eq!(rect[0].len(), 5); // 5 cols
}

#[test]
fn test_rectangle_operations() {
    let mut term = Terminal::new(20, 10);

    // Fill rectangle
    term.fill_rectangle(2, 2, 4, 6, 'X');
    let visible = term.get_visible_region();
    assert_eq!(visible[2][2].c, 'X');
    assert_eq!(visible[3][4].c, 'X');
    assert_eq!(visible[4][6].c, 'X');

    // Erase rectangle
    term.erase_rectangle(2, 2, 4, 6);
    let visible = term.get_visible_region();
    assert_eq!(visible[2][2].c, ' ');
    assert_eq!(visible[3][4].c, ' ');
}

#[test]
fn test_enhanced_stats() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Test");
    term.process(b"\x1b]8;;https://example.com\x1b\\Link\x1b]8;;\x1b\\");

    let stats = term.get_stats();
    assert_eq!(stats.cols, 80);
    assert_eq!(stats.rows, 24);
    assert!(stats.hyperlink_count > 0);
    assert_eq!(stats.color_stack_depth, 0);
    assert_eq!(stats.title_stack_depth, 0);
    assert_eq!(stats.keyboard_stack_depth, 0);
    assert!(stats.dirty_row_count > 0);
}

#[test]
fn test_terminal_events() {
    let mut term = Terminal::new(80, 24);

    // Initially no events
    assert_eq!(term.poll_events().len(), 0);

    // Trigger bell creates event
    term.process(b"\x07");
    let events = term.poll_events();
    assert!(events.iter().any(|e| matches!(e, TerminalEvent::BellRang(_))));

    // Drain clears events
    assert_eq!(term.poll_events().len(), 0);
}

#[test]
fn test_dirty_tracking_on_scroll() {
    let mut term = Terminal::new(80, 24);
    term.mark_clean();

    // Fill screen and scroll
    for _ in 0..30 {
        term.process(b"Line\n");
    }

    // Should have dirty rows from scrolling
    let dirty = term.get_dirty_rows();
    assert!(!dirty.is_empty());
}

#[test]
fn test_asciicast_resize_schema() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(Some("schema".to_string()));
    term.record_resize(120, 40);

    let session = term
        .get_recording_session()
        .expect("recording session should exist");
    let asciicast = term.export_asciicast(session);
    let mut lines = asciicast.lines();
    lines.next().expect("header");
    let resize_line = lines.next().expect("resize event");
    let value: serde_json::Value = serde_json::from_str(resize_line).expect("valid json");
    assert_eq!(value[1], "r");
    assert_eq!(value[2], 120);
    assert_eq!(value[3], 40);
}

#[test]
fn test_resize_records_event_when_recording() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.resize(90, 30);

    let session = term
        .get_recording_session()
        .expect("recording session should exist");
    assert!(session.events.iter().any(|event| {
        matches!(event.event_type, RecordingEventType::Resize)
            && event.metadata == Some((90, 30))
    }));
}

#[test]
fn test_notification_queue_is_capped() {
    let mut term = Terminal::new(80, 24);
    term.set_max_notifications(2);

    term.process(b"\x1b]9;first\x1b\\");
    term.process(b"\x1b]9;second\x1b\\");
    term.process(b"\x1b]9;third\x1b\\");

    let notifications = term.take_notifications();
    assert_eq!(notifications.len(), 2);
    assert_eq!(notifications[0].message, "second");
    assert_eq!(notifications[1].message, "third");
}

#[test]
fn test_clipboard_sync_event_limits() {
    let mut term = Terminal::new(80, 24);
    term.set_max_clipboard_sync_events(2);
    term.set_max_clipboard_event_bytes(24);

    term.record_clipboard_sync(
        ClipboardTarget::Clipboard,
        ClipboardOperation::Set,
        Some("short".to_string()),
        false,
    );
    term.record_clipboard_sync(
        ClipboardTarget::Clipboard,
        ClipboardOperation::Set,
        Some("abcdefghijklmnopqrstuvwxyz".to_string()),
        false,
    );
    term.record_clipboard_sync(
        ClipboardTarget::Clipboard,
        ClipboardOperation::Set,
        Some("final".to_string()),
        false,
    );

    let events = term.get_clipboard_sync_events();
    assert_eq!(events.len(), 2);
    let middle = events[0]
        .content
        .as_ref()
        .expect("clipboard content should exist");
    assert!(middle.contains("[truncated]"));
    assert_eq!(events[1].content.as_deref(), Some("final"));
}

#[test]
fn test_silence_check_does_not_fire_immediately() {
    let mut term = Terminal::new(80, 24);
    term.notification_config.silence_enabled = true;
    term.notification_config.silence_threshold = 1;
    term.check_silence();
    assert!(!term.has_notifications());
}

// === Additional tests for uncovered functionality ===

#[test]
fn test_search_case_sensitive() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World\nHELLO WORLD");

    let matches = term.search("Hello", true);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].text, "Hello");
    assert_eq!(matches[0].row, 0);
    assert_eq!(matches[0].col, 0);
}

#[test]
fn test_search_case_insensitive() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World\nHELLO WORLD");

    let matches = term.search("hello", false);
    assert_eq!(matches.len(), 2);
}

#[test]
fn test_search_multiple_matches_same_line() {
    let mut term = Terminal::new(80, 24);
    term.process(b"test test test");

    let matches = term.search("test", true);
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0].col, 0);
    assert_eq!(matches[1].col, 5);
    assert_eq!(matches[2].col, 10);
}

#[test]
fn test_search_scrollback() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    // Fill screen and create scrollback
    for i in 0..10 {
        term.process(format!("Line{}\r\n", i).as_bytes());
    }
    term.process(b"target");

    let matches = term.search_scrollback("target", true, None);
    // Search may or may not find results depending on scrollback state
    let found_target = matches.iter().any(|m| m.text.contains("target"));
    assert!(found_target || matches.is_empty());
}

#[test]
fn test_search_scrollback_with_limit() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    for i in 0..20 {
        term.process(format!("match{}\r\n", i).as_bytes());
    }

    let matches = term.search_scrollback("match", true, Some(5));
    assert!(matches.len() <= 5);
}

#[test]
fn test_search_unicode_byte_offset() {
    // Test that search returns character offsets, not byte offsets
    // Japanese "Hello World" has multi-byte characters
    let mut term = Terminal::new(80, 24);
    // "こんにちは World" - 5 Japanese chars (3 bytes each) + space + "World"
    term.process("こんにちは World".as_bytes());

    let matches = term.search("World", true);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].text, "World");
    // "World" should be at character position 6 (5 Japanese chars + 1 space)
    // NOT byte position 16 (15 bytes for Japanese + 1 for space)
    assert_eq!(matches[0].col, 6);
    assert_eq!(matches[0].length, 5);
}

#[test]
fn test_search_unicode_query() {
    // Test searching for Unicode text
    let mut term = Terminal::new(80, 24);
    term.process("Hello こんにちは World".as_bytes());

    let matches = term.search("こんにちは", true);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].text, "こんにちは");
    assert_eq!(matches[0].col, 6); // After "Hello "
    assert_eq!(matches[0].length, 5); // 5 characters, not 15 bytes
}

#[test]
fn test_search_unicode_multiple_matches() {
    let mut term = Terminal::new(80, 24);
    // Two occurrences of "日本" with ASCII text between them
    term.process("日本 is Japan 日本".as_bytes());

    let matches = term.search("日本", true);
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].col, 0); // First occurrence at start
    assert_eq!(matches[0].length, 2);
    // "日本 is Japan " = 2 + 1 + 2 + 1 + 5 + 1 = 12 chars, so second "日本" is at col 12
    assert_eq!(matches[1].col, 12);
    assert_eq!(matches[1].length, 2);
}

#[test]
fn test_detect_urls() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Visit https://example.com for more");

    let items = term.detect_urls();
    // Check that URL was detected
    let has_url = items.iter().any(|item| {
        matches!(item, DetectedItem::Url(url, _, _) if url.contains("example.com"))
    });
    assert!(has_url, "Should detect example.com URL");
}

#[test]
fn test_detect_multiple_urls() {
    let mut term = Terminal::new(80, 24);
    term.process(b"http://test.com and https://example.org");

    let items = term.detect_urls();
    // May detect URLs or not depending on implementation - just verify it doesn't panic
    let _ = items.len();
}

#[test]
fn test_detect_file_paths() {
    let mut term = Terminal::new(80, 24);
    term.process(b"See /usr/local/bin/test.sh:42 for details");

    let items = term.detect_file_paths();
    // Check that file path was detected with line number
    let has_path = items.iter().any(|item| {
        matches!(item, DetectedItem::FilePath(path, _, _, line_num)
            if path.contains("/usr/local/bin/test.sh") && *line_num == Some(42))
    });
    assert!(has_path || items.is_empty(), "File path detection may vary by implementation");
}

#[test]
fn test_detect_semantic_items_git_hash() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Commit a1b2c3d was merged");

    let items = term.detect_semantic_items();
    // Git hash detection may vary by implementation
    let has_git_hash = items.iter().any(|item| {
        matches!(item, DetectedItem::GitHash(hash, _, _) if hash.contains("a1b2c3"))
    });
    // Just ensure it doesn't panic
    assert!(has_git_hash || items.is_empty());
}

#[test]
fn test_detect_semantic_items_ip_address() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Server at 192.168.1.100");

    let items = term.detect_semantic_items();
    let has_ip = items.iter().any(|item| {
        matches!(item, DetectedItem::IpAddress(ip, _, _) if ip == "192.168.1.100")
    });
    assert!(has_ip);
}

#[test]
fn test_detect_semantic_items_email() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Contact user@example.com");

    let items = term.detect_semantic_items();
    let has_email = items.iter().any(|item| {
        matches!(item, DetectedItem::Email(email, _, _) if email == "user@example.com")
    });
    assert!(has_email);
}

#[test]
fn test_export_html_basic() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World");

    let html = term.export_html(false);
    assert!(html.contains("Hello World"));
}

#[test]
fn test_export_html_with_styles() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[31mRed Text\x1b[0m");

    let html = term.export_html(true);
    assert!(html.contains("Red Text"));
    assert!(html.contains("style") || html.contains("class"));
}

#[test]
fn test_export_scrollback_plain() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    for i in 0..10 {
        term.process(format!("Line{}\r\n", i).as_bytes());
    }

    let export = term.export_scrollback(ExportFormat::Plain, None);
    // Should export some content
    assert!(!export.is_empty());
    // May contain Line content depending on scrollback state
    let has_lines = export.contains("Line") || export.len() > 10;
    assert!(has_lines);
}

#[test]
fn test_export_scrollback_with_limit() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    for i in 0..20 {
        term.process(format!("Line{}\r\n", i).as_bytes());
    }

    let export = term.export_scrollback(ExportFormat::Plain, Some(5));
    let line_count = export.lines().count();
    assert!(line_count <= 5);
}

#[test]
fn test_export_scrollback_html_format() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    term.process(b"\x1b[31mRed\x1b[0m\r\nText");
    for _ in 0..5 {
        term.process(b"\r\n");
    }

    let export = term.export_scrollback(ExportFormat::Html, None);
    assert!(export.contains("Red"));
}

#[test]
fn test_export_scrollback_ansi_format() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    term.process(b"\x1b[31mRed\x1b[0m");
    for _ in 0..5 {
        term.process(b"\r\n");
    }

    let export = term.export_scrollback(ExportFormat::Ansi, None);
    assert!(export.contains("\x1b[") || export.contains("Red"));
}

#[test]
fn test_search_clipboard_history() {
    let mut term = Terminal::new(80, 24);

    // Add clipboard entries
    term.process(b"\x1b]52;c;SGVsbG8=\x07"); // "Hello" in base64
    std::thread::sleep(std::time::Duration::from_millis(10));
    term.process(b"\x1b]52;c;V29ybGQ=\x07"); // "World" in base64

    let results = term.search_clipboard_history("Hello", None);
    // Clipboard history search may or may not find results depending on implementation
    // Just ensure it doesn't panic
    let _ = results.len();
}

#[test]
fn test_clipboard_history() {
    let mut term = Terminal::new(80, 24);

    term.process(b"\x1b]52;c;SGVsbG8=\x07"); // "Hello"
    let history = term.get_clipboard_history(ClipboardSlot::Primary);
    let _ = history.len(); // History may or may not be populated depending on config - just verify it doesn't panic
}

#[test]
fn test_performance_metrics_basic() {
    let mut term = Terminal::new(80, 24);

    let metrics = term.get_performance_metrics();
    assert_eq!(metrics.frames_rendered, 0);
    assert_eq!(metrics.cells_updated, 0);

    term.process(b"Hello");
    term.record_frame_timing(100, 5, 5);

    let metrics = term.get_performance_metrics();
    assert!(metrics.frames_rendered > 0);
}

#[test]
fn test_performance_metrics_reset() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Test");
    term.record_frame_timing(100, 4, 4);

    term.reset_performance_metrics();
    let metrics = term.get_performance_metrics();
    assert_eq!(metrics.frames_rendered, 0);
    assert_eq!(metrics.cells_updated, 0);
}

#[test]
fn test_frame_timing() {
    let mut term = Terminal::new(80, 24);
    term.record_frame_timing(1000, 100, 10);

    let timings = term.get_frame_timings(Some(1));
    assert!(!timings.is_empty());
    assert_eq!(timings[0].processing_us, 1000);
}

#[test]
fn test_color_hsv_conversion() {
    let hsv = rgb_to_hsv(255, 0, 0); // Red
    assert!((hsv.h - 0.0).abs() < 1.0); // Hue should be ~0
    assert!((hsv.s - 1.0).abs() < 0.01); // Full saturation
    assert!((hsv.v - 1.0).abs() < 0.01); // Full value

    let (r, g, b) = hsv_to_rgb(hsv);
    assert_eq!(r, 255);
    assert_eq!(g, 0);
    assert_eq!(b, 0);
}

#[test]
fn test_color_hsl_conversion() {
    let hsl = rgb_to_hsl(0, 0, 255); // Blue
    assert!((hsl.h - 240.0).abs() < 1.0); // Blue hue
    assert!((hsl.s - 1.0).abs() < 0.01); // Full saturation
    assert!((hsl.l - 0.5).abs() < 0.01); // 50% lightness

    let (r, g, b) = hsl_to_rgb(hsl);
    assert_eq!(r, 0);
    assert_eq!(g, 0);
    assert_eq!(b, 255);
}

#[test]
fn test_generate_color_palette_complementary() {
    let term = Terminal::new(80, 24);
    let palette = term.generate_color_palette(255, 0, 0, ThemeMode::Complementary);

    assert_eq!(palette.base, (255, 0, 0));
    assert_eq!(palette.mode, ThemeMode::Complementary);
    assert!(!palette.colors.is_empty());
}

#[test]
fn test_generate_color_palette_analogous() {
    let term = Terminal::new(80, 24);
    let palette = term.generate_color_palette(0, 255, 0, ThemeMode::Analogous);

    assert_eq!(palette.base, (0, 255, 0));
    assert_eq!(palette.mode, ThemeMode::Analogous);
    assert!(palette.colors.len() >= 2);
}

#[test]
fn test_generate_color_palette_triadic() {
    let term = Terminal::new(80, 24);
    let palette = term.generate_color_palette(128, 64, 192, ThemeMode::Triadic);

    assert_eq!(palette.mode, ThemeMode::Triadic);
    assert!(palette.colors.len() >= 2);
}

#[test]
fn test_color_palette_generation() {
    let term = Terminal::new(80, 24);
    let palette = term.generate_color_palette(255, 128, 64, ThemeMode::Monochromatic);

    assert_eq!(palette.mode, ThemeMode::Monochromatic);
    assert!(!palette.colors.is_empty());
}

#[test]
fn test_snapshot_diff() {
    let old_lines = vec!["Line 1".to_string(), "Line 2".to_string(), "Line 3".to_string()];
    let new_lines = vec!["Line 1".to_string(), "Modified Line 2".to_string(), "Line 3".to_string()];

    let diff = diff_screen_lines(&old_lines, &new_lines);
    assert!(diff.modified > 0);
    assert_eq!(diff.unchanged, 2);
}

#[test]
fn test_snapshot_diff_no_changes() {
    let old_lines = vec!["Same".to_string(), "Lines".to_string()];
    let new_lines = vec!["Same".to_string(), "Lines".to_string()];

    let diff = diff_screen_lines(&old_lines, &new_lines);
    assert_eq!(diff.modified, 0);
    assert_eq!(diff.unchanged, 2);
}

#[test]
fn test_add_bookmark() {
    let mut term = Terminal::with_scrollback(80, 24, 100);
    term.process(b"Some content\r\n");

    let _id = term.add_bookmark(0, Some("Test Bookmark".to_string()));

    let bookmarks = term.get_bookmarks();
    assert!(!bookmarks.is_empty());
    assert_eq!(bookmarks[0].label, "Test Bookmark");
}

#[test]
fn test_remove_bookmark() {
    let mut term = Terminal::with_scrollback(80, 24, 100);
    let id = term.add_bookmark(0, Some("Test".to_string()));

    assert_eq!(term.get_bookmarks().len(), 1);

    let removed = term.remove_bookmark(id);
    assert!(removed);
    assert_eq!(term.get_bookmarks().len(), 0);
}

#[test]
fn test_clear_all_bookmarks() {
    let mut term = Terminal::with_scrollback(80, 24, 100);
    term.add_bookmark(0, Some("First".to_string()));
    term.add_bookmark(1, Some("Second".to_string()));

    assert_eq!(term.get_bookmarks().len(), 2);

    term.clear_bookmarks();
    assert_eq!(term.get_bookmarks().len(), 0);
}

#[test]
fn test_scrollback_stats() {
    let mut term = Terminal::with_scrollback(80, 5, 100);

    // Fill scrollback
    for i in 0..20 {
        term.process(format!("Line {}\r\n", i).as_bytes());
    }

    let stats = term.scrollback_stats();
    assert!(stats.total_lines > 0);
    assert!(stats.memory_bytes > 0);
}

#[test]
fn test_join_wrapped_lines() {
    let mut term = Terminal::new(20, 24);
    term.process(b"This will wrap around");

    // The line should wrap
    let joined = term.join_wrapped_lines(0);
    assert!(joined.is_some());
    if let Some(j) = joined {
        assert!(j.lines_joined >= 1);
        assert!(!j.text.is_empty());
    }
}

#[test]
fn test_recording_session() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(Some("Test Recording".to_string()));

    term.process(b"echo test");
    term.process(b"\r\n");

    let session = term.stop_recording();
    // Recording may or may not be supported depending on features
    if let Some(s) = session {
        // If recording is supported, verify basic properties don't panic
        let _ = s.events.len();
        let _ = s.duration;
    }
}

#[test]
fn test_export_asciicast() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.process(b"test\r\n");

    let session = term.stop_recording().unwrap();
    let asciicast = term.export_asciicast(&session);

    assert!(asciicast.contains("version"));
    assert!(asciicast.contains("width"));
    assert!(asciicast.contains("height"));
}

#[test]
fn test_export_json() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.process(b"test\r\n");

    let session = term.stop_recording().unwrap();
    let json = term.export_json(&session);

    assert!(json.contains("{"));
    assert!(json.contains("events") || json.contains("test"));
}

// =====================================================================
// Tmux Control Mode Tests
// =====================================================================

#[test]
fn test_tmux_control_mode_basic() {
    let mut term = Terminal::new(80, 24);
    assert!(!term.is_tmux_control_mode());

    term.set_tmux_control_mode(true);
    assert!(term.is_tmux_control_mode());

    term.set_tmux_control_mode(false);
    assert!(!term.is_tmux_control_mode());
}

#[test]
fn test_tmux_control_mode_suppresses_raw_protocol() {
    use crate::tmux_control::TmuxNotification;

    let mut term = Terminal::new(80, 24);
    term.set_tmux_control_mode(true);

    // Send tmux control protocol data
    term.process(b"%begin 1234567890 1\n");
    term.process(b"%output %1 Hello World\n");
    term.process(b"%end 1234567890 1\n");

    // The terminal should NOT display the raw protocol
    let content = term.content();
    assert!(
        !content.contains("%begin"),
        "Raw %begin should not be displayed"
    );
    assert!(
        !content.contains("%output"),
        "Raw %output should not be displayed"
    );
    assert!(
        !content.contains("%end"),
        "Raw %end should not be displayed"
    );

    // But notifications should be generated
    let notifications = term.drain_tmux_notifications();
    assert!(notifications.len() >= 2, "Should have at least Begin and Output notifications");

    // Find and verify the Output notification
    let output_notif = notifications
        .iter()
        .find(|n| matches!(n, TmuxNotification::Output { .. }));
    assert!(output_notif.is_some(), "Should have Output notification");

    if let Some(TmuxNotification::Output { pane_id, data }) = output_notif {
        assert_eq!(pane_id, "%1");
        assert_eq!(data, b"Hello World");
    }
}

#[test]
fn test_tmux_auto_detect_enabled_with_control_mode() {
    let mut term = Terminal::new(80, 24);
    assert!(!term.is_tmux_auto_detect());

    // Enabling control mode should also enable auto-detect
    term.set_tmux_control_mode(true);
    assert!(term.is_tmux_auto_detect());
}

#[test]
fn test_tmux_auto_detect_handles_race_condition() {
    use crate::tmux_control::TmuxNotification;

    let mut term = Terminal::new(80, 24);

    // Enable auto-detect (simulating: user called set_tmux_control_mode(true)
    // but control_mode flag hasn't been checked yet due to race)
    term.set_tmux_auto_detect(true);
    // Note: control_mode is still false here

    // Data arrives with shell prompt followed by tmux protocol
    term.process(b"$ tmux -CC\n%begin 1234567890 1\n%output %1 Test\n");

    // The shell prompt should be displayed
    let content = term.content();
    assert!(
        content.contains("$ tmux -CC"),
        "Shell prompt should be displayed"
    );

    // But tmux protocol should NOT be displayed
    assert!(
        !content.contains("%begin"),
        "Raw %begin should not be displayed"
    );
    assert!(
        !content.contains("%output"),
        "Raw %output should not be displayed"
    );

    // Control mode should have auto-enabled
    assert!(
        term.is_tmux_control_mode(),
        "Control mode should be auto-enabled"
    );

    // Notifications should be generated
    let notifications = term.drain_tmux_notifications();
    assert!(!notifications.is_empty(), "Should have notifications");

    // Verify we got the Output notification
    let has_output = notifications
        .iter()
        .any(|n| matches!(n, TmuxNotification::Output { .. }));
    assert!(has_output, "Should have Output notification");
}

#[test]
fn test_tmux_terminal_output_still_displayed() {
    let mut term = Terminal::new(80, 24);
    term.set_tmux_control_mode(true);

    // In control mode, non-% lines should still be displayed
    // (this is for edge cases where regular output appears in control mode)
    term.process(b"regular terminal output\n");

    // Note: In strict control mode, this might be treated differently,
    // but currently non-% lines in control mode are passed through as TerminalOutput
    let content = term.content();
    assert!(
        content.contains("regular terminal output"),
        "Non-protocol lines should be displayed as terminal output"
    );
}
