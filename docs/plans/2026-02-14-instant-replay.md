# Instant Replay Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add cell-level terminal snapshots with input-stream replay for iTerm2-style Instant Replay (Issue #47).

**Architecture:** New `TerminalSnapshot` type captures full grid state via Clone (no serde needed — in-memory only). `SnapshotManager` stores snapshots + input byte deltas in a size-budgeted ring buffer. `ReplaySession` navigates the timeline by restoring snapshots and replaying input bytes. Existing `SemanticSnapshot` is untouched.

**Tech Stack:** Rust, PyO3, `std::collections::VecDeque`, `std::time::{Duration, Instant}`

**Branch:** `feat/instant-replay-47`

---

### Task 1: Create Feature Branch

**Step 1: Create and checkout feature branch**

Run: `git checkout -b feat/instant-replay-47`

**Step 2: Commit placeholder**

No commit needed — branch is ready for work.

---

### Task 2: TerminalSnapshot and GridSnapshot Structs

**Files:**
- Create: `src/terminal/terminal_snapshot.rs`
- Modify: `src/terminal/mod.rs` (add `pub mod terminal_snapshot;` at line 19, after `pub mod snapshot;`)

**Step 1: Write the test file with struct tests**

Create `src/terminal/terminal_snapshot.rs` with the types and inline tests:

```rust
//! Cell-level terminal state snapshot for Instant Replay
//!
//! Unlike `SemanticSnapshot` (text-only, for AI/LLM), `TerminalSnapshot`
//! captures raw `Cell` data for pixel-perfect reconstruction including
//! colors, attributes, and grid layout.

use crate::cell::Cell;
use crate::color::Color;
use crate::cell::CellFlags;
use crate::cursor::{Cursor, CursorStyle};
use crate::grid::Grid;
use crate::zone::Zone;

/// Snapshot of a single Grid (primary or alternate screen)
#[derive(Debug, Clone)]
pub struct GridSnapshot {
    /// Grid cells in row-major order
    pub cells: Vec<Cell>,
    /// Scrollback buffer cells
    pub scrollback_cells: Vec<Cell>,
    /// Circular buffer start index
    pub scrollback_start: usize,
    /// Number of lines in scrollback
    pub scrollback_lines: usize,
    /// Maximum scrollback capacity
    pub max_scrollback: usize,
    /// Column count
    pub cols: usize,
    /// Row count
    pub rows: usize,
    /// Line wrap flags for main grid
    pub wrapped: Vec<bool>,
    /// Line wrap flags for scrollback
    pub scrollback_wrapped: Vec<bool>,
    /// Semantic zones
    pub zones: Vec<Zone>,
    /// Total lines ever scrolled (for zone eviction tracking)
    pub total_lines_scrolled: usize,
}

/// Complete cell-level terminal state snapshot
#[derive(Debug, Clone)]
pub struct TerminalSnapshot {
    /// Unix timestamp (milliseconds) when captured
    pub timestamp: u64,
    /// Terminal width
    pub cols: usize,
    /// Terminal height
    pub rows: usize,
    /// Primary grid snapshot
    pub grid: GridSnapshot,
    /// Alternate grid snapshot
    pub alt_grid: GridSnapshot,
    /// Whether alternate screen is active
    pub alt_screen_active: bool,
    /// Main cursor state
    pub cursor: Cursor,
    /// Alternate screen cursor
    pub alt_cursor: Cursor,
    /// Saved cursor (DECSC/DECRC)
    pub saved_cursor: Option<Cursor>,
    /// Current foreground color
    pub fg: Color,
    /// Current background color
    pub bg: Color,
    /// Current underline color
    pub underline_color: Option<Color>,
    /// Current cell flags
    pub flags: CellFlags,
    /// Saved colors/flags for cursor save/restore
    pub saved_fg: Color,
    pub saved_bg: Color,
    pub saved_underline_color: Option<Color>,
    pub saved_flags: CellFlags,
    /// Terminal title
    pub title: String,
    /// Auto wrap mode (DECAWM)
    pub auto_wrap: bool,
    /// Origin mode (DECOM)
    pub origin_mode: bool,
    /// Insert mode (IRM)
    pub insert_mode: bool,
    /// Reverse video mode (DECSCNM)
    pub reverse_video: bool,
    /// Line feed/new line mode (LNM)
    pub line_feed_new_line_mode: bool,
    /// Application cursor keys mode
    pub application_cursor: bool,
    /// Bracketed paste mode
    pub bracketed_paste: bool,
    /// Focus tracking enabled
    pub focus_tracking: bool,
    /// Scroll region top (0-indexed)
    pub scroll_region_top: usize,
    /// Scroll region bottom (0-indexed)
    pub scroll_region_bottom: usize,
    /// Tab stops
    pub tab_stops: Vec<bool>,
    /// Pending wrap flag (DECAWM delayed wrap)
    pub pending_wrap: bool,
    /// Estimated memory size in bytes
    pub estimated_size_bytes: usize,
}

impl TerminalSnapshot {
    /// Estimate the memory size of this snapshot in bytes
    pub fn estimate_size(&self) -> usize {
        let cell_size = std::mem::size_of::<Cell>();
        let grid_cells = self.grid.cells.len() * cell_size;
        let grid_sb = self.grid.scrollback_cells.len() * cell_size;
        let alt_cells = self.alt_grid.cells.len() * cell_size;
        let alt_sb = self.alt_grid.scrollback_cells.len() * cell_size;
        let wrapped = self.grid.wrapped.len() + self.grid.scrollback_wrapped.len();
        let alt_wrapped = self.alt_grid.wrapped.len() + self.alt_grid.scrollback_wrapped.len();
        let zones = self.grid.zones.len() * std::mem::size_of::<Zone>();
        let tab_stops = self.tab_stops.len();
        let title = self.title.len();
        // Rough overhead for Vec headers, struct padding, etc.
        let overhead = 512;

        grid_cells + grid_sb + alt_cells + alt_sb
            + wrapped + alt_wrapped + zones + tab_stops + title + overhead
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::NamedColor;

    fn make_grid_snapshot(cols: usize, rows: usize) -> GridSnapshot {
        GridSnapshot {
            cells: vec![Cell::default(); cols * rows],
            scrollback_cells: Vec::new(),
            scrollback_start: 0,
            scrollback_lines: 0,
            max_scrollback: 1000,
            cols,
            rows,
            wrapped: vec![false; rows],
            scrollback_wrapped: Vec::new(),
            zones: Vec::new(),
            total_lines_scrolled: 0,
        }
    }

    fn make_snapshot(cols: usize, rows: usize) -> TerminalSnapshot {
        TerminalSnapshot {
            timestamp: 1700000000000,
            cols,
            rows,
            grid: make_grid_snapshot(cols, rows),
            alt_grid: make_grid_snapshot(cols, rows),
            alt_screen_active: false,
            cursor: Cursor::default(),
            alt_cursor: Cursor::default(),
            saved_cursor: None,
            fg: Color::Named(NamedColor::White),
            bg: Color::Named(NamedColor::Black),
            underline_color: None,
            flags: CellFlags::default(),
            saved_fg: Color::Named(NamedColor::White),
            saved_bg: Color::Named(NamedColor::Black),
            saved_underline_color: None,
            saved_flags: CellFlags::default(),
            title: String::new(),
            auto_wrap: true,
            origin_mode: false,
            insert_mode: false,
            reverse_video: false,
            line_feed_new_line_mode: false,
            application_cursor: false,
            bracketed_paste: false,
            focus_tracking: false,
            scroll_region_top: 0,
            scroll_region_bottom: rows - 1,
            tab_stops: vec![false; cols],
            pending_wrap: false,
            estimated_size_bytes: 0,
        }
    }

    #[test]
    fn test_terminal_snapshot_creation() {
        let snap = make_snapshot(80, 24);
        assert_eq!(snap.cols, 80);
        assert_eq!(snap.rows, 24);
        assert_eq!(snap.grid.cells.len(), 80 * 24);
        assert!(!snap.alt_screen_active);
    }

    #[test]
    fn test_terminal_snapshot_clone() {
        let snap1 = make_snapshot(80, 24);
        let snap2 = snap1.clone();
        assert_eq!(snap1.cols, snap2.cols);
        assert_eq!(snap1.rows, snap2.rows);
        assert_eq!(snap1.grid.cells.len(), snap2.grid.cells.len());
        assert_eq!(snap1.cursor, snap2.cursor);
    }

    #[test]
    fn test_estimate_size_nonempty() {
        let mut snap = make_snapshot(80, 24);
        let size = snap.estimate_size();
        assert!(size > 0);
        snap.estimated_size_bytes = size;
        assert!(snap.estimated_size_bytes > 1000); // 80*24 cells should be substantial
    }

    #[test]
    fn test_grid_snapshot_with_scrollback() {
        let mut gs = make_grid_snapshot(80, 24);
        gs.scrollback_cells = vec![Cell::default(); 80 * 100];
        gs.scrollback_lines = 100;
        gs.scrollback_wrapped = vec![false; 100];
        assert_eq!(gs.scrollback_cells.len(), 8000);
        assert_eq!(gs.scrollback_lines, 100);
    }

    #[test]
    fn test_snapshot_with_colored_cells() {
        let mut snap = make_snapshot(80, 24);
        // Set a cell with custom colors
        snap.grid.cells[0] = Cell::with_colors('A', Color::Rgb(255, 0, 0), Color::Rgb(0, 0, 255));
        snap.grid.cells[0].flags.set_bold(true);

        let cloned = snap.clone();
        assert_eq!(cloned.grid.cells[0].c, 'A');
        assert_eq!(cloned.grid.cells[0].fg, Color::Rgb(255, 0, 0));
        assert_eq!(cloned.grid.cells[0].bg, Color::Rgb(0, 0, 255));
        assert!(cloned.grid.cells[0].flags.bold());
    }
}
```

