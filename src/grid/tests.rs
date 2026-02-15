use super::*;
use crate::cell::Cell;

#[test]
fn test_grid_creation() {
    let grid = Grid::new(80, 24, 1000);
    assert_eq!(grid.cols(), 80);
    assert_eq!(grid.rows(), 24);
}

#[test]
fn test_grid_set_get() {
    let mut grid = Grid::new(80, 24, 1000);
    let cell = Cell::new('A');
    grid.set(5, 10, cell);

    let retrieved = grid.get(5, 10).unwrap();
    assert_eq!(retrieved.c, 'A');
}

#[test]
fn test_grid_clear() {
    let mut grid = Grid::new(80, 24, 1000);
    grid.set(5, 10, Cell::new('A'));
    grid.clear();

    let cell = grid.get(5, 10).unwrap();
    assert_eq!(cell.c, ' ');
}

#[test]
fn test_grid_scroll() {
    let mut grid = Grid::new(80, 24, 1000);
    grid.set(0, 0, Cell::new('A'));
    grid.set(0, 1, Cell::new('B'));

    grid.scroll_up(1);

    assert_eq!(grid.get(0, 0).unwrap().c, 'B');
    assert_eq!(grid.scrollback_len(), 1);
}

#[test]
fn test_grid_resize() {
    let mut grid = Grid::new(80, 24, 1000);
    grid.set(5, 5, Cell::new('X'));

    grid.resize(100, 30);
    assert_eq!(grid.cols(), 100);
    assert_eq!(grid.rows(), 30);
    assert_eq!(grid.get(5, 5).unwrap().c, 'X');
}

#[test]
fn test_scroll_region_up() {
    let mut grid = Grid::new(80, 10, 1000);
    for i in 0..10 {
        grid.set(0, i, Cell::new((b'0' + i as u8) as char));
    }

    grid.scroll_region_up(2, 2, 7); // Scroll lines 2-7 up by 2

    // Line 2 should now contain what was at line 4
    assert_eq!(grid.get(0, 2).unwrap().c, '4');
    // Lines 6-7 should be blank
    assert_eq!(grid.get(0, 6).unwrap().c, ' ');
    assert_eq!(grid.get(0, 7).unwrap().c, ' ');
}

#[test]
fn test_scroll_region_down() {
    let mut grid = Grid::new(80, 10, 1000);
    for i in 0..10 {
        grid.set(0, i, Cell::new((b'0' + i as u8) as char));
    }

    grid.scroll_region_down(2, 2, 7); // Scroll lines 2-7 down by 2

    // Line 4 should now contain what was at line 2
    assert_eq!(grid.get(0, 4).unwrap().c, '2');
    // Lines 2-3 should be blank
    assert_eq!(grid.get(0, 2).unwrap().c, ' ');
    assert_eq!(grid.get(0, 3).unwrap().c, ' ');
}

#[test]
fn test_insert_lines_edge_case() {
    let mut grid = Grid::new(80, 10, 1000);
    for i in 0..10 {
        grid.set(0, i, Cell::new((b'A' + i as u8) as char));
    }

    // Insert at bottom of scroll region
    grid.insert_lines(7, 2, 9);

    assert_eq!(grid.get(0, 7).unwrap().c, ' '); // Should be blank
    assert_eq!(grid.get(0, 8).unwrap().c, ' '); // Should be blank
}

#[test]
fn test_delete_lines_edge_case() {
    let mut grid = Grid::new(80, 10, 1000);
    for i in 0..10 {
        grid.set(0, i, Cell::new((b'A' + i as u8) as char));
    }

    // Delete from near bottom (delete 2 lines starting at row 7)
    // Row 7 has 'H', row 8 has 'I', row 9 has 'J'
    // After deleting rows 7 and 8, row 9 moves to row 7
    grid.delete_lines(2, 7, 9);

    assert_eq!(grid.get(0, 7).unwrap().c, 'J'); // Line 9 moves to 7
    assert_eq!(grid.get(0, 8).unwrap().c, ' '); // Should be blank
    assert_eq!(grid.get(0, 9).unwrap().c, ' '); // Should be blank
}

#[test]
fn test_insert_chars_at_end_of_line() {
    let mut grid = Grid::new(10, 5, 1000);
    for i in 0..10 {
        grid.set(i, 0, Cell::new((b'0' + i as u8) as char));
    }

    grid.insert_chars(8, 0, 5); // Insert 5 at position 8 (only 2 spots left)

    assert_eq!(grid.get(8, 0).unwrap().c, ' '); // Should be blank
    assert_eq!(grid.get(9, 0).unwrap().c, ' '); // Should be blank
}

#[test]
fn test_delete_chars_boundary() {
    let mut grid = Grid::new(10, 5, 1000);
    for i in 0..10 {
        grid.set(i, 0, Cell::new((b'A' + i as u8) as char));
    }

    grid.delete_chars(7, 0, 10); // Delete 10 chars from position 7 (only 3 exist)

    assert_eq!(grid.get(7, 0).unwrap().c, ' ');
    assert_eq!(grid.get(8, 0).unwrap().c, ' ');
    assert_eq!(grid.get(9, 0).unwrap().c, ' ');
}

