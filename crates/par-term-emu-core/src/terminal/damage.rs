//! Damage tracking (ARC-003): dirty rows, damage generations, trigger-tracked
//! scroll/insert/delete helpers, and dirty-range iteration.

use super::Terminal;

impl Terminal {
    /// Mark a row as dirty (needs redrawing). Damage normally comes from the
    /// Grid mutators themselves (ENH-025); this forwarder stays for the
    /// non-grid visual changes (cursor shape, selection) and the Python API.
    pub fn mark_row_dirty(&mut self, row: usize) {
        self.active_grid_mut().mark_row_damage(row);
    }

    /// Scan the pending rows a region scroll is about to push off the
    /// screen, before the grid moves them (ARC-091). The row selection is
    /// [`trigger::TriggerState::take_departing`]; a departing row's match
    /// reports the row it occupied before the scroll.
    pub(super) fn scan_departing_trigger_rows(
        &mut self,
        down: bool,
        n: usize,
        top: usize,
        bottom: usize,
    ) {
        let grid_rows = self.active_grid().rows();
        let departing = self
            .triggers
            .take_departing(down, n, top, bottom, grid_rows);
        if departing.is_empty() {
            return;
        }
        crate::terminal::TriggerEngine::scan_rows(self, departing);
    }

    /// Scroll `[top, bottom]` of the active grid up by `n`, keeping
    /// graphics and pending trigger rows with their content. Every
    /// Terminal-level region scroll goes through here so none can skip
    /// the trigger bookkeeping (ARC-091).
    pub(crate) fn scroll_region_up_tracked(&mut self, n: usize, top: usize, bottom: usize) {
        self.scan_departing_trigger_rows(false, n, top, bottom);
        self.active_grid_mut().scroll_region_up(n, top, bottom);
        self.adjust_graphics_for_scroll_up(n, top, bottom);
        self.triggers.shift_pending(false, n, top, bottom);
    }

    /// Scroll `[top, bottom]` of the active grid down by `n`; the mirror of
    /// [`Terminal::scroll_region_up_tracked`].
    pub(crate) fn scroll_region_down_tracked(&mut self, n: usize, top: usize, bottom: usize) {
        self.scan_departing_trigger_rows(true, n, top, bottom);
        self.active_grid_mut().scroll_region_down(n, top, bottom);
        self.adjust_graphics_for_scroll_down(n, top, bottom);
        self.triggers.shift_pending(true, n, top, bottom);
    }

    /// IL: insert `n` blank lines at `row`, pushing `[row, bottom]` down.
    /// Rows pushed past `bottom` are scanned first (ARC-091).
    pub(crate) fn insert_lines_tracked(&mut self, n: usize, row: usize, bottom: usize) {
        self.scan_departing_trigger_rows(true, n, row, bottom);
        self.active_grid_mut().insert_lines(n, row, bottom);
        self.triggers.shift_pending(true, n, row, bottom);
    }

    /// DL: delete `n` lines at `row`, pulling `[row, bottom]` up. The
    /// deleted rows are scanned first (ARC-091).
    pub(crate) fn delete_lines_tracked(&mut self, n: usize, row: usize, bottom: usize) {
        self.scan_departing_trigger_rows(false, n, row, bottom);
        self.active_grid_mut().delete_lines(n, row, bottom);
        self.triggers.shift_pending(false, n, row, bottom);
    }

    /// Scan pending rows before content moves somewhere row indices cannot
    /// follow (alt-screen grid swap, reflowing resize).
    pub(crate) fn flush_pending_trigger_rows(&mut self) {
        if self.triggers.has_pending() {
            crate::terminal::TriggerEngine::process_trigger_scans(self);
        }
    }

    /// Mark an inclusive range of rows dirty — scrolling changes every row
    /// in the scrolled region, not just the ones that received cells.
    pub(crate) fn mark_rows_dirty(&mut self, top: usize, bottom: usize) {
        self.active_grid_mut().mark_rows_damage(top, bottom);
    }

    /// Carry the maximum damage generation across both grids. Generations
    /// are per grid, but consumers remember one number; syncing before any
    /// wholesale mark keeps both grids' stamps comparable (ENH-025).
    ///
    /// Callers: `invalidate_visible_screen` and `resize` only. `resize` syncs
    /// before the regrid because `Grid::resize` marks every row itself.
    pub(super) fn sync_damage_generations(&mut self) {
        let gen = self.grid.generation().max(self.alt_grid.generation());
        self.grid.raise_generation(gen);
        self.alt_grid.raise_generation(gen);
    }

