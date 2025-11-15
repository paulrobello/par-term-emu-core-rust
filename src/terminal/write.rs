//! Character writing and text output
//!
//! Handles character output including:
//! - Special character handling (CR, LF, TAB, BS)
//! - Wide character support (emoji, CJK)
//! - Auto-wrap mode (DECAWM)
//! - Scrolling behavior
//! - Insert mode
//! - Character attributes and hyperlinks

use crate::cell::Cell;
use crate::debug;
use crate::terminal::Terminal;

impl Terminal {
    /// Write a character to the terminal at the current cursor position
    pub(super) fn write_char(&mut self, c: char) {
        let (cols, _rows) = self.size();

        // Handle special characters
        match c {
            '\r' => {
                // Carriage return moves to left margin when DECLRMM is enabled
                if self.use_lr_margins {
                    self.cursor.col = self.left_margin.min(cols.saturating_sub(1));
                } else {
                    self.cursor.move_to_line_start();
                }
                // CR clears pending wrap
                self.pending_wrap = false;
                return;
            }
            '\n' => {
                // LNM (Line Feed/New Line Mode): when enabled, LF does CR+LF
                if self.line_feed_new_line_mode {
                    // Do carriage return first
                    if self.use_lr_margins {
                        self.cursor.col = self.left_margin.min(cols.saturating_sub(1));
                    } else {
                        self.cursor.move_to_line_start();
                    }
                }
                // VT spec behavior: Line feed moves cursor down. If at bottom of scroll region, scroll the region.
                // Per VT220 manual: "Index (IND) moves the cursor down one line in the same column.
                // If the cursor is at the bottom margin, the screen performs a scroll up."
                let (_, rows) = self.size();
                let in_scroll_region = self.cursor.row >= self.scroll_region_top
                    && self.cursor.row <= self.scroll_region_bottom;
                // If DECLRMM is enabled and the cursor is outside left/right margins,
                // ignore the scroll (match iTerm2 behavior) to avoid corrupting panes/status bars.
                let outside_lr_margin = self.use_lr_margins
                    && (self.cursor.col < self.left_margin || self.cursor.col > self.right_margin);

                if in_scroll_region
                    && self.cursor.row == self.scroll_region_bottom
                    && !outside_lr_margin
                {
                    // At bottom of scroll region - scroll the region per VT spec
                    // The scroll is confined to the region boundaries, preserving content outside it
                    let top = self.scroll_region_top;
                    let bottom = self.scroll_region_bottom;
                    debug::log_scroll("newline-at-scroll-bottom", top, bottom, 1);
                    self.active_grid_mut().scroll_region_up(1, top, bottom);
                    // Adjust graphics to scroll with content
                    self.adjust_graphics_for_scroll_up(1, top, bottom);
                    // Cursor stays at scroll_region_bottom per VT spec
                } else {
                    // Not at scroll region bottom, or outside region - just move cursor down
                    self.cursor.row += 1;
                    if self.cursor.row >= rows {
                        self.cursor.row = rows - 1;
                    }
                }
                // LF/IND semantics clear pending wrap
                self.pending_wrap = false;
                return;
            }
            '\t' => {
                // Tab to next tab stop
                let mut next_col = self.cursor.col + 1;
                while next_col < cols {
                    if self.tab_stops.get(next_col).copied().unwrap_or(false) {
                        break;
                    }
                    next_col += 1;
                }
                self.cursor.col = next_col.min(cols - 1);
                // Horizontal cursor movement clears pending wrap
                self.pending_wrap = false;
                return;
            }
            '\x08' => {
                // Backspace
                if self.cursor.col > 0 {
                    self.cursor.col -= 1;
                }
                // Horizontal movement clears pending wrap
                self.pending_wrap = false;
                return;
            }
            c if c.is_control() => {
                // Ignore other control characters
                return;
            }
            _ => {}
        }

        // Handle wide characters (emoji, CJK, etc.)
        let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);