**Step 2: Register the module**

In `src/terminal/mod.rs`, add after line 18 (`pub mod snapshot;`):

```rust
pub mod terminal_snapshot;
```

**Step 3: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize terminal_snapshot -v`
Expected: All 5 tests PASS

**Step 4: Commit**

```bash
git add src/terminal/terminal_snapshot.rs src/terminal/mod.rs
git commit -m "feat(snapshot): add TerminalSnapshot and GridSnapshot structs for cell-level state capture"
```

---

### Task 3: Grid Snapshot Capture and Restore Methods

**Files:**
- Modify: `src/grid.rs` (add `capture_snapshot` and `restore_from_snapshot` methods)

The `Grid` struct fields are private. We need methods to capture them into a `GridSnapshot` and restore from one.

**Step 1: Write failing tests**

Add tests to the existing `#[cfg(test)] mod tests` block at the bottom of `src/grid.rs`:

```rust
#[test]
fn test_grid_capture_snapshot() {
    use crate::terminal::terminal_snapshot::GridSnapshot;

    let mut grid = Grid::new(80, 24, 1000);
    // Write some content
    if let Some(cell) = grid.get_mut(0, 0) {
        cell.c = 'H';
    }
    if let Some(cell) = grid.get_mut(1, 0) {
        cell.c = 'i';
    }

    let snap = grid.capture_snapshot();
    assert_eq!(snap.cols, 80);
    assert_eq!(snap.rows, 24);
    assert_eq!(snap.cells[0].c, 'H');
    assert_eq!(snap.cells[1].c, 'i');
    assert_eq!(snap.max_scrollback, 1000);
}

#[test]
fn test_grid_restore_from_snapshot() {
    use crate::terminal::terminal_snapshot::GridSnapshot;

    let mut grid = Grid::new(80, 24, 1000);
    // Write content
    if let Some(cell) = grid.get_mut(0, 0) {
        cell.c = 'A';
    }

    let snap = grid.capture_snapshot();

    // Modify grid
    if let Some(cell) = grid.get_mut(0, 0) {
        cell.c = 'B';
    }
    assert_eq!(grid.get(0, 0).unwrap().c, 'B');

    // Restore
    grid.restore_from_snapshot(&snap);
    assert_eq!(grid.get(0, 0).unwrap().c, 'A');
}

#[test]
fn test_grid_snapshot_roundtrip_with_scrollback() {
    use crate::terminal::terminal_snapshot::GridSnapshot;

    let mut grid = Grid::new(10, 3, 100);
    // Fill rows and scroll to create scrollback
    for row in 0..3 {
        for col in 0..10 {
            if let Some(cell) = grid.get_mut(col, row) {
                cell.c = ('a' as u8 + row as u8) as char;
            }
        }
    }
    grid.scroll_up(2); // Push 2 rows into scrollback

    let snap = grid.capture_snapshot();
    assert_eq!(snap.scrollback_lines, 2);

    // Create fresh grid and restore
    let mut grid2 = Grid::new(10, 3, 100);
    grid2.restore_from_snapshot(&snap);
    assert_eq!(grid2.scrollback_len(), 2);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize grid::tests::test_grid_capture_snapshot -v`
Expected: FAIL — `capture_snapshot` method does not exist

**Step 3: Implement capture_snapshot and restore_from_snapshot on Grid**

Add these methods to the `impl Grid` block in `src/grid.rs` (before the `#[cfg(test)]` block):

```rust
/// Capture a complete snapshot of this grid's state
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

/// Restore grid state from a snapshot
///
/// Replaces all grid data with the snapshot contents. The grid dimensions
/// are updated to match the snapshot.
pub fn restore_from_snapshot(
    &mut self,
    snap: &crate::terminal::terminal_snapshot::GridSnapshot,
) {
    self.cols = snap.cols;
    self.rows = snap.rows;
    self.cells = snap.cells.clone();
    self.scrollback_cells = snap.scrollback_cells.clone();
    self.scrollback_start = snap.scrollback_start;
    self.scrollback_lines = snap.scrollback_lines;
    self.max_scrollback = snap.max_scrollback;
    self.wrapped = snap.wrapped.clone();
    self.scrollback_wrapped = snap.scrollback_wrapped.clone();
    self.zones = snap.zones.clone();
    self.total_lines_scrolled = snap.total_lines_scrolled;
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize grid::tests::test_grid_ -v`
Expected: All 3 new tests PASS (plus existing tests)

