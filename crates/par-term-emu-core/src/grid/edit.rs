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

        // Shift [row..=effective_bottom - n] down by n by rotating row
        // pointers; the rows pushed past the bottom wrap around to `row`
        // and are blanked. Content generations and wrap flags move with
        // their rows (ENH-038); the inserted blanks take fresh stamps.
        self.rotate_region(n, row, effective_bottom, false, false);
        self.mark_rows_damage(row, effective_bottom);
        if n > 0 {
            self.record_scroll_op(row, effective_bottom, -(n as i32));
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

        // Shift [row + n..=effective_bottom] up by n by rotating row
        // pointers; the deleted rows wrap around to the bottom of the
        // region and are blanked (ENH-038 generations move with them).
        self.rotate_region(n, row, effective_bottom, true, false);
        self.mark_rows_damage(row, effective_bottom);
        if n > 0 {
            self.record_scroll_op(row, effective_bottom, n as i32);
        }
    }

    /// Insert n blank characters at position
    pub fn insert_chars(&mut self, col: usize, row: usize, n: usize) {
        if row >= self.rows || col >= self.cols || n == 0 {
            return;
        }
        let n = n.min(self.cols - col);
        let cols = self.cols;

        if let Some(row_cells) = self.row_mut(row) {
            // A wide character the insertion splits is blanked (both
            // halves), and so is one pushed half off the row, as in xterm.
            if col > 0 && row_cells[col - 1].flags.wide_char() {
                row_cells[col - 1].reset();
                if row_cells[col].flags.wide_char_spacer() {
                    row_cells[col].reset();
                }
            }
            for i in ((col + n)..cols).rev() {
                row_cells[i] = row_cells[i - n].clone();
            }
            for cell in row_cells.iter_mut().skip(col).take(n) {
                cell.reset();
            }
            if row_cells[cols - 1].flags.wide_char() {
                row_cells[cols - 1].reset();
            }
        }
    }

    /// Delete n characters at position
    pub fn delete_chars(&mut self, col: usize, row: usize, n: usize) {
        if row >= self.rows || col >= self.cols || n == 0 {
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
            // A wide character whose spacer was deleted, and a spacer whose
            // wide character was deleted, are blanked, as in xterm.
            if col > 0 && row_cells[col - 1].flags.wide_char() {
                row_cells[col - 1].reset();
            }
            if row_cells[col].flags.wide_char_spacer() {
                row_cells[col].reset();
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