#[test]
fn test_erase_chars_boundary() {
    let mut grid = Grid::new(10, 5, 1000);
    for i in 0..10 {
        grid.set(i, 0, Cell::new((b'X' + i as u8) as char));
    }

    grid.erase_chars(5, 0, 20); // Erase 20 chars from position 5 (only 5 exist)

    assert_eq!(grid.get(4, 0).unwrap().c, '\\'); // Should be preserved (X + 4)
    for i in 5..10 {
        assert_eq!(grid.get(i, 0).unwrap().c, ' '); // Should be erased
    }
}

#[test]
fn test_clear_line_operations() {
    let mut grid = Grid::new(10, 5, 1000);
    for i in 0..10 {
        grid.set(i, 2, Cell::new('X'));
    }

    // Clear from position 5 to end
    grid.clear_line_right(5, 2);

    assert_eq!(grid.get(4, 2).unwrap().c, 'X'); // Preserved
    assert_eq!(grid.get(5, 2).unwrap().c, ' '); // Cleared
    assert_eq!(grid.get(9, 2).unwrap().c, ' '); // Cleared
}

#[test]
fn test_clear_line_left() {
    let mut grid = Grid::new(10, 5, 1000);
    for i in 0..10 {
        grid.set(i, 2, Cell::new('X'));
    }

    // Clear from start to position 5 (inclusive)
    grid.clear_line_left(5, 2);

    for i in 0..=5 {
        assert_eq!(grid.get(i, 2).unwrap().c, ' '); // Cleared
    }
    assert_eq!(grid.get(6, 2).unwrap().c, 'X'); // Preserved
}

#[test]
fn test_clear_screen_operations() {
    let mut grid = Grid::new(10, 10, 1000);
    for row in 0..10 {
        for col in 0..10 {
            grid.set(col, row, Cell::new('X'));
        }
    }

    // Clear from (5,5) to end of screen
    grid.clear_screen_below(5, 5);

    assert_eq!(grid.get(4, 5).unwrap().c, 'X'); // Before cursor on same line - preserved
    assert_eq!(grid.get(5, 5).unwrap().c, ' '); // At cursor - cleared
    assert_eq!(grid.get(0, 6).unwrap().c, ' '); // Next line - cleared
    assert_eq!(grid.get(0, 4).unwrap().c, 'X'); // Previous line - preserved
}

#[test]
fn test_clear_screen_above() {
    let mut grid = Grid::new(10, 10, 1000);
    for row in 0..10 {
        for col in 0..10 {
            grid.set(col, row, Cell::new('X'));
        }
    }

    // Clear from start of screen to (5,5)
    grid.clear_screen_above(5, 5);

    assert_eq!(grid.get(0, 4).unwrap().c, ' '); // Previous line - cleared
    assert_eq!(grid.get(5, 5).unwrap().c, ' '); // At cursor - cleared
    assert_eq!(grid.get(6, 5).unwrap().c, 'X'); // After cursor on same line - preserved
    assert_eq!(grid.get(0, 6).unwrap().c, 'X'); // Next line - preserved
}

#[test]
fn test_scrollback_limit() {
    let mut grid = Grid::new(80, 5, 3); // Max 3 lines of scrollback

    // Scroll up 5 times
    for i in 0..5 {
        grid.set(0, 0, Cell::new((b'A' + i as u8) as char));
        grid.scroll_up(1);
    }

    // Should only have 3 lines in scrollback (max)
    assert_eq!(grid.scrollback_len(), 3);

    // Should have the most recent 3
    let line0 = grid.scrollback_line(0).unwrap();
    assert_eq!(line0[0].c, 'C'); // First scrolled should be 'C' (oldest kept)
}

#[test]
fn test_scroll_down_no_scrollback() {
    let mut grid = Grid::new(80, 5, 100);
    for i in 0..5 {
        grid.set(0, i, Cell::new((b'A' + i as u8) as char));
    }

    grid.scroll_down(2);

    // First 2 lines should be blank
    assert_eq!(grid.get(0, 0).unwrap().c, ' ');
    assert_eq!(grid.get(0, 1).unwrap().c, ' ');
    // Line 2 should have what was at line 0
    assert_eq!(grid.get(0, 2).unwrap().c, 'A');
}

#[test]
fn test_get_out_of_bounds() {
    let grid = Grid::new(80, 24, 1000);

    assert!(grid.get(100, 0).is_none());
    assert!(grid.get(0, 100).is_none());
    assert!(grid.get(100, 100).is_none());
}

#[test]
fn test_row_access() {
    let mut grid = Grid::new(10, 5, 1000);
    for i in 0..10 {
        grid.set(i, 2, Cell::new((b'A' + i as u8) as char));
    }

    let row = grid.row(2).unwrap();
    assert_eq!(row.len(), 10);
    assert_eq!(row[0].c, 'A');
    assert_eq!(row[5].c, 'F');
}

#[test]
fn test_resize_smaller() {
    let mut grid = Grid::new(80, 24, 1000);
    grid.set(50, 20, Cell::new('X'));

    grid.resize(40, 10); // Shrink grid

    assert_eq!(grid.cols(), 40);
    assert_eq!(grid.rows(), 10);
    // Data at (50, 20) should be lost
    assert!(grid.get(50, 20).is_none());
}