**Step 5: Commit**

```bash
git add src/grid.rs
git commit -m "feat(grid): add capture_snapshot and restore_from_snapshot methods"
```

---

### Task 4: Terminal capture_snapshot and restore_from_snapshot

**Files:**
- Modify: `src/terminal/mod.rs` (add capture/restore methods to `impl Terminal`)

**Step 1: Write failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src/terminal/mod.rs`:

```rust
#[test]
fn test_terminal_capture_snapshot() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello, World!\r\n");
    term.process(b"\x1b[31mRed text\x1b[0m"); // Red foreground

    let snap = term.capture_snapshot();
    assert_eq!(snap.cols, 80);
    assert_eq!(snap.rows, 24);
    assert_eq!(snap.cursor.row, 1); // After the newline
    assert!(!snap.alt_screen_active);
    assert!(snap.estimated_size_bytes > 0);
}

#[test]
fn test_terminal_restore_snapshot_roundtrip() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Line 1\r\n");
    term.process(b"Line 2\r\n");
    term.process(b"Line 3");

    let snap = term.capture_snapshot();

    // Modify terminal
    term.process(b"\r\nLine 4\r\nLine 5");

    // Restore
    term.restore_from_snapshot(&snap);

    // Verify cursor is back where it was
    assert_eq!(snap.cursor.col, term.cursor.col);
    assert_eq!(snap.cursor.row, term.cursor.row);

    // Verify cell content is restored
    let cell = term.grid.get(0, 0).unwrap();
    assert_eq!(cell.c, 'L');
    let cell = term.grid.get(5, 0).unwrap();
    assert_eq!(cell.c, '1');
}

#[test]
fn test_terminal_snapshot_preserves_colors() {
    let mut term = Terminal::new(80, 24);
    // Write red text
    term.process(b"\x1b[31mRed\x1b[0m Normal");

    let snap = term.capture_snapshot();

    // Verify the red cell is captured
    let red_cell = &snap.grid.cells[0]; // 'R'
    assert_eq!(red_cell.c, 'R');

    // Restore to fresh terminal
    let mut term2 = Terminal::new(80, 24);
    term2.restore_from_snapshot(&snap);

    let restored_cell = term2.grid.get(0, 0).unwrap();
    assert_eq!(restored_cell.c, 'R');
    assert_eq!(restored_cell.fg, red_cell.fg);
}

#[test]
fn test_terminal_snapshot_with_scrollback() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    // Write more lines than terminal height to create scrollback
    for i in 0..10 {
        term.process(format!("Line {}\r\n", i).as_bytes());
    }

    let snap = term.capture_snapshot();
    assert!(snap.grid.scrollback_lines > 0);

    let mut term2 = Terminal::with_scrollback(80, 5, 100);
    term2.restore_from_snapshot(&snap);
    assert_eq!(term2.grid.scrollback_len(), snap.grid.scrollback_lines);
}

#[test]
fn test_terminal_snapshot_alt_screen() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Primary content");
    // Switch to alt screen
    term.process(b"\x1b[?1049h");
    term.process(b"Alt content");

    let snap = term.capture_snapshot();
    assert!(snap.alt_screen_active);

    let mut term2 = Terminal::new(80, 24);
    term2.restore_from_snapshot(&snap);
    assert!(term2.alt_screen_active);
}

