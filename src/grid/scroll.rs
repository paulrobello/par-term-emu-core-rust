//! Scrolling and reflow logic for the terminal grid

use crate::cell::Cell;
use crate::grid::Grid;

impl Grid {
    /// Scroll up by n lines
    pub fn scroll_up(&mut self, n: usize) {
        let n = n.min(self.rows);

        if self.max_scrollback > 0 {
            self.total_lines_scrolled += n;
            if self.scrollback_lines >= self.max_scrollback {
                let floor = self
                    .total_lines_scrolled
                    .saturating_sub(self.max_scrollback);
                self.evict_zones(floor);
            }
        }

        if self.max_scrollback > 0 {
            for i in 0..n {
                let src_start = i * self.cols;
                let src_end = src_start + self.cols;
                let is_wrapped = self.wrapped.get(i).copied().unwrap_or(false);

                if self.scrollback_lines < self.max_scrollback {
                    self.scrollback_cells
                        .extend_from_slice(&self.cells[src_start..src_end]);
                    self.scrollback_wrapped.push(is_wrapped);
                    self.scrollback_lines += 1;
                } else {
                    let write_idx = self.scrollback_start;
                    let dst_start = write_idx * self.cols;
                    let dst_end = dst_start + self.cols;

                    self.scrollback_cells[dst_start..dst_end]
                        .clone_from_slice(&self.cells[src_start..src_end]);
                    self.scrollback_wrapped[write_idx] = is_wrapped;
                    self.scrollback_start = (self.scrollback_start + 1) % self.max_scrollback;
                }
            }
        }

        for i in n..self.rows {
            let src_start = i * self.cols;
            let dst_start = (i - n) * self.cols;
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
            if i < self.wrapped.len() && (i - n) < self.wrapped.len() {
                self.wrapped[i - n] = self.wrapped[i];
            }
        }

        for i in (self.rows - n)..self.rows {
            self.clear_row(i);
            if i < self.wrapped.len() {
                self.wrapped[i] = false;
            }
        }
    }

    /// Scroll down by n lines
    pub fn scroll_down(&mut self, n: usize) {
        let n = n.min(self.rows);

        for i in (n..self.rows).rev() {
            let src_start = (i - n) * self.cols;
            let dst_start = i * self.cols;
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
            if (i - n) < self.wrapped.len() && i < self.wrapped.len() {
                self.wrapped[i] = self.wrapped[i - n];
            }
        }

        for i in 0..n {
            self.clear_row(i);
            if i < self.wrapped.len() {
                self.wrapped[i] = false;
            }
        }
    }

    /// Scroll up within a region. Returns `false` if parameters are invalid.
    pub fn scroll_region_up(&mut self, n: usize, top: usize, bottom: usize) -> bool {
        if top >= self.rows || bottom >= self.rows || top > bottom {
            #[cfg(debug_assertions)]
            eprintln!(
                "Invalid scroll region up: top={} bottom={} rows={}",
                top, bottom, self.rows
            );
            return false;
        }

        let n = n.min(bottom - top + 1);
        let effective_bottom = bottom.min(self.rows - 1);
        let region_size = effective_bottom - top + 1;

        if top == 0 && effective_bottom == self.rows - 1 && self.max_scrollback > 0 {
            self.scroll_up(n);
            return true;
        }

        if n >= region_size {
            for i in top..=effective_bottom {
                self.clear_row(i);
            }
            return true;
        }

        for i in top..=(effective_bottom - n) {
            let src_start = (i + n) * self.cols;
            let dst_start = i * self.cols;
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
        }

        for i in (effective_bottom - n + 1)..=effective_bottom {
            if i < self.rows {
                self.clear_row(i);
            }
        }
        true
    }

    /// Scroll down within a region. Returns `false` if parameters are invalid.
    pub fn scroll_region_down(&mut self, n: usize, top: usize, bottom: usize) -> bool {
        if top >= self.rows || bottom >= self.rows || top > bottom {
            #[cfg(debug_assertions)]
            eprintln!(
                "Invalid scroll region down: top={} bottom={} rows={}",
                top, bottom, self.rows
            );
            return false;
        }

        let n = n.min(bottom - top + 1);
        let effective_bottom = bottom.min(self.rows - 1);

        if n > effective_bottom - top {
            for i in top..=effective_bottom {
                self.clear_row(i);
            }
            return true;
        }

        for i in ((top + n)..=effective_bottom).rev() {
            let src_start = (i - n) * self.cols;
            let dst_start = i * self.cols;
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
        }

        for i in top..(top + n).min(self.rows) {
            self.clear_row(i);
        }
        true
    }