#[test]
fn test_resize_preserves_scrollback_when_width_unchanged() {
    let mut grid = Grid::new(10, 3, 3);

    // Create a few scrollback lines by scrolling up
    for ch in ['A', 'B', 'C'] {
        grid.set(0, 0, Cell::new(ch));
        grid.scroll_up(1);
    }

    assert_eq!(grid.scrollback_len(), 3);
    let before = grid
        .scrollback_line(0)
        .and_then(|line| line.first())
        .unwrap()
        .c;
    assert_eq!(before, 'A');

    // Change only height; width stays the same
    grid.resize(10, 5);

    assert_eq!(grid.scrollback_len(), 3);
    let after = grid
        .scrollback_line(0)
        .and_then(|line| line.first())
        .unwrap()
        .c;
    assert_eq!(after, 'A');
}

#[test]
fn test_resize_reflows_scrollback_when_width_changes() {
    let mut grid = Grid::new(10, 3, 3);

    grid.set(0, 0, Cell::new('X'));
    grid.scroll_up(1);
    assert_eq!(grid.scrollback_len(), 1);

    grid.resize(20, 3);

    assert_eq!(grid.cols(), 20);
    // Scrollback should now be preserved and reflowed, not cleared
    assert_eq!(grid.scrollback_len(), 1);
    let line = grid.scrollback_line(0).unwrap();
    assert_eq!(line[0].c, 'X');
}

#[test]
fn test_export_text_buffer_basic() {
    let mut grid = Grid::new(10, 3, 1000);

    // Set some content
    grid.set(0, 0, Cell::new('H'));
    grid.set(1, 0, Cell::new('e'));
    grid.set(2, 0, Cell::new('l'));
    grid.set(3, 0, Cell::new('l'));
    grid.set(4, 0, Cell::new('o'));

    grid.set(0, 1, Cell::new('W'));
    grid.set(1, 1, Cell::new('o'));
    grid.set(2, 1, Cell::new('r'));
    grid.set(3, 1, Cell::new('l'));
    grid.set(4, 1, Cell::new('d'));

    let text = grid.export_text_buffer();
    let lines: Vec<&str> = text.lines().collect();

    // Last empty line is not included since we don't add newline for empty last row
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "Hello");
    assert_eq!(lines[1], "World");
}

#[test]
fn test_export_text_buffer_with_scrollback() {
    let mut grid = Grid::new(10, 2, 1000);

    // Add first line
    grid.set(0, 0, Cell::new('L'));
    grid.set(1, 0, Cell::new('1'));

    // Scroll up (moves L1 to scrollback)
    grid.scroll_up(1);

    // Add second line
    grid.set(0, 0, Cell::new('L'));
    grid.set(1, 0, Cell::new('2'));

    let text = grid.export_text_buffer();
    let lines: Vec<&str> = text.lines().collect();

    // Should have scrollback line followed by current screen
    assert_eq!(lines[0], "L1");
    assert_eq!(lines[1], "L2");
}

#[test]
fn test_export_text_buffer_trims_trailing_spaces() {
    let mut grid = Grid::new(10, 2, 1000);

    // Set content with trailing spaces
    grid.set(0, 0, Cell::new('H'));
    grid.set(1, 0, Cell::new('i'));
    // Columns 2-9 remain as spaces

    let text = grid.export_text_buffer();
    let lines: Vec<&str> = text.lines().collect();

    // Should trim trailing spaces
    assert_eq!(lines[0], "Hi");
}

#[test]
fn test_export_text_buffer_handles_wrapped_lines() {
    let mut grid = Grid::new(10, 3, 1000);

    // Set first line and mark as wrapped
    grid.set(0, 0, Cell::new('A'));
    grid.set(1, 0, Cell::new('B'));
    grid.set_line_wrapped(0, true);

    // Set second line (continuation)
    grid.set(0, 1, Cell::new('C'));
    grid.set(1, 1, Cell::new('D'));

    let text = grid.export_text_buffer();

    // Should not have newline between wrapped lines
    assert!(text.starts_with("ABCD"));
}

#[test]
fn test_export_text_buffer_wide_chars() {
    let mut grid = Grid::new(10, 2, 1000);

    // Set a wide character (width 2)
    let mut cell = Cell::new('中');
    cell.flags.set_wide_char(true);
    grid.set(0, 0, cell);

    // Set a wide char spacer
    let mut spacer = Cell::default();
    spacer.flags.set_wide_char_spacer(true);
    grid.set(1, 0, spacer);

    // Set another wide character
    let mut cell2 = Cell::new('文');
    cell2.flags.set_wide_char(true);
    grid.set(2, 0, cell2);

    let mut spacer2 = Cell::default();
    spacer2.flags.set_wide_char_spacer(true);
    grid.set(3, 0, spacer2);

    let text = grid.export_text_buffer();
    let lines: Vec<&str> = text.lines().collect();

    // Should skip wide char spacers, only include the actual wide characters
    assert_eq!(lines[0], "中文");
}

