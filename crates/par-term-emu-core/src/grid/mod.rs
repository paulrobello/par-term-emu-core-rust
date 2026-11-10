//! Terminal grid implementation
//!
//! Provides a 2D grid of cells with scrollback support, reflow capability,
//! and semantic zone tracking.

use std::collections::VecDeque;

use crate::cell::{Cell, PackedColor};
use crate::zone::Zone;

mod edit;
mod erase;
mod export;
mod rect;
mod reflow;
mod scroll;
mod snapshot;
mod zone;

pub use reflow::{BufferPositions, BufferReflow};
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
    /// The visible screen, top row first: one boxed row of exactly `cols`
    /// cells per screen line. Rows are separate allocations, so scrolling
    /// (and IL/DL) moves row pointers instead of cells, and a line scrolling
    /// off the top moves into the scrollback without a copy.
    pub(in crate::grid) screen: Vec<Box<[Cell]>>,
    /// Per screen row, how many leading cells may differ from
    /// `Cell::default()`: every cell at or past it is known to be blank.
    /// Every mutable access raises it, so blanking a row and copying it
    /// into the scrollback only touch the cells that were written.
    pub(in crate::grid) row_used: Vec<usize>,
    /// Scrollback buffer: one boxed row per retained line, oldest first.
    /// Each row keeps the column width it scrolled off at, so a grid resize
    /// only touches the viewport — stored lines are never reallocated or
    /// copied (they are re-sliced only by an explicit width reflow, which
    /// this grid no longer performs automatically). A ring buffer: evicting
    /// the oldest line at capacity is O(1), and the evicted row's
    /// allocation is reused for the blank line entering the screen.
    pub(in crate::grid) scrollback_rows: VecDeque<Box<[Cell]>>,
    /// Number of lines currently in scrollback
    pub(in crate::grid) scrollback_lines: usize,
    /// Maximum scrollback lines
    pub(in crate::grid) max_scrollback: usize,
    /// Store scrollback lines without their trailing default cells (see
    /// [`Grid::set_compact_scrollback`]).
    pub(in crate::grid) compact_scrollback: bool,
    /// Track which lines are wrapped
    pub(in crate::grid) wrapped: Vec<bool>,
    /// Wrapped state for scrollback lines (index 0 = oldest retained line)
    pub(in crate::grid) scrollback_wrapped: VecDeque<bool>,
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
    /// Positional damage of every row at once: a row counts as damaged at
    /// `max(row_gen[row], all_rows_gen)`. A full-screen mark (every
    /// scrolled line makes one) stamps this instead of every row.
    pub(in crate::grid) all_rows_gen: u64,
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
    /// Where the last xterm.js resize moved each line
    /// ([`Grid::resize_xterm`]).
    pub(in crate::grid) last_buffer_reflow: Option<BufferReflow>,
}

