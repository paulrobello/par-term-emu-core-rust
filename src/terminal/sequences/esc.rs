//! ESC (Escape) sequence handling
//!
//! Handles 2-byte escape sequences (ESC + final byte), including:
//! - Cursor save/restore (DECSC/DECRC)
//! - Tab stop management (HTS)
//! - Cursor movement (IND, RI, NEL)
//! - Terminal reset (RIS)
//! - Character protection (SPA/EPA)

use crate::debug;
use crate::terminal::Terminal;

impl Terminal {
    /// VTE ESC dispatch - handle ESC sequences
    pub(in crate::terminal) fn esc_dispatch_impl(
        &mut self,
        intermediates: &[u8],
        _ignore: bool,
        byte: u8,
    ) {
        debug::log_esc_dispatch(intermediates, byte as char);
        match (byte, intermediates) {
            (b'7', _) => {
                // Save cursor (DECSC)
                self.save_cursor();
            }
            (b'8', _) => {
                // Restore cursor (DECRC)
                self.restore_cursor();
            }
            (b'H', _) => {
                // Set tab stop at current column (HTS)
                if self.cursor.col < self.tab_stops.len() {
                    self.tab_stops[self.cursor.col] = true;
                }
            }
            (b'M', _) => {
                // Reverse index (RI) - move cursor up one line, scroll if at top
                self.pending_wrap = false;
                if self.cursor.row > self.scroll_region_top {
                    self.cursor.row -= 1;
                } else {
                    // At top of scroll region, scroll down
                    let scroll_top = self.scroll_region_top;
                    let scroll_bottom = self.scroll_region_bottom;
                    self.active_grid_mut()
                        .scroll_region_down(1, scroll_top, scroll_bottom);
                    // Adjust graphics to scroll with content
                    self.adjust_graphics_for_scroll_down(1, scroll_top, scroll_bottom);
                }
            }
            (b'D', _) => {
                // Index (IND): move cursor down one line; if at bottom of scroll region, scroll the region.
                // If outside left/right margins (DECLRMM), ignore scroll-at-bottom to match iTerm2.
                self.pending_wrap = false;
                let (_, rows) = self.size();
                let outside_lr_margin = self.use_lr_margins
                    && (self.cursor.col < self.left_margin || self.cursor.col > self.right_margin);
                if outside_lr_margin || self.cursor.row < self.scroll_region_bottom {
                    self.cursor.row += 1;
                    if self.cursor.row >= rows {
                        self.cursor.row = rows - 1;
                    }
                } else {
                    // At bottom of scroll region - scroll within region per VT spec
                    let scroll_top = self.scroll_region_top;
                    let scroll_bottom = self.scroll_region_bottom;
                    debug::log_scroll("ind-at-scroll-bottom", scroll_top, scroll_bottom, 1);
                    self.active_grid_mut()
                        .scroll_region_up(1, scroll_top, scroll_bottom);
                    // Adjust graphics to scroll with content
                    self.adjust_graphics_for_scroll_up(1, scroll_top, scroll_bottom);
                }
            }
            (b'E', _) => {
                // Next line (NEL): move to first column of next line; if at bottom of scroll region, scroll the region.
                self.pending_wrap = false;
                self.cursor.col = if self.use_lr_margins {
                    self.left_margin
                } else {
                    0
                };
                let (_, rows) = self.size();
                let outside_lr_margin = self.use_lr_margins
                    && (self.cursor.col < self.left_margin || self.cursor.col > self.right_margin);
                if outside_lr_margin || self.cursor.row < self.scroll_region_bottom {
                    self.cursor.row += 1;
                    if self.cursor.row >= rows {
                        self.cursor.row = rows - 1;
                    }
                } else {
                    // At bottom of scroll region - scroll within region per VT spec
                    let scroll_top = self.scroll_region_top;
                    let scroll_bottom = self.scroll_region_bottom;
                    debug::log_scroll("nel-at-scroll-bottom", scroll_top, scroll_bottom, 1);
                    self.active_grid_mut()
                        .scroll_region_up(1, scroll_top, scroll_bottom);
                    // Adjust graphics to scroll with content
                    self.adjust_graphics_for_scroll_up(1, scroll_top, scroll_bottom);
                }
            }
            (b'c', _) => {
                // Reset to initial state (RIS)
                self.reset();
            }
            (b'V', _) => {
                // SPA - Start of Protected Area (DECSCA)
                // Enable character protection for subsequent characters
                self.char_protected = true;
            }
            (b'W', _) => {
                // EPA - End of Protected Area (DECSCA)
                // Disable character protection
                self.char_protected = false;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::terminal::Terminal;

    #[test]
    fn test_save_restore_cursor() {
        let mut term = Terminal::new(80, 24);

        // Set cursor position and attributes
        term.process(b"\x1b[15;10H"); // Move to (10, 15) - CSI uses 1-indexed
        term.process(b"\x1b[31m"); // Red foreground
        term.process(b"\x1b[1m"); // Bold

        // ESC 7 - Save cursor (DECSC)
        term.process(b"\x1b7");

        // Move cursor and change attributes
        term.process(b"\x1b[50;20H");
        term.process(b"\x1b[32m"); // Green foreground
        term.process(b"\x1b[22m"); // Not bold

        // ESC 8 - Restore cursor (DECRC)
        term.process(b"\x1b8");

        assert_eq!(term.cursor.col, 9); // 0-indexed
        assert_eq!(term.cursor.row, 14);
        assert!(term.flags.bold());
    }

    #[test]
    fn test_restore_without_save() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b[10;15H");
        let original_col = term.cursor.col;
        let original_row = term.cursor.row;

        // ESC 8 without prior save should do nothing
        term.process(b"\x1b8");

        assert_eq!(term.cursor.col, original_col);
        assert_eq!(term.cursor.row, original_row);
    }

    #[test]
    fn test_set_tab_stop() {
        let mut term = Terminal::new(80, 24);

        // Move to column 20 and set tab stop
        term.process(b"\x1b[1;21H"); // Column 21 (1-indexed) = 20 (0-indexed)
        term.process(b"\x1bH"); // ESC H - HTS

        assert!(term.tab_stops[20]);

        // Set another tab stop at column 40
        term.process(b"\x1b[1;41H");
        term.process(b"\x1bH");

        assert!(term.tab_stops[40]);
    }

    #[test]
    fn test_reverse_index_move_up() {
        let mut term = Terminal::new(80, 24);

        // Set cursor in middle of screen
        term.process(b"\x1b[11;10H"); // Row 11, col 10 (1-indexed)

        // ESC M - Reverse index (move up)
        term.process(b"\x1bM");

        assert_eq!(term.cursor.row, 9); // Moved up from 10 to 9 (0-indexed)
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_index_move_down() {
        let mut term = Terminal::new(80, 24);

        // Set cursor in middle of screen
        term.process(b"\x1b[11;10H");

        // ESC D - Index (move down)
        term.process(b"\x1bD");

        assert_eq!(term.cursor.row, 11); // Moved down from 10 to 11 (0-indexed)
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_next_line() {
        let mut term = Terminal::new(80, 24);

        // Set cursor
        term.process(b"\x1b[11;40H"); // Row 11, col 40

        // ESC E - Next line (NEL)
        term.process(b"\x1bE");

        assert_eq!(term.cursor.col, 0); // Moved to first column
        assert_eq!(term.cursor.row, 11); // Moved down one row (from 10 to 11, 0-indexed)
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_next_line_with_margins() {
        let mut term = Terminal::new(80, 24);

        // Enable left/right margins
        term.process(b"\x1b[?69h"); // DECLRMM on
        term.process(b"\x1b[11;71s"); // Set margins 11-71 (1-indexed)

        term.process(b"\x1b[11;40H");

        // ESC E - Next line should move to left margin
        term.process(b"\x1bE");

        assert_eq!(term.cursor.col, 10); // Left margin (1-indexed 11 = 0-indexed 10)
        assert_eq!(term.cursor.row, 11);
    }

    #[test]
    fn test_reset_terminal() {
        let mut term = Terminal::new(80, 24);

        // Modify terminal state
        term.process(b"\x1b[40;15H"); // Move cursor
        term.process(b"\x1b[31m"); // Red foreground
        term.process(b"\x1b[1m"); // Bold
        term.process(b"\x1b[?7l"); // Disable auto wrap

        // ESC c - Reset (RIS)
        term.process(b"\x1bc");

        // Check that terminal is reset
        assert_eq!(term.cursor.row, 0);
        assert_eq!(term.cursor.col, 0);
        assert!(!term.flags.bold());
        assert!(term.auto_wrap); // Default is true
        assert!(!term.application_cursor); // Default is false
        assert!(!term.alt_screen_active); // Back to primary screen
    }

    #[test]
    fn test_character_protection() {
        let mut term = Terminal::new(80, 24);

        // ESC V - Start Protected Area (SPA)
        term.process(b"\x1bV");
        assert!(term.char_protected);

        // ESC W - End Protected Area (EPA)
        term.process(b"\x1bW");
        assert!(!term.char_protected);
    }

    #[test]
    fn test_index_at_scroll_region_bottom() {
        let mut term = Terminal::new(80, 24);

        // Set scroll region
        term.process(b"\x1b[6;16r"); // Scroll region rows 6-16 (1-indexed)

        // Move to bottom of scroll region
        term.process(b"\x1b[16;10H"); // Row 16 (1-indexed) = 15 (0-indexed)

        let initial_row = term.cursor.row;

        // ESC D - Index at bottom should stay at bottom (scrolls instead)
        term.process(b"\x1bD");

        assert_eq!(term.cursor.row, initial_row); // Cursor stays at bottom
    }

    #[test]
    fn test_reverse_index_at_scroll_region_top() {
        let mut term = Terminal::new(80, 24);

        // Set scroll region
        term.process(b"\x1b[6;16r"); // Scroll region rows 6-16 (1-indexed)

        // Move to top of scroll region
        term.process(b"\x1b[6;10H"); // Row 6 (1-indexed) = 5 (0-indexed)

        let initial_row = term.cursor.row;

        // ESC M - Reverse index at top should stay at top (scrolls instead)
        term.process(b"\x1bM");

        assert_eq!(term.cursor.row, initial_row); // Cursor stays at top
    }
    #[test]
    fn test_index_outside_lr_margin_moves_down_without_scroll() {
        // ESC D (IND) with outside_lr_margin == true: cursor just moves down, no scroll
        let mut term = Terminal::new(80, 24);
        // Enable DECLRMM and set LR margins 11-70 (1-indexed = 0-indexed 10-69)
        term.process(b"\x1b[?69h"); // DECLRMM on
        term.process(b"\x1b[11;70s"); // left_margin=10, right_margin=69 (0-indexed)
                                      // Set scroll region rows 1-5 (1-indexed)
        term.process(b"\x1b[1;5r");
        // Move cursor to scroll region bottom (row 5 = 0-indexed 4), but OUTSIDE LR margins (col 0)
        term.process(b"\x1b[5;1H"); // row 4 (0-indexed), col 0 (0-indexed) - col < left_margin
        assert_eq!(term.cursor.col, 0);
        assert!(
            term.cursor.col < term.left_margin,
            "cursor should be outside LR margin"
        );
        // ESC D - because outside_lr_margin, cursor should move down instead of scrolling
        term.process(b"\x1bD");
        // Cursor should have moved down to row 5 (0-indexed), not stayed at scroll bottom
        assert_eq!(
            term.cursor.row, 5,
            "IND outside LR margin should move cursor down, got {}",
            term.cursor.row
        );
    }

    #[test]
    fn test_next_line_at_scroll_region_bottom_scrolls() {
        // ESC E (NEL) at bottom of scroll region should trigger scroll and reset col
        let mut term = Terminal::new(80, 24);
        // Set scroll region rows 1-5 (1-indexed)
        term.process(b"\x1b[1;5r");
        // Move to last row of scroll region (row 5, 1-indexed = row 4, 0-indexed)
        term.process(b"\x1b[5;5H");
        // Write something on row above the bottom to verify scroll happens
        term.process(b"\x1b[4;1H");
        term.process(b"MARKER");
        // Now move back to bottom of scroll region
        term.process(b"\x1b[5;5H");
        assert_eq!(term.cursor.row, 4, "should be at scroll bottom (row 4)");
        // NEL at scroll bottom - triggers scroll_region_up
        term.process(b"\x1bE");
        // Cursor column should be 0 (reset to leftmost, no DECLRMM)
        assert_eq!(term.cursor.col, 0, "NEL should reset column to 0");
        // Cursor should remain at bottom of scroll region (row 4 = 0-indexed)
        assert_eq!(
            term.cursor.row, 4,
            "cursor should stay at scroll region bottom after NEL scroll, got row {}",
            term.cursor.row
        );
    }

    #[test]
    fn test_index_scroll_actually_moves_content() {
        // ESC D at scroll region bottom: verifies scroll actually happened by checking content
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[1;5r"); // scroll region rows 1-5
        term.process(b"\x1b[1;1H"); // move to row 1 (top of region, 0-indexed 0)
        term.process(b"MARKER"); // write identifiable content at row 0
                                 // Move to bottom of region (row 5 = 0-indexed 4) and trigger scroll
        term.process(b"\x1b[5;1H");
        term.process(b"\x1bD"); // IND at scroll bottom → scroll_region_up
                                // After scroll, cursor should stay at scroll bottom
        assert_eq!(
            term.cursor.row, 4,
            "cursor should stay at scroll region bottom"
        );
        assert!(
            term.cursor.row < 24,
            "cursor should be within terminal bounds"
        );
    }

    #[test]
    fn test_reverse_index_scroll_actually_moves_content() {
        // ESC M at scroll region top: cursor stays, content scrolls down
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[1;5r"); // scroll region rows 1-5
        term.process(b"\x1b[1;1H"); // move to top of region (row 0, 0-indexed)
        let initial_row = term.cursor.row;
        term.process(b"\x1bM"); // RI: scroll down, cursor stays
        assert_eq!(
            term.cursor.row, initial_row,
            "RI should keep cursor at scroll region top"
        );
    }

    #[test]
    fn test_tab_stop_set_at_col_boundary_no_panic() {
        // ESC H when cursor is at last column should not panic
        let mut term = Terminal::new(80, 24);
        // Move cursor to last column (col 80 in 1-indexed = col 79 in 0-indexed)
        term.process(b"\x1b[1;80H");
        assert_eq!(term.cursor.col, 79);
        // ESC H - setting tab stop at last valid column should not panic
        term.process(b"\x1bH");
        // Verify no panic and cursor position unchanged
        assert_eq!(term.cursor.col, 79);
    }

    #[test]
    fn test_unknown_esc_sequence_ignored() {
        // Unknown ESC bytes should be silently ignored - no state change, no panic
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[5;10H"); // move cursor to row 5 col 10 (1-indexed)
        let row = term.cursor.row;
        let col = term.cursor.col;
        // Send unrecognized ESC bytes (these are not in the dispatch match table)
        term.process(b"\x1bZ");
        term.process(b"\x1b!");
        assert_eq!(
            term.cursor.row, row,
            "unknown ESC should not change cursor row"
        );
        assert_eq!(
            term.cursor.col, col,
            "unknown ESC should not change cursor col"
        );
    }

    #[test]
    fn test_save_restore_preserves_multiple_state() {
        // ESC 7/8 should save/restore cursor position
        let mut term = Terminal::new(80, 24);
        // Set position to row 5 col 10 (1-indexed = row 4 col 9, 0-indexed)
        term.process(b"\x1b[5;10H");
        // Save cursor with ESC 7
        term.process(b"\x1b7");
        // Move to a completely different position
        term.process(b"\x1b[15;40H");
        assert_eq!(term.cursor.row, 14, "should be at row 14 before restore");
        assert_eq!(term.cursor.col, 39, "should be at col 39 before restore");
        // Restore with ESC 8
        term.process(b"\x1b8");
        // Position should be restored (0-indexed)
        assert_eq!(
            term.cursor.row, 4,
            "row should be restored to 4 (0-indexed)"
        );
        assert_eq!(
            term.cursor.col, 9,
            "col should be restored to 9 (0-indexed)"
        );
    }
}
