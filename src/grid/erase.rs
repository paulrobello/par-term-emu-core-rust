//! Erase and clear operations for the terminal grid

use crate::cell::Cell;
use crate::grid::Grid;

impl Grid {
    /// Clear the entire grid
    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
        self.zones.clear();
    }

    /// Clear a specific row
    pub fn clear_row(&mut self, row: usize) {
        if let Some(row_cells) = self.row_mut(row) {
            row_cells.fill(Cell::default());
        }
    }

    /// Clear from cursor to end of line
    pub fn clear_line_right(&mut self, col: usize, row: usize) {
        if row < self.rows {
            for c in col..self.cols {
                if let Some(cell) = self.get_mut(c, row) {
                    cell.reset();
                }
            }
        }
    }

    /// Clear from beginning of line to cursor
    pub fn clear_line_left(&mut self, col: usize, row: usize) {
        if row < self.rows {
            for c in 0..=col.min(self.cols - 1) {
                if let Some(cell) = self.get_mut(c, row) {
                    cell.reset();
                }
            }
        }
    }

    /// Clear from cursor to end of screen
    pub fn clear_screen_below(&mut self, col: usize, row: usize) {
        self.clear_line_right(col, row);
        for r in (row + 1)..self.rows {
            self.clear_row(r);
        }
    }

    /// Clear from beginning of screen to cursor
    pub fn clear_screen_above(&mut self, col: usize, row: usize) {
        for r in 0..row {
            self.clear_row(r);
        }
        self.clear_line_left(col, row);
    }

    /// Erase characters at (col, row)
    pub fn erase_characters(&mut self, col: usize, row: usize, n: usize) {
        if row < self.rows {
            let end = (col + n).min(self.cols);
            for c in col..end {
                if let Some(cell) = self.get_mut(c, row) {
                    cell.reset();
                }
            }
        }
    }

    /// Alias for erase_characters
    pub fn erase_chars(&mut self, col: usize, row: usize, n: usize) {
        self.erase_characters(col, row, n);
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