impl Grid {
    /// Create a new grid with the specified dimensions
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        Self {
            cols,
            rows,
            screen: blank_rows(cols, rows),
            row_used: vec![0; rows],
            scrollback_rows: VecDeque::new(),
            scrollback_lines: 0,
            max_scrollback,
            compact_scrollback: false,
            wrapped: vec![false; rows],
            scrollback_wrapped: VecDeque::new(),
            zones: Vec::new(),
            evicted_zones: Vec::new(),
            total_lines_scrolled: 0,
            row_gen: vec![0u64; rows],
            all_rows_gen: 0,
            row_content_gen: vec![0u64; rows],
            scroll_ops: VecDeque::new(),
            scroll_epoch: 0,
            gen: 0,
            last_buffer_reflow: None,
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

    /// Mark an inclusive row range as damaged. The whole range takes one
    /// fresh generation: consumers only compare stamps against a remembered
    /// generation, so one stamp per range answers them exactly as one per
    /// row did, without a counter bump per row on every scrolled line.
    pub fn mark_rows_damage(&mut self, top: usize, bottom: usize) {
        let bottom = bottom.min(self.row_gen.len().min(self.rows).saturating_sub(1));
        if top > bottom || self.row_gen.is_empty() {
            return;
        }
        self.gen += 1;
        if top == 0 && bottom + 1 == self.row_gen.len() {
            self.all_rows_gen = self.gen;
        } else {
            self.row_gen[top..=bottom].fill(self.gen);
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

    /// Mark an inclusive row range's content as changed (one fresh
    /// generation for the range, as [`Grid::mark_rows_damage`]).
    pub fn mark_rows_content(&mut self, top: usize, bottom: usize) {
        let bottom = bottom.min(self.row_content_gen.len().min(self.rows).saturating_sub(1));
        if top > bottom || self.row_content_gen.is_empty() {
            return;
        }
        self.mark_rows_damage(top, bottom);
        self.row_content_gen[top..=bottom].fill(self.gen);
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
        let all = self.all_rows_gen;
        self.row_gen
            .iter()
            .enumerate()
            .filter_map(move |(row, &gen)| (gen.max(all) > since).then_some(row))
    }

    /// Invoke `f(start, end)` once per maximal run of consecutive rows
    /// damaged since generation `since`, ascending, without allocating.
    /// Coalesces exactly the rows [`Grid::damage_indices`] would yield
    /// one by one (ENH-026).
    pub fn for_each_damage_range_since(&self, since: u64, mut f: impl FnMut(u32, u32)) {
        if self.all_rows_gen > since {
            if !self.row_gen.is_empty() {
                f(0, self.row_gen.len() as u32 - 1);
            }
            return;
        }
        for_each_run_above(&self.row_gen, since, f);
    }

    /// Invoke `f(start, end)` once per maximal run of consecutive rows whose
    /// *content* changed since generation `since`, ascending, without
    /// allocating (ENH-038). Coalesces exactly the rows whose
    /// `row_content_gen` exceeds `since` — the rows a scroll-aware renderer
    /// must redraw after blitting its previous frame.
    pub fn for_each_content_damage_range_since(&self, since: u64, f: impl FnMut(u32, u32)) {
        for_each_run_above(&self.row_content_gen, since, f);
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
            Some(&self.screen[row][col])
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
            self.row_used[row] = self.row_used[row].max(col + 1);
            Some(&mut self.screen[row][col])
        } else {
            None
        }
    }

    /// Set a cell at (col, row)
    pub fn set(&mut self, col: usize, row: usize, cell: Cell) {
        if col < self.cols && row < self.rows {
            self.mark_row_content(row);
            self.row_used[row] = self.row_used[row].max(col + 1);
            self.screen[row][col] = cell;
        }
    }

    /// Whether every screen cell past its row's written part is blank: the
    /// bound blanking and compact scrollback rely on.
    #[cfg(test)]
    pub(crate) fn blank_past_written_part(&self) -> bool {
        let blank = Cell::default();
        self.screen
            .iter()
            .zip(&self.row_used)
            .all(|(row, &used)| row.iter().skip(used).all(|cell| *cell == blank))
    }

    /// Write printable ASCII `text` into row `row` from column `col` on,
    /// each byte as a cell like `pen` (whose `c` is ignored). The row is
    /// marked changed once. Bytes past the last column are ignored.
    pub(crate) fn write_ascii(&mut self, col: usize, row: usize, text: &[u8], pen: &Cell) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        let end = (col + text.len()).min(self.cols);
        self.mark_row_content(row);
        self.row_used[row] = self.row_used[row].max(end);
        for (cell, &byte) in self.screen[row][col..end].iter_mut().zip(text) {
            *cell = Cell {
                c: byte as char,
                combining: None,
                fg: pen.fg,
                bg: pen.bg,
                underline_color: pen.underline_color,
                flags: pen.flags,
                width: 1,
            };
        }
    }

    /// Fill every screen cell with `cell` and clear the screen rows' wrap
    /// flags (DECALN).
    pub(crate) fn fill_screen(&mut self, cell: &Cell) {
        for row in self.screen.iter_mut() {
            row.fill(cell.clone());
        }
        self.wrapped.fill(false);
        self.mark_rows_content(0, self.rows.saturating_sub(1));
    }

    /// Get a row as a slice
    pub fn row(&self, row: usize) -> Option<&[Cell]> {
        if row < self.rows {
            Some(&self.screen[row])
        } else {
            None
        }
    }

    /// Get a mutable row. Marks the row's content changed conservatively —
    /// the caller may write through the slice.
    pub fn row_mut(&mut self, row: usize) -> Option<&mut [Cell]> {
        if row < self.rows {
            self.mark_row_content(row);
            self.row_used[row] = self.cols;
            Some(&mut self.screen[row])
        } else {
            None
        }
    }

    /// Before cells `[start, end)` of screen row `row` are overwritten or
    /// erased, blank the half of a wide character that lies outside the
    /// span while its other half is inside it (with background `bg`), as
    /// xterm does: no wide character is left without its spacer and no
    /// spacer without its character, which would hide the cell next to it
    /// from rendering and copying.
    pub(crate) fn split_wide_chars_at(
        &mut self,
        row: usize,
        start: usize,
        end: usize,
        bg: PackedColor,
    ) {
        let end = end.min(self.cols);
        if row >= self.rows || start >= end {
            return;
        }
        let cells = &mut self.screen[row];
        // A wide character just left of the span loses its spacer, and a
        // spacer just right of it loses its wide character.
        let head = start > 0 && cells[start - 1].flags.wide_char();
        let spacer = end < self.cols && cells[end].flags.wide_char_spacer();
        if !(head || spacer) {
            return;
        }
        let blank = Cell {
            bg,
            ..Cell::default()
        };
        if head {
            cells[start - 1] = blank.clone();
        }
        if spacer {
            cells[end] = blank;
        }
        self.mark_row_content(row);
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

    /// Change the scrollback capacity of a running grid (a preference
    /// change, like xterm.js's `options.scrollback`). A smaller capacity
    /// drops the oldest lines, as a scroll at capacity evicts them: the
    /// lines count as trimmed (`total_lines_scrolled` is kept) and zones
    /// below the new floor are evicted. A larger one keeps every line and
    /// lets more in. Returns how many oldest lines were dropped, so graphics
    /// anchors can follow the same shrink.
    pub fn set_max_scrollback(&mut self, max_scrollback: usize) -> usize {
        self.max_scrollback = max_scrollback;
        if self.scrollback_rows.len() <= max_scrollback {
            return 0;
        }
        let excess = self.scrollback_rows.len() - max_scrollback;
        self.scrollback_rows.drain(..excess);
        self.scrollback_wrapped.drain(..excess);
        self.scrollback_lines = self.scrollback_rows.len();
        let floor = self
            .total_lines_scrolled
            .saturating_sub(self.scrollback_lines);
        self.evict_zones(floor);
        // The buffer's lines moved under every remembered position.
        self.invalidate_scroll_log();
        self.mark_rows_content(0, self.rows.saturating_sub(1));
        excess
    }

    /// Store the lines entering the scrollback without their trailing
    /// default cells (cells never written, or erased with the default
    /// background). Off by default: a stored line keeps the full width it
    /// scrolled off at. On, a scrolled line costs time and memory in
    /// proportion to its text instead of the screen width. A short line
    /// reads exactly like one padded with default cells — the form stored
    /// lines already take after the screen gets wider — so readers that pad
    /// short lines see no difference. Wrapped lines keep their full width
    /// (they continue on the next line). Lines already stored are kept as
    /// they are.
    pub fn set_compact_scrollback(&mut self, compact: bool) {
        self.compact_scrollback = compact;
    }

    /// Whether lines enter the scrollback without their trailing default
    /// cells ([`Grid::set_compact_scrollback`]).
    pub fn compact_scrollback(&self) -> bool {
        self.compact_scrollback
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

    /// Get a line from scrollback by index
    pub fn scrollback_line(&self, index: usize) -> Option<&[Cell]> {
        self.scrollback_rows.get(index).map(|row| &**row)
    }

    /// Check if a scrollback line is wrapped
    pub fn is_scrollback_wrapped(&self, index: usize) -> bool {
        self.scrollback_wrapped.get(index).copied().unwrap_or(false)
    }

    /// Capture a snapshot of this grid's entire state.
    #[must_use]
    pub fn capture_snapshot(&self) -> GridSnapshot {
        GridSnapshot {
            cells: self
                .screen
                .iter()
                .flat_map(|row| row.iter().cloned())
                .collect(),
            scrollback_cells: Self::flatten_scrollback(&self.scrollback_rows, self.cols),
            scrollback_lines: self.scrollback_lines,
            max_scrollback: self.max_scrollback,
            cols: self.cols,
            rows: self.rows,
            wrapped: self.wrapped.clone(),
            scrollback_wrapped: self.scrollback_wrapped.iter().copied().collect(),
            zones: self.zones.clone(),
            total_lines_scrolled: self.total_lines_scrolled,
        }
    }

    /// Restore this grid's state from a previously captured snapshot.
    pub fn restore_from_snapshot(&mut self, snap: &GridSnapshot) {
        self.screen = rows_from_cells(&snap.cells, snap.cols, snap.rows);
        self.row_used = vec![snap.cols; snap.rows];
        self.scrollback_rows = Self::unflatten_scrollback(&snap.scrollback_cells, snap.cols);
        self.scrollback_lines = snap.scrollback_lines;
        self.max_scrollback = snap.max_scrollback;
        self.cols = snap.cols;
        self.rows = snap.rows;
        self.wrapped = snap.wrapped.clone();
        self.wrapped.resize(snap.rows, false);
        self.scrollback_wrapped = snap.scrollback_wrapped.iter().copied().collect();
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

    /// Append one line to the scrollback buffer, evicting the oldest line
    /// when the buffer is at capacity. Rows keep their own width; nothing
    /// is copied. Returns the row that left the buffer — the evicted oldest
    /// line, or `row` itself when there is no scrollback — so the caller
    /// can reuse its allocation (see [`Grid::recycle_row`]).
    pub(in crate::grid) fn push_scrollback_row(
        &mut self,
        row: Box<[Cell]>,
        wrapped: bool,
    ) -> Option<Box<[Cell]>> {
        if self.max_scrollback == 0 {
            return Some(row);
        }
        let mut evicted = None;
        while self.scrollback_rows.len() >= self.max_scrollback {
            evicted = self.scrollback_rows.pop_front();
            self.scrollback_wrapped.pop_front();
        }
        self.scrollback_rows.push_back(row);
        self.scrollback_wrapped.push_back(wrapped);
        self.scrollback_lines = self.scrollback_rows.len();
        evicted
    }

    /// A blank screen row (default cells, `cols` wide), built in the
    /// allocation of `spare` when it has the screen's width — the row a
    /// scroll evicted — instead of a fresh allocation.
    pub(in crate::grid) fn recycle_row(&self, spare: Option<Box<[Cell]>>) -> Box<[Cell]> {
        match spare {
            Some(mut row) if row.len() == self.cols => {
                row.fill(Cell::default());
                row
            }
            _ => blank_row(self.cols),
        }
    }

    /// Serialize the per-line scrollback into the flat snapshot format
    /// (oldest first, every line padded to `cols`). Byte-compatible with
    /// the pre-refactor `GridSnapshot.scrollback_cells` payload, minus the
    /// ring rotation (lines are now always stored oldest-first, so the
    /// snapshot emits `scrollback_start = 0`).
    pub(in crate::grid) fn flatten_scrollback(
        rows: &VecDeque<Box<[Cell]>>,
        cols: usize,
    ) -> Vec<Cell> {
        let mut flat = Vec::with_capacity(rows.len() * cols);
        for row in rows {
            // A row pushed before the grid narrowed is wider than `cols`:
            // the flat format is fixed-width, so it truncates.
            let start = flat.len();
            flat.extend_from_slice(&row[..row.len().min(cols)]);
            flat.resize(start + cols, Cell::default());
        }
        flat
    }

    /// Rebuild per-line rows from a flat snapshot payload. Lines shorter
    /// than `cols` are right-padded; a trailing run shorter than `cols` is
    /// kept as its own (shorter) line — this only occurs on hand-built
    /// snapshots, since [`Grid::flatten_scrollback`] always pads fully.
    pub(in crate::grid) fn unflatten_scrollback(
        flat: &[Cell],
        cols: usize,
    ) -> VecDeque<Box<[Cell]>> {
        if cols == 0 {
            return VecDeque::new();
        }
        let mut rows = VecDeque::with_capacity(flat.len() / cols.max(1));
        for chunk in flat.chunks(cols) {
            let mut row = chunk.to_vec();
            row.resize(cols, Cell::default());
            rows.push_back(row.into_boxed_slice());
        }
        rows
    }
}

/// How many leading cells of `cells` hold anything but `Cell::default()`.
pub(in crate::grid) fn used_len(cells: &[Cell]) -> usize {
    let blank = Cell::default();
    cells
        .iter()
        .rposition(|cell| *cell != blank)
        .map_or(0, |last| last + 1)
}

/// One blank screen row of `cols` default cells.
pub(in crate::grid) fn blank_row(cols: usize) -> Box<[Cell]> {
    vec![Cell::default(); cols].into_boxed_slice()
}

/// `rows` blank screen rows of `cols` default cells.
pub(in crate::grid) fn blank_rows(cols: usize, rows: usize) -> Vec<Box<[Cell]>> {
    (0..rows).map(|_| blank_row(cols)).collect()
}

/// Screen rows from a row-major cell buffer (the snapshot and reflow
/// format): `rows` rows of exactly `cols` cells, padding a short buffer
/// with default cells.
pub(in crate::grid) fn rows_from_cells(
    cells: &[Cell],
    cols: usize,
    rows: usize,
) -> Vec<Box<[Cell]>> {
    let mut chunks = cells.chunks(cols.max(1));
    (0..rows)
        .map(|_| {
            let mut row = chunks.next().map(<[Cell]>::to_vec).unwrap_or_default();
            row.resize(cols, Cell::default());
            row.into_boxed_slice()
        })
        .collect()
}

/// Invoke `f(start, end)` once per maximal run of consecutive indices whose
/// generation in `gens` exceeds `since`, ascending, without allocating. The
/// shared walk behind [`Grid::for_each_damage_range_since`] and
/// [`Grid::for_each_content_damage_range_since`].
fn for_each_run_above(gens: &[u64], since: u64, mut f: impl FnMut(u32, u32)) {
    let mut run: Option<(u32, u32)> = None;
    for (row, &gen) in gens.iter().enumerate() {
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

#[cfg(test)]
mod tests;
