//! Grid snapshot type for the Instant Replay feature and par-mux persistence.
//!
//! Lives in `grid` so the grid can capture and restore itself without
//! depending on the `terminal` layer; `terminal::replay_snapshot` re-exports it.

use crate::cell::Cell;
use crate::zone::Zone;

/// Snapshot of a single Grid's state (primary or alternate screen).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GridSnapshot {
    /// Visible screen cells (row-major, cols * rows)
    pub cells: Vec<Cell>,
    /// Scrollback buffer cells (flat, circular buffer linearized)
    pub scrollback_cells: Vec<Cell>,
    /// Start index of the circular scrollback buffer
    pub scrollback_start: usize,
    /// Number of lines currently in scrollback
    pub scrollback_lines: usize,
    /// Maximum scrollback capacity
    pub max_scrollback: usize,
    /// Number of columns
    pub cols: usize,
    /// Number of rows
    pub rows: usize,
    /// Line-wrap flags for visible rows
    pub wrapped: Vec<bool>,
    /// Line-wrap flags for scrollback rows
    pub scrollback_wrapped: Vec<bool>,
    /// Semantic zones
    pub zones: Vec<Zone>,
    /// Total number of lines ever scrolled into scrollback
    pub total_lines_scrolled: usize,
}
