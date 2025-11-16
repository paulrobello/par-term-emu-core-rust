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
                self.saved_cursor = Some(self.cursor);
                self.saved_fg = self.fg;
                self.saved_bg = self.bg;
                self.saved_underline_color = self.underline_color;
                self.saved_flags = self.flags;
            }
            (b'8', _) => {
                // Restore cursor (DECRC)
                if let Some(saved) = self.saved_cursor {
                    self.cursor = saved;
                    self.fg = self.saved_fg;
                    self.bg = self.saved_bg;
                    self.underline_color = self.saved_underline_color;
                    self.flags = self.saved_flags;
                }
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
}
