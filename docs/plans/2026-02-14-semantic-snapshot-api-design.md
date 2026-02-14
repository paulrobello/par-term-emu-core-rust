# Semantic Snapshot API Design

**Issue:** #38 — feat: AI Terminal Inspection - structured semantic snapshot API
**Date:** 2026-02-14

## Summary

Add `get_semantic_snapshot(scope)` to Terminal, returning a structured representation of terminal state (content, zones, commands, context) suitable for AI/LLM consumption and external tooling. Supports three scope levels, JSON serialization, protobuf streaming, and Python bindings.

## Design Decisions

- **Monolithic struct** over builder or composable sub-snapshots — single call, predictable schema, simplest for consumers
- **Semantic content only** — no per-cell attributes; plain text + zone/command metadata
- **Three scopes** — Visible (screen only), Recent(N) (last N commands + screen), Full (entire history)
- **Streaming support** — SnapshotRequest/SemanticSnapshot message pair in the streaming protocol

## Data Model

### SnapshotScope

```rust
pub enum SnapshotScope {
    Visible,         // Screen content only
    Recent(usize),   // Last N commands with output + visible screen
    Full,            // Entire scrollback + all history
}
```

### SemanticSnapshot

```rust
pub struct SemanticSnapshot {
    pub timestamp: u64,
    pub cols: usize,
    pub rows: usize,
    pub title: String,
    pub cursor_position: (usize, usize),  // (col, row)
    pub alt_screen_active: bool,

    pub visible_text: String,
    pub scrollback_text: Option<String>,

    pub zones: Vec<ZoneInfo>,
    pub commands: Vec<CommandInfo>,

    pub cwd: Option<String>,
    pub hostname: Option<String>,
    pub username: Option<String>,
    pub cwd_history: Vec<CwdChangeInfo>,

    pub scrollback_lines: usize,
    pub total_zones: usize,
    pub total_commands: usize,
}
```

### ZoneInfo

```rust
pub struct ZoneInfo {
    pub id: usize,
    pub zone_type: String,  // "prompt", "command", "output"
    pub abs_row_start: usize,
    pub abs_row_end: usize,
    pub text: String,
    pub command: Option<String>,
    pub exit_code: Option<i32>,
    pub timestamp: Option<u64>,
}
```

### CommandInfo

```rust
pub struct CommandInfo {
    pub command: String,
    pub cwd: Option<String>,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub success: Option<bool>,
    pub output: Option<String>,
}
```

### CwdChangeInfo

```rust
pub struct CwdChangeInfo {
    pub old_cwd: Option<String>,
    pub new_cwd: String,
    pub hostname: Option<String>,
    pub username: Option<String>,
    pub timestamp: u64,
}
```

All structs derive `Serialize, Deserialize, Clone, Debug`.

## Scope Behavior

| Field | Visible | Recent(N) | Full |
|-------|---------|-----------|------|
| visible_text | Yes | Yes | Yes |
| scrollback_text | No | Last N commands' output region | All scrollback |
| zones | Active zones on screen | Last N command zones + screen zones | All zones (incl. evicted) |
| commands | None | Last N | All history |
| cwd_history | Current only | Recent changes | Full history |

## API

### Rust

```rust
impl Terminal {
    pub fn get_semantic_snapshot(&self, scope: SnapshotScope) -> SemanticSnapshot;
    pub fn get_semantic_snapshot_json(&self, scope: SnapshotScope) -> String;
}
```

### Python

```python
terminal.get_semantic_snapshot(scope="visible"|"recent"|"full", max_commands=10) -> dict
terminal.get_semantic_snapshot_json(scope="visible"|"recent"|"full", max_commands=10) -> str
```

### Streaming Protocol

- `ClientMessage::SnapshotRequest { scope, max_commands }` — client requests snapshot
- `ServerMessage::SemanticSnapshot { snapshot_json }` — server responds with JSON

## Files to Modify

| File | Change |
|------|--------|
| `src/terminal/snapshot.rs` (new) | Snapshot structs, scope enum, builder logic |
| `src/terminal/mod.rs` | `get_semantic_snapshot()` methods, `mod snapshot` |
| `proto/terminal.proto` | Snapshot request/response messages |
| `src/streaming/protocol.rs` | `SemanticSnapshot` and `SnapshotRequest` variants |
| `src/streaming/proto.rs` | Protobuf conversion for snapshot messages |
| `src/streaming/server.rs` | Handle `SnapshotRequest` from clients |
| `src/python_bindings/terminal.rs` | Python snapshot methods |
| `src/python_bindings/streaming.rs` | Snapshot event type handling |
| `tests/` | Rust unit tests + Python integration tests |
