//! Erase and clear operations for the terminal grid
//!
//! BCE (Background Color Erase): VT-compatible erase operations fill erased
//! cells with the current SGR background color rather than always defaulting
//! to black. All erase methods accept a `bg` parameter for this purpose.

use crate::color::{Color, NamedColor};
use crate::grid::Grid;

/// Default background used by non-erase callers (scroll, edit)
pub(crate) const DEFAULT_BG: Color = Color::Named(NamedColor::Black);

impl Grid {
    /// Clear the entire grid, filling cells with the given background color (BCE)
    pub fn clear_with_bg(&mut self, bg: Color) {
        for cell in &mut self.cells {
            cell.reset();
            cell.bg = bg;
        }
        self.zones.clear();
    }

    /// Clear the entire grid with default background
    pub fn clear(&mut self) {
        self.clear_with_bg(DEFAULT_BG);
    }

    /// Clear a specific row, filling cells with the given background color (BCE)
    pub fn clear_row_with_bg(&mut self, row: usize, bg: Color) {
        if let Some(row_cells) = self.row_mut(row) {
            for cell in row_cells.iter_mut() {
                cell.reset();
                cell.bg = bg;
            }
        }
    }

    /// Clear a specific row with default background
    pub fn clear_row(&mut self, row: usize) {
        self.clear_row_with_bg(row, DEFAULT_BG);
    }

    /// Clear from cursor to end of line, filling cells with the given background color (BCE)
    pub fn clear_line_right(&mut self, col: usize, row: usize, bg: Color) {
        if row < self.rows {
            for c in col..self.cols {
                if let Some(cell) = self.get_mut(c, row) {
                    cell.reset();
                    cell.bg = bg;
                }
            }
        }
    }

    /// Clear from beginning of line to cursor, filling cells with the given background color (BCE)
    pub fn clear_line_left(&mut self, col: usize, row: usize, bg: Color) {
        if row < self.rows {
            for c in 0..=col.min(self.cols - 1) {
                if let Some(cell) = self.get_mut(c, row) {
                    cell.reset();
                    cell.bg = bg;
                }
            }
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
            let end = (col + n).min(self.cols);
            for c in col..end {
                if let Some(cell) = self.get_mut(c, row) {
                    cell.reset();
                    cell.bg = bg;
                }
            }
        }
    }

    /// Alias for erase_characters
    pub fn erase_chars(&mut self, col: usize, row: usize, n: usize, bg: Color) {
        self.erase_characters(col, row, n, bg);
    }

    /// Clear the scrollback buffer
    pub fn clear_scrollback(&mut self) {
        self.scrollback_cells.clear();
        self.scrollback_start = 0;
        self.scrollback_lines = 0;
        self.scrollback_wrapped.clear();
        // Reset floor for zones
        self.total_lines_scrolled = 0;
        // Re-evict zones (effectively clears all zones that started in scrollback)
        self.evict_zones(0);
    }
}
