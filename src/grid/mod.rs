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
mod zone;

/// A 2D grid of terminal cells
#[derive(Debug, Clone)]
pub struct Grid {
    /// Number of columns
    pub(crate) cols: usize,
    /// Number of rows
    pub(crate) rows: usize,
    /// The actual grid data (row-major order)
    pub(crate) cells: Vec<Cell>,
    /// Scrollback buffer (flat Vec, row-major order like main grid)
    pub(crate) scrollback_cells: Vec<Cell>,
    /// Index of oldest line in circular scrollback buffer
    pub(crate) scrollback_start: usize,
    /// Number of lines currently in scrollback
    pub(crate) scrollback_lines: usize,
    /// Maximum scrollback lines
    pub(crate) max_scrollback: usize,
    /// Track which lines are wrapped
    pub(crate) wrapped: Vec<bool>,
    /// Track wrapped state for scrollback lines
    pub(crate) scrollback_wrapped: Vec<bool>,
    /// Semantic zones tracking logical blocks (Prompt, Command, Output)
    pub(crate) zones: Vec<Zone>,
    /// Zones that were evicted from scrollback
    pub(crate) evicted_zones: Vec<Zone>,
    /// Total number of lines that have ever been scrolled into scrollback.
    pub(crate) total_lines_scrolled: usize,
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

    /// Get a mutable reference to a cell at (col, row)
    pub fn get_mut(&mut self, col: usize, row: usize) -> Option<&mut Cell> {
        if col < self.cols && row < self.rows {
            Some(&mut self.cells[row * self.cols + col])
        } else {
            None
        }
    }

    /// Set a cell at (col, row)
    pub fn set(&mut self, col: usize, row: usize, cell: Cell) {
        if let Some(c) = self.get_mut(col, row) {
            *c = cell;
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

    /// Get a mutable row
    pub fn row_mut(&mut self, row: usize) -> Option<&mut [Cell]> {
        if row < self.rows {
            let start = row * self.cols;
            let end = start + self.cols;
            Some(&mut self.cells[start..end])
        } else {
            None
        }
    }

    /// Get the text content of a row
    pub fn row_text(&self, row: usize) -> String {
        if let Some(cells) = self.row(row) {
            cells
                .iter()
                .filter(|cell| !cell.flags.wide_char_spacer())
                .map(|cell| cell.get_grapheme())
                .collect::<Vec<String>>()
                .join("")
        } else {
            String::new()
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

    /// Set wrapped state for a line
    pub fn set_line_wrapped(&mut self, row: usize, wrapped: bool) {
        if let Some(w) = self.wrapped.get_mut(row) {
            *w = wrapped;
        }
    }

    /// Get a line from scrollback by index
    pub fn scrollback_line(&self, index: usize) -> Option<&[Cell]> {
        if index < self.scrollback_lines {
            let physical_index = (self.scrollback_start + index) % self.max_scrollback;
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
            let physical_index = (self.scrollback_start + index) % self.max_scrollback;
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
    pub fn capture_snapshot(&self) -> crate::terminal::terminal_snapshot::GridSnapshot {
        crate::terminal::terminal_snapshot::GridSnapshot {
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
    pub fn restore_from_snapshot(
        &mut self,
        snap: &crate::terminal::terminal_snapshot::GridSnapshot,
    ) {
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
    }
}

#[cfg(test)]
mod tests;
