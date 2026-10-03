//! Scrolling and reflow logic for the terminal grid

use crate::cell::Cell;
use crate::grid::Grid;

impl Grid {
    fn push_rows_to_scrollback(&mut self, start_row: usize, count: usize) {
        if self.max_scrollback == 0 || count == 0 || start_row >= self.rows {
            return;
        }

        let available = self.rows - start_row;
        let count = count.min(available);

        self.total_lines_scrolled += count;
        if self.scrollback_lines >= self.max_scrollback {
            let floor = self
                .total_lines_scrolled
                .saturating_sub(self.max_scrollback);
            self.evict_zones(floor);
        }

        for i in 0..count {
            let row = start_row + i;
            let src_start = row * self.cols;
            let src_end = src_start + self.cols;
            let is_wrapped = self.wrapped.get(row).copied().unwrap_or(false);
            self.push_scrollback_row(
                self.cells[src_start..src_end].to_vec().into_boxed_slice(),
                is_wrapped,
            );
        }
    }

    /// Scroll up by n lines
    pub fn scroll_up(&mut self, n: usize) {
        let n = n.min(self.rows);
        if n == 0 {
            return;
        }

        // Drain the top n rows out in one shot: the remaining rows shift up
        // via a single memmove inside drain, and the drained rows move into
        // scrollback by ownership instead of being cloned cell by cell —
        // the per-cell SmallVec clone here was ~50% of plain_ascii runtime.
        let mut drained: Vec<Cell> = self.cells.drain(0..n * self.cols).collect();
        let wrapped_len = self.wrapped.len();
        let wrapped_n = n.min(wrapped_len);
        let drained_wrapped: Vec<bool> = self.wrapped.drain(0..wrapped_n).collect();

        self.absorb_rows_into_scrollback(&mut drained, &drained_wrapped);

        // Cell::default() carries bg == DEFAULT_BG (Named Black), so the
        // refill matches what clear_row wrote on these rows before.
        self.cells.resize(self.rows * self.cols, Cell::default());
        self.wrapped.resize(wrapped_len, false);
        // Content generations move with their rows (ENH-038); the n fresh
        // blank rows at the bottom take fresh stamps.
        self.row_content_gen.drain(0..n);
        self.row_content_gen.resize(self.rows, 0);
        for row in (self.rows - n)..self.rows {
            self.mark_row_content(row);
        }
        self.mark_rows_damage(0, self.rows.saturating_sub(1));
        self.record_scroll_op(0, self.rows - 1, n as i32);
    }

    /// Absorb rows drained out of the main grid into the scrollback buffer.
    ///
    /// Takes ownership per line so each row moves into scrollback by
    /// ownership (no per-cell clone). Bookkeeping mirrors
    /// [`push_rows_to_scrollback`]: the total-lines counter, zone eviction,
    /// and oldest-line eviction when the buffer is full.
    fn absorb_rows_into_scrollback(&mut self, rows: &mut Vec<Cell>, wrapped_flags: &[bool]) {
        let cols = self.cols;
        if cols == 0 || self.max_scrollback == 0 {
            return;
        }
        let count = (rows.len() / cols).min(wrapped_flags.len());
        if count == 0 {
            return;
        }

        self.total_lines_scrolled += count;
        if self.scrollback_lines >= self.max_scrollback {
            let floor = self
                .total_lines_scrolled
                .saturating_sub(self.max_scrollback);
            self.evict_zones(floor);
        }

        for i in 0..count {
            let is_wrapped = wrapped_flags[i];
            let line: Vec<Cell> = rows[i * cols..(i + 1) * cols]
                .iter_mut()
                .map(std::mem::take)
                .collect();
            self.push_scrollback_row(line.into_boxed_slice(), is_wrapped);
        }
        rows.clear();
    }