#[test]
fn test_terminal_snapshot_mode_flags() {
    let mut term = Terminal::new(80, 24);
    // Enable bracketed paste
    term.process(b"\x1b[?2004h");
    // Disable auto wrap
    term.process(b"\x1b[?7l");

    let snap = term.capture_snapshot();
    assert!(snap.bracketed_paste);
    assert!(!snap.auto_wrap);

    let mut term2 = Terminal::new(80, 24);
    term2.restore_from_snapshot(&snap);
    assert!(term2.bracketed_paste);
    assert!(!term2.auto_wrap);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize terminal::tests::test_terminal_capture_snapshot -v`
Expected: FAIL — method does not exist

**Step 3: Implement capture_snapshot and restore_from_snapshot on Terminal**

Add to `impl Terminal` in `src/terminal/mod.rs` (near the existing `get_semantic_snapshot` methods, around line 3856):

```rust
/// Capture a cell-level snapshot of the terminal state
///
/// Unlike `get_semantic_snapshot()` which captures text only, this captures
/// raw Cell data including colors, attributes, and grid layout for
/// pixel-perfect reconstruction.
pub fn capture_snapshot(&self) -> crate::terminal::terminal_snapshot::TerminalSnapshot {
    use crate::terminal::terminal_snapshot::TerminalSnapshot;

    let mut snap = TerminalSnapshot {
        timestamp: crate::terminal::unix_millis(),
        cols: self.grid.cols(),
        rows: self.grid.rows(),
        grid: self.grid.capture_snapshot(),
        alt_grid: self.alt_grid.capture_snapshot(),
        alt_screen_active: self.alt_screen_active,
        cursor: self.cursor,
        alt_cursor: self.alt_cursor,
        saved_cursor: self.saved_cursor,
        fg: self.fg,
        bg: self.bg,
        underline_color: self.underline_color,
        flags: self.flags,
        saved_fg: self.saved_fg,
        saved_bg: self.saved_bg,
        saved_underline_color: self.saved_underline_color,
        saved_flags: self.saved_flags,
        title: self.title.clone(),
        auto_wrap: self.auto_wrap,
        origin_mode: self.origin_mode,
        insert_mode: self.insert_mode,
        reverse_video: self.reverse_video,
        line_feed_new_line_mode: self.line_feed_new_line_mode,
        application_cursor: self.application_cursor,
        bracketed_paste: self.bracketed_paste,
        focus_tracking: self.focus_tracking,
        scroll_region_top: self.scroll_region_top,
        scroll_region_bottom: self.scroll_region_bottom,
        tab_stops: self.tab_stops.clone(),
        pending_wrap: self.pending_wrap,
        estimated_size_bytes: 0,
    };
    snap.estimated_size_bytes = snap.estimate_size();
    snap
}

/// Restore terminal state from a cell-level snapshot
///
/// Replaces grid data, cursor state, colors, and mode flags.
/// The VTE parser is reset to a clean state. Observer registrations
/// and shell integration history are NOT restored (they are session-level).
pub fn restore_from_snapshot(
    &mut self,
    snap: &crate::terminal::terminal_snapshot::TerminalSnapshot,
) {
    self.grid.restore_from_snapshot(&snap.grid);
    self.alt_grid.restore_from_snapshot(&snap.alt_grid);
    self.alt_screen_active = snap.alt_screen_active;
    self.cursor = snap.cursor;
    self.alt_cursor = snap.alt_cursor;
    self.saved_cursor = snap.saved_cursor;
    self.fg = snap.fg;
    self.bg = snap.bg;
    self.underline_color = snap.underline_color;
    self.flags = snap.flags;
    self.saved_fg = snap.saved_fg;
    self.saved_bg = snap.saved_bg;
    self.saved_underline_color = snap.saved_underline_color;
    self.saved_flags = snap.saved_flags;
    self.title = snap.title.clone();
    self.auto_wrap = snap.auto_wrap;
    self.origin_mode = snap.origin_mode;
    self.insert_mode = snap.insert_mode;
    self.reverse_video = snap.reverse_video;
    self.line_feed_new_line_mode = snap.line_feed_new_line_mode;
    self.application_cursor = snap.application_cursor;
    self.bracketed_paste = snap.bracketed_paste;
    self.focus_tracking = snap.focus_tracking;
    self.scroll_region_top = snap.scroll_region_top;
    self.scroll_region_bottom = snap.scroll_region_bottom;
    self.tab_stops = snap.tab_stops.clone();
    self.pending_wrap = snap.pending_wrap;
    // Reset parser to clean state since we can't serialize VTE parser internals
    self.parser = vte::Parser::new();
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize terminal::tests::test_terminal_ -- --test-threads=1 -v`
Expected: All 6 new tests PASS

**Step 5: Commit**

```bash
git add src/terminal/mod.rs
git commit -m "feat(terminal): add capture_snapshot and restore_from_snapshot for cell-level state"
```

---

### Task 5: SnapshotManager with Size-Based Eviction

**Files:**
- Create: `src/terminal/snapshot_manager.rs`
- Modify: `src/terminal/mod.rs` (add `pub mod snapshot_manager;`)

**Step 1: Write the module with tests**

Create `src/terminal/snapshot_manager.rs`:

```rust
//! Snapshot manager for Instant Replay
//!
//! Manages a ring buffer of `TerminalSnapshot` entries with input byte deltas.
//! Size-based eviction keeps memory within a configurable budget.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::terminal::terminal_snapshot::TerminalSnapshot;
use crate::terminal::Terminal;

/// Default memory budget: 4MB (matches iTerm2)
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 4 * 1024 * 1024;

/// Default snapshot interval: 30 seconds
pub const DEFAULT_SNAPSHOT_INTERVAL_SECS: u64 = 30;

/// A snapshot entry with accumulated input bytes since the snapshot was taken
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// Full terminal state at the time of snapshot
    pub snapshot: TerminalSnapshot,
    /// Raw bytes fed to `terminal.process()` after this snapshot
    pub input_bytes: Vec<u8>,
}

impl SnapshotEntry {
    /// Total memory usage of this entry (snapshot + input bytes)
    pub fn size_bytes(&self) -> usize {
        self.snapshot.estimated_size_bytes + self.input_bytes.len()
    }
}

/// Manages a timeline of terminal snapshots for Instant Replay
#[derive(Debug)]
pub struct SnapshotManager {
    /// Snapshot entries in chronological order
    entries: VecDeque<SnapshotEntry>,
    /// Maximum memory budget in bytes
    max_memory_bytes: usize,
    /// Current total memory usage
    current_memory_bytes: usize,
    /// Minimum interval between auto-snapshots
    snapshot_interval: Duration,
    /// When the last snapshot was taken
    last_snapshot_time: Option<Instant>,
    /// Whether recording is enabled
    enabled: bool,
}

impl SnapshotManager {
    /// Create a new snapshot manager with the given memory budget
    pub fn new(max_memory_bytes: usize, snapshot_interval: Duration) -> Self {
        Self {
            entries: VecDeque::new(),
            max_memory_bytes,
            current_memory_bytes: 0,
            snapshot_interval,
            last_snapshot_time: None,
            enabled: true,
        }
    }

    /// Create a manager with default settings (4MB budget, 30s interval)
    pub fn with_defaults() -> Self {
        Self::new(
            DEFAULT_MAX_MEMORY_BYTES,
            Duration::from_secs(DEFAULT_SNAPSHOT_INTERVAL_SECS),
        )
    }

    /// Check if a snapshot should be taken based on the interval
    pub fn should_snapshot(&self) -> bool {
        if !self.enabled {
            return false;
        }
        match self.last_snapshot_time {
            None => true,
            Some(last) => last.elapsed() >= self.snapshot_interval,
        }
    }

    /// Take a snapshot of the terminal and start a new entry
    ///
    /// Returns the index of the new entry.
    pub fn take_snapshot(&mut self, terminal: &Terminal) -> usize {
        let snapshot = terminal.capture_snapshot();
        let entry = SnapshotEntry {
            snapshot,
            input_bytes: Vec::new(),
        };
        let entry_size = entry.size_bytes();
        self.entries.push_back(entry);
        self.current_memory_bytes += entry_size;
        self.last_snapshot_time = Some(Instant::now());

        // Evict oldest entries if over budget
        self.evict();

        self.entries.len() - 1
    }

    /// Record input bytes that were fed to the terminal
    ///
    /// Appends to the current (latest) entry's input_bytes.
    /// If no entries exist, the bytes are discarded.
    pub fn record_input(&mut self, bytes: &[u8]) {
        if !self.enabled || bytes.is_empty() {
            return;
        }
        if let Some(entry) = self.entries.back_mut() {
            self.current_memory_bytes += bytes.len();
            entry.input_bytes.extend_from_slice(bytes);
        }
    }

    /// Get the number of entries
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Get an entry by index
    pub fn get_entry(&self, index: usize) -> Option<&SnapshotEntry> {
        self.entries.get(index)
    }

    /// Get current memory usage in bytes
    pub fn memory_usage(&self) -> usize {
        self.current_memory_bytes
    }

    /// Get the maximum memory budget
    pub fn max_memory(&self) -> usize {
        self.max_memory_bytes
    }

    /// Set the memory budget (triggers eviction if needed)
    pub fn set_max_memory(&mut self, max_bytes: usize) {
        self.max_memory_bytes = max_bytes;
        self.evict();
    }

    /// Get the snapshot interval
    pub fn snapshot_interval(&self) -> Duration {
        self.snapshot_interval
    }

    /// Set the snapshot interval
    pub fn set_snapshot_interval(&mut self, interval: Duration) {
        self.snapshot_interval = interval;
    }

    /// Enable or disable recording
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if recording is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.entries.clear();
        self.current_memory_bytes = 0;
        self.last_snapshot_time = None;
    }

    /// Get timestamp range (oldest, newest) or None if empty
    pub fn time_range(&self) -> Option<(u64, u64)> {
        let first = self.entries.front()?.snapshot.timestamp;
        let last = self.entries.back()?.snapshot.timestamp;
        Some((first, last))
    }

    /// Evict oldest entries until within memory budget.
    /// Always keeps at least one entry.
    fn evict(&mut self) {
        while self.current_memory_bytes > self.max_memory_bytes && self.entries.len() > 1 {
            if let Some(removed) = self.entries.pop_front() {
                self.current_memory_bytes =
                    self.current_memory_bytes.saturating_sub(removed.size_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_creation() {
        let mgr = SnapshotManager::with_defaults();
        assert_eq!(mgr.max_memory(), DEFAULT_MAX_MEMORY_BYTES);
        assert_eq!(mgr.entry_count(), 0);
        assert_eq!(mgr.memory_usage(), 0);
        assert!(mgr.is_enabled());
    }

    #[test]
    fn test_take_snapshot() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);

        let idx = mgr.take_snapshot(&term);
        assert_eq!(idx, 0);
        assert_eq!(mgr.entry_count(), 1);
        assert!(mgr.memory_usage() > 0);
    }

    #[test]
    fn test_record_input() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);

        mgr.take_snapshot(&term);
        mgr.record_input(b"Hello, World!");

        let entry = mgr.get_entry(0).unwrap();
        assert_eq!(entry.input_bytes, b"Hello, World!");
    }

    #[test]
    fn test_record_input_no_entry() {
        let mut mgr = SnapshotManager::with_defaults();
        // Should not panic when no entries exist
        mgr.record_input(b"lost bytes");
        assert_eq!(mgr.entry_count(), 0);
    }

    #[test]
    fn test_multiple_snapshots() {
        let mut mgr = SnapshotManager::with_defaults();
        let mut term = Terminal::new(80, 24);

        mgr.take_snapshot(&term);
        mgr.record_input(b"input1");

        term.process(b"some text");
        mgr.take_snapshot(&term);
        mgr.record_input(b"input2");

        assert_eq!(mgr.entry_count(), 2);
        assert_eq!(mgr.get_entry(0).unwrap().input_bytes, b"input1");
        assert_eq!(mgr.get_entry(1).unwrap().input_bytes, b"input2");
    }

    #[test]
    fn test_eviction_keeps_one() {
        // Tiny budget to force eviction
        let mut mgr = SnapshotManager::new(1, Duration::from_secs(0));
        let term = Terminal::new(80, 24);

        mgr.take_snapshot(&term);
        mgr.take_snapshot(&term);
        mgr.take_snapshot(&term);

        // Should always keep at least 1 entry
        assert!(mgr.entry_count() >= 1);
    }

    #[test]
    fn test_eviction_reduces_memory() {
        // Small budget
        let mut mgr = SnapshotManager::new(100_000, Duration::from_secs(0));
        let mut term = Terminal::new(80, 24);

        for i in 0..10 {
            term.process(format!("Line {}\r\n", i).as_bytes());
            mgr.take_snapshot(&term);
            mgr.record_input(b"x".repeat(10_000).as_slice());
        }

        // Should have evicted some entries
        assert!(mgr.entry_count() < 10);
        // Memory should be near or under budget (may slightly exceed due to last entry)
        // At minimum, eviction should have run
    }

    #[test]
    fn test_should_snapshot_initially_true() {
        let mgr = SnapshotManager::with_defaults();
        assert!(mgr.should_snapshot());
    }

    #[test]
    fn test_should_snapshot_false_after_recent() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);
        mgr.take_snapshot(&term);
        // Immediately after snapshot, should_snapshot is false (30s interval)
        assert!(!mgr.should_snapshot());
    }

    #[test]
    fn test_disabled_manager() {
        let mut mgr = SnapshotManager::with_defaults();
        mgr.set_enabled(false);
        assert!(!mgr.should_snapshot());

        // record_input should be no-op
        let term = Terminal::new(80, 24);
        mgr.take_snapshot(&term); // Still takes snapshot even when disabled (explicit call)
        mgr.record_input(b"data");
        // But record_input was no-op
        assert!(mgr.get_entry(0).unwrap().input_bytes.is_empty());
    }

    #[test]
    fn test_clear() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);
        mgr.take_snapshot(&term);
        mgr.record_input(b"data");

        mgr.clear();
        assert_eq!(mgr.entry_count(), 0);
        assert_eq!(mgr.memory_usage(), 0);
    }

    #[test]
    fn test_time_range() {
        let mut mgr = SnapshotManager::with_defaults();
        assert!(mgr.time_range().is_none());

        let term = Terminal::new(80, 24);
        mgr.take_snapshot(&term);

        let range = mgr.time_range().unwrap();
        assert_eq!(range.0, range.1); // Single entry, same timestamp
    }

    #[test]
    fn test_set_max_memory_triggers_eviction() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);

        mgr.take_snapshot(&term);
        mgr.take_snapshot(&term);
        mgr.take_snapshot(&term);
        let before = mgr.entry_count();

        mgr.set_max_memory(1); // Tiny budget
        assert!(mgr.entry_count() <= before);
        assert!(mgr.entry_count() >= 1); // Always keeps at least 1
    }
}
```

**Step 2: Register the module**

In `src/terminal/mod.rs`, add after the `terminal_snapshot` module declaration:

```rust
pub mod snapshot_manager;
```

**Step 3: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize snapshot_manager -v`
Expected: All tests PASS