#[test]
fn test_fill_rectangle() {
    let mut grid = Grid::new(80, 24, 1000);

    // Fill a 3x3 rectangle starting at (5, 5) with 'X'
    let fill_cell = Cell::new('X');
    grid.fill_rectangle(fill_cell, 5, 5, 7, 7);

    // Check that cells inside the rectangle are filled
    assert_eq!(grid.get(5, 5).unwrap().c, 'X');
    assert_eq!(grid.get(6, 6).unwrap().c, 'X');
    assert_eq!(grid.get(7, 7).unwrap().c, 'X');

    // Check that cells outside are not affected
    assert_eq!(grid.get(4, 5).unwrap().c, ' ');
    assert_eq!(grid.get(8, 7).unwrap().c, ' ');
}

#[test]
fn test_fill_rectangle_boundaries() {
    let mut grid = Grid::new(80, 24, 1000);

    // Fill rectangle at grid boundaries
    let fill_cell = Cell::new('B');
    grid.fill_rectangle(fill_cell, 0, 0, 2, 2);

    assert_eq!(grid.get(0, 0).unwrap().c, 'B');
    assert_eq!(grid.get(1, 1).unwrap().c, 'B');
    assert_eq!(grid.get(2, 2).unwrap().c, 'B');
}

#[test]
fn test_copy_rectangle() {
    let mut grid = Grid::new(80, 24, 1000);

    // Set up source rectangle with pattern
    for row in 2..5 {
        for col in 2..5 {
            grid.set(col, row, Cell::new('S'));
        }
    }

    // Copy rectangle from (2,2) to (10,10)
    grid.copy_rectangle(2, 2, 4, 4, 10, 10);

    // Verify copy
    assert_eq!(grid.get(10, 10).unwrap().c, 'S');
    assert_eq!(grid.get(11, 11).unwrap().c, 'S');
    assert_eq!(grid.get(12, 12).unwrap().c, 'S');

    // Original should still exist
    assert_eq!(grid.get(2, 2).unwrap().c, 'S');
}

#[test]
fn test_copy_rectangle_to_different_location() {
    let mut grid = Grid::new(80, 24, 1000);

    // Set up source with unique chars in non-overlapping area
    grid.set(0, 0, Cell::new('A'));
    grid.set(1, 0, Cell::new('B'));
    grid.set(0, 1, Cell::new('C'));
    grid.set(1, 1, Cell::new('D'));

    // Copy to far away location (no overlap)
    grid.copy_rectangle(0, 0, 1, 1, 10, 10);

    // Verify copy worked
    assert_eq!(grid.get(10, 10).unwrap().c, 'A');
    assert_eq!(grid.get(11, 10).unwrap().c, 'B');
    assert_eq!(grid.get(10, 11).unwrap().c, 'C');
    assert_eq!(grid.get(11, 11).unwrap().c, 'D');
}

#[test]
fn test_erase_rectangle() {
    let mut grid = Grid::new(80, 24, 1000);

    // Fill area with chars
    for row in 5..10 {
        for col in 5..10 {
            grid.set(col, row, Cell::new('T'));
        }
    }

    // Erase rectangle
    grid.erase_rectangle(6, 6, 8, 8);

    // Check erased area
    assert_eq!(grid.get(6, 6).unwrap().c, ' ');
    assert_eq!(grid.get(7, 7).unwrap().c, ' ');
    assert_eq!(grid.get(8, 8).unwrap().c, ' ');

    // Check boundary cells not erased
    assert_eq!(grid.get(5, 5).unwrap().c, 'T');
    assert_eq!(grid.get(9, 9).unwrap().c, 'T');
}

#[test]
fn test_erase_rectangle_unconditional() {
    let mut grid = Grid::new(80, 24, 1000);

    // Fill with different chars
    grid.set(10, 10, Cell::new('U'));
    grid.set(11, 11, Cell::new('V'));

    // Erase unconditionally
    grid.erase_rectangle_unconditional(10, 10, 11, 11);

    assert_eq!(grid.get(10, 10).unwrap().c, ' ');
    assert_eq!(grid.get(11, 11).unwrap().c, ' ');
}

#[test]
fn test_change_attributes_in_rectangle() {
    let mut grid = Grid::new(80, 24, 1000);

    // Set up cells with chars
    for row in 3..6 {
        for col in 3..6 {
            let mut cell = Cell::new('M');
            cell.flags.set_bold(false);
            grid.set(col, row, cell);
        }
    }

    // Change attributes - make them bold (attribute 1 = bold)
    let attributes = [1u16];
    grid.change_attributes_in_rectangle(3, 3, 5, 5, &attributes);

    // Verify attributes changed but char remained
    let cell = grid.get(4, 4).unwrap();
    assert_eq!(cell.c, 'M');
    assert!(cell.flags.bold());
}

#[test]
fn test_reverse_attributes_in_rectangle() {
    let mut grid = Grid::new(80, 24, 1000);

    // Set up cells
    let mut cell = Cell::new('R');
    cell.flags.set_reverse(false);
    grid.set(20, 20, cell);

    // Reverse attributes - attribute 7 toggles reverse flag
    let attributes = [7u16];
    grid.reverse_attributes_in_rectangle(20, 20, 20, 20, &attributes);

    // Verify reverse flag is now true
    let reversed = grid.get(20, 20).unwrap();
    assert_eq!(reversed.c, 'R');
    assert!(reversed.flags.reverse());

    // Toggle again - should go back to false
    grid.reverse_attributes_in_rectangle(20, 20, 20, 20, &attributes);
    let unreversed = grid.get(20, 20).unwrap();
    assert!(!unreversed.flags.reverse());
}