    /// Scroll down by n lines
    pub fn scroll_down(&mut self, n: usize) {
        let n = n.min(self.rows);
        if n == 0 {
            return;
        }

        // One rotation of the whole grid replaces the reverse per-cell clone
        // loop: rotate_right relocates rows n places down without cloning
        // the SmallVec in every cell (down-mirror of the scroll_up drain
        // fix; rows [0, n) land on stale bottom content and are cleared).
        self.cells.rotate_right(n * self.cols);
        self.wrapped.rotate_right(n);
        self.row_content_gen.rotate_right(n);

        for i in 0..n {
            self.clear_row(i);
            if i < self.wrapped.len() {
                self.wrapped[i] = false;
            }
        }
        self.mark_rows_damage(0, self.rows.saturating_sub(1));
        self.record_scroll_op(0, self.rows - 1, -(n as i32));
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
        self.mark_rows_damage(top, effective_bottom);

        if top == 0 && effective_bottom == self.rows - 1 && self.max_scrollback > 0 {
            self.scroll_up(n);
            return true;
        }

        if top == 0 {
            self.push_rows_to_scrollback(0, n);
        }

        if n >= region_size {
            for i in top..=effective_bottom {
                self.clear_row(i);
            }
            if n > 0 {
                self.record_scroll_op(top, effective_bottom, n as i32);
            }
            return true;
        }

        // One rotation of the contiguous region replaces the per-row
        // clone_from_slice moves: rotate_left relocates elements without
        // cloning the SmallVec in every cell.
        let region_start = top * self.cols;
        let region_end = (effective_bottom + 1) * self.cols;
        self.cells[region_start..region_end].rotate_left(n * self.cols);
        self.row_content_gen[top..=effective_bottom].rotate_left(n);

        for i in (effective_bottom - n + 1)..=effective_bottom {
            if i < self.rows {
                self.clear_row(i);
            }
        }
        if n > 0 {
            self.record_scroll_op(top, effective_bottom, n as i32);
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
        self.mark_rows_damage(top, effective_bottom);

        if n > effective_bottom - top {
            for i in top..=effective_bottom {
                self.clear_row(i);
            }
            if n > 0 {
                self.record_scroll_op(top, effective_bottom, -(n as i32));
            }
            return true;
        }

        // One rotation of the contiguous region replaces the reverse per-row
        // clone loop — down-mirror of scroll_region_up's rotate_left.
        let region_start = top * self.cols;
        let region_end = (effective_bottom + 1) * self.cols;
        self.cells[region_start..region_end].rotate_right(n * self.cols);
        self.row_content_gen[top..=effective_bottom].rotate_right(n);

        for i in top..(top + n).min(self.rows) {
            self.clear_row(i);
        }
        if n > 0 {
            self.record_scroll_op(top, effective_bottom, -(n as i32));
        }
        true
    }

    /// Resize the grid
    /// Resize WITHOUT reflow: every row keeps its screen position and is
    /// truncated or right-padded to the new width; wrap flags are cleared.
    ///
    /// The alternate screen must resize this way. A full-screen app draws
    /// the alt screen cell by cell and redraws it on SIGWINCH — often only
    /// the cells it believes changed (ratatui, curses). Reflowing joins or
    /// splits its rows, so cells the app thinks are still in place move,
    /// and the scramble persists (kanban-tui/top garbled after any
    /// resize). xterm, tmux, and every other emulator leave the alt screen
    /// un-reflowed for exactly this reason.
    pub fn resize_without_reflow(&mut self, cols: usize, rows: usize) {
        if self.cols == cols && self.rows == rows {
            return;
        }
        if cols == 0 || rows == 0 {
            return;
        }
        let mut cells = vec![Cell::default(); cols * rows];
        let copy_rows = rows.min(self.rows);
        let copy_cols = cols.min(self.cols);
        for row in 0..copy_rows {
            let src = row * self.cols;
            let dst = row * cols;
            cells[dst..dst + copy_cols].clone_from_slice(&self.cells[src..src + copy_cols]);
        }
        self.cells = cells;
        self.wrapped = vec![false; rows];
        self.cols = cols;
        self.rows = rows;
        self.reset_damage_for_resize();
    }

    /// Reset per-row damage generations to the new row count and mark every
    /// row: a resize moves content even when the cell data survives
    /// unchanged.
    fn reset_damage_for_resize(&mut self) {
        self.row_gen = vec![0u64; self.rows];
        self.row_content_gen = vec![0u64; self.rows];
        self.mark_rows_content(0, self.rows.saturating_sub(1));
        // A resize or reflow moves/rewraps content wholesale; the scroll
        // log cannot describe it, so older generations get the full-redraw
        // sentinel (ENH-038).
        self.invalidate_scroll_log();
    }

    /// Resize the visible grid to `cols` × `rows`. A width change reflows the
    /// visible screen; the scrollback is never rebuilt — stored lines keep
    /// the width they scrolled off at (renderers handle short lines by
    /// padding). A height-only change resizes in place. Every visible row is
    /// marked damaged. A no-op when the size is unchanged or either
    /// dimension is zero.
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
            self.reset_damage_for_resize();

            // Scrollback remains identical (no push/pull)
            // Zones remain valid as they track absolute indices
            return;
        }

        // Width changed: reflow the visible screen only. Scrollback lines
        // keep their original widths — resizing cost is bounded by the
        // viewport rows, never the scrollback.
        let old_cols = self.cols;
        let old_rows = self.rows;