        // If a wrap is pending from a prior write at the right margin, perform the wrap now
        if self.pending_wrap {
            let (cols, rows) = self.size();
            let was_outside_lr = self.use_lr_margins
                && (self.cursor.col < self.left_margin || self.cursor.col > self.right_margin);

            // Mark the current row as wrapped (line continues to next row)
            let current_row = self.cursor.row;
            self.active_grid_mut().set_line_wrapped(current_row, true);

            // Move to left margin or column 0
            self.cursor.col = if self.use_lr_margins {
                self.left_margin.min(cols.saturating_sub(1))
            } else {
                0
            };
            if self.cursor.row == self.scroll_region_bottom && !was_outside_lr {
                let scroll_top = self.scroll_region_top;
                let scroll_bottom = self.scroll_region_bottom;
                debug::log_scroll("wrap-pending-advance", scroll_top, scroll_bottom, 1);
                self.active_grid_mut()
                    .scroll_region_up(1, scroll_top, scroll_bottom);
                // Adjust graphics to scroll with content
                self.adjust_graphics_for_scroll_up(1, scroll_top, scroll_bottom);
                // Cursor remains at bottom of region
            } else {
                self.cursor.row += 1;
                if self.cursor.row >= rows {
                    self.cursor.row = rows - 1;
                }
            }
            self.pending_wrap = false;
        }

        // If wide character won't fit on current line, wrap first
        if char_width == 2 && self.cursor.col >= cols - 1 && self.auto_wrap {
            // Mark the current row as wrapped (line continues to next row)
            let current_row = self.cursor.row;
            self.active_grid_mut().set_line_wrapped(current_row, true);

            // Wrap to left margin if DECLRMM is enabled
            self.cursor.col = if self.use_lr_margins {
                self.left_margin.min(cols.saturating_sub(1))
            } else {
                0
            };
            // VT spec behavior: scroll if at scroll region bottom
            let (_, rows) = self.size();
            let outside_lr_margin = self.use_lr_margins
                && (self.cursor.col < self.left_margin || self.cursor.col > self.right_margin);
            if self.cursor.row == self.scroll_region_bottom && !outside_lr_margin {
                let scroll_top = self.scroll_region_top;
                let scroll_bottom = self.scroll_region_bottom;
                self.active_grid_mut()
                    .scroll_region_up(1, scroll_top, scroll_bottom);
                // Adjust graphics to scroll with content
                self.adjust_graphics_for_scroll_up(1, scroll_top, scroll_bottom);
                // Cursor stays at scroll_region_bottom
            } else {
                self.cursor.row += 1;
                if self.cursor.row >= rows {
                    self.cursor.row = rows - 1;
                }
            }
        }

        // Write the character with appropriate wide_char flag
        let mut cell_flags = self.flags;
        if char_width == 2 {
            cell_flags.set_wide_char(true);
        }
        // Apply current hyperlink ID
        cell_flags.hyperlink_id = self.current_hyperlink_id;
        // Apply character protection (DECSCA)
        cell_flags.set_guarded(self.char_protected);

        let cell = Cell {
            c,
            fg: self.fg,
            bg: self.bg,
            underline_color: self.underline_color,
            flags: cell_flags,
            width: char_width as u8,
        };

        let cursor_col = self.cursor.col;
        let cursor_row = self.cursor.row;

        // If insert mode (IRM) is enabled, insert space by shifting chars right
        if self.insert_mode {
            self.active_grid_mut()
                .insert_chars(cursor_col, cursor_row, char_width);
        }

        self.active_grid_mut().set(cursor_col, cursor_row, cell);

        // Advance cursor by character width
        self.cursor.col += char_width;

        // If it's a wide character, fill the next cell with a spacer
        if char_width == 2 && self.cursor.col - 1 < cols {
            let mut spacer_flags = self.flags;
            spacer_flags.set_wide_char_spacer(true);
            // Apply hyperlink ID to spacer as well
            spacer_flags.hyperlink_id = self.current_hyperlink_id;

            let spacer = Cell {
                c: ' ', // Spacer character
                fg: self.fg,
                bg: self.bg,
                underline_color: self.underline_color,
                flags: spacer_flags,
                width: 1, // Spacers always have width 1
            };
            let spacer_col = self.cursor.col - 1;
            let spacer_row = self.cursor.row;
            self.active_grid_mut().set(spacer_col, spacer_row, spacer);
        }

        // Handle delayed autowrap for width-1 characters
        if self.auto_wrap && char_width == 1 && self.cursor.col >= cols {
            // Stay at last column and set wrap-pending; do not move yet
            self.cursor.col = cols - 1;
            self.pending_wrap = true;
        } else if self.cursor.col >= cols {
            // Fallback: if auto-wrap is disabled or some edge case, clamp to last column
            self.cursor.col = cols - 1;
        }
    }
}