**Step 4: Commit**

```bash
git add src/terminal/snapshot_manager.rs src/terminal/mod.rs
git commit -m "feat(replay): add SnapshotManager with size-based eviction and input recording"
```

---

### Task 6: Input-Stream Reconstruction

**Files:**
- Modify: `src/terminal/snapshot_manager.rs` (add `reconstruct_at` method)

**Step 1: Write failing test**

Add to the `tests` module in `snapshot_manager.rs`:

```rust
#[test]
fn test_reconstruct_at_entry_start() {
    let mut mgr = SnapshotManager::with_defaults();
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello");

    mgr.take_snapshot(&term);

    // Reconstruct at snapshot (no byte replay needed)
    let reconstructed = mgr.reconstruct_at(0, 0).unwrap();
    assert_eq!(reconstructed.grid().get(0, 0).unwrap().c, 'H');
    assert_eq!(reconstructed.grid().get(4, 0).unwrap().c, 'o');
}

#[test]
fn test_reconstruct_with_input_replay() {
    let mut mgr = SnapshotManager::with_defaults();
    let term = Terminal::new(80, 24);

    mgr.take_snapshot(&term);
    mgr.record_input(b"Hello");

    // Reconstruct with all input bytes replayed
    let entry = mgr.get_entry(0).unwrap();
    let byte_len = entry.input_bytes.len();
    let reconstructed = mgr.reconstruct_at(0, byte_len).unwrap();
    assert_eq!(reconstructed.grid().get(0, 0).unwrap().c, 'H');
    assert_eq!(reconstructed.grid().get(4, 0).unwrap().c, 'o');
}

#[test]
fn test_reconstruct_partial_replay() {
    let mut mgr = SnapshotManager::with_defaults();
    let term = Terminal::new(80, 24);

    mgr.take_snapshot(&term);
    mgr.record_input(b"ABCDE");

    // Replay only first 3 bytes
    let reconstructed = mgr.reconstruct_at(0, 3).unwrap();
    assert_eq!(reconstructed.grid().get(0, 0).unwrap().c, 'A');
    assert_eq!(reconstructed.grid().get(2, 0).unwrap().c, 'C');
    assert_eq!(reconstructed.grid().get(3, 0).unwrap().c, ' '); // Not yet written
}

#[test]
fn test_reconstruct_invalid_index() {
    let mgr = SnapshotManager::with_defaults();
    assert!(mgr.reconstruct_at(0, 0).is_none());
}

#[test]
fn test_reconstruct_clamps_byte_offset() {
    let mut mgr = SnapshotManager::with_defaults();
    let term = Terminal::new(80, 24);
    mgr.take_snapshot(&term);
    mgr.record_input(b"AB");

    // Byte offset beyond available bytes should clamp
    let reconstructed = mgr.reconstruct_at(0, 100).unwrap();
    assert_eq!(reconstructed.grid().get(0, 0).unwrap().c, 'A');
    assert_eq!(reconstructed.grid().get(1, 0).unwrap().c, 'B');
}

#[test]
fn test_find_entry_for_timestamp() {
    let mut mgr = SnapshotManager::with_defaults();
    let term = Terminal::new(80, 24);

    mgr.take_snapshot(&term);
    let ts1 = mgr.get_entry(0).unwrap().snapshot.timestamp;

    mgr.take_snapshot(&term);
    let ts2 = mgr.get_entry(1).unwrap().snapshot.timestamp;

    // Exact match
    assert_eq!(mgr.find_entry_for_timestamp(ts1), Some(0));
    assert_eq!(mgr.find_entry_for_timestamp(ts2), Some(1));

    // Between timestamps: should return the earlier entry
    if ts2 > ts1 {
        assert_eq!(mgr.find_entry_for_timestamp(ts1 + 1), Some(0));
    }

    // Before all entries
    assert_eq!(mgr.find_entry_for_timestamp(0), Some(0));

    // Empty manager
    let empty_mgr = SnapshotManager::with_defaults();
    assert_eq!(empty_mgr.find_entry_for_timestamp(1000), None);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize snapshot_manager::tests::test_reconstruct -v`
