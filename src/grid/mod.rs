//! Terminal grid implementation
//!
//! Provides a 2D grid of cells with scrollback support, reflow capability,
//! and semantic zone tracking.

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

    /// Get a mutable reference to a cell at (col, row). Marks the row
    /// damaged conservatively — the caller may write through the reference.
    pub fn get_mut(&mut self, col: usize, row: usize) -> Option<&mut Cell> {
        if col < self.cols && row < self.rows {
            self.mark_row_damage(row);
            Some(&mut self.cells[row * self.cols + col])
        } else {
            None
        }
    }

    /// Set a cell at (col, row)
    pub fn set(&mut self, col: usize, row: usize, cell: Cell) {
        if col < self.cols && row < self.rows {
            self.mark_row_damage(row);
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

    /// Get a mutable row. Marks the row damaged conservatively — the
    /// caller may write through the slice.
    pub fn row_mut(&mut self, row: usize) -> Option<&mut [Cell]> {
        if row < self.rows {
            self.mark_row_damage(row);
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
    /// renders (joined lines), so the row is damaged too.
    pub fn set_line_wrapped(&mut self, row: usize, wrapped: bool) {
        if let Some(w) = self.wrapped.get_mut(row) {
            *w = wrapped;
            self.mark_row_damage(row);
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
        self.mark_rows_damage(0, self.rows.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests;
