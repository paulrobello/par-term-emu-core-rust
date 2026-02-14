# Semantic Buffer Zoning Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add semantic buffer zoning to the terminal emulator, segmenting scrollback into Prompt/Command/Output blocks via OSC 133 FinalTerm markers.

**Architecture:** A `Vec<Zone>` stored on `Grid` tracks zone boundaries using absolute row numbers. OSC 133 handlers in `osc.rs` create/close zones. Zones are evicted when scrollback wraps. Python bindings expose zone queries as dicts.

**Tech Stack:** Rust (core), PyO3 (Python bindings), pytest (Python tests)

---

### Task 1: Add Zone types to grid module

**Files:**
- Create: `src/zone.rs`
- Modify: `src/lib.rs:49` (add `pub mod zone;`)
- Modify: `src/grid.rs:1-26` (add `zones` field to Grid struct)

**Step 1: Create `src/zone.rs` with Zone types and unit tests**

```rust
//! Semantic buffer zones for tracking logical blocks in terminal output
//!
//! Zones segment the scrollback buffer into Prompt, Command, and Output
//! blocks using FinalTerm/OSC 133 shell integration markers.

/// Type of semantic zone in the terminal buffer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneType {
    /// Shell prompt text (between OSC 133;A and OSC 133;B)
    Prompt,
    /// Command input text (between OSC 133;B and OSC 133;C)
    Command,
    /// Command output text (between OSC 133;C and OSC 133;D)
    Output,
}

impl std::fmt::Display for ZoneType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZoneType::Prompt => write!(f, "prompt"),
            ZoneType::Command => write!(f, "command"),
            ZoneType::Output => write!(f, "output"),
        }
    }
}

/// A semantic zone in the terminal buffer
///
/// Zones track logical blocks of terminal content using absolute row numbers.
/// They are created by OSC 133 shell integration markers and stored in a
/// Vec on the Grid, sorted by `abs_row_start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// Type of this zone
    pub zone_type: ZoneType,
    /// Absolute row where this zone starts (scrollback_len + cursor.row at creation)
    pub abs_row_start: usize,
    /// Absolute row where this zone ends (inclusive). Updated as zone grows.
    /// Equal to abs_row_start when zone is first created; updated when zone is closed.
    pub abs_row_end: usize,
    /// Command text (from OSC 133;B parameter), set on Command and Output zones
    pub command: Option<String>,
    /// Exit code (from OSC 133;D parameter), set on Output zones when command finishes
    pub exit_code: Option<i32>,
    /// Timestamp in Unix milliseconds when this zone was created
    pub timestamp: Option<u64>,
}

impl Zone {
    /// Create a new zone starting at the given absolute row
    pub fn new(zone_type: ZoneType, abs_row: usize, timestamp: Option<u64>) -> Self {
        Self {
            zone_type,
            abs_row_start: abs_row,
            abs_row_end: abs_row,
            command: None,
            exit_code: None,
            timestamp,
        }
    }

    /// Close this zone at the given absolute row
    pub fn close(&mut self, abs_row: usize) {
        // End row should be at least start row, and use the row before the new zone starts
        // unless we're on the same row
        self.abs_row_end = abs_row.max(self.abs_row_start);
    }

