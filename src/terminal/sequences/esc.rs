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
