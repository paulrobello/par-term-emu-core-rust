//! Character and line editing operations for the terminal grid

use crate::grid::Grid;

impl Grid {
    /// Insert n blank lines at row
    pub fn insert_lines(&mut self, n: usize, row: usize, scroll_bottom: usize) {
        if row >= self.rows || row > scroll_bottom {
            return;
        }
        let effective_bottom = scroll_bottom.min(self.rows - 1);
        // Clamp n against the clamped bottom: a scroll_bottom past the
        // screen (or an n past the region size) must empty the region, not
        // underflow the copy range. row <= effective_bottom always holds
        // here (row < rows and row <= scroll_bottom), so this cannot
        // underflow either.
        let n = n.min(effective_bottom - row + 1);

        // Shift [row..=effective_bottom - n] down by n, iterating by
        // destination so neither range endpoint can underflow: the range
        // is empty exactly when the whole region shifts out.
        for i in ((row + n)..=effective_bottom).rev() {
            let src_start = (i - n) * self.cols;
            let dst_start = i * self.cols;
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
        }

        for i in row..(row + n).min(self.rows) {
            self.clear_row(i);
        }
    }

    /// Delete n lines at row
    pub fn delete_lines(&mut self, n: usize, row: usize, scroll_bottom: usize) {
        if row >= self.rows || row > scroll_bottom {
            return;
        }
        let effective_bottom = scroll_bottom.min(self.rows - 1);
        // Same clamped-region bound as insert_lines (see above).
        let n = n.min(effective_bottom - row + 1);

        // Shift [row + n..=effective_bottom] up by n. Sources exist while
        // i + n is inside the region; once past it the remaining rows are
        // blanked by the tail loop, so the shift stops there.
        for i in row..=effective_bottom {
            let src = i + n;
            if src > effective_bottom {
                break;
            }
            let src_start = src * self.cols;
            let dst_start = i * self.cols;
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
        }

        let clear_start = effective_bottom + 1 - n;
        for i in clear_start..=effective_bottom {
            self.clear_row(i);
        }
    }

    /// Insert n blank characters at position
    pub fn insert_chars(&mut self, col: usize, row: usize, n: usize) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        let n = n.min(self.cols - col);
        let cols = self.cols;

        if let Some(row_cells) = self.row_mut(row) {
            for i in ((col + n)..cols).rev() {
                row_cells[i] = row_cells[i - n].clone();
            }
            for cell in row_cells.iter_mut().skip(col).take(n) {
                cell.reset();
            }
        }
    }

    /// Delete n characters at position
    pub fn delete_chars(&mut self, col: usize, row: usize, n: usize) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        let n = n.min(self.cols - col);
        let cols = self.cols;

        if let Some(row_cells) = self.row_mut(row) {
            for i in col..(cols - n) {
                row_cells[i] = row_cells[i + n].clone();
            }
            for cell in row_cells.iter_mut().skip(cols - n).take(n) {
                cell.reset();
            }
        }
    }

    /// Alias for insert_chars to satisfy CSI dispatcher
    pub fn insert_characters(&mut self, col: usize, row: usize, n: usize) {
        self.insert_chars(col, row, n);
    }

    /// Alias for delete_chars to satisfy CSI dispatcher
    pub fn delete_characters(&mut self, col: usize, row: usize, n: usize) {
        self.delete_chars(col, row, n);
    }
}
