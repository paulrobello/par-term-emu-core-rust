//! Character writing and text output
//!
//! Handles character output including:
//! - Special character handling (CR, LF, TAB, BS)
//! - Wide character support (emoji, CJK)
//! - Auto-wrap mode (DECAWM)
//! - Scrolling behavior
//! - Insert mode
//! - Character attributes and hyperlinks
//! - Grapheme clusters (variation selectors, ZWJ, skin tone modifiers, regional indicators)

use crate::cell::Cell;
use crate::debug;
use crate::grapheme;
use crate::terminal::Terminal;

impl Terminal {
    /// Write a character to the terminal at the current cursor position
    pub(super) fn write_char(&mut self, c: char) {
        let (cols, _rows) = self.size();

        // Handle regional indicator pairs (flag emoji like 🇺🇸)
        // When the second regional indicator arrives, combine it with the first
        if grapheme::is_regional_indicator(c) {
            // Check if previous cell is also a regional indicator (first half of a flag)
            let (prev_col, prev_row) = if self.cursor.col > 0 {
                (self.cursor.col - 1, self.cursor.row)
            } else if self.cursor.row > 0 {
                (cols - 1, self.cursor.row - 1)
            } else {
                // At position (0, 0), this is the first regional indicator
                // Continue to write it as a normal character below
                return self.write_regional_indicator_first(c, cols);
            };

            // Check if previous cell is a regional indicator without a pair yet
            let should_combine = if let Some(prev_cell) = self.active_grid().get(prev_col, prev_row)
            {
                grapheme::is_regional_indicator(prev_cell.c) && prev_cell.combining.is_empty()
            } else {
                false
            };

            if should_combine {
                // Extract cursor position before mutable borrow
                let cursor_col = self.cursor.col;
                let cursor_row = self.cursor.row;

                // Extract spacer cell properties from target cell first
                let spacer = if cursor_col < cols {
                    if let Some(target_cell) = self.active_grid().get(prev_col, prev_row) {
                        let mut spacer_flags = target_cell.flags;
                        spacer_flags.set_wide_char(false);
                        spacer_flags.set_wide_char_spacer(true);
                        Some(Cell {
                            c: ' ',
                            combining: Vec::new(),
                            fg: target_cell.fg,
                            bg: target_cell.bg,
                            underline_color: target_cell.underline_color,
                            flags: spacer_flags,
                            width: 1,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Previous cell is a lone regional indicator - combine them
                if let Some(target_cell) = self.active_grid_mut().get_mut(prev_col, prev_row) {
                    target_cell.combining.push(c);
                    target_cell.width = 2;
                    target_cell.flags.set_wide_char(true);
                }

                // Create spacer cell at current position
                if let Some(spacer) = spacer {
                    self.active_grid_mut().set(cursor_col, cursor_row, spacer);
                }

                // Advance cursor past the spacer
                self.cursor.col += 1;
                if self.cursor.col >= cols {
                    if self.auto_wrap {
                        self.cursor.col = cols - 1;
                        self.pending_wrap = true;
                    } else {
                        self.cursor.col = cols - 1;
                    }
                }

                self.mark_row_dirty(prev_row);
                if cursor_row != prev_row {
                    self.mark_row_dirty(cursor_row);
                }
                return;
            }
            // Previous cell is not a lone regional indicator
            // Continue to write this as the first of a new pair
            return self.write_regional_indicator_first(c, cols);
        }

        // Handle combining characters (variation selectors, ZWJ, skin tone modifiers)
        // These should be added to the previous cell instead of creating a new cell
        if grapheme::is_variation_selector(c)
            || grapheme::is_zwj(c)
            || grapheme::is_skin_tone_modifier(c)
            || grapheme::is_combining_mark(c)
        {
            // Find the previous actual character cell (skip over wide char spacers)
            let (prev_col, prev_row) = if self.cursor.col > 0 {
                (self.cursor.col - 1, self.cursor.row)
            } else if self.cursor.row > 0 {
                (cols - 1, self.cursor.row - 1)
            } else {
                // At position (0, 0), nowhere to add combining char
                return;
            };

            // Check if the previous cell is a wide char spacer
            // If so, the actual wide char is one more cell to the left
            let (target_col, target_row) =
                if let Some(cell) = self.active_grid().get(prev_col, prev_row) {
                    if cell.flags.wide_char_spacer() {
                        // Previous cell is a spacer, go back one more cell for the wide char
                        if prev_col > 0 {
                            (prev_col - 1, prev_row)
                        } else if prev_row > 0 {
                            (cols - 1, prev_row - 1)
                        } else {
                            // Spacer at start of grid, nowhere to go
                            return;
                        }
                    } else {
                        (prev_col, prev_row)
                    }
                } else {
                    return;
                };

            // Add combining character to the target cell
            if let Some(target_cell) = self.active_grid_mut().get_mut(target_col, target_row) {
                target_cell.combining.push(c);

                // Recalculate width if needed (e.g., emoji with variation selector)
                let grapheme = target_cell.get_grapheme();
                let new_width = grapheme::is_wide_grapheme(&grapheme);
                if new_width && target_cell.width() == 1 {
                    target_cell.width = 2;
                    target_cell.flags.set_wide_char(true);

                    if target_col + 1 < cols {
                        let mut spacer_flags = target_cell.flags;
                        spacer_flags.set_wide_char(false);
                        spacer_flags.set_wide_char_spacer(true);

                        let spacer = Cell {
                            c: ' ',
                            combining: Vec::new(),
                            fg: target_cell.fg,
                            bg: target_cell.bg,
                            underline_color: target_cell.underline_color,
                            flags: spacer_flags,
                            width: 1,
                        };
                        self.active_grid_mut()
                            .set(target_col + 1, target_row, spacer);
                    }
                }

                self.mark_row_dirty(target_row);
            }
            return;
        }

        // Check if previous cell has ZWJ - if so, this char is part of ZWJ sequence
        // and should be added as combining character (e.g., 👨 + ZWJ + 💻 = 👨‍💻)
        // OPTIMIZATION: Only check for emoji characters (most text won't trigger this)
        let char_code = c as u32;
        let is_potential_emoji = matches!(char_code,
            0x2600..=0x27BF  // Misc Symbols (❤️, ☀️, etc.)
            | 0x1F000..=0x1FFFF // Emoji blocks
        );

        if is_potential_emoji && (self.cursor.col > 0 || self.cursor.row > 0) {
            let (prev_col, prev_row) = if self.cursor.col > 0 {
                (self.cursor.col - 1, self.cursor.row)
            } else {
                (cols - 1, self.cursor.row - 1)
            };

            // Skip spacers to find actual wide char
            let (target_col, target_row) =
                if let Some(cell) = self.active_grid().get(prev_col, prev_row) {
                    if cell.flags.wide_char_spacer() {
                        if prev_col > 0 {
                            (prev_col - 1, prev_row)
                        } else if prev_row > 0 {
                            (cols - 1, prev_row - 1)
                        } else {
                            (prev_col, prev_row) // Fallback
                        }
                    } else {
                        (prev_col, prev_row)
                    }
                } else {
                    (prev_col, prev_row)
                };

            // Check if target cell has ZWJ in combining chars
            if let Some(target_cell) = self.active_grid().get(target_col, target_row) {
                if target_cell.combining.contains(&'\u{200D}') {
                    // Previous cell has ZWJ, add current char as combining
                    if let Some(target_cell_mut) =
                        self.active_grid_mut().get_mut(target_col, target_row)
                    {
                        target_cell_mut.combining.push(c);

                        // Recalculate width if needed
                        let grapheme = target_cell_mut.get_grapheme();
                        let new_width = grapheme::is_wide_grapheme(&grapheme);
                        if new_width && target_cell_mut.width() == 1 {
                            target_cell_mut.width = 2;
                            target_cell_mut.flags.set_wide_char(true);

                            if target_col + 1 < cols {
                                let mut spacer_flags = target_cell_mut.flags;
                                spacer_flags.set_wide_char(false);
                                spacer_flags.set_wide_char_spacer(true);

                                let spacer = Cell {
                                    c: ' ',
                                    combining: Vec::new(),
                                    fg: target_cell_mut.fg,
                                    bg: target_cell_mut.bg,
                                    underline_color: target_cell_mut.underline_color,
                                    flags: spacer_flags,
                                    width: 1,
                                };
                                self.active_grid_mut()
                                    .set(target_col + 1, target_row, spacer);
                            }
                        }

                        self.mark_row_dirty(target_row);
                    }
                    return;
                }
            }
        }

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
                    // Mark all rows in scroll region as dirty
                    for row in top..=bottom {
                        self.mark_row_dirty(row);
                    }
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
            self.mark_row_dirty(current_row);

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
            self.mark_row_dirty(current_row);

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
            combining: Vec::new(),
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
        // Mark row as dirty for rendering
        self.mark_row_dirty(cursor_row);

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
                combining: Vec::new(),
                fg: self.fg,
                bg: self.bg,
                underline_color: self.underline_color,
                flags: spacer_flags,
                width: 1, // Spacers always have width 1
            };
            let spacer_col = self.cursor.col - 1;
            let spacer_row = self.cursor.row;
            self.active_grid_mut().set(spacer_col, spacer_row, spacer);
            // Spacer is on same row, already marked dirty above
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

    /// Write the first regional indicator of a potential flag pair.
    /// This is written as a width-1 character initially. If followed by another
    /// regional indicator, they will be combined into a width-2 flag emoji.
    fn write_regional_indicator_first(&mut self, c: char, cols: usize) {
        // If a wrap is pending from a prior write at the right margin, perform the wrap now
        if self.pending_wrap {
            let (_cols, rows) = self.size();
            let was_outside_lr = self.use_lr_margins
                && (self.cursor.col < self.left_margin || self.cursor.col > self.right_margin);

            // Mark the current row as wrapped (line continues to next row)
            let current_row = self.cursor.row;
            self.active_grid_mut().set_line_wrapped(current_row, true);
            self.mark_row_dirty(current_row);

            // Move to left margin or column 0
            self.cursor.col = if self.use_lr_margins {
                self.left_margin.min(cols.saturating_sub(1))
            } else {
                0
            };
            if self.cursor.row == self.scroll_region_bottom && !was_outside_lr {
                let scroll_top = self.scroll_region_top;
                let scroll_bottom = self.scroll_region_bottom;
                debug::log_scroll("wrap-pending-regional", scroll_top, scroll_bottom, 1);
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

        // Write the regional indicator as width 1 initially
        // (It will become width 2 if followed by another regional indicator)
        let mut cell_flags = self.flags;
        cell_flags.hyperlink_id = self.current_hyperlink_id;
        cell_flags.set_guarded(self.char_protected);

        let cell = Cell {
            c,
            combining: Vec::new(),
            fg: self.fg,
            bg: self.bg,
            underline_color: self.underline_color,
            flags: cell_flags,
            width: 1, // Initially width 1, will become 2 when paired
        };

        let cursor_col = self.cursor.col;
        let cursor_row = self.cursor.row;

        // If insert mode (IRM) is enabled, insert space by shifting chars right
        if self.insert_mode {
            self.active_grid_mut()
                .insert_chars(cursor_col, cursor_row, 1);
        }

        self.active_grid_mut().set(cursor_col, cursor_row, cell);
        self.mark_row_dirty(cursor_row);

        // Advance cursor by 1
        self.cursor.col += 1;

        // Handle delayed autowrap
        if self.auto_wrap && self.cursor.col >= cols {
            self.cursor.col = cols - 1;
            self.pending_wrap = true;
        } else if self.cursor.col >= cols {
            self.cursor.col = cols - 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;

    fn create_test_terminal() -> Terminal {
        Terminal::new(80, 24)
    }

    #[test]
    fn test_write_char_basic() {
        let mut term = create_test_terminal();
        term.write_char('A');

        let cell = term.active_grid().get(0, 0).unwrap();
        assert_eq!(cell.c, 'A');
        assert_eq!(term.cursor.col, 1);
        assert_eq!(term.cursor.row, 0);
    }

    #[test]
    fn test_write_char_carriage_return() {
        let mut term = create_test_terminal();
        term.write_char('A');
        term.write_char('B');
        term.write_char('C');
        assert_eq!(term.cursor.col, 3);

        term.write_char('\r');
        assert_eq!(term.cursor.col, 0);
        assert_eq!(term.cursor.row, 0);
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_write_char_carriage_return_with_lr_margins() {
        let mut term = create_test_terminal();
        term.use_lr_margins = true;
        term.left_margin = 5;
        term.right_margin = 75;

        term.cursor.col = 10;
        term.write_char('\r');

        assert_eq!(term.cursor.col, 5); // Should move to left margin
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_write_char_line_feed() {
        let mut term = create_test_terminal();
        term.cursor.col = 5;
        term.write_char('\n');

        assert_eq!(term.cursor.col, 5); // Column unchanged
        assert_eq!(term.cursor.row, 1); // Row advanced
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_write_char_line_feed_new_line_mode() {
        let mut term = create_test_terminal();
        term.line_feed_new_line_mode = true;
        term.cursor.col = 5;
        term.write_char('\n');

        assert_eq!(term.cursor.col, 0); // CR+LF behavior
        assert_eq!(term.cursor.row, 1);
    }

    #[test]
    fn test_write_char_line_feed_at_scroll_bottom() {
        let mut term = create_test_terminal();
        term.scroll_region_top = 0;
        term.scroll_region_bottom = 23;
        term.cursor.row = 23;

        // Write some content in first row
        term.cursor.row = 0;
        term.write_char('X');

        // Go to bottom and trigger scroll
        term.cursor.row = 23;
        term.write_char('\n');

        // Should stay at row 23 after scrolling
        assert_eq!(term.cursor.row, 23);

        // First row should be blank after scroll
        let cell = term.active_grid().get(0, 0).unwrap();
        assert_eq!(cell.c, ' ');
    }

    #[test]
    fn test_write_char_tab() {
        let mut term = create_test_terminal();
        term.cursor.col = 0;

        // Default tab stops at every 8 columns
        term.write_char('\t');
        assert_eq!(term.cursor.col, 8);
        assert!(!term.pending_wrap);

        term.write_char('\t');
        assert_eq!(term.cursor.col, 16);
    }

    #[test]
    fn test_write_char_tab_at_end() {
        let mut term = create_test_terminal();
        term.cursor.col = 78;

        term.write_char('\t');
        assert_eq!(term.cursor.col, 79); // Clamped to last column
    }

    #[test]
    fn test_write_char_backspace() {
        let mut term = create_test_terminal();
        term.cursor.col = 5;

        term.write_char('\x08');
        assert_eq!(term.cursor.col, 4);
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_write_char_backspace_at_start() {
        let mut term = create_test_terminal();
        term.cursor.col = 0;

        term.write_char('\x08');
        assert_eq!(term.cursor.col, 0); // Stays at 0
    }

    #[test]
    fn test_write_char_control_chars_ignored() {
        let mut term = create_test_terminal();
        term.cursor.col = 5;

        // Test various control characters (except CR, LF, TAB, BS)
        term.write_char('\x01'); // SOH
        term.write_char('\x02'); // STX
        term.write_char('\x1B'); // ESC

        assert_eq!(term.cursor.col, 5); // Cursor unchanged

        let cell = term.active_grid().get(5, 0).unwrap();
        assert_eq!(cell.c, ' '); // No character written
    }

    #[test]
    fn test_write_char_wide_character() {
        let mut term = create_test_terminal();

        term.write_char('😀'); // Emoji (wide char)

        let cell = term.active_grid().get(0, 0).unwrap();
        assert_eq!(cell.c, '😀');
        assert_eq!(cell.width, 2);
        assert!(cell.flags.wide_char());

        // Check spacer cell
        let spacer = term.active_grid().get(1, 0).unwrap();
        assert_eq!(spacer.c, ' ');
        assert!(spacer.flags.wide_char_spacer());

        assert_eq!(term.cursor.col, 2); // Cursor advanced by 2
    }

    #[test]
    fn test_write_char_wide_character_wrap() {
        let mut term = create_test_terminal();
        term.auto_wrap = true;
        term.cursor.col = 79; // Last column

        term.write_char('😀'); // Wide char won't fit

        // Should wrap to next line
        assert_eq!(term.cursor.col, 2);
        assert_eq!(term.cursor.row, 1);

        // Character should be on second row
        let cell = term.active_grid().get(0, 1).unwrap();
        assert_eq!(cell.c, '😀');

        // First row should be marked as wrapped
        assert!(term.active_grid().is_line_wrapped(0));
    }

    #[test]
    fn test_write_char_pending_wrap() {
        let mut term = create_test_terminal();
        term.auto_wrap = true;

        // Fill line to last column
        for _ in 0..80 {
            term.write_char('A');
        }

        assert_eq!(term.cursor.col, 79);
        assert!(term.pending_wrap);

        // Next character should trigger wrap
        term.write_char('B');
        assert_eq!(term.cursor.col, 1);
        assert_eq!(term.cursor.row, 1);
        assert!(!term.pending_wrap);

        let cell = term.active_grid().get(0, 1).unwrap();
        assert_eq!(cell.c, 'B');
    }

    #[test]
    fn test_write_char_no_auto_wrap() {
        let mut term = create_test_terminal();
        term.auto_wrap = false;

        // Fill line to last column
        for _ in 0..80 {
            term.write_char('A');
        }

        assert_eq!(term.cursor.col, 79); // Stays at last column
        assert!(!term.pending_wrap);

        // Next character overwrites last cell
        term.write_char('B');
        assert_eq!(term.cursor.col, 79);
        assert_eq!(term.cursor.row, 0);

        let cell = term.active_grid().get(79, 0).unwrap();
        assert_eq!(cell.c, 'B');
    }

    #[test]
    fn test_write_char_insert_mode() {
        let mut term = create_test_terminal();
        term.insert_mode = true;

        // Write some characters
        term.write_char('A');
        term.write_char('B');
        term.write_char('C');

        // Move back and insert
        term.cursor.col = 1;
        term.write_char('X');

        // Should have: A X B C (C shifted right)
        assert_eq!(term.active_grid().get(0, 0).unwrap().c, 'A');
        assert_eq!(term.active_grid().get(1, 0).unwrap().c, 'X');
        assert_eq!(term.active_grid().get(2, 0).unwrap().c, 'B');
        assert_eq!(term.active_grid().get(3, 0).unwrap().c, 'C');
    }

    #[test]
    fn test_write_char_with_attributes() {
        let mut term = create_test_terminal();

        // Set some attributes
        term.fg = Color::Rgb(255, 0, 0);
        term.bg = Color::Rgb(0, 255, 0);
        term.flags.set_bold(true);
        term.flags.set_italic(true);

        term.write_char('A');

        let cell = term.active_grid().get(0, 0).unwrap();
        assert_eq!(cell.c, 'A');
        assert_eq!(cell.fg, Color::Rgb(255, 0, 0));
        assert_eq!(cell.bg, Color::Rgb(0, 255, 0));
        assert!(cell.flags.bold());
        assert!(cell.flags.italic());
    }

    #[test]
    fn test_write_char_with_hyperlink() {
        let mut term = create_test_terminal();
        term.current_hyperlink_id = Some(42);

        term.write_char('A');

        let cell = term.active_grid().get(0, 0).unwrap();
        assert_eq!(cell.flags.hyperlink_id, Some(42));
    }

    #[test]
    fn test_write_char_guarded() {
        let mut term = create_test_terminal();
        term.char_protected = true;

        term.write_char('A');

        let cell = term.active_grid().get(0, 0).unwrap();
        assert!(cell.flags.guarded());
    }

    #[test]
    fn test_write_char_pending_wrap_clears_on_cr() {
        let mut term = create_test_terminal();
        term.auto_wrap = true;
        term.pending_wrap = true;

        term.write_char('\r');
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_write_char_pending_wrap_clears_on_lf() {
        let mut term = create_test_terminal();
        term.auto_wrap = true;
        term.pending_wrap = true;

        term.write_char('\n');
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_write_char_pending_wrap_clears_on_tab() {
        let mut term = create_test_terminal();
        term.auto_wrap = true;
        term.pending_wrap = true;

        term.write_char('\t');
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_write_char_pending_wrap_clears_on_backspace() {
        let mut term = create_test_terminal();
        term.auto_wrap = true;
        term.pending_wrap = true;

        term.write_char('\x08');
        assert!(!term.pending_wrap);
    }

    #[test]
    fn test_write_char_line_wrapping_marks() {
        let mut term = create_test_terminal();
        term.auto_wrap = true;

        // Fill first line
        for _ in 0..80 {
            term.write_char('A');
        }

        assert!(term.pending_wrap);

        // Next char triggers wrap and marks line
        term.write_char('B');

        assert!(term.active_grid().is_line_wrapped(0));
        assert!(!term.active_grid().is_line_wrapped(1));
    }

    #[test]
    fn test_write_char_wide_at_scroll_bottom() {
        let mut term = create_test_terminal();
        term.scroll_region_top = 0;
        term.scroll_region_bottom = 23;
        term.cursor.row = 23;
        term.cursor.col = 79;
        term.auto_wrap = true;

        // Write wide char that triggers wrap and scroll
        term.write_char('😀');

        // Should be on row 23 after scroll
        assert_eq!(term.cursor.row, 23);
        assert_eq!(term.cursor.col, 2);
    }

    #[test]
    fn test_write_char_cjk_characters() {
        let mut term = create_test_terminal();

        // Test various CJK characters (all wide)
        term.write_char('中'); // Chinese
        assert_eq!(term.cursor.col, 2);

        term.write_char('日'); // Japanese
        assert_eq!(term.cursor.col, 4);

        term.write_char('한'); // Korean
        assert_eq!(term.cursor.col, 6);

        // Verify all are marked as wide
        assert!(term.active_grid().get(0, 0).unwrap().flags.wide_char());
        assert!(term.active_grid().get(2, 0).unwrap().flags.wide_char());
        assert!(term.active_grid().get(4, 0).unwrap().flags.wide_char());
    }

    #[test]
    fn test_write_char_insert_mode_wide_char() {
        let mut term = create_test_terminal();
        term.insert_mode = true;

        term.write_char('A');
        term.write_char('B');

        term.cursor.col = 1;
        term.write_char('😀');

        // Should insert wide char (shifts by 2)
        assert_eq!(term.active_grid().get(0, 0).unwrap().c, 'A');
        assert_eq!(term.active_grid().get(1, 0).unwrap().c, '😀');
        assert!(term
            .active_grid()
            .get(2, 0)
            .unwrap()
            .flags
            .wide_char_spacer());
        assert_eq!(term.active_grid().get(3, 0).unwrap().c, 'B');
    }

    #[test]
    fn test_write_char_lf_with_lr_margins_outside() {
        let mut term = create_test_terminal();
        term.use_lr_margins = true;
        term.left_margin = 10;
        term.right_margin = 70;
        term.scroll_region_top = 0;
        term.scroll_region_bottom = 23;

        // Position cursor outside left/right margins but in scroll region
        term.cursor.col = 5;
        term.cursor.row = 23;

        term.write_char('\n');

        // Should not scroll when outside LR margins (iTerm2 behavior)
        assert_eq!(term.cursor.row, 23); // Clamped to bottom
    }

    #[test]
    fn test_write_char_pending_wrap_with_lr_margins() {
        let mut term = create_test_terminal();
        term.auto_wrap = true;
        term.use_lr_margins = true;
        term.left_margin = 10;
        term.right_margin = 70;

        term.cursor.col = 79;
        term.pending_wrap = true;

        term.write_char('A');

        // Should wrap to left margin
        assert_eq!(term.cursor.col, 11);
        assert_eq!(term.cursor.row, 1);
    }
}
