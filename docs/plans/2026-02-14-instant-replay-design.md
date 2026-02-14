# Instant Replay System Design

**Issue**: #47
**Date**: 2026-02-14
**Status**: Approved

## Summary

Extend the terminal emulator with cell-level state snapshots, input-stream delta recording, and a managed replay buffer to enable iTerm2-style Instant Replay.

## Current State

The core library has one-way text-based state capture via `SemanticSnapshot`:
- Captures text content, zones, commands, cursor position, dimensions
- No cell-level data (colors, flags, attributes lost)
- No state restoration capability
- No snapshot management or replay navigation

## Architecture

### New Snapshot Type: TerminalSnapshot

A new `TerminalSnapshot` struct captures raw `Cell` data for pixel-perfect reconstruction, distinct from `SemanticSnapshot` (which remains unchanged for text/AI use cases).

```rust
pub struct TerminalSnapshot {
    pub timestamp: u64,
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,                  // main grid (row-major)
    pub scrollback_cells: Vec<Cell>,       // scrollback buffer
    pub scrollback_start: usize,           // circular buffer index
    pub scrollback_lines: usize,           // current scrollback size
    pub max_scrollback: usize,             // scrollback capacity
    pub wrapped: Vec<bool>,                // main grid line wrap flags
    pub scrollback_wrapped: Vec<bool>,     // scrollback wrap flags
    pub cursor_col: usize,
    pub cursor_row: usize,
    pub cursor_visible: bool,
    pub alt_screen_active: bool,
    pub alt_grid: Option<GridSnapshot>,    // alternate screen if active
    pub zones: Vec<Zone>,
    pub title: String,
    pub cwd: Option<String>,
    pub estimated_size_bytes: usize,
}

pub struct GridSnapshot {
    pub cells: Vec<Cell>,
    pub scrollback_cells: Vec<Cell>,
    pub scrollback_start: usize,
    pub scrollback_lines: usize,
    pub wrapped: Vec<bool>,
    pub scrollback_wrapped: Vec<bool>,
    pub zones: Vec<Zone>,
}
```

### Delta Strategy: Input-Stream Replay

Between full snapshots, record the raw bytes fed to `terminal.process()`. To reconstruct any point:
1. Restore the nearest prior full snapshot
2. Replay accumulated bytes through a fresh terminal

This is simple, compact, and perfectly accurate since it replays the exact same input.

### Snapshot Manager

```rust
pub struct SnapshotEntry {
    pub snapshot: TerminalSnapshot,
    pub input_bytes: Vec<u8>,          // bytes received AFTER this snapshot
    pub entry_size_bytes: usize,       // snapshot size + input_bytes.len()
}

pub struct SnapshotManager {
    entries: VecDeque<SnapshotEntry>,
    max_memory_bytes: usize,           // default 4MB
    current_memory_bytes: usize,
    snapshot_interval: Duration,
    last_snapshot_time: Instant,
}
```

**Eviction**: Size-based. Drop oldest entries when `current_memory_bytes > max_memory_bytes`.

### Replay Session

```rust
pub struct ReplaySession {
    entries: Vec<SnapshotEntry>,       // cloned from manager
    current_index: usize,
    current_byte_offset: usize,
    current_frame: Terminal,           // reconstructed terminal
    is_live: bool,                     // tracking live terminal?
}
```

Navigation: `step_forward()`, `step_backward()`, `seek_to_entry()`, `seek_to_timestamp()`, `get_current_frame()`.

## Phases

### Phase 1: TerminalSnapshot + Capture/Restore
- New `src/terminal/terminal_snapshot.rs` with `TerminalSnapshot` and `GridSnapshot`
- `Terminal::capture_snapshot() -> TerminalSnapshot`
- `Terminal::restore_from_snapshot(&TerminalSnapshot)`
- Handle dimension mismatches (resize after restore)
- Handle alt screen state preservation
- Serde serialization support
- Comprehensive tests

### Phase 2: Snapshot Manager
- New `src/terminal/snapshot_manager.rs`
- `SnapshotManager::new(max_memory_bytes, snapshot_interval)`
- `record_input(&mut self, bytes: &[u8])` — append to current entry
- `take_snapshot(&mut self, terminal: &Terminal)` — capture + start new entry
- `should_snapshot(&self) -> bool` — check interval
- Size-based eviction on insert
- Memory tracking and reporting

### Phase 3: Input-Stream Reconstruction
- `SnapshotManager::reconstruct_at(entry_index, byte_offset) -> Terminal`
- Restore snapshot, replay bytes through `Terminal::process()`
- Timestamp-based lookup (binary search within entries)

### Phase 4: Replay Session + Python Bindings
- `ReplaySession` struct with timeline navigation
- `step_forward()` / `step_backward()` for frame-by-frame
- `seek_to_timestamp()` for jumping to a point
- `get_current_frame()` returns renderable terminal state
- Python bindings for all replay APIs
- Observer notifications for replay state changes

## Memory Considerations

- Full snapshot for 80x24 terminal with 10K scrollback: ~2-5MB
- Input byte deltas: typically 1-100KB per snapshot interval
- Default budget: 4MB (matches iTerm2 default)
- With 30-second intervals, budget holds ~1-2 full snapshots plus deltas

## What Stays Unchanged

- `SemanticSnapshot` — untouched, still serves text/AI use cases
- `TerminalObserver` — extended with replay events, no breaking changes
- All existing Python bindings — purely additive changes