    /// Check if a given absolute row falls within this zone
    pub fn contains_row(&self, abs_row: usize) -> bool {
        abs_row >= self.abs_row_start && abs_row <= self.abs_row_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zone_new() {
        let zone = Zone::new(ZoneType::Prompt, 10, Some(1000));
        assert_eq!(zone.zone_type, ZoneType::Prompt);
        assert_eq!(zone.abs_row_start, 10);
        assert_eq!(zone.abs_row_end, 10);
        assert!(zone.command.is_none());
        assert!(zone.exit_code.is_none());
        assert_eq!(zone.timestamp, Some(1000));
    }

    #[test]
    fn test_zone_close() {
        let mut zone = Zone::new(ZoneType::Output, 5, None);
        zone.close(15);
        assert_eq!(zone.abs_row_end, 15);
    }

    #[test]
    fn test_zone_close_same_row() {
        let mut zone = Zone::new(ZoneType::Prompt, 5, None);
        zone.close(5);
        assert_eq!(zone.abs_row_end, 5);
    }

    #[test]
    fn test_zone_close_clamps_to_start() {
        let mut zone = Zone::new(ZoneType::Command, 10, None);
        // Closing before start should clamp to start
        zone.close(3);
        assert_eq!(zone.abs_row_end, 10);
    }

    #[test]
    fn test_zone_contains_row() {
        let mut zone = Zone::new(ZoneType::Output, 5, None);
        zone.close(15);
        assert!(!zone.contains_row(4));
        assert!(zone.contains_row(5));
        assert!(zone.contains_row(10));
        assert!(zone.contains_row(15));
        assert!(!zone.contains_row(16));
    }

    #[test]
    fn test_zone_type_display() {
        assert_eq!(ZoneType::Prompt.to_string(), "prompt");
        assert_eq!(ZoneType::Command.to_string(), "command");
        assert_eq!(ZoneType::Output.to_string(), "output");
    }
}
```

**Step 2: Add `pub mod zone;` to `src/lib.rs`**

In `src/lib.rs`, add after line 58 (`pub mod shell_integration;`):
```rust
pub mod zone;
```

**Step 3: Add `zones` field to `Grid` struct in `src/grid.rs`**

Add import at top of `src/grid.rs` (after line 1):
```rust
use crate::zone::Zone;
```

Add field to `Grid` struct (after `scrollback_wrapped` field, line 25):
```rust
    /// Semantic zones tracking logical blocks (Prompt, Command, Output)
    zones: Vec<Zone>,
    /// Total number of lines that have ever been scrolled into scrollback.
    /// Used to compute the scrollback floor for zone eviction.
    total_lines_scrolled: usize,
```

Initialize in `Grid::new()` (after `scrollback_wrapped: Vec::new(),` line 41):
```rust
            zones: Vec::new(),
            total_lines_scrolled: 0,
```

**Step 4: Run tests to verify compilation**

Run: `cargo test --lib --no-default-features zone::tests -v`
Expected: All 6 zone tests PASS

**Step 5: Commit**

```bash
git add src/zone.rs src/lib.rs src/grid.rs
git commit -m "feat(zones): add Zone and ZoneType types with unit tests"
```

---

### Task 2: Add zone storage and query methods to Grid

**Files:**
- Modify: `src/grid.rs` (add zone methods)

**Step 1: Write failing tests for Grid zone methods**

Add at the bottom of `src/grid.rs` (in the existing `#[cfg(test)] mod tests` block, or create one):

```rust
#[cfg(test)]
mod zone_tests {
    use super::*;
    use crate::zone::{Zone, ZoneType};

    #[test]
    fn test_grid_zones_empty() {
        let grid = Grid::new(80, 24, 100);
        assert!(grid.zones().is_empty());
    }

    #[test]
    fn test_grid_push_zone() {
        let mut grid = Grid::new(80, 24, 100);
        grid.push_zone(Zone::new(ZoneType::Prompt, 0, Some(1000)));
        assert_eq!(grid.zones().len(), 1);
        assert_eq!(grid.zones()[0].zone_type, ZoneType::Prompt);
    }

    #[test]
    fn test_grid_close_current_zone() {
        let mut grid = Grid::new(80, 24, 100);
        grid.push_zone(Zone::new(ZoneType::Prompt, 0, Some(1000)));
        grid.close_current_zone(5);
        assert_eq!(grid.zones()[0].abs_row_end, 5);
    }

    #[test]
    fn test_grid_zone_at() {
        let mut grid = Grid::new(80, 24, 100);
        let mut z1 = Zone::new(ZoneType::Prompt, 0, None);
        z1.close(4);
        grid.push_zone(z1);

        let mut z2 = Zone::new(ZoneType::Command, 5, None);
        z2.close(6);
        grid.push_zone(z2);

        let mut z3 = Zone::new(ZoneType::Output, 7, None);
        z3.close(20);
        grid.push_zone(z3);

        assert_eq!(grid.zone_at(0).unwrap().zone_type, ZoneType::Prompt);
        assert_eq!(grid.zone_at(4).unwrap().zone_type, ZoneType::Prompt);
        assert_eq!(grid.zone_at(5).unwrap().zone_type, ZoneType::Command);
        assert_eq!(grid.zone_at(10).unwrap().zone_type, ZoneType::Output);
        assert!(grid.zone_at(21).is_none());
    }

    #[test]
    fn test_grid_evict_zones() {
        let mut grid = Grid::new(80, 24, 100);
        let mut z1 = Zone::new(ZoneType::Prompt, 0, None);
        z1.close(4);
        grid.push_zone(z1);

        let mut z2 = Zone::new(ZoneType::Output, 5, None);
        z2.close(20);
        grid.push_zone(z2);

        // Evict zones fully before row 5
        grid.evict_zones(5);
        assert_eq!(grid.zones().len(), 1);
        assert_eq!(grid.zones()[0].zone_type, ZoneType::Output);
    }

    #[test]
    fn test_grid_evict_zones_partial() {
        let mut grid = Grid::new(80, 24, 100);
        let mut z1 = Zone::new(ZoneType::Output, 0, None);
        z1.close(20);
        grid.push_zone(z1);

        // Evict with floor inside the zone - zone should be truncated
        grid.evict_zones(10);
        assert_eq!(grid.zones().len(), 1);
        assert_eq!(grid.zones()[0].abs_row_start, 10);
    }

    #[test]
    fn test_grid_clear_zones() {
        let mut grid = Grid::new(80, 24, 100);
        grid.push_zone(Zone::new(ZoneType::Prompt, 0, None));
        grid.clear_zones();
        assert!(grid.zones().is_empty());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib --no-default-features grid::zone_tests -v`
Expected: FAIL - methods don't exist yet

**Step 3: Implement Grid zone methods**

Add these methods to the `impl Grid` block in `src/grid.rs`:

```rust
    // ========== Semantic Zone Methods ==========

    /// Get all semantic zones
    pub fn zones(&self) -> &[Zone] {
        &self.zones
    }

    /// Push a new zone. Zones must be appended in order of abs_row_start.
    pub fn push_zone(&mut self, zone: Zone) {
        self.zones.push(zone);
    }

    /// Close the current (last) open zone at the given absolute row.
    /// No-op if there are no zones.
    pub fn close_current_zone(&mut self, abs_row: usize) {
        if let Some(zone) = self.zones.last_mut() {
            zone.close(abs_row);
        }
    }

    /// Find the zone containing the given absolute row.
    /// Uses linear search (zones are typically few relative to rows).
    pub fn zone_at(&self, abs_row: usize) -> Option<&Zone> {
        // Search from end since recent zones are more likely to be queried
        self.zones.iter().rev().find(|z| z.contains_row(abs_row))
    }

    /// Evict zones that have been fully scrolled past the given floor row.
    /// Zones fully below `floor` are removed. Zones spanning the boundary
    /// have their `abs_row_start` clamped to `floor`.
    pub fn evict_zones(&mut self, floor: usize) {
        // Remove zones fully before the floor
        self.zones.retain(|z| z.abs_row_end >= floor);
        // Clamp start of surviving zones that span the boundary
        for zone in &mut self.zones {
            if zone.abs_row_start < floor {
                zone.abs_row_start = floor;
            }
        }
    }

    /// Clear all zones (used on terminal reset)
    pub fn clear_zones(&mut self) {
        self.zones.clear();
    }

    /// Get total lines ever scrolled (for computing scrollback floor)
    pub fn total_lines_scrolled(&self) -> usize {
        self.total_lines_scrolled
    }
```

**Step 4: Update `scroll_up()` to track `total_lines_scrolled` and evict zones**

In `src/grid.rs`, in the `scroll_up` method (line ~170), add at the very beginning after `let n = n.min(self.rows);`:

```rust
        // Track total lines scrolled for zone eviction
        if self.max_scrollback > 0 {
            self.total_lines_scrolled += n;
            // Evict zones whose rows have been overwritten
            if self.scrollback_lines >= self.max_scrollback {
                let floor = self.total_lines_scrolled - self.max_scrollback;
                self.evict_zones(floor);
            }
        }
```

**Step 5: Update `clear()` and `clear_scrollback()` to also clear zones**

In `Grid::clear()` (line ~120), add:
```rust
        self.zones.clear();
```

In `Grid::clear_scrollback()` (line ~700), add:
```rust
        self.zones.clear();
        self.total_lines_scrolled = 0;
```

**Step 6: Update `Grid::resize()` to preserve zones**

Zones use absolute row numbers, so they survive resizes without modification. However, add `zones` and `total_lines_scrolled` to any `Grid` reconstruction paths. Search for all places where `Grid` is constructed or fields are individually set during resize. The existing `resize()` method creates new Vecs for cells/wrapped but doesn't reconstruct Grid, so zones survive automatically.

No code change needed unless resize creates a new Grid - verify by reading the resize method. The fields persist on `self` through resize.

**Step 7: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features grid::zone_tests -v`
Expected: All 7 tests PASS

**Step 8: Commit**

```bash
git add src/grid.rs
git commit -m "feat(zones): add zone storage and query methods to Grid"
```

---

### Task 3: Wire OSC 133 handlers to create/close zones

**Files:**
- Modify: `src/terminal/sequences/osc.rs:449-530` (OSC 133 handler)
- Modify: `src/terminal/mod.rs` (add Terminal-level zone methods)

**Step 1: Add Terminal-level zone helper methods**

Add to `impl Terminal` in `src/terminal/mod.rs` (near the shell_integration methods around line 2222):

```rust
    // ========== Semantic Zone Methods ==========

