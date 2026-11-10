//! Erase and clear operations for the terminal grid
//!
//! BCE (Background Color Erase): VT-compatible erase operations fill erased
//! cells with the current SGR background color rather than always defaulting
//! to black. All erase methods accept a `bg` parameter for this purpose.

use crate::cell::PackedColor;
use crate::color::{Color, NamedColor};
use crate::grid::Grid;

/// Default background used by non-erase callers (scroll, edit)
pub(crate) const DEFAULT_BG: Color = Color::Named(NamedColor::Black);

impl Grid {
    /// Clear the entire grid, filling cells with the given background color (BCE)
    pub fn clear_with_bg(&mut self, bg: Color) {
        for cell in self.screen.iter_mut().flat_map(|row| row.iter_mut()) {
            cell.reset();
            cell.bg = PackedColor::pack(bg);
        }
        self.wrapped.fill(false);
        // A default background leaves default cells behind.
        let used = if bg == DEFAULT_BG { 0 } else { self.cols };
        self.row_used.fill(used);
        self.zones.clear();
        // Every cell changed, not just moved (ENH-038).
        self.mark_rows_content(0, self.rows.saturating_sub(1));
    }

    /// Clear the entire grid with default background
    pub fn clear(&mut self) {
        self.clear_with_bg(DEFAULT_BG);
    }

    /// Clear a specific row, filling cells with the given background color (BCE)
    pub fn clear_row_with_bg(&mut self, row: usize, bg: Color) {
        if row < self.rows {
            let end = self.cols;
            self.erase_span(row, 0, end, bg);
        }
    }

    /// Reset `[start, end)` of screen row `row` with background `bg`,
    /// marking the row changed once. With the default background the
    /// cells past the row's written part are already blank, so only the
    /// written part is touched; the written-part bound follows the erase.
    fn erase_span(&mut self, row: usize, start: usize, end: usize, bg: Color) {
        let end = end.min(self.cols);
        if start >= end {
            return;
        }
        // Blank the halves of wide pairs cut by the span edges before the
        // span resets cells (the split also pre-blanks the partner cell).
        self.split_wide_chars_at(row, start, end, PackedColor::pack(bg));
        self.mark_row_content(row);
        let used = self.row_used[row];
        if bg == DEFAULT_BG {
            // A default cell is exactly what reset + the default bg write.
            let cells = &mut self.screen[row];
            for cell in &mut cells[start..end.min(used.max(start))] {
                cell.reset();
            }
            if end >= used {
                self.row_used[row] = used.min(start);
            }
        } else {
            let packed = PackedColor::pack(bg);
            for cell in &mut self.screen[row][start..end] {
                cell.reset();
                cell.bg = packed;
            }
            self.row_used[row] = used.max(end);
        }
    }

    /// Clear a specific row with default background
    pub fn clear_row(&mut self, row: usize) {
        self.clear_row_with_bg(row, DEFAULT_BG);
    }

    /// Clear from cursor to end of line, filling cells with the given background color (BCE)
    pub fn clear_line_right(&mut self, col: usize, row: usize, bg: Color) {
        if row < self.rows {
            let end = self.cols;
            self.erase_span(row, col, end, bg);
        }
    }

    /// Clear from beginning of line to cursor, filling cells with the given background color (BCE)
    pub fn clear_line_left(&mut self, col: usize, row: usize, bg: Color) {
        if row < self.rows && self.cols > 0 {
            let end = col.min(self.cols - 1) + 1;
            self.erase_span(row, 0, end, bg);
        }
    }

    /// Clear from cursor to end of screen, filling cells with the given background color (BCE)
    pub fn clear_screen_below(&mut self, col: usize, row: usize, bg: Color) {
        self.clear_line_right(col, row, bg);
        for r in (row + 1)..self.rows {
            self.clear_row_with_bg(r, bg);
        }
    }

    /// Clear from beginning of screen to cursor, filling cells with the given background color (BCE)
    pub fn clear_screen_above(&mut self, col: usize, row: usize, bg: Color) {
        for r in 0..row {
            self.clear_row_with_bg(r, bg);
        }
        self.clear_line_left(col, row, bg);
    }

    /// Erase characters at (col, row), filling cells with the given background color (BCE)
    pub fn erase_characters(&mut self, col: usize, row: usize, n: usize, bg: Color) {
        if row < self.rows {
            let end = col.saturating_add(n).min(self.cols);
            self.erase_span(row, col, end, bg);
        }
    }

    /// Alias for erase_characters
    pub fn erase_chars(&mut self, col: usize, row: usize, n: usize, bg: Color) {
        self.erase_characters(col, row, n, bg);
    }

    /// Clear the scrollback buffer
    pub fn clear_scrollback(&mut self) {
        self.scrollback_rows.clear();
        self.scrollback_rows.shrink_to_fit();
        self.scrollback_lines = 0;
        self.scrollback_wrapped.clear();
        // Reset floor for zones
        self.total_lines_scrolled = 0;
        // Re-evict zones (effectively clears all zones that started in scrollback)
        self.evict_zones(0);
        // ENH-038: a cleared scrollback cannot be reconciled with the scroll
        // log's history — older generations get the full-redraw sentinel.
        self.invalidate_scroll_log();
    }
}