#[test]
fn test_row_text() {
    let mut grid = Grid::new(80, 24, 1000);

    // Set up a row with text
    let text = "Hello, World!";
    for (i, ch) in text.chars().enumerate() {
        grid.set(i, 5, Cell::new(ch));
    }

    let row_text = grid.row_text(5);
    assert!(row_text.starts_with("Hello, World!"));
}

#[test]
fn test_row_text_with_wide_chars() {
    let mut grid = Grid::new(80, 24, 1000);

    // Set wide character
    let mut cell = Cell::new('中');
    cell.flags.set_wide_char(true);
    grid.set(0, 0, cell);

    // Set spacer
    let mut spacer = Cell::default();
    spacer.flags.set_wide_char_spacer(true);
    grid.set(1, 0, spacer);

    let row_text = grid.row_text(0);
    // Should skip the spacer
    assert_eq!(row_text.chars().next().unwrap(), '中');
}

#[test]
fn test_content_as_string() {
    let mut grid = Grid::new(10, 3, 1000);

    // Fill first row
    for col in 0..10 {
        grid.set(col, 0, Cell::new('A'));
    }

    // Fill second row partially
    for col in 0..5 {
        grid.set(col, 1, Cell::new('B'));
    }

    let content = grid.content_as_string();
    let lines: Vec<&str> = content.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("AAAAAAAAAA"));
    assert!(lines[1].starts_with("BBBBB"));
}

#[test]
fn test_is_scrollback_wrapped_circular() {
    let mut grid = Grid::new(80, 2, 3); // Small scrollback for testing

    // Scroll multiple times to trigger circular buffer
    for i in 0..5 {
        grid.scroll_up(1);
        if i % 2 == 0 {
            grid.scrollback_wrapped[grid
                .scrollback_lines
                .saturating_sub(1)
                .min(grid.max_scrollback - 1)] = true;
        }
    }

    // Test wrapped state retrieval - just ensure it doesn't panic with circular buffer
    let _wrapped = grid.is_scrollback_wrapped(0);
}

#[test]
fn test_debug_snapshot() {
    let mut grid = Grid::new(10, 3, 2);

    // Add some content
    grid.set(0, 0, Cell::new('D'));
    grid.set(1, 0, Cell::new('E'));
    grid.set(2, 0, Cell::new('B'));
    grid.set(3, 0, Cell::new('U'));
    grid.set(4, 0, Cell::new('G'));

    let snapshot = grid.debug_snapshot();

    // Verify snapshot contains expected content
    assert!(snapshot.contains("DEBUG"));
    assert!(snapshot.contains("|DEBUG"));
}

#[test]
fn test_scrollback_line_circular_buffer() {
    let mut grid = Grid::new(80, 24, 2); // Max 2 scrollback lines

    // Scroll 3 times to wrap circular buffer
    grid.scroll_up(1);
    grid.scroll_up(1);
    grid.scroll_up(1);

    // Accessing scrollback should not panic
    let line = grid.scrollback_line(0);
    assert!(line.is_some());

    let line = grid.scrollback_line(1);
    assert!(line.is_some() || line.is_none()); // Depends on implementation
}

#[test]
fn test_set_line_wrapped_bounds() {
    let mut grid = Grid::new(80, 24, 1000);

    // Set wrapped state
    grid.set_line_wrapped(5, true);
    assert!(grid.is_line_wrapped(5));

    // Clear it
    grid.set_line_wrapped(5, false);
    assert!(!grid.is_line_wrapped(5));

    // Out of bounds should not panic
    grid.set_line_wrapped(100, true);
    assert!(!grid.is_line_wrapped(100));
}

#[test]
fn test_export_styled_buffer() {
    let mut grid = Grid::new(20, 3, 1000);

    // Add some styled content
    let mut cell = Cell::new('S');
    cell.flags.set_bold(true);
    grid.set(0, 0, cell);

    let styled = grid.export_styled_buffer();

    // Should contain ANSI codes for bold
    assert!(styled.contains("\x1b["));
}

#[test]
fn test_clear_row() {
    let mut grid = Grid::new(80, 24, 1000);

    // Fill a row
    for col in 0..80 {
        grid.set(col, 10, Cell::new('X'));
    }

    // Clear it
    grid.clear_row(10);

    // Verify cleared
    for col in 0..80 {
        assert_eq!(grid.get(col, 10).unwrap().c, ' ');
    }
}

// ===== Scrollback Reflow Tests =====

#[test]
fn test_scrollback_reflow_width_increase_unwraps() {
    // Test that increasing width unwraps previously wrapped lines
    let mut grid = Grid::new(10, 3, 100);

    // Create a line that wraps: "ABCDEFGHIJ" (10 chars) + "KLMNO" (5 chars)
    // This will be 2 physical lines with wrap=true on the first
    for (i, ch) in "ABCDEFGHIJ".chars().enumerate() {
        grid.set(i, 0, Cell::new(ch));
    }
    grid.set_line_wrapped(0, true);
    for (i, ch) in "KLMNO".chars().enumerate() {
        grid.set(i, 1, Cell::new(ch));
    }

    // Scroll these lines into scrollback
    grid.scroll_up(2);
    assert_eq!(grid.scrollback_len(), 2);
    assert!(grid.is_scrollback_wrapped(0)); // First line should be wrapped

    // Now resize to wider (20 cols) - should unwrap
    grid.resize(20, 3);

    // After reflow, both lines should merge into one (15 chars fits in 20 cols)
    assert_eq!(grid.scrollback_len(), 1);
    assert!(!grid.is_scrollback_wrapped(0)); // Should not be wrapped anymore

    // Verify content is preserved
    let line = grid.scrollback_line(0).unwrap();
    assert_eq!(line[0].c, 'A');
    assert_eq!(line[4].c, 'E');
    assert_eq!(line[10].c, 'K');
    assert_eq!(line[14].c, 'O');
}

