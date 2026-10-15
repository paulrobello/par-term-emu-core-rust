//! Rectangle operations (ARC-003): DECRQCRA checksums, rectangle reads,
//! fill/erase, and visible-region/row-range queries.

use super::Terminal;
use crate::cell::Cell;

impl Terminal {
    /// Calculate a checksum for a rectangular region of cells
    pub fn calculate_rectangle_checksum(
        &self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> u16 {
        let mut checksum: u32 = 0;
        let grid = self.active_grid();

        for row in top..=bottom {
            if let Some(line) = grid.row(row) {
                for col in left..=right {
                    if let Some(cell) = line.get(col) {
                        checksum = checksum.wrapping_add(cell.c as u32);
                        // Add other attributes to checksum if needed by spec
                    }
                }
            }
        }

        (checksum & 0xFFFF) as u16
    }

    /// Get a rectangular region of cells
    pub fn get_rectangle(
        &self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> Vec<Vec<Cell>> {
        let mut rows = Vec::new();
        let grid = self.active_grid();

        for row in top..=bottom {
            let mut cells = Vec::new();
            if let Some(line) = grid.row(row) {
                for col in left..=right {
                    if let Some(cell) = line.get(col) {
                        cells.push(cell.clone());
                    }
                }
            }
            rows.push(cells);
        }

        rows
    }

    /// Fill a rectangular region with a character
    pub fn fill_rectangle(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        ch: char,
    ) {
        let cell = Cell::with_colors(ch, self.attrs.fg, self.attrs.bg);

        for row in top..=bottom {
            for col in left..=right {
                self.active_grid_mut().set(col, row, cell.clone());
            }
            self.mark_row_dirty(row);
        }
    }

    /// Erase a rectangular region (fill with spaces)
    pub fn erase_rectangle(&mut self, top: usize, left: usize, bottom: usize, right: usize) {
        self.fill_rectangle(top, left, bottom, right, ' ');
    }

    /// Get the visible region bounds
    pub fn get_visible_region(&self) -> (usize, usize, usize, usize) {
        let (cols, rows) = self.size();
        (0, 0, rows.saturating_sub(1), cols.saturating_sub(1))
    }

    /// Get a range of rows as vectors of cells (end is exclusive)
    pub fn get_row_range(&self, start: usize, end: usize) -> Vec<Vec<Cell>> {
        let mut rows = Vec::new();
        let grid = self.active_grid();
        for r in start..end {
            if let Some(line) = grid.row(r) {
                rows.push(line.to_vec());
            }
        }
        rows
    }
}