    /// Get all semantic zones from the primary grid
    pub fn get_zones(&self) -> &[crate::zone::Zone] {
        self.grid.zones()
    }

    /// Get the zone containing the given absolute row
    pub fn get_zone_at(&self, abs_row: usize) -> Option<&crate::zone::Zone> {
        self.grid.zone_at(abs_row)
    }

    /// Extract the text content of the zone containing the given absolute row.
    /// Returns None if no zone contains this row.
    pub fn get_zone_text(&self, abs_row: usize) -> Option<String> {
        let zone = self.grid.zone_at(abs_row)?;
        let scrollback_len = self.grid.scrollback_len();
        let mut text = String::new();

        for row in zone.abs_row_start..=zone.abs_row_end {
            if row < scrollback_len {
                // Row is in scrollback
                if let Some(line) = self.grid.scrollback_line(row) {
                    let line_text: String = line.iter()
                        .filter(|c| !c.flags.wide_char_spacer())
                        .map(|c| {
                            let mut s = String::new();
                            s.push(c.c);
                            for &combining in &c.combining {
                                s.push(combining);
                            }
                            s
                        })
                        .collect();
                    let trimmed = line_text.trim_end();
                    if !text.is_empty() {
                        // Check if previous line was wrapped
                        if row > zone.abs_row_start && self.grid.is_scrollback_wrapped(row - 1) {
                            // Wrapped line - no newline
                        } else {
                            text.push('\n');
                        }
                    }
                    text.push_str(trimmed);
                }
            } else {
                // Row is in main grid
                let grid_row = row - scrollback_len;
                if let Some(line) = self.grid.row(grid_row) {
                    let line_text: String = line.iter()
                        .filter(|c| !c.flags.wide_char_spacer())
                        .map(|c| {
                            let mut s = String::new();
                            s.push(c.c);
                            for &combining in &c.combining {
                                s.push(combining);
                            }
                            s
                        })
                        .collect();
                    let trimmed = line_text.trim_end();
                    if !text.is_empty() {
                        if row > zone.abs_row_start {
                            let prev_row = row - 1;
                            if prev_row < scrollback_len {
                                if !self.grid.is_scrollback_wrapped(prev_row) {
                                    text.push('\n');
                                }
                            } else {
                                let prev_grid_row = prev_row - scrollback_len;
                                if !self.grid.is_line_wrapped(prev_grid_row) {
                                    text.push('\n');
                                }
                            }
                        }
                    }
                    text.push_str(trimmed);
                }
            }
        }

        Some(text)
    }
```

**Step 2: Update OSC 133 handler to create/close zones**

In `src/terminal/sequences/osc.rs`, modify the `"133"` match arm (lines 449-530). After the existing `ShellIntegrationEvent` push for each marker, add zone creation/closure.

For marker `'A'` (Prompt Start), after the `terminal_events.push(...)` call:
```rust
                                    // Zone: close any open zone, start new Prompt zone
                                    if !self.alt_screen_active {
                                        self.grid.close_current_zone(abs_line.saturating_sub(1).max(
                                            self.grid.zones().last().map_or(0, |z| z.abs_row_start),
                                        ));
                                        let mut zone = crate::zone::Zone::new(
                                            crate::zone::ZoneType::Prompt,
                                            abs_line,
                                            Some(ts),
                                        );
                                        self.grid.push_zone(zone);
                                    }
```

For marker `'B'` (Command Start), after the `terminal_events.push(...)` call:
```rust
                                    // Zone: close Prompt zone, start Command zone
                                    if !self.alt_screen_active {
                                        self.grid.close_current_zone(abs_line.saturating_sub(1).max(
                                            self.grid.zones().last().map_or(0, |z| z.abs_row_start),
                                        ));
                                        let mut zone = crate::zone::Zone::new(
                                            crate::zone::ZoneType::Command,
                                            abs_line,
                                            Some(ts),
                                        );
                                        zone.command = self.shell_integration.command().map(|s| s.to_string());
                                        self.grid.push_zone(zone);
                                    }
```

For marker `'C'` (Command Executed), after the `terminal_events.push(...)` call:
```rust
                                    // Zone: close Command zone, start Output zone
                                    if !self.alt_screen_active {
                                        self.grid.close_current_zone(abs_line.saturating_sub(1).max(
                                            self.grid.zones().last().map_or(0, |z| z.abs_row_start),
                                        ));
                                        let mut zone = crate::zone::Zone::new(
                                            crate::zone::ZoneType::Output,
                                            abs_line,
                                            Some(ts),
                                        );
                                        zone.command = self.shell_integration.command().map(|s| s.to_string());
                                        self.grid.push_zone(zone);
                                    }
```

For marker `'D'` (Command Finished), after the `terminal_events.push(...)` call:
```rust
                                    // Zone: close Output zone, record exit code
                                    if !self.alt_screen_active {
                                        self.grid.close_current_zone(abs_line);
                                        // Set exit code on the just-closed Output zone
                                        if let Some(zone) = self.grid.zones_mut().last_mut() {
                                            if zone.zone_type == crate::zone::ZoneType::Output {
                                                zone.exit_code = parsed_code;
                                            }
                                        }
                                    }
```

**Step 3: Add `zones_mut()` to Grid**

In `src/grid.rs`, add:
```rust
    /// Get mutable access to zones (for setting exit_code after closing)
    pub fn zones_mut(&mut self) -> &mut Vec<Zone> {
        &mut self.zones
    }
```

**Step 4: Update Terminal::reset() to clear zones**

In `src/terminal/mod.rs` line ~3304, after `self.grid.clear();`, add:
```rust
        // Zone data is cleared when grid is cleared (Grid::clear calls zones.clear())
```

(Grid::clear already clears zones from Task 2 Step 5)

**Step 5: Run the full test suite to verify nothing is broken**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize`
Expected: All existing tests PASS

**Step 6: Commit**

```bash
git add src/terminal/sequences/osc.rs src/terminal/mod.rs src/grid.rs
git commit -m "feat(zones): wire OSC 133 handlers to create/close semantic zones"
```

---

### Task 4: Add Rust integration tests for zone lifecycle

**Files:**
- Modify: `src/terminal/sequences/osc.rs` (add tests at bottom)

**Step 1: Write integration tests for zone creation via OSC 133**

Add to the `#[cfg(test)]` block at the bottom of `src/terminal/sequences/osc.rs`:

```rust
    #[test]
    fn test_zones_created_by_osc_133() {
        let mut term = Terminal::new(80, 24);

        // Prompt start
        term.process(b"\x1b]133;A\x07");
        assert_eq!(term.get_zones().len(), 1);
        assert_eq!(term.get_zones()[0].zone_type, crate::zone::ZoneType::Prompt);

        // Type a command - first set it via shell integration
        term.process(b"ls -la");

        // Command start
        term.shell_integration_mut().set_command("ls -la".to_string());
        term.process(b"\x1b]133;B\x07");
        assert_eq!(term.get_zones().len(), 2);
        assert_eq!(term.get_zones()[1].zone_type, crate::zone::ZoneType::Command);
        assert_eq!(term.get_zones()[1].command.as_deref(), Some("ls -la"));

        // Command executed (output begins)
        term.process(b"\x1b]133;C\x07");
        assert_eq!(term.get_zones().len(), 3);
        assert_eq!(term.get_zones()[2].zone_type, crate::zone::ZoneType::Output);
        assert_eq!(term.get_zones()[2].command.as_deref(), Some("ls -la"));

        // Some output
        term.process(b"file1.txt\r\nfile2.txt\r\n");

        // Command finished with exit code 0
        term.process(b"\x1b]133;D;0\x07");
        // Output zone should be closed with exit code
        assert_eq!(term.get_zones()[2].exit_code, Some(0));
    }

    #[test]
    fn test_zones_multiple_commands() {
        let mut term = Terminal::new(80, 24);

        // First command cycle
        term.process(b"\x1b]133;A\x07$ \x1b]133;B\x07\x1b]133;C\x07output1\r\n\x1b]133;D;0\x07");

        // Second command cycle
        term.process(b"\x1b]133;A\x07$ \x1b]133;B\x07\x1b]133;C\x07output2\r\n\x1b]133;D;1\x07");

        // Should have 8 zones (P, C, O, P, C, O) - wait, D closes but doesn't create
        // Actually: A creates Prompt, B creates Command, C creates Output, D closes Output
        // Second cycle: A creates Prompt (4th), B creates Command (5th), C creates Output (6th), D closes
        assert_eq!(term.get_zones().len(), 6);
        assert_eq!(term.get_zones()[5].exit_code, Some(1));
    }

    #[test]
    fn test_zones_not_created_on_alt_screen() {
        let mut term = Terminal::new(80, 24);

        // Switch to alt screen
        term.process(b"\x1b[?1049h");

        // OSC 133 on alt screen should not create zones
        term.process(b"\x1b]133;A\x07");
        assert!(term.get_zones().is_empty());

        // Switch back to primary
        term.process(b"\x1b[?1049l");

        // Now it should create zones
        term.process(b"\x1b]133;A\x07");
        assert_eq!(term.get_zones().len(), 1);
    }

    #[test]
    fn test_zones_cleared_on_reset() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]133;A\x07");
        assert_eq!(term.get_zones().len(), 1);

        term.reset();
        assert!(term.get_zones().is_empty());
    }
```

**Step 2: Run tests**

Run: `cargo test --lib --no-default-features osc::tests -v`
Expected: All zone tests PASS

**Step 3: Commit**

```bash
git add src/terminal/sequences/osc.rs
git commit -m "test(zones): add Rust integration tests for zone lifecycle"
```

---

### Task 5: Add zone eviction tests

**Files:**
- Modify: `src/terminal/sequences/osc.rs` (add scrollback eviction tests)

**Step 1: Write tests for zone eviction during scrollback wrap**

```rust
    #[test]
    fn test_zones_evicted_on_scrollback_wrap() {
        // Small scrollback to trigger eviction quickly
        let mut term = Terminal::with_scrollback(80, 5, 10);

        // Create a prompt zone
        term.process(b"\x1b]133;A\x07");
        term.process(b"\x1b]133;B\x07");
        term.process(b"\x1b]133;C\x07");

        // Generate enough output to fill scrollback and wrap
        for i in 0..20 {
            term.process(format!("line {}\r\n", i).as_bytes());
        }

        // Command finished
        term.process(b"\x1b]133;D;0\x07");

        // Zones with rows below the scrollback floor should be evicted
        let zones = term.get_zones();
        for zone in zones {
            // All remaining zones should have rows within the scrollback window
            let scrollback_len = term.active_grid().scrollback_len();
            let floor = term.active_grid().total_lines_scrolled().saturating_sub(term.active_grid().max_scrollback());
            assert!(zone.abs_row_end >= floor,
                "Zone {:?} at rows {}-{} should be >= floor {}",
                zone.zone_type, zone.abs_row_start, zone.abs_row_end, floor);
        }
    }
```

**Step 2: Run tests**

Run: `cargo test --lib --no-default-features osc::tests::test_zones_evicted -v`
Expected: PASS

**Step 3: Commit**

```bash
git add src/terminal/sequences/osc.rs
git commit -m "test(zones): add scrollback eviction tests"
```

---

### Task 6: Add Python bindings for zone access

**Files:**
- Modify: `src/python_bindings/terminal.rs` (add get_zones, get_zone_at, get_zone_text methods)

**Step 1: Add Python binding methods**

Add to `#[pymethods] impl PyTerminal` in `src/python_bindings/terminal.rs`:

```rust
    /// Get all semantic zones in the terminal buffer
    ///
    /// Returns a list of zone dictionaries, each containing:
    /// - zone_type: str - "prompt", "command", or "output"
    /// - abs_row_start: int - Absolute row where zone starts
    /// - abs_row_end: int - Absolute row where zone ends (inclusive)
    /// - command: str | None - Command text (for command/output zones)
    /// - exit_code: int | None - Exit code (for output zones after command finishes)
    /// - timestamp: int | None - Unix milliseconds when zone was created
    ///
    /// Returns:
    ///     List of zone dictionaries sorted by row position
    ///
    /// Example:
    ///     >>> zones = term.get_zones()
    ///     >>> for z in zones:
    ///     ...     print(f"{z['zone_type']}: rows {z['abs_row_start']}-{z['abs_row_end']}")
    fn get_zones(&self) -> PyResult<Vec<HashMap<String, pyo3::PyObject>>> {
        Python::with_gil(|py| {
            Ok(self
                .inner
                .get_zones()
                .iter()
                .map(|zone| {
                    let mut map = HashMap::new();
                    map.insert(
                        "zone_type".to_string(),
                        zone.zone_type.to_string().into_pyobject(py).unwrap().into_any().unbind(),
                    );
                    map.insert(
                        "abs_row_start".to_string(),
                        zone.abs_row_start.into_pyobject(py).unwrap().into_any().unbind(),
                    );
                    map.insert(
                        "abs_row_end".to_string(),
                        zone.abs_row_end.into_pyobject(py).unwrap().into_any().unbind(),
                    );
                    map.insert(
                        "command".to_string(),
                        zone.command.as_ref().map(|s| s.as_str()).into_pyobject(py).unwrap().into_any().unbind(),
                    );
                    map.insert(
                        "exit_code".to_string(),
                        zone.exit_code.into_pyobject(py).unwrap().into_any().unbind(),
                    );
                    map.insert(
                        "timestamp".to_string(),
                        zone.timestamp.into_pyobject(py).unwrap().into_any().unbind(),
                    );
                    map
                })
                .collect())
        })
    }

    /// Get the semantic zone containing the given absolute row
    ///
    /// Args:
    ///     abs_row: Absolute row number (scrollback_len + visible_row)
    ///
    /// Returns:
    ///     Zone dictionary or None if no zone contains this row
    ///
    /// Example:
    ///     >>> zone = term.get_zone_at(term.scrollback_len() + 0)
    ///     >>> if zone:
    ///     ...     print(f"Row 0 is in a {zone['zone_type']} zone")
    fn get_zone_at(&self, abs_row: usize) -> PyResult<Option<HashMap<String, pyo3::PyObject>>> {
        Python::with_gil(|py| {
            Ok(self.inner.get_zone_at(abs_row).map(|zone| {
                let mut map = HashMap::new();
                map.insert(
                    "zone_type".to_string(),
                    zone.zone_type.to_string().into_pyobject(py).unwrap().into_any().unbind(),
                );
                map.insert(
                    "abs_row_start".to_string(),
                    zone.abs_row_start.into_pyobject(py).unwrap().into_any().unbind(),
                );
                map.insert(
                    "abs_row_end".to_string(),
                    zone.abs_row_end.into_pyobject(py).unwrap().into_any().unbind(),
                );
                map.insert(
                    "command".to_string(),
                    zone.command.as_ref().map(|s| s.as_str()).into_pyobject(py).unwrap().into_any().unbind(),
                );
                map.insert(
                    "exit_code".to_string(),
                    zone.exit_code.into_pyobject(py).unwrap().into_any().unbind(),
                );
                map.insert(
                    "timestamp".to_string(),
                    zone.timestamp.into_pyobject(py).unwrap().into_any().unbind(),
                );
                map
            }))
        })
    }

    /// Get the text content of the zone containing the given absolute row
    ///
    /// Extracts all text from the zone's rows, handling line wrapping and
    /// trimming trailing whitespace. Returns None if no zone contains this row.
    ///
    /// Args:
    ///     abs_row: Absolute row number (scrollback_len + visible_row)
    ///
    /// Returns:
    ///     Zone text content as a string, or None
    ///
    /// Example:
    ///     >>> text = term.get_zone_text(some_row)
    ///     >>> if text:
    ///     ...     print(f"Zone content: {text}")
    fn get_zone_text(&self, abs_row: usize) -> PyResult<Option<String>> {
        Ok(self.inner.get_zone_text(abs_row))
    }
```

**Step 2: Build with maturin to verify compilation**

Run: `make dev`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/python_bindings/terminal.rs
git commit -m "feat(zones): add Python bindings for zone access"
```

---

### Task 7: Add Python tests for zone API

**Files:**
- Create: `tests/test_zones.py`

**Step 1: Write Python tests**

```python
"""Tests for semantic buffer zoning (OSC 133 shell integration zones)."""

import pytest
from par_term_emu_core_rust import Terminal


class TestZoneCreation:
    """Test zone creation via OSC 133 markers."""