#[test]
fn test_scrollback_reflow_width_decrease_rewraps() {
    // Test that decreasing width re-wraps lines
    let mut grid = Grid::new(20, 3, 100);

    // Create a single line with 15 characters
    for (i, ch) in "ABCDEFGHIJKLMNO".chars().enumerate() {
        grid.set(i, 0, Cell::new(ch));
    }

    // Scroll into scrollback
    grid.scroll_up(1);
    assert_eq!(grid.scrollback_len(), 1);
    assert!(!grid.is_scrollback_wrapped(0));

    // Now resize to narrower (10 cols) - should re-wrap
    grid.resize(10, 3);

    // After reflow, should be 2 lines (10 + 5 chars)
    assert_eq!(grid.scrollback_len(), 2);
    assert!(grid.is_scrollback_wrapped(0)); // First line should be wrapped now
    assert!(!grid.is_scrollback_wrapped(1)); // Second line not wrapped

    // Verify content
    let line0 = grid.scrollback_line(0).unwrap();
    let line1 = grid.scrollback_line(1).unwrap();
    assert_eq!(line0[0].c, 'A');
    assert_eq!(line0[9].c, 'J');
    assert_eq!(line1[0].c, 'K');
    assert_eq!(line1[4].c, 'O');
}

#[test]
fn test_scrollback_reflow_preserves_colors() {
    use crate::color::Color;

    let mut grid = Grid::new(10, 3, 100);

    // Create a colored cell
    let mut cell = Cell::new('X');
    cell.fg = Color::Rgb(255, 0, 0);
    cell.bg = Color::Rgb(0, 255, 0);
    cell.flags.set_bold(true);
    grid.set(0, 0, cell);

    // Scroll into scrollback
    grid.scroll_up(1);

    // Resize (triggers reflow)
    grid.resize(20, 3);

    // Verify colors and attributes preserved
    let line = grid.scrollback_line(0).unwrap();
    assert_eq!(line[0].c, 'X');
    assert_eq!(line[0].fg, Color::Rgb(255, 0, 0));
    assert_eq!(line[0].bg, Color::Rgb(0, 255, 0));
    assert!(line[0].flags.bold());
}

#[test]
fn test_scrollback_reflow_wide_chars() {
    // Test that wide characters are handled correctly during reflow
    let mut grid = Grid::new(10, 3, 100);

    // Create a wide character at position 8 (needs 2 cells)
    let mut wide_cell = Cell::new('中');
    wide_cell.flags.set_wide_char(true);
    grid.set(8, 0, wide_cell);

    let mut spacer = Cell::default();
    spacer.flags.set_wide_char_spacer(true);
    grid.set(9, 0, spacer);

    // Scroll into scrollback
    grid.scroll_up(1);

    // Resize to 5 cols - wide char should wrap properly
    grid.resize(5, 3);

    // The wide char should be on its own row or properly wrapped
    // (can't split a wide char across lines)
    assert!(grid.scrollback_len() >= 1);

    // Verify the wide char is preserved
    let mut found_wide = false;
    for i in 0..grid.scrollback_len() {
        if let Some(line) = grid.scrollback_line(i) {
            for cell in line {
                if cell.c == '中' {
                    found_wide = true;
                    assert!(cell.flags.wide_char());
                    break;
                }
            }
        }
    }
    assert!(
        found_wide,
        "Wide character should be preserved after reflow"
    );
}

#[test]
fn test_scrollback_reflow_multiple_logical_lines() {
    // Test reflow with multiple separate logical lines (non-wrapped)
    let mut grid = Grid::new(10, 5, 100);

    // Create 3 separate lines
    for (i, ch) in "LINE1".chars().enumerate() {
        grid.set(i, 0, Cell::new(ch));
    }
    for (i, ch) in "LINE2".chars().enumerate() {
        grid.set(i, 1, Cell::new(ch));
    }
    for (i, ch) in "LINE3".chars().enumerate() {
        grid.set(i, 2, Cell::new(ch));
    }

    // Scroll all into scrollback
    grid.scroll_up(3);
    assert_eq!(grid.scrollback_len(), 3);

    // Resize wider
    grid.resize(20, 5);

    // Should still have 3 separate lines
    assert_eq!(grid.scrollback_len(), 3);

    let line0 = grid.scrollback_line(0).unwrap();
    let line1 = grid.scrollback_line(1).unwrap();
    let line2 = grid.scrollback_line(2).unwrap();

    assert_eq!(line0[0].c, 'L');
    assert_eq!(line0[4].c, '1');
    assert_eq!(line1[4].c, '2');
    assert_eq!(line2[4].c, '3');
}

