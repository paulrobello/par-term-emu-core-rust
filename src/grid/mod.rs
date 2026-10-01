//! Terminal grid implementation
//!
//! Provides a 2D grid of cells with scrollback support, reflow capability,
//! and semantic zone tracking.

use std::collections::VecDeque;

use crate::cell::Cell;
use crate::zone::Zone;

mod edit;
mod erase;
mod export;
mod rect;
mod scroll;
mod snapshot;
mod zone;

pub use snapshot::GridSnapshot;

/// One recorded vertical scroll (ENH-038): `[top, bottom]` rotated by
/// `delta` rows at generation `gen` (positive = content moved up). The
/// scroll log lets `scroll_damage_since` tell a renderer "blit the region
/// by the net delta, redraw only the content-dirty rows".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScrollOp {
    pub(crate) gen: u64,
    pub(crate) top: u32,
    pub(crate) bottom: u32,
    pub(crate) delta: i32,
}

/// Cap on the scroll log. Beyond it the oldest entries would be lost
/// silently, so the log resets to the full-redraw epoch instead.
pub(crate) const MAX_SCROLL_OPS: usize = 64;

/// Scroll-aware damage report (ENH-038): how the visible content moved
/// since a generation. When [`ScrollDamage::full_redraw`] is false, a
/// renderer can blit its previous frame's `[top, bottom]` region by
/// `delta` rows and redraw only the rows `for_each_content_damage_range_since`
/// reports; otherwise it must fall back to a full redraw (today's behavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollDamage {
    /// The scroll history cannot describe the change (screen switch,
    /// resize, RIS, snapshot restore, scrollback clear, log overflow,
    /// mixed regions): treat every row as dirty.
    pub full_redraw: bool,
    /// First row of the scrolled region (inclusive, 0-indexed).
    pub top: u32,
    /// Last row of the scrolled region (inclusive, 0-indexed).
    pub bottom: u32,
    /// Net rows the region content moved; positive means content moved up.
    pub delta: i32,
}

/// A 2D grid of terminal cells
#[derive(Debug, Clone)]
pub struct Grid {
    /// Number of columns
    pub(in crate::grid) cols: usize,
    /// Number of rows
    pub(in crate::grid) rows: usize,
    /// The actual grid data (row-major order)
    pub(in crate::grid) cells: Vec<Cell>,
    /// Scrollback buffer (flat Vec, row-major order like main grid)
    pub(in crate::grid) scrollback_cells: Vec<Cell>,
    /// Index of oldest line in circular scrollback buffer
    pub(in crate::grid) scrollback_start: usize,
    /// Number of lines currently in scrollback
    pub(in crate::grid) scrollback_lines: usize,
    /// Maximum scrollback lines
    pub(in crate::grid) max_scrollback: usize,
    /// Track which lines are wrapped
    pub(in crate::grid) wrapped: Vec<bool>,
    /// Track wrapped state for scrollback lines
    pub(in crate::grid) scrollback_wrapped: Vec<bool>,
    /// Semantic zones tracking logical blocks (Prompt, Command, Output)
    pub(in crate::grid) zones: Vec<Zone>,
    /// Zones that were evicted from scrollback
    pub(in crate::grid) evicted_zones: Vec<Zone>,
    /// Total number of lines that have ever been scrolled into scrollback.
    pub(in crate::grid) total_lines_scrolled: usize,
    /// Per-row damage generations over `rows` (ENH-025). Mutators stamp the
    /// rows they change with the grid's monotonic counter so the damage
    /// contract cannot be forgotten at a call site; `get_mut`/`row_mut`
    /// mark conservatively because they hand out `&mut`.
    pub(in crate::grid) row_gen: Vec<u64>,
    /// Per-row content generations (ENH-038): the same clock as `row_gen`,
    /// but stamped only when a row's *content* may have changed and rotated
    /// with the cells by the scroll primitives. So while `row_gen` answers
    /// "this position needs a repaint", `row_content_gen` answers "this
    /// position's pixels differ from any earlier frame" — rows that merely
    /// moved keep the generation they already had.
    pub(in crate::grid) row_content_gen: Vec<u64>,
    /// Recorded scroll ops since the last invalidation (ENH-038), oldest
    /// first. Bounded by [`MAX_SCROLL_OPS`]; overflow resets the log to the
    /// full-redraw epoch.
    pub(in crate::grid) scroll_ops: VecDeque<ScrollOp>,
    /// Full-redraw floor (ENH-038): a consumer generation below this
    /// predates an event (resize, snapshot restore, scrollback clear,
    /// screen switch) after which the scroll log no longer describes how
    /// the visible content moved.
    pub(in crate::grid) scroll_epoch: u64,
    /// Monotonic damage generation counter — every mark stamps a fresh
    /// value from it, so consumers diff against a remembered generation
    /// instead of clearing shared state.
    pub(in crate::grid) gen: u64,
}