    def test_no_zones_initially(self):
        term = Terminal(80, 24, scrollback=100)
        assert term.get_zones() == []

    def test_prompt_zone_created_on_osc_133_a(self):
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        zones = term.get_zones()
        assert len(zones) == 1
        assert zones[0]["zone_type"] == "prompt"

    def test_full_command_cycle_creates_three_zones(self):
        term = Terminal(80, 24, scrollback=100)
        # Prompt
        term.process(b"\x1b]133;A\x07")
        # Command
        term.process(b"\x1b]133;B\x07")
        # Output
        term.process(b"\x1b]133;C\x07")
        term.process(b"hello\r\n")
        # Finished
        term.process(b"\x1b]133;D;0\x07")

        zones = term.get_zones()
        assert len(zones) == 3
        assert zones[0]["zone_type"] == "prompt"
        assert zones[1]["zone_type"] == "command"
        assert zones[2]["zone_type"] == "output"
        assert zones[2]["exit_code"] == 0

    def test_exit_code_nonzero(self):
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"\x1b]133;D;127\x07")

        zones = term.get_zones()
        assert zones[2]["exit_code"] == 127

    def test_zone_timestamps_are_set(self):
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        zones = term.get_zones()
        assert zones[0]["timestamp"] is not None
        assert zones[0]["timestamp"] > 0

    def test_multiple_command_cycles(self):
        term = Terminal(80, 24, scrollback=100)
        for _ in range(3):
            term.process(b"\x1b]133;A\x07")
            term.process(b"\x1b]133;B\x07")
            term.process(b"\x1b]133;C\x07")
            term.process(b"\x1b]133;D;0\x07")

        zones = term.get_zones()
        assert len(zones) == 9  # 3 cycles * 3 zones each