        self.reflow_main_grid(old_cols, old_rows, cols, rows);
        self.reset_damage_for_resize();
    }

    /// Push one reflowed excess line into scrollback (resize path).
    fn reflow_push_line(&mut self, row_cells: &[Cell], is_wrapped: bool) {
        if self.scrollback_lines < self.max_scrollback {
            self.scrollback_rows
                .push(row_cells.to_vec().into_boxed_slice());
            self.scrollback_wrapped.push(is_wrapped);
            self.scrollback_lines += 1;
        } else {
            self.scrollback_rows.remove(0);
            self.scrollback_wrapped.remove(0);
            self.scrollback_rows
                .push(row_cells.to_vec().into_boxed_slice());
            self.scrollback_wrapped.push(is_wrapped);
        }
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
                    self.reflow_push_line(row_cells, is_wrapped);
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

#[cfg(test)]
mod tests {
    use crate::cell::Cell;
    use crate::grid::{Grid, ScrollDamage, MAX_SCROLL_OPS};

    fn grid() -> Grid {
        Grid::new(8, 6, 100)
    }

    fn fill(grid: &mut Grid, ch: char) {
        for row in 0..grid.rows() {
            for col in 0..grid.cols() {
                grid.set(col, row, Cell::new(ch));
            }
        }
    }

    fn content_dirty_rows(grid: &Grid, since: u64) -> Vec<u32> {
        let mut rows = Vec::new();
        grid.for_each_content_damage_range_since(since, |s, e| rows.extend(s..=e));
        rows
    }

    #[test]
    fn scroll_up_reports_delta_and_only_cleared_rows() {
        let mut g = grid();
        fill(&mut g, 'x');
        let since = g.generation();
        g.scroll_up(1);

        let d = g.scroll_damage_since(since);
        assert_eq!(
            d,
            ScrollDamage {
                full_redraw: false,
                top: 0,
                bottom: 5,
                delta: 1
            }
        );
        assert_eq!(content_dirty_rows(&g, since), vec![5]);
        // Positional damage still covers every row: ENH-025 behavior unchanged.
        assert_eq!(g.damage_indices(since).count(), 6);
    }

    #[test]
    fn scroll_down_reports_negative_delta_and_cleared_top_rows() {
        let mut g = grid();
        fill(&mut g, 'x');
        let since = g.generation();
        g.scroll_down(2);

        let d = g.scroll_damage_since(since);
        assert_eq!(d.delta, -2);
        assert_eq!(d.top, 0);
        assert_eq!(d.bottom, 5);
        assert!(!d.full_redraw);
        assert_eq!(content_dirty_rows(&g, since), vec![0, 1]);
    }

    #[test]
    fn region_scroll_reports_the_region() {
        let mut g = grid();
        fill(&mut g, 'x');
        let since = g.generation();
        assert!(g.scroll_region_up(1, 2, 4));

        let d = g.scroll_damage_since(since);
        assert_eq!(d.top, 2);
        assert_eq!(d.bottom, 4);
        assert_eq!(d.delta, 1);
        assert!(!d.full_redraw);
        assert_eq!(content_dirty_rows(&g, since), vec![4]);
    }

    #[test]
    fn same_region_ops_compose_by_sum() {
        let mut g = grid();
        fill(&mut g, 'x');
        let since = g.generation();
        g.scroll_up(1);
        g.scroll_up(2);

        assert_eq!(g.scroll_damage_since(since).delta, 3);
    }

    #[test]
    fn mixed_regions_fall_back_to_full_redraw() {
        let mut g = grid();
        fill(&mut g, 'x');
        let since = g.generation();
        g.scroll_up(1);
        assert!(g.scroll_region_up(1, 1, 3));

        assert!(g.scroll_damage_since(since).full_redraw);
    }

    #[test]
    fn no_scrollback_grid_still_records_ops() {
        // Alt-screen-shaped grid: max_scrollback == 0, so scroll_region_up
        // takes the inline path instead of delegating to scroll_up.
        let mut g = Grid::new(8, 6, 0);
        fill(&mut g, 'x');
        let since = g.generation();
        assert!(g.scroll_region_up(1, 0, 5));

        let d = g.scroll_damage_since(since);
        assert_eq!(d.delta, 1);
        assert_eq!(content_dirty_rows(&g, since), vec![5]);
        assert_eq!(g.total_lines_scrolled(), 0);
    }

    #[test]
    fn resize_clears_scrollback_and_restore_invalidate_the_log() {
        let mut g = grid();
        fill(&mut g, 'x');
        let snap = g.capture_snapshot();

        let since = g.generation();
        g.resize(4, 3);
        assert!(g.scroll_damage_since(since).full_redraw);

        let since = g.generation();
        g.clear_scrollback();
        assert!(g.scroll_damage_since(since).full_redraw);

        let since = g.generation();
        g.restore_from_snapshot(&snap);
        assert!(g.scroll_damage_since(since).full_redraw);
    }

    #[test]
    fn log_overflow_falls_back_to_full_redraw() {
        let mut g = grid();
        fill(&mut g, 'x');
        let since = g.generation();
        for _ in 0..MAX_SCROLL_OPS {
            g.scroll_up(1);
        }
        // At the cap the whole history is still expressible.
        assert_eq!(g.scroll_damage_since(since).delta, MAX_SCROLL_OPS as i32);

        g.scroll_up(1);
        assert!(g.scroll_damage_since(since).full_redraw);
    }
}