    /// The only way to report a wholesale change of what the renderer
    /// sees; call it after the active grid or its content was replaced
    /// (screen switch, RIS, snapshot restore) (ARC-092).
    ///
    /// Generations are per grid but consumers remember one number
    /// (`damage_generation()`, the max of both), so the newly visible grid
    /// must stamp above everything either grid has handed out before its
    /// rows are marked.
    pub(super) fn invalidate_visible_screen(&mut self) {
        self.sync_damage_generations();
        let rows = self.active_grid().rows();
        // Content marks, not just positional ones (ENH-038): after a
        // wholesale change the content-dirty set must cover every row of
        // the newly visible grid, or a blit-aware renderer misses the
        // swap entirely.
        self.active_grid_mut()
            .mark_rows_content(0, rows.saturating_sub(1));
        // The visible content was replaced wholesale (screen switch, RIS,
        // snapshot restore): the scroll log cannot describe how it moved,
        // so every older generation gets the full-redraw sentinel (ENH-038).
        self.grid.invalidate_scroll_log();
        self.alt_grid.invalidate_scroll_log();
    }

    /// Mark the entire screen as clean
    ///
    /// Records the current damage generation for the built-in default
    /// consumer; consumers holding their own generations from
    /// `damage_generation()` are unaffected (ENH-025).
    pub fn mark_clean(&mut self) {
        self.default_consumer_gen = self.damage_generation();
    }

    /// Current damage generation. Consumers remember this value and pass it
    /// to `dirty_rows_since` to observe only what changed since.
    pub fn damage_generation(&self) -> u64 {
        self.grid.generation().max(self.alt_grid.generation())
    }

    /// Rows damaged since the given generation (ascending). The generation
    /// comes from an earlier `damage_generation()` call; a screen switch
    /// dirties every row of the newly visible grid.
    pub fn dirty_rows_since(&self, gen: u64) -> impl Iterator<Item = usize> + '_ {
        self.active_grid().damage_indices(gen)
    }

    /// Iterate the dirty row numbers in ascending order
    pub(super) fn dirty_row_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.dirty_rows_since(self.default_consumer_gen)
    }

    /// Invoke `f(start, end)` once per maximal run of consecutive dirty
    /// rows (inclusive, ascending) — the allocation-free coalescing behind
    /// the FFI dirty-range surface (ENH-026). Serves the built-in default
    /// consumer, like `dirty_row_indices`.
    #[cfg(feature = "ffi")]
    #[doc(hidden)]
    pub fn for_each_dirty_range(&self, f: impl FnMut(u32, u32)) {
        self.for_each_dirty_range_since(self.default_consumer_gen, f);
    }

    /// Generation-aware [`Terminal::for_each_dirty_range`]: a run per
    /// maximal streak of rows damaged since `gen`.
    #[cfg(feature = "ffi")]
    #[doc(hidden)]
    pub fn for_each_dirty_range_since(&self, gen: u64, f: impl FnMut(u32, u32)) {
        self.active_grid().for_each_damage_range_since(gen, f);
    }

    /// Scroll-aware damage report for a consumer holding generation `since`
    /// (ENH-038): the net movement of the visible content and the single
    /// region it moved in, or the full-redraw sentinel. A renderer blits
    /// its previous frame's `[top, bottom]` region by `delta` rows
    /// (positive = content moved up), then redraws the rows
    /// [`Terminal::for_each_content_dirty_range_since`] reports — rows
    /// vacated by the blit carry a fresh content generation, so they are
    /// always in that set.
    pub fn scroll_damage_since(&self, since: u64) -> crate::grid::ScrollDamage {
        self.active_grid().scroll_damage_since(since)
    }

    /// Invoke `f(start, end)` once per maximal run of consecutive rows whose
    /// *content* changed since `gen` — the blit-complement of
    /// [`Terminal::scroll_damage_since`].
    pub fn for_each_content_dirty_range_since(&self, since: u64, f: impl FnMut(u32, u32)) {
        self.active_grid()
            .for_each_content_damage_range_since(since, f);
    }

    /// Get all dirty rows (ascending)
    pub fn get_dirty_rows(&self) -> Vec<usize> {
        self.dirty_row_indices().collect()
    }

    /// Get the bounding box of the dirty region
    pub fn get_dirty_region(&self) -> Option<(usize, usize, usize, usize)> {
        // Ascending iteration: the first dirty row is the min, the last is the max.
        let mut indices = self.dirty_row_indices();
        let first_row = indices.next()?;
        let last_row = indices.last().unwrap_or(first_row);
        let cols = self.grid.cols();

        Some((first_row, 0, last_row, cols.saturating_sub(1)))
    }

    /// Count lines with non-whitespace content
    pub fn count_non_whitespace_lines(&self) -> usize {
        let mut count = 0;
        let grid = self.active_grid();
        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                if line.iter().any(|c| c.c != ' ') {
                    count += 1;
                }
            }
        }
        count
    }
}
