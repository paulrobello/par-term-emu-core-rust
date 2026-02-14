# Semantic Buffer Zoning Design

**Issue**: #35 - Segment scrollback into logical blocks using FinalTerm/OSC 133 markers
**Date**: 2026-02-13

## Overview

Add a `ZoneMap` to the terminal's primary grid that segments the scrollback buffer into logical blocks (Prompt, Command, Output) using OSC 133 shell integration markers. This enables structured understanding of terminal content for downstream features like command output capture and AI terminal inspection.

## Data Model

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneType {
    Prompt,
    Command,
    Output,
}

#[derive(Debug, Clone)]
pub struct Zone {
    pub zone_type: ZoneType,
    pub abs_row_start: usize,   // Absolute row at zone creation
    pub abs_row_end: usize,     // Updated as zone grows; inclusive
    pub command: Option<String>, // From OSC 133;B parameter
    pub exit_code: Option<i32>,  // From OSC 133;D parameter
    pub timestamp: Option<u64>,  // Unix millis at zone creation
}
```

### Absolute Row Addressing

Zones use absolute row numbers: `scrollback_len + cursor.row` at the time the marker is received. This provides a stable coordinate system that doesn't shift as new lines scroll into the buffer.

## Storage: Vec\<Zone\> on Grid

A `Vec<Zone>` field (`zones`) on `Grid`, always sorted by `abs_row_start` (guaranteed by append-only insertion). The last element is the "current open zone" if its `abs_row_end` hasn't been finalized.

**Why Vec over BTreeMap**: Zones are naturally appended in order. Binary search gives O(log n) lookup. For typical scrollback sizes (1000-10000 lines), zone counts are small enough that Vec outperforms BTreeMap. Front-drain eviction is simple.

## Zone Lifecycle

| OSC 133 Marker | Action |
|---|---|
| `A` (Prompt Start) | Close any open zone at current row. Push `Zone { Prompt, ... }` |
| `B` (Command Start) | Close Prompt zone. Push `Zone { Command, ... }` with command text |
| `C` (Command Executed) | Close Command zone. Push `Zone { Output, ... }` |
| `D` (Command Finished) | Close Output zone. Record exit code on it |

"Closing" a zone means setting `abs_row_end` to the current absolute row (minus 1 if the new marker starts on a new semantic boundary).

## Scrollback Eviction

When the circular scrollback buffer wraps and overwrites the oldest line, compute the scrollback floor:

```
floor = total_lines_scrolled - max_scrollback
```

Drain zones from the front of the Vec while `zone.abs_row_end < floor`. Zones that span the floor boundary are truncated (their `abs_row_start` is clamped to `floor`).

## Query APIs

### Rust (on Grid)

```rust
pub fn zones(&self) -> &[Zone];
pub fn zone_at(&self, abs_row: usize) -> Option<&Zone>;  // binary search
pub fn evict_zones(&mut self, floor: usize);
```

### Rust (on Terminal)

```rust
pub fn get_zones(&self) -> Vec<Zone>;           // Clone of all zones
pub fn get_zone_at(&self, abs_row: usize) -> Option<Zone>;
pub fn get_zone_text(&self, abs_row: usize) -> Option<String>; // Extract text from zone's rows
```

### Python Bindings

```python
term.get_zones()        # List[dict] with zone_type, row_start, row_end, command, exit_code, timestamp
term.get_zone_at(row)   # dict | None - row is absolute
term.get_zone_text(row) # str | None - text content of zone containing row
```

## Scope Decisions

- **Primary screen only** - Alt screen has no scrollback and fullscreen apps don't use shell integration
- **Silent eviction** - No events emitted when zones are lost; consumers can check zone existence
- **Rich zone data** - Command text, exit code, timestamps included when available from OSC 133 markers

## Testing Strategy

- **Rust unit tests**: Zone creation, lifecycle, eviction, binary search, resize behavior
- **Rust integration tests**: OSC 133 sequence processing end-to-end
- **Python tests**: Binding correctness, zone query API, zone text extraction