    /// Resize the grid
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if self.cols == cols && self.rows == rows {
            return;
        }

        if cols == 0 || rows == 0 {
            return;
        }

        if self.cols == cols {
            // Width unchanged: Optimized path using simple Vec resizing
            // This implicitly handles growing (padding with default) and shrinking (truncating)
            // for the main grid, without touching scrollback.

            self.cells.resize(cols * rows, Cell::default());
            self.wrapped.resize(rows, false);
            self.rows = rows;

            // Scrollback remains identical (no push/pull)
            // Zones remain valid as they track absolute indices
            return;
        }

        // Width changed: Full reflow
        let old_cols = self.cols;
        let old_rows = self.rows;

        if self.max_scrollback > 0 && self.scrollback_lines > 0 {
            self.reflow_scrollback(old_cols, cols);
        }

        self.reflow_main_grid(old_cols, old_rows, cols, rows);
    }

    fn reflow_scrollback(&mut self, old_cols: usize, new_cols: usize) {
        let logical_lines = self.extract_scrollback_logical_lines(old_cols);
        let mut new_sb_cells = Vec::new();
        let mut new_sb_wrapped = Vec::new();

        for logical_line in logical_lines {
            let (cells, wrapped_flags) = self.rewrap_logical_line(&logical_line, new_cols);

            if cells.is_empty() {
                for _ in 0..new_cols {
                    new_sb_cells.push(Cell::default());
                }
                new_sb_wrapped.push(false);
                continue;
            }

            for (i, row_cells) in cells.chunks(new_cols).enumerate() {
                new_sb_cells.extend(row_cells.iter().cloned());
                while new_sb_cells.len() % new_cols != 0 {
                    new_sb_cells.push(Cell::default());
                }
                new_sb_wrapped.push(wrapped_flags.get(i).copied().unwrap_or(false));
            }
        }

        if new_sb_wrapped.len() > self.max_scrollback {
            let excess = new_sb_wrapped.len() - self.max_scrollback;
            let cells_to_drop = excess * new_cols;
            new_sb_cells.drain(0..cells_to_drop);
            new_sb_wrapped.drain(0..excess);
        }

        self.scrollback_cells = new_sb_cells;
        self.scrollback_wrapped = new_sb_wrapped;
        self.scrollback_lines = self.scrollback_wrapped.len();
        self.scrollback_start = 0;
    }

    fn reflow_main_grid(
        &mut self,
        old_cols: usize,
        old_rows: usize,
        new_cols: usize,
        new_rows: usize,
    ) {
        let logical_lines = self.extract_main_grid_logical_lines(old_cols, old_rows);
        let mut all_cells = Vec::new();
        let mut all_wrapped = Vec::new();

        for logical_line in logical_lines {
            let (cells, wrapped_flags) = self.rewrap_logical_line(&logical_line, new_cols);

            if cells.is_empty() {
                for _ in 0..new_cols {
                    all_cells.push(Cell::default());
                }
                all_wrapped.push(false);
                continue;
            }

            for (i, row_cells) in cells.chunks(new_cols).enumerate() {
                all_cells.extend(row_cells.iter().cloned());
                while all_cells.len() % new_cols != 0 {
                    all_cells.push(Cell::default());
                }
                all_wrapped.push(wrapped_flags.get(i).copied().unwrap_or(false));
            }
        }

        let mut last_content_line = 0;
        for (line_idx, _) in all_wrapped.iter().enumerate() {
            let start = line_idx * new_cols;
            let end = (start + new_cols).min(all_cells.len());
            if all_cells[start..end]
                .iter()
                .any(|c| c.c != ' ' || !c.is_empty())
            {
                last_content_line = line_idx + 1;
            }
        }

        let effective_lines = last_content_line.max(1);
        if effective_lines > new_rows {
            let excess_lines = effective_lines - new_rows;
            if self.max_scrollback > 0 {
                for line_idx in 0..excess_lines {
                    let start = line_idx * new_cols;
                    let end = start + new_cols;
                    let row_cells = &all_cells[start..end];
                    let is_wrapped = all_wrapped.get(line_idx).copied().unwrap_or(false);

                    if self.scrollback_lines < self.max_scrollback {
                        self.scrollback_cells.extend(row_cells.iter().cloned());
                        self.scrollback_wrapped.push(is_wrapped);
                        self.scrollback_lines += 1;
                    } else {
                        let physical_index = self.scrollback_start;
                        let sb_start = physical_index * new_cols;
                        self.scrollback_cells[sb_start..sb_start + new_cols]
                            .clone_from_slice(row_cells);
                        self.scrollback_wrapped[physical_index] = is_wrapped;
                        self.scrollback_start = (self.scrollback_start + 1) % self.max_scrollback;
                    }
                }
            }
            let keep_start = excess_lines * new_cols;
            all_cells = all_cells[keep_start..].to_vec();
            all_wrapped = all_wrapped[excess_lines..].to_vec();
        }

        let mut new_cells = vec![Cell::default(); new_cols * new_rows];
        let mut new_wrapped = vec![false; new_rows];
        let lines_to_copy = all_wrapped.len().min(new_rows);
        for row in 0..lines_to_copy {
            let src_start = row * new_cols;
            let dst_start = row * new_cols;
            if src_start + new_cols <= all_cells.len() {
                new_cells[dst_start..dst_start + new_cols]
                    .clone_from_slice(&all_cells[src_start..src_start + new_cols]);
            }
            new_wrapped[row] = all_wrapped[row];
        }

        self.cols = new_cols;
        self.rows = new_rows;
        self.cells = new_cells;
        self.wrapped = new_wrapped;
    }

    fn extract_main_grid_logical_lines(&self, old_cols: usize, old_rows: usize) -> Vec<Vec<Cell>> {
        let mut logical_lines = Vec::new();
        let mut current_line = Vec::new();
        for row in 0..old_rows {
            for col in 0..old_cols {
                if let Some(cell) = self.get(col, row) {
                    if !cell.flags.wide_char_spacer() {
                        current_line.push(cell.clone());
                    }
                }
            }
            if !self.is_line_wrapped(row) {
                while current_line
                    .last()
                    .is_some_and(|c| c.c == ' ' && c.is_empty())
                {
                    current_line.pop();
                }
                logical_lines.push(std::mem::take(&mut current_line));
            }
        }
        if !current_line.is_empty() {
            logical_lines.push(current_line);
        }
        logical_lines
    }

    fn extract_scrollback_logical_lines(&self, _old_cols: usize) -> Vec<Vec<Cell>> {
        let mut logical_lines = Vec::new();
        let mut current_line = Vec::new();
        for i in 0..self.scrollback_lines {
            if let Some(line) = self.scrollback_line(i) {
                for cell in line {
                    if !cell.flags.wide_char_spacer() {
                        current_line.push(cell.clone());
                    }
                }
                if !self.is_scrollback_wrapped(i) {
                    while current_line
                        .last()
                        .is_some_and(|c| c.c == ' ' && c.is_empty())
                    {
                        current_line.pop();
                    }
                    logical_lines.push(std::mem::take(&mut current_line));
                }
            }
        }
        if !current_line.is_empty() {
            logical_lines.push(current_line);
        }
        logical_lines
    }

    fn rewrap_logical_line(&self, line: &[Cell], width: usize) -> (Vec<Cell>, Vec<bool>) {
        let mut new_cells = Vec::new();
        let mut wrapped_flags = Vec::new();
        let mut current_col = 0;

        for cell in line {
            let char_width = cell.width as usize;
            if current_col + char_width > width {
                while current_col < width {
                    new_cells.push(Cell::default());
                    current_col += 1;
                }
                wrapped_flags.push(true);
                current_col = 0;
            }
            new_cells.push(cell.clone());
            current_col += char_width;
            for _ in 1..char_width {
                let mut spacer = Cell::default();
                spacer.flags.set_wide_char_spacer(true);
                new_cells.push(spacer);
            }
        }
        wrapped_flags.push(false);
        (new_cells, wrapped_flags)
    }
}