impl Grid {
    /// Create a new grid with the specified dimensions
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        let cells = vec![Cell::default(); cols * rows];
        Self {
            cols,
            rows,
            cells,
            scrollback_cells: Vec::new(),
            scrollback_start: 0,
            scrollback_lines: 0,
            max_scrollback,
            wrapped: vec![false; rows],
            scrollback_wrapped: Vec::new(),
            zones: Vec::new(),
            evicted_zones: Vec::new(),
            total_lines_scrolled: 0,
            row_gen: vec![0u64; rows],
            row_content_gen: vec![0u64; rows],
            scroll_ops: VecDeque::new(),
            scroll_epoch: 0,
            gen: 0,
        }
    }

    /// Mark a row as damaged (needs redraw). Stamps the row with a fresh
    /// generation so every consumer decides dirtiness against its own
    /// remembered generation (ENH-025).
    pub fn mark_row_damage(&mut self, row: usize) {
        self.gen += 1;
        if row < self.row_gen.len() {
            self.row_gen[row] = self.gen;
        }
    }

    /// Mark an inclusive row range as damaged
    pub fn mark_rows_damage(&mut self, top: usize, bottom: usize) {
        let last_row = self.rows.saturating_sub(1);
        for row in top..=bottom.min(last_row) {
            self.mark_row_damage(row);
        }
    }

    /// Mark a row's *content* as changed (ENH-038): stamps both the
    /// positional damage generation (today's behavior) and the content
    /// generation, so content mutators cannot forget either contract.
    pub fn mark_row_content(&mut self, row: usize) {
        self.mark_row_damage(row);
        if row < self.row_content_gen.len() {
            self.row_content_gen[row] = self.gen;
        }
    }

    /// Mark an inclusive row range's content as changed
    pub fn mark_rows_content(&mut self, top: usize, bottom: usize) {
        let last_row = self.rows.saturating_sub(1);
        for row in top..=bottom.min(last_row) {
            self.mark_row_content(row);
        }
    }

    /// Record a completed scroll of `[top, bottom]` by `delta` rows
    /// (positive = content moved up). Must be called after the op's damage
    /// marks, so `op.gen` postdates every generation a pre-scroll consumer
    /// could hold. Overflow drops the history into the full-redraw epoch
    /// rather than silently losing entries.
    pub(crate) fn record_scroll_op(&mut self, top: usize, bottom: usize, delta: i32) {
        if self.scroll_ops.len() >= MAX_SCROLL_OPS {
            self.invalidate_scroll_log();
        }
        self.scroll_ops.push_back(ScrollOp {
            gen: self.gen,
            top: top as u32,
            bottom: bottom as u32,
            delta,
        });
    }

    /// Drop the scroll log and raise its full-redraw floor: every consumer
    /// generation handed out before this call now predates the epoch and
    /// queries with it fall back to a full redraw (ENH-038).
    pub(crate) fn invalidate_scroll_log(&mut self) {
        self.gen += 1;
        self.scroll_epoch = self.gen;
        self.scroll_ops.clear();
    }

    /// Current damage generation of this grid
    pub fn generation(&self) -> u64 {
        self.gen
    }

    /// Raise this grid's generation floor to `gen`. Screen switches and
    /// wholesale invalidations use this to keep both grids' stamps
    /// comparable against generations consumers captured earlier.
    pub fn raise_generation(&mut self, gen: u64) {
        self.gen = self.gen.max(gen);
    }

    /// Iterate row numbers damaged since generation `since`, ascending
    pub fn damage_indices(&self, since: u64) -> impl Iterator<Item = usize> + '_ {
        self.row_gen
            .iter()
            .enumerate()
            .filter_map(move |(row, &gen)| (gen > since).then_some(row))
    }

    /// Invoke `f(start, end)` once per maximal run of consecutive rows
    /// damaged since generation `since`, ascending, without allocating.
    /// Coalesces exactly the rows [`Grid::damage_indices`] would yield
    /// one by one (ENH-026).
    pub fn for_each_damage_range_since(&self, since: u64, mut f: impl FnMut(u32, u32)) {
        let mut run: Option<(u32, u32)> = None;
        for (row, &gen) in self.row_gen.iter().enumerate() {
            if gen > since {
                let row = row as u32;
                match run {
                    Some((start, end)) if end + 1 == row => run = Some((start, row)),
                    Some((start, end)) => {
                        f(start, end);
                        run = Some((row, row));
                    }
                    None => run = Some((row, row)),
                }
            }
        }
        if let Some((start, end)) = run {
            f(start, end);
        }
    }

    /// Invoke `f(start, end)` once per maximal run of consecutive rows whose
    /// *content* changed since generation `since`, ascending, without
    /// allocating (ENH-038). Coalesces exactly the rows whose
    /// `row_content_gen` exceeds `since` — the rows a scroll-aware renderer
    /// must redraw after blitting its previous frame.
    pub fn for_each_content_damage_range_since(&self, since: u64, mut f: impl FnMut(u32, u32)) {
        let mut run: Option<(u32, u32)> = None;
        for (row, &gen) in self.row_content_gen.iter().enumerate() {
            if gen > since {
                let row = row as u32;
                match run {
                    Some((start, end)) if end + 1 == row => run = Some((start, row)),
                    Some((start, end)) => {
                        f(start, end);
                        run = Some((row, row));
                    }
                    None => run = Some((row, row)),
                }
            }
        }
        if let Some((start, end)) = run {
            f(start, end);
        }
    }

    /// Scroll-aware damage report for a consumer holding generation `since`
    /// (ENH-038): the net scroll of the visible content and the single
    /// region it moved in, or the full-redraw sentinel when the history
    /// cannot express the change. Pairs with
    /// [`Grid::for_each_content_damage_range_since`].
    pub fn scroll_damage_since(&self, since: u64) -> ScrollDamage {
        let last_row = self.rows.saturating_sub(1) as u32;
        let full = ScrollDamage {
            full_redraw: true,
            top: 0,
            bottom: last_row,
            delta: 0,
        };
        if since < self.scroll_epoch {
            return full;
        }
        let mut region: Option<(u32, u32)> = None;
        let mut net: i64 = 0;
        for op in &self.scroll_ops {
            if op.gen > since {
                match region {
                    None => region = Some((op.top, op.bottom)),
                    Some((top, bottom)) if top == op.top && bottom == op.bottom => {}
                    // Mixed scroll regions: one delta cannot describe them.
                    Some(_) => return full,
                }
                net += i64::from(op.delta);
            }
        }
        let delta = match i32::try_from(net) {
            Ok(delta) => delta,
            // Unrepresentable net delta (unreachable for real geometries):
            // fail closed to a full redraw rather than report a wrong blit.
            Err(_) => return full,
        };
        let (top, bottom) = region.unwrap_or((0, last_row));
        ScrollDamage {
            full_redraw: false,
            top,
            bottom,
            delta,
        }
    }

    /// Get the number of columns
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Get the number of rows
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Get a reference to a cell at (col, row)
    pub fn get(&self, col: usize, row: usize) -> Option<&Cell> {
        if col < self.cols && row < self.rows {
            Some(&self.cells[row * self.cols + col])
        } else {
            None
        }
    }

    /// Get a mutable reference to a cell at (col, row). Marks the row's
    /// content changed conservatively — the caller may write through the
    /// reference.
    pub fn get_mut(&mut self, col: usize, row: usize) -> Option<&mut Cell> {
        if col < self.cols && row < self.rows {
            self.mark_row_content(row);
            Some(&mut self.cells[row * self.cols + col])
        } else {
            None
        }
    }

    /// Set a cell at (col, row)
    pub fn set(&mut self, col: usize, row: usize, cell: Cell) {
        if col < self.cols && row < self.rows {
            self.mark_row_content(row);
            self.cells[row * self.cols + col] = cell;
        }
    }

    /// Get a row as a slice
    pub fn row(&self, row: usize) -> Option<&[Cell]> {
        if row < self.rows {
            let start = row * self.cols;
            let end = start + self.cols;
            Some(&self.cells[start..end])
        } else {
            None
        }
    }

    /// Get a mutable row. Marks the row's content changed conservatively —
    /// the caller may write through the slice.
    pub fn row_mut(&mut self, row: usize) -> Option<&mut [Cell]> {
        if row < self.rows {
            self.mark_row_content(row);
            let start = row * self.cols;
            let end = start + self.cols;
            Some(&mut self.cells[start..end])
        } else {
            None
        }
    }

    /// Get the text content of a row
    pub fn row_text(&self, row: usize) -> String {
        // Write directly into one String instead of allocating a Vec<String>
        // per row (QA-006).
        match self.row(row) {
            Some(cells) => {
                let mut result = String::with_capacity(cells.len());
                for cell in cells.iter() {
                    if !cell.flags.wide_char_spacer() {
                        cell.push_grapheme(&mut result);
                    }
                }
                result
            }
            None => String::new(),
        }
    }

    /// Get total number of lines currently in scrollback
    pub fn scrollback_len(&self) -> usize {
        self.scrollback_lines
    }

    /// Get total number of lines that have ever been scrolled
    pub fn total_lines_scrolled(&self) -> usize {
        self.total_lines_scrolled
    }

    /// Get maximum scrollback capacity
    pub fn max_scrollback(&self) -> usize {
        self.max_scrollback
    }

    /// Check if a line is wrapped
    pub fn is_line_wrapped(&self, row: usize) -> bool {
        self.wrapped.get(row).copied().unwrap_or(false)
    }

    /// Set wrapped state for a line. The wrap flag changes how the row
    /// renders (joined lines), so the row's content is marked changed too.
    pub fn set_line_wrapped(&mut self, row: usize, wrapped: bool) {
        if let Some(w) = self.wrapped.get_mut(row) {
            *w = wrapped;
            self.mark_row_content(row);
        }
    }

    /// Physical index into `scrollback_cells` for a logical scrollback line
    /// (0 = oldest). Centralized circular-buffer math (ARC-026).
    #[inline]
    fn scrollback_physical_index(&self, logical: usize) -> usize {
        (self.scrollback_start + logical) % self.max_scrollback
    }

    /// Advance the circular-buffer write head by one (ARC-026).
    #[inline]
    fn advance_scrollback_head(&mut self) {
        self.scrollback_start = (self.scrollback_start + 1) % self.max_scrollback;
    }

    /// Get a line from scrollback by index
    pub fn scrollback_line(&self, index: usize) -> Option<&[Cell]> {
        if index < self.scrollback_lines {
            let physical_index = self.scrollback_physical_index(index);
            let start = physical_index * self.cols;
            let end = start + self.cols;
            Some(&self.scrollback_cells[start..end])
        } else {
            None
        }
    }

    /// Check if a scrollback line is wrapped
    pub fn is_scrollback_wrapped(&self, index: usize) -> bool {
        if index < self.scrollback_lines {
            let physical_index = self.scrollback_physical_index(index);
            self.scrollback_wrapped
                .get(physical_index)
                .copied()
                .unwrap_or(false)
        } else {
            false
        }
    }

    /// Capture a snapshot of this grid's entire state.
    #[must_use]
    pub fn capture_snapshot(&self) -> GridSnapshot {
        GridSnapshot {
            cells: self.cells.clone(),
            scrollback_cells: self.scrollback_cells.clone(),
            scrollback_start: self.scrollback_start,
            scrollback_lines: self.scrollback_lines,
            max_scrollback: self.max_scrollback,
            cols: self.cols,
            rows: self.rows,
            wrapped: self.wrapped.clone(),
            scrollback_wrapped: self.scrollback_wrapped.clone(),
            zones: self.zones.clone(),
            total_lines_scrolled: self.total_lines_scrolled,
        }
    }

    /// Restore this grid's state from a previously captured snapshot.
    pub fn restore_from_snapshot(&mut self, snap: &GridSnapshot) {
        self.cells = snap.cells.clone();
        self.scrollback_cells = snap.scrollback_cells.clone();
        self.scrollback_start = snap.scrollback_start;
        self.scrollback_lines = snap.scrollback_lines;
        self.max_scrollback = snap.max_scrollback;
        self.cols = snap.cols;
        self.rows = snap.rows;
        self.wrapped = snap.wrapped.clone();
        self.scrollback_wrapped = snap.scrollback_wrapped.clone();
        self.zones = snap.zones.clone();
        self.evicted_zones.clear();
        self.total_lines_scrolled = snap.total_lines_scrolled;
        // `gen` is kept, never lowered: the fresh stamps land above every
        // generation this grid handed out before the restore. The other
        // grid's generations are the owning Terminal's to sync (ARC-092).
        self.row_gen = vec![0u64; self.rows];
        self.row_content_gen = vec![0u64; self.rows];
        self.mark_rows_content(0, self.rows.saturating_sub(1));
        // A restore replaces the screen wholesale; the scroll log's motion
        // history describes content that no longer exists (ENH-038).
        self.invalidate_scroll_log();
    }
}

#[cfg(test)]
mod tests;