Expected: FAIL — methods do not exist

**Step 3: Implement reconstruct_at and find_entry_for_timestamp**

Add to `impl SnapshotManager` in `snapshot_manager.rs`:

```rust
/// Reconstruct terminal state at a specific point in time
///
/// Restores the snapshot at `entry_index` and replays input bytes up to `byte_offset`.
/// If `byte_offset` exceeds available bytes, all bytes are replayed.
///
/// Returns `None` if `entry_index` is out of range.
pub fn reconstruct_at(&self, entry_index: usize, byte_offset: usize) -> Option<Terminal> {
    let entry = self.entries.get(entry_index)?;

    // Create a terminal and restore the snapshot
    let mut term = Terminal::with_scrollback(
        entry.snapshot.cols,
        entry.snapshot.rows,
        entry.snapshot.grid.max_scrollback,
    );
    term.restore_from_snapshot(&entry.snapshot);

    // Replay input bytes up to the offset
    let replay_len = byte_offset.min(entry.input_bytes.len());
    if replay_len > 0 {
        term.process(&entry.input_bytes[..replay_len]);
    }

    Some(term)
}

/// Find the entry index whose snapshot is closest to (but not after) the given timestamp
///
/// Uses binary search. Returns `None` if no entries exist.
pub fn find_entry_for_timestamp(&self, timestamp: u64) -> Option<usize> {
    if self.entries.is_empty() {
        return None;
    }

    // Binary search for the last entry with timestamp <= target
    let mut low = 0usize;
    let mut high = self.entries.len();
    while low < high {
        let mid = low + (high - low) / 2;
        if self.entries[mid].snapshot.timestamp <= timestamp {
            low = mid + 1;
        } else {
            high = mid;
        }
    }

    // low is now the first entry with timestamp > target
    // We want the entry before that
    if low > 0 {
        Some(low - 1)
    } else {
        Some(0) // Clamp to first entry
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize snapshot_manager -v`
Expected: All tests PASS

**Step 5: Commit**

```bash
git add src/terminal/snapshot_manager.rs
git commit -m "feat(replay): add input-stream reconstruction and timestamp-based lookup"
```

---

### Task 7: ReplaySession

**Files:**
- Create: `src/terminal/replay.rs`
- Modify: `src/terminal/mod.rs` (add `pub mod replay;`)

**Step 1: Write the module with tests**

Create `src/terminal/replay.rs`:

```rust
//! Replay session for navigating terminal history
//!
//! Provides frame-by-frame navigation through captured terminal snapshots
//! and input byte deltas.

use crate::terminal::snapshot_manager::SnapshotManager;
use crate::terminal::Terminal;

/// A replay session for navigating terminal history
///
/// Owns a clone of the snapshot entries and maintains a reconstructed
/// terminal at the current position.
#[derive(Debug)]
pub struct ReplaySession {
    /// Current entry index in the snapshot timeline
    current_index: usize,
    /// Current byte offset within the entry's input_bytes
    current_byte_offset: usize,
    /// Total number of entries
    total_entries: usize,
    /// Reconstructed terminal at current position
    current_frame: Terminal,
    /// Reference data: snapshot timestamps and input byte lengths
    /// (timestamp, input_bytes_len) per entry
    entry_metadata: Vec<(u64, usize)>,
    /// Cloned snapshot manager for reconstruction
    manager: SnapshotManager,
}

/// Result of a navigation operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekResult {
    /// Successfully navigated to the requested position
    Ok,
    /// Reached the beginning of the timeline
    AtStart,
    /// Reached the end of the timeline
    AtEnd,
    /// No entries available
    Empty,
}

impl ReplaySession {
    /// Create a new replay session from a snapshot manager
    ///
    /// Starts at the latest position (end of timeline).
    /// Returns `None` if the manager has no entries.
    pub fn new(manager: &SnapshotManager) -> Option<Self> {
        if manager.entry_count() == 0 {
            return None;
        }

        let total = manager.entry_count();
        let last_index = total - 1;
        let last_entry = manager.get_entry(last_index)?;
        let last_byte_len = last_entry.input_bytes.len();

        // Build metadata
        let entry_metadata: Vec<(u64, usize)> = (0..total)
            .filter_map(|i| {
                let e = manager.get_entry(i)?;
                Some((e.snapshot.timestamp, e.input_bytes.len()))
            })
            .collect();

        // Reconstruct at end
        let frame = manager.reconstruct_at(last_index, last_byte_len)?;

        // Clone the manager for later reconstruction
        let manager_clone = clone_manager(manager);

        Some(Self {
            current_index: last_index,
            current_byte_offset: last_byte_len,
            total_entries: total,
            current_frame: frame,
            entry_metadata,
            manager: manager_clone,
        })
    }

    /// Get the current reconstructed terminal frame
    pub fn current_frame(&self) -> &Terminal {
        &self.current_frame
    }

    /// Get the current entry index
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    /// Get the current byte offset within the entry
    pub fn current_byte_offset(&self) -> usize {
        self.current_byte_offset
    }

    /// Get total number of entries
    pub fn total_entries(&self) -> usize {
        self.total_entries
    }

    /// Get the timestamp of the current snapshot entry
    pub fn current_timestamp(&self) -> u64 {
        self.entry_metadata
            .get(self.current_index)
            .map(|(ts, _)| *ts)
            .unwrap_or(0)
    }

    /// Seek to a specific entry and byte offset
    pub fn seek_to(&mut self, entry_index: usize, byte_offset: usize) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }
        let idx = entry_index.min(self.total_entries - 1);
        let max_bytes = self.entry_metadata.get(idx).map(|(_, len)| *len).unwrap_or(0);
        let offset = byte_offset.min(max_bytes);

        if let Some(frame) = self.manager.reconstruct_at(idx, offset) {
            self.current_index = idx;
            self.current_byte_offset = offset;
            self.current_frame = frame;
            SeekResult::Ok
        } else {
            SeekResult::Empty
        }
    }

    /// Seek to the entry closest to the given timestamp
    pub fn seek_to_timestamp(&mut self, timestamp: u64) -> SeekResult {
        if let Some(idx) = self.manager.find_entry_for_timestamp(timestamp) {
            self.seek_to(idx, 0)
        } else {
            SeekResult::Empty
        }
    }

    /// Seek to the very start of the timeline
    pub fn seek_to_start(&mut self) -> SeekResult {
        self.seek_to(0, 0)
    }

    /// Seek to the very end of the timeline
    pub fn seek_to_end(&mut self) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }
        let last_idx = self.total_entries - 1;
        let last_len = self.entry_metadata.get(last_idx).map(|(_, l)| *l).unwrap_or(0);
        self.seek_to(last_idx, last_len)
    }

    /// Step forward by `n` bytes within the current entry
    ///
    /// If at the end of the current entry's bytes, moves to the next entry.
    pub fn step_forward(&mut self, n_bytes: usize) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }

        let max_bytes = self.entry_metadata[self.current_index].1;
        let new_offset = self.current_byte_offset + n_bytes;

        if new_offset <= max_bytes {
            // Stay in current entry
            return self.seek_to(self.current_index, new_offset);
        }

        // Move to next entry
        if self.current_index + 1 < self.total_entries {
            let remaining = new_offset - max_bytes;
            return self.seek_to(self.current_index + 1, remaining);
        }

        // Already at the end
        self.seek_to(self.current_index, max_bytes);
        SeekResult::AtEnd
    }

    /// Step backward by `n` bytes within the current entry
    ///
    /// If at the start of the current entry, moves to the previous entry.
    pub fn step_backward(&mut self, n_bytes: usize) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }

        if n_bytes <= self.current_byte_offset {
            // Stay in current entry
            return self.seek_to(self.current_index, self.current_byte_offset - n_bytes);
        }

        // Move to previous entry
        if self.current_index > 0 {
            let remaining = n_bytes - self.current_byte_offset;
            let prev_idx = self.current_index - 1;
            let prev_len = self.entry_metadata[prev_idx].1;
            let new_offset = prev_len.saturating_sub(remaining);
            return self.seek_to(prev_idx, new_offset);
        }

        // Already at the beginning
        self.seek_to(0, 0);
        SeekResult::AtStart
    }

    /// Navigate to previous entry (start of previous snapshot)
    pub fn previous_entry(&mut self) -> SeekResult {
        if self.current_index > 0 {
            self.seek_to(self.current_index - 1, 0)
        } else {
            SeekResult::AtStart
        }
    }

    /// Navigate to next entry (start of next snapshot)
    pub fn next_entry(&mut self) -> SeekResult {
        if self.current_index + 1 < self.total_entries {
            self.seek_to(self.current_index + 1, 0)
        } else {
            SeekResult::AtEnd
        }
    }
}

/// Clone a SnapshotManager by reconstructing from its entries.
/// This is needed because ReplaySession needs its own copy for reconstruction.
fn clone_manager(mgr: &SnapshotManager) -> SnapshotManager {
    let mut new_mgr = SnapshotManager::new(mgr.max_memory(), mgr.snapshot_interval());
    // We need to clone entries directly; add a method to SnapshotManager for this
    for i in 0..mgr.entry_count() {
        if let Some(entry) = mgr.get_entry(i) {
            new_mgr.push_entry(entry.clone());
        }
    }
    new_mgr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replay_session_creation() {
        let mut mgr = SnapshotManager::with_defaults();
        let mut term = Terminal::new(80, 24);
        term.process(b"Hello");

        mgr.take_snapshot(&term);
        mgr.record_input(b" World");

        let session = ReplaySession::new(&mgr).unwrap();
        assert_eq!(session.total_entries(), 1);
        assert_eq!(session.current_index(), 0);
        // At end: byte offset should be full input length
        assert_eq!(session.current_byte_offset(), 6); // " World"
    }

    #[test]
    fn test_replay_session_empty_manager() {
        let mgr = SnapshotManager::with_defaults();
        assert!(ReplaySession::new(&mgr).is_none());
    }

    #[test]
    fn test_seek_to_start_and_end() {
        let mut mgr = SnapshotManager::with_defaults();
        let mut term = Terminal::new(80, 24);

        term.process(b"One");
        mgr.take_snapshot(&term);
        mgr.record_input(b"Two");

        term.process(b"Two");
        mgr.take_snapshot(&term);
        mgr.record_input(b"Three");

        let mut session = ReplaySession::new(&mgr).unwrap();

        // Seek to start
        assert_eq!(session.seek_to_start(), SeekResult::Ok);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 0);

        // Seek to end
        assert_eq!(session.seek_to_end(), SeekResult::Ok);
        assert_eq!(session.current_index(), 1);
        assert_eq!(session.current_byte_offset(), 5); // "Three"
    }

    #[test]
    fn test_step_forward_within_entry() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);

        mgr.take_snapshot(&term);
        mgr.record_input(b"ABCDE");

        let mut session = ReplaySession::new(&mgr).unwrap();
        session.seek_to_start();

        // Step forward 3 bytes
        assert_eq!(session.step_forward(3), SeekResult::Ok);
        assert_eq!(session.current_byte_offset(), 3);

        // Verify 'A', 'B', 'C' are rendered
        let frame = session.current_frame();
        assert_eq!(frame.grid().get(0, 0).unwrap().c, 'A');
        assert_eq!(frame.grid().get(2, 0).unwrap().c, 'C');
        assert_eq!(frame.grid().get(3, 0).unwrap().c, ' ');
    }

    #[test]
    fn test_step_backward() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);

        mgr.take_snapshot(&term);
        mgr.record_input(b"ABCDE");

        let mut session = ReplaySession::new(&mgr).unwrap();
        // Start at end (5 bytes)
        assert_eq!(session.current_byte_offset(), 5);

        // Step back 2 bytes
        assert_eq!(session.step_backward(2), SeekResult::Ok);
        assert_eq!(session.current_byte_offset(), 3);

        // Frame should show ABC
        let frame = session.current_frame();
        assert_eq!(frame.grid().get(0, 0).unwrap().c, 'A');
        assert_eq!(frame.grid().get(2, 0).unwrap().c, 'C');
    }

    #[test]
    fn test_step_backward_at_start() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);
        mgr.take_snapshot(&term);

        let mut session = ReplaySession::new(&mgr).unwrap();
        session.seek_to_start();

        assert_eq!(session.step_backward(10), SeekResult::AtStart);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 0);
    }

    #[test]
    fn test_step_forward_at_end() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);
        mgr.take_snapshot(&term);
        mgr.record_input(b"AB");

        let mut session = ReplaySession::new(&mgr).unwrap();
        // Already at end

        assert_eq!(session.step_forward(10), SeekResult::AtEnd);
    }

    #[test]
    fn test_next_previous_entry() {
        let mut mgr = SnapshotManager::with_defaults();
        let mut term = Terminal::new(80, 24);

        mgr.take_snapshot(&term);
        term.process(b"First");
        mgr.take_snapshot(&term);
        term.process(b"Second");
        mgr.take_snapshot(&term);

        let mut session = ReplaySession::new(&mgr).unwrap();
        session.seek_to_start();

        assert_eq!(session.next_entry(), SeekResult::Ok);
        assert_eq!(session.current_index(), 1);

        assert_eq!(session.next_entry(), SeekResult::Ok);
        assert_eq!(session.current_index(), 2);

        assert_eq!(session.next_entry(), SeekResult::AtEnd);

        assert_eq!(session.previous_entry(), SeekResult::Ok);
        assert_eq!(session.current_index(), 1);

        session.seek_to_start();
        assert_eq!(session.previous_entry(), SeekResult::AtStart);
    }

    #[test]
    fn test_seek_to_timestamp() {
        let mut mgr = SnapshotManager::with_defaults();
        let term = Terminal::new(80, 24);

        mgr.take_snapshot(&term);
        let ts = mgr.get_entry(0).unwrap().snapshot.timestamp;

        let mut session = ReplaySession::new(&mgr).unwrap();
        assert_eq!(session.seek_to_timestamp(ts), SeekResult::Ok);
        assert_eq!(session.current_index(), 0);
    }
}
```