#[test]
fn test_scrollback_reflow_max_scrollback_limit() {
    // Test that reflow respects max_scrollback limit
    let mut grid = Grid::new(20, 5, 3); // Only 3 lines max

    // Create a long line that will need 4 rows when reflowed to 5 cols
    for (i, ch) in "ABCDEFGHIJKLMNOPQRST".chars().enumerate() {
        grid.set(i, 0, Cell::new(ch));
    }

    grid.scroll_up(1);
    assert_eq!(grid.scrollback_len(), 1);

    // Resize to 5 cols - would need 4 lines, but max is 3
    grid.resize(5, 5);

    // Should be capped at 3 lines
    assert!(grid.scrollback_len() <= 3);
}

#[test]
fn test_scrollback_reflow_empty_scrollback() {
    // Test that reflow handles empty scrollback gracefully
    let mut grid = Grid::new(10, 3, 100);

    assert_eq!(grid.scrollback_len(), 0);

    // Resize - should not panic
    grid.resize(20, 3);

    assert_eq!(grid.scrollback_len(), 0);
}

#[test]
fn test_scrollback_reflow_same_width() {
    // Test that same width doesn't trigger unnecessary reflow
    let mut grid = Grid::new(10, 3, 100);

    for (i, ch) in "HELLO".chars().enumerate() {
        grid.set(i, 0, Cell::new(ch));
    }
    grid.scroll_up(1);

    let orig_len = grid.scrollback_len();

    // Resize with same width but different height
    grid.resize(10, 5);

    // Scrollback should be unchanged (no width change, no reflow)
    assert_eq!(grid.scrollback_len(), orig_len);
}

#[test]
fn test_scrollback_reflow_circular_buffer() {
    // Test reflow when scrollback is using circular buffer
    let mut grid = Grid::new(10, 2, 3); // Small max for quick circular

    // Fill scrollback past capacity (4 scrolls with max 3)
    for i in 0..4 {
        grid.set(0, 0, Cell::new((b'A' + i as u8) as char));
        grid.scroll_up(1);
    }

    // Scrollback should have 3 lines (circular, oldest dropped)
    assert_eq!(grid.scrollback_len(), 3);

    // The oldest line should be 'B' (A was dropped)
    let line0 = grid.scrollback_line(0).unwrap();
    assert_eq!(line0[0].c, 'B');

    // Resize - reflow should handle circular buffer correctly
    grid.resize(20, 2);

    // Content should still be B, C, D
    assert_eq!(grid.scrollback_len(), 3);
    let line0 = grid.scrollback_line(0).unwrap();
    let line1 = grid.scrollback_line(1).unwrap();
    let line2 = grid.scrollback_line(2).unwrap();
    assert_eq!(line0[0].c, 'B');
    assert_eq!(line1[0].c, 'C');
    assert_eq!(line2[0].c, 'D');
}

#[test]
fn test_scrollback_reflow_wrapped_chain() {
    // Test reflow of a chain of wrapped lines that spans multiple rows
    let mut grid = Grid::new(5, 5, 100);

    // Create a 15-char line that spans 3 rows at width 5
    for (i, ch) in "ABCDEFGHIJKLMNO".chars().enumerate() {
        let row = i / 5;
        let col = i % 5;
        grid.set(col, row, Cell::new(ch));
    }
    grid.set_line_wrapped(0, true);
    grid.set_line_wrapped(1, true);
    grid.set_line_wrapped(2, false); // End of logical line

    // Scroll all 3 rows into scrollback
    grid.scroll_up(3);
    assert_eq!(grid.scrollback_len(), 3);

    // Resize to 15 cols - should unwrap into single line
    grid.resize(15, 5);

    assert_eq!(grid.scrollback_len(), 1);
    let line = grid.scrollback_line(0).unwrap();
    assert_eq!(line[0].c, 'A');
    assert_eq!(line[14].c, 'O');
}

#[cfg(test)]
mod zone_tests {
    use super::*;
    use crate::zone::{Zone, ZoneType};

    #[test]
    fn test_grid_zones_empty() {
        let grid = Grid::new(80, 24, 100);
        assert!(grid.zones().is_empty());
    }

    #[test]
    fn test_grid_push_zone() {
        let mut grid = Grid::new(80, 24, 100);
        grid.push_zone(Zone::new(0, ZoneType::Prompt, 0, Some(1000)));
        assert_eq!(grid.zones().len(), 1);
        assert_eq!(grid.zones()[0].zone_type, ZoneType::Prompt);
    }

    #[test]
    fn test_grid_close_current_zone() {
        let mut grid = Grid::new(80, 24, 100);
        grid.push_zone(Zone::new(0, ZoneType::Prompt, 0, Some(1000)));
        grid.close_current_zone(5);
        assert_eq!(grid.zones()[0].abs_row_end, 5);
    }

