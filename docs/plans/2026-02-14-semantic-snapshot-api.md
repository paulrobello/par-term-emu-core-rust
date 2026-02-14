# Semantic Snapshot API Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `get_semantic_snapshot(scope)` to Terminal that returns a structured representation of terminal state (content, zones, commands, metadata) for AI/LLM consumption and external tooling.

**Architecture:** New `snapshot.rs` module defines `SemanticSnapshot`, `SnapshotScope`, and supporting structs with serde derives. Terminal gets `get_semantic_snapshot()` and `get_semantic_snapshot_json()` methods that assemble the snapshot from existing APIs (`get_zones()`, `get_command_history()`, `get_cwd_changes()`, `export_text()`, etc.). The streaming protocol gains `SnapshotRequest` (client) and `SemanticSnapshot` (server) message variants, with protobuf and Python binding support.

**Tech Stack:** Rust, serde/serde_json, PyO3, Protocol Buffers (prost), tokio/axum streaming

---

### Task 1: Create snapshot data types in `src/terminal/snapshot.rs`

**Files:**
- Create: `src/terminal/snapshot.rs`
- Modify: `src/terminal/mod.rs:11-18` (add `pub mod snapshot;`)

**Step 1: Write failing test for snapshot types**

Add to the bottom of the new file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_scope_default() {
        let scope = SnapshotScope::Visible;
        assert!(matches!(scope, SnapshotScope::Visible));
    }

    #[test]
    fn test_snapshot_scope_recent() {
        let scope = SnapshotScope::Recent(5);
        assert!(matches!(scope, SnapshotScope::Recent(5)));
    }

    #[test]
    fn test_snapshot_serialization() {
        let snapshot = SemanticSnapshot {
            timestamp: 1000,
            cols: 80,
            rows: 24,
            title: "test".to_string(),
            cursor_col: 0,
            cursor_row: 0,
            alt_screen_active: false,
            visible_text: "hello".to_string(),
            scrollback_text: None,
            zones: vec![],
            commands: vec![],
            cwd: None,
            hostname: None,
            username: None,
            cwd_history: vec![],
            scrollback_lines: 0,
            total_zones: 0,
            total_commands: 0,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"cols\":80"));
        assert!(json.contains("\"visible_text\":\"hello\""));
        let deser: SemanticSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.cols, 80);
    }

    #[test]
    fn test_zone_info_serialization() {
        let zone = ZoneInfo {
            id: 1,
            zone_type: "output".to_string(),
            abs_row_start: 10,
            abs_row_end: 20,
            text: "some output".to_string(),
            command: Some("ls".to_string()),
            exit_code: Some(0),
            timestamp: Some(12345),
        };
        let json = serde_json::to_string(&zone).unwrap();
        assert!(json.contains("\"zone_type\":\"output\""));
        assert!(json.contains("\"command\":\"ls\""));
    }

    #[test]
    fn test_command_info_serialization() {
        let cmd = CommandInfo {
            command: "echo hi".to_string(),
            cwd: Some("/home".to_string()),
            start_time: 1000,
            end_time: Some(2000),
            exit_code: Some(0),
            duration_ms: Some(1000),
            success: Some(true),
            output: Some("hi\n".to_string()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"success\":true"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib snapshot::tests -- --nocapture 2>&1 | head -30`
Expected: FAIL (module doesn't exist)

**Step 3: Create the snapshot module with all types**

Create `src/terminal/snapshot.rs`:

```rust
//! Semantic snapshot types for structured terminal state extraction
//!
//! Provides `SemanticSnapshot` and supporting types for capturing a
//! point-in-time view of terminal state suitable for AI/LLM consumption.

use serde::{Deserialize, Serialize};

/// Controls how much terminal history is included in a snapshot
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotScope {
    /// Only the visible screen (no scrollback, no history)
    Visible,
    /// Last N commands with output, plus visible screen
    Recent(usize),
    /// Entire scrollback buffer and all command/zone history
    Full,
}

/// A structured point-in-time view of terminal state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSnapshot {
    /// When this snapshot was taken (Unix epoch milliseconds)
    pub timestamp: u64,
    /// Terminal width in columns
    pub cols: usize,
    /// Terminal height in rows
    pub rows: usize,
    /// Terminal title (from OSC 0/2)
    pub title: String,
    /// Cursor column (0-indexed)
    pub cursor_col: usize,
    /// Cursor row (0-indexed, relative to visible screen)
    pub cursor_row: usize,
    /// Whether the alternate screen buffer is active
    pub alt_screen_active: bool,

    /// Plain text of the visible screen
    pub visible_text: String,
    /// Scrollback text (only for Recent/Full scopes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollback_text: Option<String>,

    /// Semantic zones with text content
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<ZoneInfo>,
    /// Command execution history with output
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandInfo>,

    /// Current working directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Current hostname
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Current username
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// CWD change history
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cwd_history: Vec<CwdChangeInfo>,

    /// Number of scrollback lines currently in buffer
    pub scrollback_lines: usize,
    /// Total zones currently tracked
    pub total_zones: usize,
    /// Total commands in history
    pub total_commands: usize,
}

/// Information about a semantic zone
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneInfo {
    /// Unique zone identifier
    pub id: usize,
    /// Zone type: "prompt", "command", "output"
    pub zone_type: String,
    /// Absolute row where zone starts
    pub abs_row_start: usize,
    /// Absolute row where zone ends (inclusive)
    pub abs_row_end: usize,
    /// Extracted text content of this zone
    pub text: String,
    /// Command text (for command/output zones)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Exit code (for output zones)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// When this zone was created (Unix epoch milliseconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
}

/// Command execution information with output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    /// Command that was executed
    pub command: String,
    /// Working directory when command ran
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// When command started (Unix epoch milliseconds)
    pub start_time: u64,
    /// When command ended (Unix epoch milliseconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<u64>,
    /// Exit code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Whether command succeeded (exit code == 0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// Extracted output text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// CWD change record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwdChangeInfo {
    /// Previous directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_cwd: Option<String>,
    /// New directory
    pub new_cwd: String,
    /// Hostname (if remote)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Username (if provided)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// When the change occurred (Unix epoch milliseconds)
    pub timestamp: u64,
}
```

Then add `pub mod snapshot;` to the submodule declarations in `src/terminal/mod.rs` (after line 17, alongside the other submodules).

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib terminal::snapshot::tests -- --nocapture`
Expected: All 4 tests PASS

**Step 5: Commit**

```bash
git add src/terminal/snapshot.rs src/terminal/mod.rs
git commit -m "feat(snapshot): add semantic snapshot data types with serde serialization"
```

---

### Task 2: Implement `get_semantic_snapshot()` on Terminal

**Files:**
- Modify: `src/terminal/mod.rs` (add methods near line 3600, after `export_text()`)

**Step 1: Write failing test**

Add to `src/terminal/snapshot.rs` tests module:

```rust
#[test]
fn test_terminal_visible_snapshot() {
    use crate::terminal::Terminal;

    let mut term = Terminal::new(80, 24);
    term.process(b"Hello, World!\r\n");
    term.process(b"Second line");

    let snap = term.get_semantic_snapshot(SnapshotScope::Visible);
    assert_eq!(snap.cols, 80);
    assert_eq!(snap.rows, 24);
    assert!(!snap.alt_screen_active);
    assert!(snap.visible_text.contains("Hello, World!"));
    assert!(snap.visible_text.contains("Second line"));
    assert!(snap.scrollback_text.is_none());
    assert!(snap.commands.is_empty());
    assert_eq!(snap.total_commands, 0);
}

#[test]
fn test_terminal_full_snapshot_with_zones_and_commands() {
    use crate::terminal::Terminal;

    let mut term = Terminal::new(80, 24);
    term.set_accept_osc7(true);

    // Simulate shell integration: prompt -> command -> output -> finish
    // OSC 133;A = prompt start
    term.process(b"\x1b]133;A\x07");
    term.process(b"$ ");
    // OSC 133;B = command start
    term.process(b"\x1b]133;B\x07");
    // OSC 133;C = command executed (with command text in preceding input)
    term.process(b"echo hello\r\n");
    term.process(b"\x1b]133;C\x07");
    term.process(b"hello\r\n");
    // OSC 133;D;0 = command finished with exit code 0
    term.process(b"\x1b]133;D;0\x07");

    let snap = term.get_semantic_snapshot(SnapshotScope::Full);
    assert!(snap.total_zones > 0);
    assert!(snap.visible_text.contains("hello"));
}

#[test]
fn test_terminal_snapshot_json() {
    use crate::terminal::Terminal;

    let mut term = Terminal::new(80, 24);
    term.process(b"Test content");

    let json = term.get_semantic_snapshot_json(SnapshotScope::Visible);
    assert!(json.contains("\"cols\":80"));
    assert!(json.contains("Test content"));

    // Verify it's valid JSON by parsing
    let _parsed: SemanticSnapshot = serde_json::from_str(&json).unwrap();
}

#[test]
fn test_terminal_recent_snapshot_limits_commands() {
    use crate::terminal::Terminal;

    let mut term = Terminal::new(80, 24);

    // Simulate 3 commands with shell integration
    for i in 0..3 {
        term.process(b"\x1b]133;A\x07");
        term.process(b"$ ");
        term.process(b"\x1b]133;B\x07");
        term.process(format!("cmd{}\r\n", i).as_bytes());
        term.process(b"\x1b]133;C\x07");
        term.process(format!("output{}\r\n", i).as_bytes());
        term.process(b"\x1b]133;D;0\x07");
    }

    // Recent(1) should only include last command
    let snap = term.get_semantic_snapshot(SnapshotScope::Recent(1));
    assert!(snap.commands.len() <= 1);
    assert_eq!(snap.total_commands, 3); // total_commands reflects full history
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib terminal::snapshot::tests::test_terminal_visible_snapshot -- --nocapture 2>&1 | head -20`
Expected: FAIL — `get_semantic_snapshot` method not found

**Step 3: Implement the methods on Terminal**

In `src/terminal/mod.rs`, add after the `export_text()` method (around line 3603):

```rust
    // ========== Semantic Snapshot Methods ==========

    /// Create a semantic snapshot of the terminal state.
    ///
    /// The snapshot captures terminal content, zone map, command history,
    /// and environment metadata in a structured format suitable for
    /// AI/LLM consumption or external tooling.
    ///
    /// # Arguments
    /// * `scope` - Controls how much history is included:
    ///   - `Visible`: Only the visible screen
    ///   - `Recent(n)`: Last N commands with output + visible screen
    ///   - `Full`: Entire scrollback + all history
    pub fn get_semantic_snapshot(
        &self,
        scope: crate::terminal::snapshot::SnapshotScope,
    ) -> crate::terminal::snapshot::SemanticSnapshot {
        use crate::terminal::snapshot::*;

        let (cols, rows) = self.size();
        let cursor = self.cursor();
        let timestamp = unix_millis();

        // Always include visible text (use content() for visible screen only)
        let visible_text = self.content();

        // Scrollback text depends on scope
        let scrollback_text = match &scope {
            SnapshotScope::Visible => None,
            SnapshotScope::Recent(_) | SnapshotScope::Full => {
                let full = self.export_text();
                // Remove the visible portion to get just scrollback
                if full.len() > visible_text.len() {
                    Some(full)
                } else {
                    None
                }
            }
        };

        // Zones: build ZoneInfo from active zones
        let all_zones = self.get_zones();
        let zones: Vec<ZoneInfo> = match &scope {
            SnapshotScope::Visible => {
                // Only zones that overlap with the visible screen
                let scrollback_len = self.grid.scrollback_len();
                let screen_start = scrollback_len;
                let screen_end = scrollback_len + rows;
                all_zones
                    .iter()
                    .filter(|z| z.abs_row_end >= screen_start && z.abs_row_start < screen_end)
                    .map(|z| self.zone_to_info(z))
                    .collect()
            }
            SnapshotScope::Recent(n) => {
                // Zones related to the last N commands
                let cmd_history = self.get_command_history();
                let recent_start = cmd_history.len().saturating_sub(*n);
                let recent_cmds = &cmd_history[recent_start..];

                // Collect row ranges from recent commands
                let min_row = recent_cmds
                    .iter()
                    .filter_map(|c| c.output_start_row)
                    .min();

                match min_row {
                    Some(min) => all_zones
                        .iter()
                        .filter(|z| z.abs_row_end >= min)
                        .map(|z| self.zone_to_info(z))
                        .collect(),
                    None => {
                        // No recent commands with output rows, return visible zones
                        let scrollback_len = self.grid.scrollback_len();
                        let screen_start = scrollback_len;
                        let screen_end = scrollback_len + rows;
                        all_zones
                            .iter()
                            .filter(|z| {
                                z.abs_row_end >= screen_start && z.abs_row_start < screen_end
                            })
                            .map(|z| self.zone_to_info(z))
                            .collect()
                    }
                }
            }
            SnapshotScope::Full => all_zones.iter().map(|z| self.zone_to_info(z)).collect(),
        };

        // Commands: build CommandInfo from history
        let cmd_history = self.get_command_history();
        let commands: Vec<CommandInfo> = match &scope {
            SnapshotScope::Visible => vec![],
            SnapshotScope::Recent(n) => {
                let start = cmd_history.len().saturating_sub(*n);
                cmd_history[start..]
                    .iter()
                    .map(|c| self.command_to_info(c))
                    .collect()
            }
            SnapshotScope::Full => cmd_history.iter().map(|c| self.command_to_info(c)).collect(),
        };

        // CWD history
        let all_cwd = self.get_cwd_changes();
        let cwd_history: Vec<CwdChangeInfo> = match &scope {
            SnapshotScope::Visible => {
                // Just current CWD as a single entry if available
                vec![]
            }
            SnapshotScope::Recent(n) => {
                let start = all_cwd.len().saturating_sub(*n);
                all_cwd[start..]
                    .iter()
                    .map(|c| CwdChangeInfo {
                        old_cwd: c.old_cwd.clone(),
                        new_cwd: c.new_cwd.clone(),
                        hostname: c.hostname.clone(),
                        username: c.username.clone(),
                        timestamp: c.timestamp,
                    })
                    .collect()
            }
            SnapshotScope::Full => all_cwd
                .iter()
                .map(|c| CwdChangeInfo {
                    old_cwd: c.old_cwd.clone(),
                    new_cwd: c.new_cwd.clone(),
                    hostname: c.hostname.clone(),
                    username: c.username.clone(),
                    timestamp: c.timestamp,
                })
                .collect(),
        };

        // Context from shell integration
        let si = self.shell_integration();
        let cwd = self.current_directory().map(String::from);
        let hostname = si.hostname().map(String::from);
        let username = si.username().map(String::from);

        SemanticSnapshot {
            timestamp,
            cols,
            rows,
            title: self.title().to_string(),
            cursor_col: cursor.col,
            cursor_row: cursor.row,
            alt_screen_active: self.is_alt_screen_active(),
            visible_text,
            scrollback_text,
            zones,
            commands,
            cwd,
            hostname,
            username,
            cwd_history,
            scrollback_lines: self.grid.scrollback_len(),
            total_zones: all_zones.len(),
            total_commands: cmd_history.len(),
        }
    }

    /// Create a semantic snapshot and serialize it as JSON.
    pub fn get_semantic_snapshot_json(
        &self,
        scope: crate::terminal::snapshot::SnapshotScope,
    ) -> String {
        let snapshot = self.get_semantic_snapshot(scope);
        serde_json::to_string(&snapshot).unwrap_or_else(|e| {
            format!("{{\"error\": \"Serialization failed: {}\"}}", e)
        })
    }

    /// Convert a Zone to ZoneInfo, extracting text content
    fn zone_to_info(&self, zone: &crate::zone::Zone) -> crate::terminal::snapshot::ZoneInfo {
        let text = self
            .extract_text_from_row_range(zone.abs_row_start, zone.abs_row_end)
            .unwrap_or_default();
        crate::terminal::snapshot::ZoneInfo {
            id: zone.id,
            zone_type: zone.zone_type.to_string(),
            abs_row_start: zone.abs_row_start,
            abs_row_end: zone.abs_row_end,
            text,
            command: zone.command.clone(),
            exit_code: zone.exit_code,
            timestamp: zone.timestamp,
        }
    }

    /// Convert a CommandExecution to CommandInfo, extracting output text
    fn command_to_info(
        &self,
        cmd: &CommandExecution,
    ) -> crate::terminal::snapshot::CommandInfo {
        let output = match (cmd.output_start_row, cmd.output_end_row) {
            (Some(start), Some(end)) => self.extract_text_from_row_range(start, end),
            _ => None,
        };
        crate::terminal::snapshot::CommandInfo {
            command: cmd.command.clone(),
            cwd: cmd.cwd.clone(),
            start_time: cmd.start_time,
            end_time: cmd.end_time,
            exit_code: cmd.exit_code,
            duration_ms: cmd.duration_ms,
            success: cmd.success,
            output,
        }
    }
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib terminal::snapshot::tests -- --nocapture`
Expected: All tests PASS

**Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20`
Expected: No errors

**Step 6: Commit**

```bash
git add src/terminal/mod.rs
git commit -m "feat(snapshot): implement get_semantic_snapshot() and get_semantic_snapshot_json() on Terminal"
```

---

### Task 3: Add protobuf messages for snapshot request/response

**Files:**
- Modify: `proto/terminal.proto:24-49` (add EVENT_TYPE_SNAPSHOT)
- Modify: `proto/terminal.proto:55-88` (add SemanticSnapshot to ServerMessage)
- Modify: `proto/terminal.proto:367-380` (add SnapshotRequest to ClientMessage)
- Add new messages after existing ones

**Step 1: Add snapshot messages to proto file**

Add `EVENT_TYPE_SNAPSHOT = 23;` to the EventType enum (after `EVENT_TYPE_SUB_SHELL = 22`).

Add new messages before the Client->Server section:

```protobuf
// Semantic snapshot of terminal state
message SemanticSnapshotData {
  string snapshot_json = 1;   // JSON-encoded SemanticSnapshot
}

// Snapshot scope for requests
message SnapshotRequest {
  string scope = 1;                      // "visible", "recent", "full"
  optional uint32 max_commands = 2;      // For "recent" scope, how many commands
}
```

Add to ServerMessage oneof (after `sub_shell_detected = 30`):

```protobuf
    SemanticSnapshotData semantic_snapshot = 31;
```

Add to ClientMessage oneof (after `clipboard = 10`):

```protobuf
    SnapshotRequest snapshot_request = 11;
```

**Step 2: Regenerate protobuf Rust code**

Run: `make proto-rust`
Expected: Generates updated `src/streaming/terminal.pb.rs`

**Step 3: Commit**

```bash
git add proto/terminal.proto src/streaming/terminal.pb.rs
git commit -m "feat(proto): add SemanticSnapshot and SnapshotRequest messages to streaming protocol"
```

---

### Task 4: Add snapshot variants to streaming protocol.rs

**Files:**
- Modify: `src/streaming/protocol.rs:83-477` (add ServerMessage variant)
- Modify: `src/streaming/protocol.rs:480-565` (add ClientMessage variant)
- Modify: `src/streaming/protocol.rs:567-620` (add EventType variant)
- Modify: `src/streaming/protocol.rs:622-1114` (add constructor methods)

**Step 1: Write failing test**

Add to `src/streaming/protocol.rs` tests module (around line 1206):

```rust
    #[test]
    fn test_semantic_snapshot_serialization() {
        let msg = ServerMessage::semantic_snapshot("{\"cols\":80}".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"semantic_snapshot""#));
        assert!(json.contains(r#""snapshot_json":"{\"cols\":80}""#));
    }

    #[test]
    fn test_snapshot_request_serialization() {
        let msg = ClientMessage::snapshot_request("recent".to_string(), Some(5));
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"snapshot_request""#));
        assert!(json.contains(r#""scope":"recent""#));
    }

    #[test]
    fn test_event_type_snapshot_serialization() {
        let events = vec![EventType::Snapshot];
        let json = serde_json::to_string(&events).unwrap();
        assert!(json.contains(r#""snapshot""#));
    }
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::protocol::tests::test_semantic_snapshot -- --nocapture 2>&1 | head -20`
Expected: FAIL

**Step 3: Add the variants**

Add to `ServerMessage` enum (after `SubShellDetected` variant, around line 476):

```rust
    /// Semantic snapshot of terminal state
    #[serde(rename = "semantic_snapshot")]
    SemanticSnapshot {
        /// JSON-encoded SemanticSnapshot struct
        snapshot_json: String,
    },
```

Add to `ClientMessage` enum (after `ClipboardRequest` variant, around line 564):

```rust
    /// Request a semantic snapshot
    #[serde(rename = "snapshot_request")]
    SnapshotRequest {
        /// Scope: "visible", "recent", "full"
        scope: String,
        /// Max commands for "recent" scope
        #[serde(skip_serializing_if = "Option::is_none")]
        max_commands: Option<u32>,
    },
```

Add to `EventType` enum (after `SubShell`, around line 619):

```rust
    /// Semantic snapshot events
    Snapshot,
```

Add constructor methods to `impl ServerMessage` (after `sub_shell_detected`, around line 1091):

```rust
    /// Create a semantic snapshot message
    pub fn semantic_snapshot(snapshot_json: String) -> Self {
        Self::SemanticSnapshot { snapshot_json }
    }
```

Add constructor to `impl ClientMessage` (after `clipboard_request`, around line 1202):

```rust
    /// Create a snapshot request message
    pub fn snapshot_request(scope: String, max_commands: Option<u32>) -> Self {
        Self::SnapshotRequest { scope, max_commands }
    }
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::protocol::tests -- --nocapture`
Expected: All tests PASS

**Step 5: Commit**

```bash
git add src/streaming/protocol.rs
git commit -m "feat(streaming): add SemanticSnapshot and SnapshotRequest to protocol message types"
```

---

### Task 5: Add protobuf conversions in proto.rs

**Files:**
- Modify: `src/streaming/proto.rs` (ServerMessage -> pb and pb -> ServerMessage conversions, ClientMessage conversions, EventType conversions)

**Step 1: Write failing round-trip test**

Add to `src/streaming/proto.rs` tests (find the existing test module):

```rust
    #[test]
    fn test_snapshot_round_trip() {
        let msg = AppServerMessage::semantic_snapshot("{\"cols\":80}".to_string());
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::SemanticSnapshot { snapshot_json } => {
                assert_eq!(snapshot_json, "{\"cols\":80}");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_snapshot_request_round_trip() {
        let msg = AppClientMessage::snapshot_request("recent".to_string(), Some(10));
        let encoded = encode_client_message(&msg).unwrap();
        let decoded = decode_client_message(&encoded).unwrap();
        match decoded {
            AppClientMessage::SnapshotRequest { scope, max_commands } => {
                assert_eq!(scope, "recent");
                assert_eq!(max_commands, Some(10));
            }
            _ => panic!("Wrong message type"),
        }
    }
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::proto::tests::test_snapshot_round_trip -- --nocapture 2>&1 | head -20`
Expected: FAIL — missing match arms

**Step 3: Add conversions**

In the `impl From<&AppServerMessage> for pb::ServerMessage` block, add a match arm for `SemanticSnapshot`:

```rust
AppServerMessage::SemanticSnapshot { snapshot_json } => {
    Some(Message::SemanticSnapshot(pb::SemanticSnapshotData {
        snapshot_json: snapshot_json.clone(),
    }))
}
```

In the `impl TryFrom<pb::ServerMessage> for AppServerMessage` block, add:

```rust
Some(Message::SemanticSnapshot(snap)) => Ok(AppServerMessage::SemanticSnapshot {
    snapshot_json: snap.snapshot_json,
}),
```

In the `impl From<&AppClientMessage> for pb::ClientMessage` block, add:

```rust
AppClientMessage::SnapshotRequest { scope, max_commands } => {
    Some(Message::SnapshotRequest(pb::SnapshotRequest {
        scope: scope.clone(),
        max_commands: max_commands.map(|v| v),
    }))
}
```

In the `impl TryFrom<pb::ClientMessage> for AppClientMessage` block, add:

```rust
Some(Message::SnapshotRequest(req)) => Ok(AppClientMessage::SnapshotRequest {
    scope: req.scope,
    max_commands: req.max_commands,
}),
```

In the EventType conversion (both `From<AppEventType> for pb::EventType` and `From<pb::EventType> for AppEventType`), add:

```rust
AppEventType::Snapshot => pb::EventType::Snapshot,
// and reverse:
pb::EventType::Snapshot => AppEventType::Snapshot,
```

(Use the correct `i32` value `23` for the proto EventType if the generated code uses integer mapping.)

**Step 4: Run tests**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::proto::tests -- --nocapture`
Expected: All tests PASS

**Step 5: Commit**

```bash
git add src/streaming/proto.rs
git commit -m "feat(streaming): add protobuf conversion for snapshot messages"
```

---

### Task 6: Handle SnapshotRequest in streaming server

**Files:**
- Modify: `src/streaming/server.rs` (3 handler locations where RequestRefresh is handled)

**Step 1: Add snapshot request handling**

In `src/streaming/server.rs`, there are 3 places where `ClientMessage::RequestRefresh` is handled (around lines 1895, 2269, 2628). After each `RequestRefresh` handler, add a similar handler for `SnapshotRequest`:

```rust
crate::streaming::protocol::ClientMessage::SnapshotRequest { scope, max_commands } => {
    let snapshot_msg = {
        if let Ok(terminal) = Ok::<_, ()>(terminal_for_refresh.lock()) {
            use crate::terminal::snapshot::SnapshotScope;
            let snapshot_scope = match scope.as_str() {
                "recent" => SnapshotScope::Recent(max_commands.unwrap_or(10) as usize),
                "full" => SnapshotScope::Full,
                _ => SnapshotScope::Visible,
            };
            let json = terminal.get_semantic_snapshot_json(snapshot_scope);
            Some(ServerMessage::semantic_snapshot(json))
        } else {
            None
        }
    };
    if let Some(msg) = snapshot_msg {
        // Send using the same pattern as refresh (client.send or ws_tx.send)
    }
}
```

Each of the 3 locations uses a slightly different send mechanism:
1. Line ~1895 (plain WS): uses `client.send(msg).await`
2. Line ~2269 (TLS WS): uses `ws_tx.send(Message::Binary(bytes.into())).await`
3. Line ~2628 (axum WS): uses `ws_tx.send(AxumMessage::Binary(bytes.into())).await`

Copy the exact send pattern from the adjacent `RequestRefresh` handler for each location.

**Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20`
Expected: No errors

**Step 3: Commit**

```bash
git add src/streaming/server.rs
git commit -m "feat(streaming): handle SnapshotRequest in all WebSocket handler paths"
```

---

### Task 7: Add Python bindings for semantic snapshot

**Files:**
- Modify: `src/python_bindings/terminal.rs` (add methods after `get_zone_text`)

**Step 1: Add Python methods**

After the `get_zone_text` method in `src/python_bindings/terminal.rs` (around line 5838), add:

```rust
    /// Get a semantic snapshot of the terminal state as a Python dict.
    ///
    /// Returns a structured representation of terminal state including
    /// content, zones, commands, and environment metadata, suitable
    /// for AI/LLM consumption.
    ///
    /// Args:
    ///     scope: Snapshot scope - "visible", "recent", or "full" (default: "visible")
    ///     max_commands: For "recent" scope, max number of commands to include (default: 10)
    ///
    /// Returns:
    ///     dict with keys: timestamp, cols, rows, title, cursor_col, cursor_row,
    ///     alt_screen_active, visible_text, scrollback_text, zones, commands,
    ///     cwd, hostname, username, cwd_history, scrollback_lines, total_zones,
    ///     total_commands
    ///
    /// Example:
    ///     >>> term = Terminal(80, 24)
    ///     >>> term.process(b"Hello")
    ///     >>> snap = term.get_semantic_snapshot(scope="visible")
    ///     >>> snap["cols"]
    ///     80
    #[pyo3(signature = (scope="visible", max_commands=10))]
    fn get_semantic_snapshot(
        &self,
        scope: &str,
        max_commands: usize,
    ) -> PyResult<pyo3::Py<pyo3::types::PyDict>> {
        use crate::terminal::snapshot::SnapshotScope;

        let snapshot_scope = match scope {
            "recent" => SnapshotScope::Recent(max_commands),
            "full" => SnapshotScope::Full,
            "visible" => SnapshotScope::Visible,
            _ => {
                return Err(PyValueError::new_err(
                    "scope must be 'visible', 'recent', or 'full'",
                ))
            }
        };

        let snapshot = self.inner.get_semantic_snapshot(snapshot_scope);

        // Serialize to JSON then parse into Python dict (simplest approach)
        let json = serde_json::to_string(&snapshot)
            .map_err(|e| PyRuntimeError::new_err(format!("Serialization failed: {}", e)))?;

        Python::attach(|py| {
            let json_module = py.import("json")?;
            let dict = json_module.call_method1("loads", (json,))?;
            Ok(dict.into())
        })
    }

    /// Get a semantic snapshot of the terminal state as a JSON string.
    ///
    /// This is more efficient than get_semantic_snapshot() when you need
    /// the data as a string (e.g., for sending to an LLM API).
    ///
    /// Args:
    ///     scope: Snapshot scope - "visible", "recent", or "full" (default: "visible")
    ///     max_commands: For "recent" scope, max number of commands to include (default: 10)
    ///
    /// Returns:
    ///     JSON string containing the semantic snapshot
    ///
    /// Example:
    ///     >>> term = Terminal(80, 24)
    ///     >>> json_str = term.get_semantic_snapshot_json(scope="full")
    #[pyo3(signature = (scope="visible", max_commands=10))]
    fn get_semantic_snapshot_json(
        &self,
        scope: &str,
        max_commands: usize,
    ) -> PyResult<String> {
        use crate::terminal::snapshot::SnapshotScope;

        let snapshot_scope = match scope {
            "recent" => SnapshotScope::Recent(max_commands),
            "full" => SnapshotScope::Full,
            "visible" => SnapshotScope::Visible,
            _ => {
                return Err(PyValueError::new_err(
                    "scope must be 'visible', 'recent', or 'full'",
                ))
            }
        };

        Ok(self.inner.get_semantic_snapshot_json(snapshot_scope))
    }
```

**Step 2: Build with maturin**

Run: `make dev`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/python_bindings/terminal.rs
git commit -m "feat(python): add get_semantic_snapshot() and get_semantic_snapshot_json() bindings"
```

---

### Task 8: Add Python bindings for streaming snapshot events

**Files:**
- Modify: `src/python_bindings/streaming.rs` (add snapshot handling to message creation and event type mapping)

**Step 1: Add snapshot message creation**

Find the match block where server messages are constructed from string types (around the "sub_shell" arm). Add after it:

```rust
"semantic_snapshot" => {
    let snapshot_json = get_str("snapshot_json").unwrap_or_else(|| "{}".to_string());
    ServerMessage::semantic_snapshot(snapshot_json)
}
```

Find where event types are mapped to strings and add:

```rust
"snapshot" => EventType::Snapshot,
// and reverse mapping if present:
EventType::Snapshot => "snapshot",
```

**Step 2: Build with maturin**

Run: `make dev`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/python_bindings/streaming.rs
git commit -m "feat(python): add snapshot event type and message support to streaming bindings"
```

---

### Task 9: Add Python integration tests

**Files:**
- Create: `tests/test_semantic_snapshot.py`

**Step 1: Write tests**

```python
"""Integration tests for the semantic snapshot API."""
import json

from par_term_emu_core_rust import Terminal


def test_visible_snapshot_basic():
    """Visible snapshot returns current screen content."""
    term = Terminal(80, 24)
    term.process(b"Hello, World!\r\n")
    term.process(b"Second line")

    snap = term.get_semantic_snapshot(scope="visible")
    assert snap["cols"] == 80
    assert snap["rows"] == 24
    assert snap["alt_screen_active"] is False
    assert "Hello, World!" in snap["visible_text"]
    assert "Second line" in snap["visible_text"]
    assert snap["scrollback_text"] is None
    assert snap["commands"] == []
    assert snap["total_commands"] == 0
    assert isinstance(snap["timestamp"], int)
    assert snap["timestamp"] > 0


def test_visible_snapshot_no_history():
    """Visible scope excludes command history and CWD history."""
    term = Terminal(80, 24)
    snap = term.get_semantic_snapshot(scope="visible")
    assert snap["commands"] == []
    assert snap["cwd_history"] == []


def test_full_snapshot_includes_all():
    """Full scope includes all available data."""
    term = Terminal(80, 24)
    term.process(b"Some content\r\n")
    snap = term.get_semantic_snapshot(scope="full")
    assert snap["cols"] == 80
    assert snap["rows"] == 24
    assert "Some content" in snap["visible_text"]


def test_recent_snapshot_with_commands():
    """Recent scope includes only last N commands."""
    term = Terminal(80, 24)
    term.set_accept_osc7(True)

    # Simulate 3 commands
    for i in range(3):
        term.process(b"\x1b]133;A\x07")  # prompt start
        term.process(b"$ ")
        term.process(b"\x1b]133;B\x07")  # command start
        term.process(f"cmd{i}\r\n".encode())
        term.process(b"\x1b]133;C\x07")  # command executed
        term.process(f"output{i}\r\n".encode())
        term.process(b"\x1b]133;D;0\x07")  # command finished

    snap = term.get_semantic_snapshot(scope="recent", max_commands=1)
    assert len(snap["commands"]) <= 1
    assert snap["total_commands"] == 3


def test_snapshot_json_string():
    """JSON string output is valid JSON matching dict output."""
    term = Terminal(80, 24)
    term.process(b"Test content")

    json_str = term.get_semantic_snapshot_json(scope="visible")
    parsed = json.loads(json_str)
    assert parsed["cols"] == 80
    assert "Test content" in parsed["visible_text"]


def test_snapshot_json_matches_dict():
    """JSON string and dict output contain the same data."""
    term = Terminal(80, 24)
    term.process(b"Compare me\r\n")

    snap_dict = term.get_semantic_snapshot(scope="visible")
    snap_json = term.get_semantic_snapshot_json(scope="visible")
    parsed = json.loads(snap_json)

    assert snap_dict["cols"] == parsed["cols"]
    assert snap_dict["rows"] == parsed["rows"]
    assert snap_dict["visible_text"] == parsed["visible_text"]


def test_snapshot_with_title():
    """Snapshot captures terminal title."""
    term = Terminal(80, 24)
    term.process(b"\x1b]0;My Title\x07")
    snap = term.get_semantic_snapshot(scope="visible")
    assert snap["title"] == "My Title"


def test_snapshot_cursor_position():
    """Snapshot captures cursor position."""
    term = Terminal(80, 24)
    term.process(b"Hello")  # Cursor at col 5, row 0
    snap = term.get_semantic_snapshot(scope="visible")
    assert snap["cursor_col"] == 5
    assert snap["cursor_row"] == 0


def test_snapshot_invalid_scope():
    """Invalid scope raises ValueError."""
    term = Terminal(80, 24)
    try:
        term.get_semantic_snapshot(scope="invalid")
        assert False, "Should have raised ValueError"
    except ValueError as e:
        assert "scope must be" in str(e)


def test_snapshot_json_invalid_scope():
    """Invalid scope in JSON method raises ValueError."""
    term = Terminal(80, 24)
    try:
        term.get_semantic_snapshot_json(scope="invalid")
        assert False, "Should have raised ValueError"
    except ValueError as e:
        assert "scope must be" in str(e)


def test_snapshot_alt_screen():
    """Snapshot reflects alternate screen state."""
    term = Terminal(80, 24)
    term.process(b"Primary content")
    snap_primary = term.get_semantic_snapshot(scope="visible")
    assert snap_primary["alt_screen_active"] is False

    # Switch to alt screen
    term.process(b"\x1b[?1049h")
    snap_alt = term.get_semantic_snapshot(scope="visible")
    assert snap_alt["alt_screen_active"] is True
```

**Step 2: Run tests**

Run: `make dev && uv run pytest tests/test_semantic_snapshot.py -v`
Expected: All tests PASS

**Step 3: Commit**

```bash
git add tests/test_semantic_snapshot.py
git commit -m "test(python): add integration tests for semantic snapshot API"
```

---

### Task 10: Add Rust integration tests for streaming snapshot

**Files:**
- Modify: `tests/test_streaming.rs` (add snapshot message tests)

**Step 1: Add streaming snapshot tests**

Add tests to the existing `tests/test_streaming.rs` file:

```rust
#[test]
fn test_snapshot_message_constructors() {
    let msg = ServerMessage::semantic_snapshot("{\"cols\":80}".to_string());
    match msg {
        ServerMessage::SemanticSnapshot { snapshot_json } => {
            assert_eq!(snapshot_json, "{\"cols\":80}");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_snapshot_request_constructor() {
    let msg = ClientMessage::snapshot_request("recent".to_string(), Some(5));
    match msg {
        ClientMessage::SnapshotRequest { scope, max_commands } => {
            assert_eq!(scope, "recent");
            assert_eq!(max_commands, Some(5));
        }
        _ => panic!("Wrong message type"),
    }
}
```

**Step 2: Run tests**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming test_snapshot_message -- --nocapture`
Expected: PASS

**Step 3: Commit**

```bash
git add tests/test_streaming.rs
git commit -m "test: add streaming snapshot message tests"
```

---

### Task 11: Regenerate TypeScript proto and rebuild web frontend

**Files:**
- Generated: TypeScript proto files and web build

**Step 1: Regenerate TypeScript**

Run: `make proto-typescript`
Expected: TypeScript proto files regenerated

**Step 2: Rebuild web frontend**

Run: `make web-build-static`
Expected: Static web build succeeds

**Step 3: Commit**

```bash
git add -A
git commit -m "chore: regenerate TypeScript proto and rebuild web frontend"
```

---

### Task 12: Run full verification and update docs

**Files:**
- Modify: `docs/API_REFERENCE.md` (add snapshot methods)
- Modify: `README.md` (mention snapshot in features if appropriate)

**Step 1: Run full checks**

Run: `make checkall`
Expected: All checks pass (fmt, lint, clippy, pyright, tests)

**Step 2: Update API_REFERENCE.md**

Add a new section for the semantic snapshot methods under the Terminal API:

```markdown
### Semantic Snapshot

#### `get_semantic_snapshot(scope="visible", max_commands=10)`
Returns a structured snapshot of terminal state as a Python dict.

**Args:**
- `scope` (str): `"visible"` (screen only), `"recent"` (last N commands + screen), or `"full"` (all history)
- `max_commands` (int): For "recent" scope, maximum commands to include (default: 10)

**Returns:** dict with keys: `timestamp`, `cols`, `rows`, `title`, `cursor_col`, `cursor_row`, `alt_screen_active`, `visible_text`, `scrollback_text`, `zones`, `commands`, `cwd`, `hostname`, `username`, `cwd_history`, `scrollback_lines`, `total_zones`, `total_commands`

#### `get_semantic_snapshot_json(scope="visible", max_commands=10)`
Returns the same snapshot data as a JSON string. More efficient when the data will be forwarded as a string (e.g., to an LLM API).
```

**Step 3: Commit**

```bash
git add docs/API_REFERENCE.md README.md
git commit -m "docs: add semantic snapshot API to reference documentation"
```

---

### Task 13: Create PR

**Step 1: Push branch and create PR**

```bash
git push -u origin feat/semantic-snapshot-api
gh pr create --title "feat: AI Terminal Inspection - structured semantic snapshot API" --body "$(cat <<'EOF'
## Summary
- Adds `get_semantic_snapshot(scope)` and `get_semantic_snapshot_json(scope)` to Terminal
- Three scope levels: Visible (screen only), Recent(N) (last N commands), Full (all history)
- Returns structured data: content, zones, commands, CWD history, metadata
- Streaming protocol: SnapshotRequest (client) and SemanticSnapshot (server) messages
- Python bindings with dict and JSON string output
- Protobuf support for binary transport

Closes #38

## Test plan
- [ ] `make checkall` passes
- [ ] Rust unit tests for snapshot types, serialization, and Terminal methods
- [ ] Rust integration tests for streaming protocol round-trips
- [ ] Python integration tests for dict/JSON output, scopes, edge cases
- [ ] Protobuf encode/decode round-trip tests
EOF
)"
```
