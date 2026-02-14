# Command Output Capture - Design Document

**Issue:** #36
**Date:** 2026-02-13
**Status:** Approved

## Problem

`CommandExecution` tracks command metadata (command text, cwd, timestamps, exit code) but has no link to the actual output text. The semantic zone system already identifies Output zones (OSC 133;C to 133;D) with absolute row ranges, but there's no bridge between a command's execution record and its output zone.

## Design

### Approach: Lightweight Row-Range Linking

Store the Output zone's absolute row range on `CommandExecution` when the zone closes. Extract text on demand using existing grid text extraction.

**Why this approach:**
- Memory-efficient: no text duplication
- Always reflects current buffer state (evicted output returns None)
- Builds on existing `get_zone_text()` pattern
- Minimal new code

### Data Model Changes

**`CommandExecution` (terminal/mod.rs)** — add:
```rust
pub output_start_row: Option<usize>,
pub output_end_row: Option<usize>,
```

**New `CommandOutput` struct:**
```rust
pub struct CommandOutput {
    pub command: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub output: String,
}
```

### Rust API

On `Terminal`:
- `get_command_output(index: usize) -> Option<String>` — 0 = most recent. Returns None if OOB or evicted.
- `get_command_outputs() -> Vec<CommandOutput>` — all commands with extractable output.

### Wiring

In OSC 133;D handler, before calling `end_command_execution()`:
1. Get the closing Output zone's `abs_row_start` and `abs_row_end`
2. Set on `current_command`

### Python Bindings

- `PyCommandExecution`: add `output_start_row`, `output_end_row` fields
- `Terminal.get_command_output(index: int) -> Optional[str]`
- `Terminal.get_command_outputs() -> list[dict]`

### Edge Cases

- **Evicted scrollback**: row range falls below scrollback floor → return None
- **Incomplete command**: no OSC 133;D → output rows remain None
- **Empty output**: valid range, returns empty string
- **Alt screen**: no zones created, no output captured
- **Wrapped lines**: handled by existing text extraction logic