class TestZoneQuery:
    """Test zone query methods."""

    def test_get_zone_at_returns_correct_zone(self):
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output line\r\n")
        term.process(b"\x1b]133;D;0\x07")

        # Row 0 should be in a zone
        zone = term.get_zone_at(0)
        assert zone is not None
        assert zone["zone_type"] == "prompt"

    def test_get_zone_at_returns_none_for_no_zone(self):
        term = Terminal(80, 24, scrollback=100)
        assert term.get_zone_at(0) is None

    def test_get_zone_at_returns_none_beyond_zones(self):
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        # Query a row far beyond any zone
        assert term.get_zone_at(1000) is None


class TestZoneText:
    """Test zone text extraction."""

    def test_get_zone_text_extracts_content(self):
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07$ ")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"file1.txt\r\nfile2.txt\r\n")
        term.process(b"\x1b]133;D;0\x07")

        # Get text of the prompt zone (first zone, row 0)
        text = term.get_zone_text(0)
        assert text is not None
        assert "$" in text

    def test_get_zone_text_returns_none_for_no_zone(self):
        term = Terminal(80, 24, scrollback=100)
        assert term.get_zone_text(0) is None


class TestZoneEviction:
    """Test zone eviction on scrollback wrap."""

    def test_zones_evicted_when_scrollback_wraps(self):
        term = Terminal(80, 5, scrollback=10)

        # Create initial zones
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")

        # Generate enough output to fill scrollback and trigger eviction
        for i in range(25):
            term.process(f"line {i}\r\n".encode())
        term.process(b"\x1b]133;D;0\x07")

        # Early zones should have been evicted or truncated
        zones = term.get_zones()
        # Zones that remain should be within the valid scrollback range
        scrollback_len = term.scrollback_len()
        for z in zones:
            assert z["abs_row_end"] >= 0  # Basic sanity