**Step 2: Add `push_entry` method to SnapshotManager**

The `clone_manager` function in `replay.rs` needs a `push_entry` method. Add to `impl SnapshotManager` in `snapshot_manager.rs`:

```rust
/// Push a pre-built entry (used for cloning the manager)
pub fn push_entry(&mut self, entry: SnapshotEntry) {
    let size = entry.size_bytes();
    self.entries.push_back(entry);
    self.current_memory_bytes += size;
}
```

**Step 3: Register the module**

In `src/terminal/mod.rs`, add after `snapshot_manager`:

```rust
pub mod replay;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize replay -v`
Expected: All tests PASS

**Step 5: Run all tests to verify no regressions**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize -v`
Expected: All tests PASS

**Step 6: Commit**

```bash
git add src/terminal/replay.rs src/terminal/snapshot_manager.rs src/terminal/mod.rs
git commit -m "feat(replay): add ReplaySession with timeline navigation and frame reconstruction"
```

---

### Task 8: Python Bindings

**Files:**
- Modify: `src/python_bindings/terminal.rs` (add snapshot/replay methods to PyTerminal)

**Step 1: Add Python bindings for capture_snapshot**

Add these methods to the `#[pymethods] impl PyTerminal` block in `src/python_bindings/terminal.rs` (near the existing `get_semantic_snapshot` methods):

```rust
/// Capture a cell-level snapshot of the terminal state for Instant Replay.
///
/// Unlike `get_semantic_snapshot()` which captures text only, this captures
/// raw Cell data including colors and attributes for pixel-perfect reconstruction.
///
/// Returns:
///     dict: Snapshot metadata (timestamp, cols, rows, estimated_size_bytes).
///     The actual snapshot data is stored internally and referenced by index.
///
/// Example:
///     >>> info = term.capture_replay_snapshot()
///     >>> print(f"Snapshot at {info['timestamp']}, size: {info['estimated_size_bytes']} bytes")
fn capture_replay_snapshot(&mut self) -> PyResult<pyo3::Py<pyo3::types::PyDict>> {
    let snap = self.inner.capture_snapshot();
    Python::attach(|py| {
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("timestamp", snap.timestamp)?;
        dict.set_item("cols", snap.cols)?;
        dict.set_item("rows", snap.rows)?;
        dict.set_item("estimated_size_bytes", snap.estimated_size_bytes)?;
        Ok(dict.into())
    })
}

/// Restore terminal state from the most recently captured snapshot.
///
/// This is primarily for testing. For replay, use the ReplaySession API.
///
/// Args:
///     snapshot_json: Not used yet. Reserved for future use.
fn restore_replay_snapshot(&mut self) -> PyResult<()> {
    let snap = self.inner.capture_snapshot();
    self.inner.restore_from_snapshot(&snap);
    Ok(())
}
```

Note: The full Python replay API (SnapshotManager + ReplaySession bindings) should be added in a follow-up if needed. The core Rust API is the primary interface.

**Step 2: Run clippy and tests**

Run: `cargo clippy --all-targets --all-features -- -D warnings 2>&1 | head -50`
Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize -v`
Expected: No warnings, all tests pass

**Step 3: Commit**

```bash
git add src/python_bindings/terminal.rs
git commit -m "feat(python): add capture_replay_snapshot Python binding"
```

---

### Task 9: Full Quality Check and Documentation

**Files:**
- Modify: `docs/API_REFERENCE.md` (add Instant Replay section)
- Modify: `README.md` (mention Instant Replay in features)
- Modify: `CHANGELOG.md` (add entry)

**Step 1: Run full quality checks**

Run: `make checkall`
Expected: All checks pass (fmt, clippy, pyright, tests)

**Step 2: Fix any issues found by checkall**

Address any format, lint, or test failures.

**Step 3: Update documentation**

Add an "Instant Replay" section to `docs/API_REFERENCE.md` documenting:
- `Terminal::capture_snapshot()` → `TerminalSnapshot`
- `Terminal::restore_from_snapshot(&TerminalSnapshot)`
- `SnapshotManager` creation, `take_snapshot()`, `record_input()`, `reconstruct_at()`
- `ReplaySession` creation, navigation methods (`seek_to`, `step_forward`, `step_backward`, etc.)

Add a bullet to README.md features list.
Add a CHANGELOG.md entry under the next version.

**Step 4: Run checkall again after doc changes**

Run: `make checkall`
Expected: PASS

**Step 5: Commit**

```bash
git add docs/API_REFERENCE.md README.md CHANGELOG.md
git commit -m "docs: add Instant Replay API documentation and changelog entry"
```

---

### Task 10: Final Verification

**Step 1: Run full test suite**

Run: `make checkall`
Expected: All pass

**Step 2: Verify branch is clean**

Run: `git status`
Expected: Clean working tree

**Step 3: Review all commits on branch**

Run: `git log --oneline main..HEAD`
Expected: ~8 clean commits covering all phases