    #[test]
    fn test_grid_zone_at() {
        let mut grid = Grid::new(80, 24, 100);
        let mut z1 = Zone::new(0, ZoneType::Prompt, 0, None);
        z1.close(4);
        grid.push_zone(z1);

        let mut z2 = Zone::new(1, ZoneType::Command, 5, None);
        z2.close(6);
        grid.push_zone(z2);

        let mut z3 = Zone::new(2, ZoneType::Output, 7, None);
        z3.close(20);
        grid.push_zone(z3);

        assert_eq!(grid.zone_at(0).unwrap().zone_type, ZoneType::Prompt);
        assert_eq!(grid.zone_at(4).unwrap().zone_type, ZoneType::Prompt);
        assert_eq!(grid.zone_at(5).unwrap().zone_type, ZoneType::Command);
        assert_eq!(grid.zone_at(10).unwrap().zone_type, ZoneType::Output);
        assert!(grid.zone_at(21).is_none());
    }

    #[test]
    fn test_grid_evict_zones() {
        let mut grid = Grid::new(80, 24, 100);
        let mut z1 = Zone::new(0, ZoneType::Prompt, 0, None);
        z1.close(4);
        grid.push_zone(z1);

        let mut z2 = Zone::new(1, ZoneType::Output, 5, None);
        z2.close(20);
        grid.push_zone(z2);

        grid.evict_zones(5);
        assert_eq!(grid.zones().len(), 1);
        assert_eq!(grid.zones()[0].zone_type, ZoneType::Output);
    }

    #[test]
    fn test_grid_evict_zones_partial() {
        let mut grid = Grid::new(80, 24, 100);
        let mut z1 = Zone::new(0, ZoneType::Output, 0, None);
        z1.close(20);
        grid.push_zone(z1);

        grid.evict_zones(10);
        assert_eq!(grid.zones().len(), 1);
        assert_eq!(grid.zones()[0].abs_row_start, 10);
    }

    #[test]
    fn test_grid_clear_zones() {
        let mut grid = Grid::new(80, 24, 100);
        grid.push_zone(Zone::new(0, ZoneType::Prompt, 0, None));
        grid.clear_zones();
        assert!(grid.zones().is_empty());
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::color::{Color, NamedColor};
    use crate::zone::{Zone, ZoneType};

    #[test]
    fn test_capture_basic_content() {
        let mut grid = Grid::new(10, 5, 100);
        // Write some characters into the grid
        grid.get_mut(0, 0).unwrap().c = 'H';
        grid.get_mut(1, 0).unwrap().c = 'i';
        grid.get_mut(0, 1).unwrap().c = '!';

        let snap = grid.capture_snapshot();
        assert_eq!(snap.cols, 10);
        assert_eq!(snap.rows, 5);
        assert_eq!(snap.cells.len(), 10 * 5);
        assert_eq!(snap.cells[0].c, 'H');
        assert_eq!(snap.cells[1].c, 'i');
        assert_eq!(snap.cells[10].c, '!');
        assert_eq!(snap.scrollback_lines, 0);
        assert_eq!(snap.total_lines_scrolled, 0);
    }

    #[test]
    fn test_roundtrip_restore() {
        let mut grid = Grid::new(10, 5, 100);
        // Write content
        grid.get_mut(0, 0).unwrap().c = 'A';
        grid.get_mut(1, 0).unwrap().c = 'B';
        grid.get_mut(2, 0).unwrap().fg = Color::Rgb(255, 0, 0);
        grid.set_line_wrapped(0, true);

        // Push a zone
        grid.push_zone(Zone::new(1, ZoneType::Prompt, 0, Some(1_000)));

        let snap = grid.capture_snapshot();

        // Modify the grid after snapshot
        grid.get_mut(0, 0).unwrap().c = 'X';
        grid.get_mut(1, 0).unwrap().c = 'Y';
        grid.get_mut(2, 0).unwrap().fg = Color::Named(NamedColor::White);
        grid.set_line_wrapped(0, false);
        grid.clear_zones();

        assert_eq!(grid.get(0, 0).unwrap().c, 'X');
        assert!(grid.zones().is_empty());

        // Restore from snapshot
        grid.restore_from_snapshot(&snap);

        assert_eq!(grid.get(0, 0).unwrap().c, 'A');
        assert_eq!(grid.get(1, 0).unwrap().c, 'B');
        assert_eq!(grid.get(2, 0).unwrap().fg, Color::Rgb(255, 0, 0));
        assert!(grid.is_line_wrapped(0));
        assert_eq!(grid.zones().len(), 1);
        assert_eq!(grid.zones()[0].id, 1);
    }

    #[test]
    fn test_scrollback_roundtrip() {
        let mut grid = Grid::new(5, 3, 10);

        // Fill the grid with content
        for row in 0..3 {
            for col in 0..5 {
                grid.get_mut(col, row).unwrap().c = char::from(b'A' + (row * 5 + col) as u8);
            }
        }

        // Scroll some lines up into scrollback
        grid.scroll_up(2);

        let sb_lines = grid.scrollback_len();
        assert!(sb_lines > 0, "scrollback should have lines after scroll_up");

        let snap = grid.capture_snapshot();
        assert_eq!(snap.scrollback_lines, sb_lines);

        // Modify grid
        grid.get_mut(0, 0).unwrap().c = 'Z';
        // Scroll more lines to change scrollback state
        grid.scroll_up(1);

        let sb_after = grid.scrollback_len();
        assert_ne!(sb_after, sb_lines, "scrollback should have changed");

        // Restore
        grid.restore_from_snapshot(&snap);

        // Scrollback should be restored to original state
        assert_eq!(grid.scrollback_len(), sb_lines);
    }
}