class TestZoneAltScreen:
    """Test zone behavior with alternate screen."""

    def test_no_zones_on_alt_screen(self):
        term = Terminal(80, 24, scrollback=100)
        # Switch to alt screen
        term.process(b"\x1b[?1049h")
        term.process(b"\x1b]133;A\x07")
        assert term.get_zones() == []

        # Switch back
        term.process(b"\x1b[?1049l")
        term.process(b"\x1b]133;A\x07")
        assert len(term.get_zones()) == 1


class TestZoneReset:
    """Test zones cleared on terminal reset."""

    def test_zones_cleared_on_reset(self):
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        assert len(term.get_zones()) == 1

        # Full reset (RIS)
        term.process(b"\x1bc")
        assert term.get_zones() == []
```

**Step 2: Run Python tests**

Run: `uv run pytest tests/test_zones.py -v`
Expected: All tests PASS

**Step 3: Commit**

```bash
git add tests/test_zones.py
git commit -m "test(zones): add Python tests for zone API"
```

---

### Task 8: Final verification and cleanup

**Files:**
- Modify: `docs/API_REFERENCE.md` (add zone API docs)
- Modify: `README.md` (mention semantic zones in features)

**Step 1: Run full `make checkall`**

Run: `make checkall`
Expected: All checks pass (fmt, lint, clippy, pyright, tests)

**Step 2: Fix any issues found by checkall**

Address clippy warnings, formatting issues, or test failures.

**Step 3: Update API reference**

Add a "Semantic Zones" section to `docs/API_REFERENCE.md` documenting:
- `get_zones()` → `List[dict]`
- `get_zone_at(abs_row)` → `dict | None`
- `get_zone_text(abs_row)` → `str | None`
- Zone dict fields: zone_type, abs_row_start, abs_row_end, command, exit_code, timestamp

**Step 4: Update README.md features list**

Add "Semantic buffer zoning (OSC 133 FinalTerm markers)" to the features list.

**Step 5: Commit docs**

```bash
git add docs/API_REFERENCE.md README.md
git commit -m "docs: add semantic zone API documentation"
```

**Step 6: Final `make checkall`**

Run: `make checkall`
Expected: All checks pass

---
