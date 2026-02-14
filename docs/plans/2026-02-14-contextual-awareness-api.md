# Contextual Awareness API Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add granular TerminalEvent variants for zone lifecycle, environment changes, remote host transitions, and sub-shell detection.

**Architecture:** New event variants coexist alongside existing ShellIntegrationEvent. Zone events emit from OSC 133 handlers after zone push/close. Environment/host events derive from OSC 7 and OSC 1337 RemoteHost signals. Grid eviction returns evicted zones so Terminal can emit ZoneScrolledOut. All events flow through Python bindings, streaming protocol, and proto conversion layers.

**Tech Stack:** Rust, PyO3, Protocol Buffers (prost), tokio/axum streaming

---

### Task 1: Add zone_id to Zone struct and Terminal tracking state

**Files:**
- Modify: `src/zone.rs:32-60`
- Modify: `src/terminal/mod.rs` (Terminal struct fields, near line 1035+)

**Step 1: Add `id` field to Zone struct**

In `src/zone.rs`, add `pub id: usize` to the `Zone` struct and update `Zone::new()`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// Unique zone ID (monotonically increasing)
    pub id: usize,
    /// Type of this zone
    pub zone_type: ZoneType,
    // ... rest unchanged
}

impl Zone {
    pub fn new(id: usize, zone_type: ZoneType, abs_row: usize, timestamp: Option<u64>) -> Self {
        Self {
            id,
            zone_type,
            abs_row_start: abs_row,
            abs_row_end: abs_row,
            command: None,
            exit_code: None,
            timestamp,
        }
    }
}
```

**Step 2: Fix all Zone::new() call sites**

Every call to `Zone::new(zone_type, abs_row, timestamp)` must be updated to `Zone::new(id, zone_type, abs_row, timestamp)`. The `id` comes from `self.next_zone_id` on Terminal.

Call sites are in `src/terminal/sequences/osc.rs` lines 472, 498, 527 (OSC 133 A/B/C handlers).

**Step 3: Add tracking fields to Terminal struct**

Find the Terminal struct field declarations (near `terminal_events: Vec<TerminalEvent>`) and add:

```rust
/// Monotonically increasing zone ID counter
next_zone_id: usize,
/// Last known hostname for detecting transitions
last_hostname: Option<String>,
/// Last known username for detecting transitions
last_username: Option<String>,
/// Current shell nesting depth (for sub-shell detection)
shell_depth: usize,
```

Initialize all to `0`/`None` in `Terminal::new()` and `Terminal::reset()`.

**Step 4: Fix zone tests in src/zone.rs**

Update all test calls from `Zone::new(ZoneType, row, ts)` to `Zone::new(0, ZoneType, row, ts)`.

**Step 5: Run tests to verify compilation**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize zone`
Expected: PASS (zone tests compile and pass with new id field)

**Step 6: Commit**

```bash
git add src/zone.rs src/terminal/mod.rs src/terminal/sequences/osc.rs
git commit -m "feat(zones): add zone_id and Terminal tracking state for contextual awareness"
```

---

### Task 2: Add new TerminalEvent variants and TerminalEventKind entries

**Files:**
- Modify: `src/terminal/mod.rs:94-180` (TerminalEvent enum, TerminalEventKind enum)
- Modify: `src/terminal/mod.rs:2981-2996` (event_kind helper)

**Step 1: Add new variants to TerminalEvent enum (after ShellIntegrationEvent)**

```rust
/// A zone was opened (prompt, command, or output block started)
ZoneOpened {
    /// Unique zone identifier
    zone_id: usize,
    /// Type of zone
    zone_type: crate::zone::ZoneType,
    /// Absolute row where zone starts
    abs_row_start: usize,
},
/// A zone was closed (prompt, command, or output block ended)
ZoneClosed {
    /// Unique zone identifier
    zone_id: usize,
    /// Type of zone
    zone_type: crate::zone::ZoneType,
    /// Absolute row where zone starts
    abs_row_start: usize,
    /// Absolute row where zone ends
    abs_row_end: usize,
    /// Exit code (for output zones only)
    exit_code: Option<i32>,
},
/// A zone was evicted from scrollback
ZoneScrolledOut {
    /// Unique zone identifier
    zone_id: usize,
    /// Type of zone that was evicted
    zone_type: crate::zone::ZoneType,
},
/// An environment variable changed (CWD, hostname, username)
EnvironmentChanged {
    /// The key that changed ("cwd", "hostname", "username")
    key: String,
    /// The new value
    value: String,
    /// The previous value (if any)
    old_value: Option<String>,
},
/// Remote host transition detected (hostname changed)
RemoteHostTransition {
    /// New hostname
    hostname: String,
    /// New username (if known)
    username: Option<String>,
    /// Previous hostname (if any)
    old_hostname: Option<String>,
    /// Previous username (if any)
    old_username: Option<String>,
},
/// Sub-shell detected (shell nesting depth changed)
SubShellDetected {
    /// Current shell nesting depth
    depth: usize,
    /// Shell type if known (e.g., "bash", "zsh")
    shell_type: Option<String>,
},
```

**Step 2: Add corresponding TerminalEventKind entries**

```rust
pub enum TerminalEventKind {
    // ... existing variants ...
    ZoneOpened,
    ZoneClosed,
    ZoneScrolledOut,
    EnvironmentChanged,
    RemoteHostTransition,
    SubShellDetected,
}
```

**Step 3: Update event_kind() helper (src/terminal/mod.rs:2981)**

Add the new arms:

```rust
TerminalEvent::ZoneOpened { .. } => TerminalEventKind::ZoneOpened,
TerminalEvent::ZoneClosed { .. } => TerminalEventKind::ZoneClosed,
TerminalEvent::ZoneScrolledOut { .. } => TerminalEventKind::ZoneScrolledOut,
TerminalEvent::EnvironmentChanged { .. } => TerminalEventKind::EnvironmentChanged,
TerminalEvent::RemoteHostTransition { .. } => TerminalEventKind::RemoteHostTransition,
TerminalEvent::SubShellDetected { .. } => TerminalEventKind::SubShellDetected,
```

**Step 4: Run compilation check**

Run: `cargo check --no-default-features --features pyo3/auto-initialize`
Expected: Compile errors in Python bindings (non-exhaustive match) — this is expected, we fix those in Task 5.

**Step 5: Commit**

```bash
git add src/terminal/mod.rs
git commit -m "feat(events): add zone lifecycle, environment, host transition, and sub-shell event variants"
```

---

### Task 3: Emit zone events from OSC 133 handlers and grid eviction

**Files:**
- Modify: `src/terminal/sequences/osc.rs:449-570` (OSC 133 A/B/C/D handlers)
- Modify: `src/grid.rs:1915-1922` (evict_zones method)
- Modify: `src/terminal/mod.rs` (scroll_up zone eviction call site)

**Step 1: Emit ZoneOpened after each push_zone() call**

In the OSC 133 handler (osc.rs), after each `self.grid.push_zone(zone)` call, emit a ZoneOpened event.

For marker 'A' (~line 472-476):
```rust
// After: self.grid.push_zone(...)
let zone_id = self.next_zone_id;
self.next_zone_id += 1;
// Create zone with id
let zone = crate::zone::Zone::new(zone_id, crate::zone::ZoneType::Prompt, abs_line, Some(ts));
self.grid.push_zone(zone);
self.terminal_events.push(TerminalEvent::ZoneOpened {
    zone_id,
    zone_type: crate::zone::ZoneType::Prompt,
    abs_row_start: abs_line,
});
```

Same pattern for markers 'B' and 'C' (with Command and Output zone types respectively).

**Step 2: Emit ZoneClosed before each push_zone() and at marker 'D'**

Before closing a zone with `self.grid.close_current_zone(close_row)`, check if there is a current zone and emit ZoneClosed:

```rust
if let Some(zone) = self.grid.zones().last() {
    let zone_id = zone.id;
    let zone_type = zone.zone_type;
    let abs_row_start = zone.abs_row_start;
    self.grid.close_current_zone(close_row);
    self.terminal_events.push(TerminalEvent::ZoneClosed {
        zone_id,
        zone_type,
        abs_row_start,
        abs_row_end: close_row,
        exit_code: None,
    });
} else {
    self.grid.close_current_zone(close_row);
}
```

For marker 'D', the exit code is available:
```rust
// After closing and setting exit code on the Output zone:
self.terminal_events.push(TerminalEvent::ZoneClosed {
    zone_id: zone.id,
    zone_type: crate::zone::ZoneType::Output,
    abs_row_start: zone.abs_row_start,
    abs_row_end: abs_line,
    exit_code: parsed_code,
});
```

**Step 3: Make evict_zones return evicted zones**

Change `grid.rs:evict_zones` to return the evicted zones:

```rust
pub fn evict_zones(&mut self, floor: usize) -> Vec<Zone> {
    let mut evicted = Vec::new();
    let mut retained = Vec::new();
    for zone in std::mem::take(&mut self.zones) {
        if zone.abs_row_end < floor {
            evicted.push(zone);
        } else {
            retained.push(zone);
        }
    }
    // Clamp remaining zones
    for zone in &mut retained {
        if zone.abs_row_start < floor {
            zone.abs_row_start = floor;
        }
    }
    self.zones = retained;
    evicted
}
```

**Step 4: Emit ZoneScrolledOut from Terminal when zones are evicted**

Find where `evict_zones()` is called. Search for `evict_zones` in `src/grid.rs` (it's called within `scroll_region_up`). The evicted zones need to be plumbed back. Since `evict_zones` is on Grid and Terminal holds the event queue, we need to either:
- Return evicted zones from the grid method and have Terminal process them, OR
- Collect evicted zones during scroll operations

The simplest approach: change `evict_zones` to return `Vec<Zone>`, and wherever `evict_zones()` is called within Grid's scroll methods, store the evicted zones in a new `pub evicted_zones: Vec<Zone>` field on Grid. Then in Terminal, after any operation that might scroll, drain `grid.evicted_zones` and emit events.

Actually, the cleanest approach: add a `pub evicted_zones: Vec<Zone>` field to Grid. In `evict_zones()`, push evicted zones there. In Terminal's `poll_events()` or after process calls, drain them.

Better yet: in `evict_zones`, append to `self.evicted_zones`. Then in Terminal, after `process()` or in `poll_events()`, drain grid's evicted zones and emit ZoneScrolledOut events.

Add to Grid struct: `pub evicted_zones: Vec<Zone>` (initialized to empty Vec).

In `evict_zones`:
```rust
pub fn evict_zones(&mut self, floor: usize) {
    let before_len = self.zones.len();
    // Collect evicted before retain
    for zone in &self.zones {
        if zone.abs_row_end < floor {
            self.evicted_zones.push(zone.clone());
        }
    }
    self.zones.retain(|z| z.abs_row_end >= floor);
    for zone in &mut self.zones {
        if zone.abs_row_start < floor {
            zone.abs_row_start = floor;
        }
    }
}
```

In Terminal's `poll_events()`, before returning, drain evicted zones:
```rust
pub fn poll_events(&mut self) -> Vec<TerminalEvent> {
    // Drain any evicted zones from grid into events
    for zone in self.active_grid_mut().drain_evicted_zones() {
        self.terminal_events.push(TerminalEvent::ZoneScrolledOut {
            zone_id: zone.id,
            zone_type: zone.zone_type,
        });
    }
    std::mem::take(&mut self.terminal_events)
}
```

Add to Grid:
```rust
pub fn drain_evicted_zones(&mut self) -> Vec<Zone> {
    std::mem::take(&mut self.evicted_zones)
}
```

**Step 5: Run tests**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize`
Expected: Some tests may fail due to Python binding non-exhaustive match (expected, fixed in Task 5). Zone/grid tests should pass.

**Step 6: Commit**

```bash
git add src/terminal/sequences/osc.rs src/grid.rs src/terminal/mod.rs
git commit -m "feat(events): emit zone lifecycle events from OSC 133 handlers and grid eviction"
```

---

### Task 4: Emit environment, remote host, and sub-shell events

**Files:**
- Modify: `src/terminal/mod.rs:6299-6340` (record_cwd_change)
- Modify: `src/terminal/sequences/osc.rs:934-990` (handle_remote_host)
- Modify: `src/terminal/sequences/osc.rs:456-477` (OSC 133 A handler for shell depth)

**Step 1: Emit EnvironmentChanged + RemoteHostTransition from record_cwd_change**

In `record_cwd_change` (mod.rs:6299), after the existing `CwdChanged` event push, add:

```rust
// Emit granular environment events
if old_cwd.as_deref() != Some(&new_cwd) {
    self.terminal_events.push(TerminalEvent::EnvironmentChanged {
        key: "cwd".to_string(),
        value: new_cwd.clone(),
        old_value: old_cwd.clone(),
    });
}

if old_hostname.as_deref() != hostname.as_deref() {
    if let Some(h) = &hostname {
        self.terminal_events.push(TerminalEvent::EnvironmentChanged {
            key: "hostname".to_string(),
            value: h.clone(),
            old_value: old_hostname.clone(),
        });
    }
}

if old_username.as_deref() != username.as_deref() {
    if let Some(u) = &username {
        self.terminal_events.push(TerminalEvent::EnvironmentChanged {
            key: "username".to_string(),
            value: u.clone(),
            old_value: old_username.clone(),
        });
    }
}

// Emit RemoteHostTransition if hostname changed
if old_hostname.as_deref() != hostname.as_deref() {
    if let Some(h) = &hostname {
        self.terminal_events.push(TerminalEvent::RemoteHostTransition {
            hostname: h.clone(),
            username: username.clone(),
            old_hostname: old_hostname.clone(),
            old_username: old_username.clone(),
        });
    } else if old_hostname.is_some() {
        // Returned to local - emit with empty hostname
        self.terminal_events.push(TerminalEvent::RemoteHostTransition {
            hostname: String::new(),
            username: username.clone(),
            old_hostname: old_hostname.clone(),
            old_username: old_username.clone(),
        });
    }
}
```

**Step 2: Emit RemoteHostTransition from handle_remote_host (OSC 1337)**

In `handle_remote_host` (osc.rs:934), before the existing `record_cwd_change` call, capture old values and after the call emit a RemoteHostTransition if the hostname changed. However, since `handle_remote_host` already calls `record_cwd_change` which now emits these events, this may already be covered. Verify by reading the flow: `handle_remote_host` calls `self.record_cwd_change(current_cwd, resolved_hostname, username)`. Since `record_cwd_change` now emits `RemoteHostTransition` when hostname changes, this is already handled.

**Step 3: Track shell depth for SubShellDetected**

In the OSC 133 marker 'A' handler (prompt_start), increment shell depth tracking:

```rust
Some('A') => {
    // Track shell nesting depth
    self.shell_depth += 1;
    if self.shell_depth > 1 {
        self.terminal_events.push(TerminalEvent::SubShellDetected {
            depth: self.shell_depth,
            shell_type: None, // Could be derived from user vars if available
        });
    }
    // ... existing code ...
}
```

Wait — this isn't quite right. Shell depth should track nesting, not just prompt count. The heuristic: when we see a `prompt_start` (marker A) without a preceding `command_finished` (marker D) for the same depth, it indicates a new sub-shell.

Simpler approach: track depth as number of active prompt-command cycles. When a new prompt_start arrives while the previous cycle is incomplete (no matching command_finished), depth increases. When command_finished arrives, depth decreases (if > 1).

Actually, the simplest viable heuristic: when the shell integration markers reset (we get an 'A' without a preceding 'D'), the shell might have changed. But this is normal flow too. Let's keep it simple for initial implementation:

- `shell_depth` starts at 0
- On first prompt_start ('A'): set to 1
- Track whether we're in an "active command" (between 'C' and 'D')
- If we get a new 'A' while in an active command, increment depth
- When 'D' fires and depth > 1, decrement depth

This is a basic heuristic. Let's implement it with a `in_command_output: bool` flag:

In Terminal struct, add: `in_command_output: bool`

```rust
Some('A') => {
    // Sub-shell detection heuristic:
    // If we receive prompt_start while still in command output,
    // the shell has spawned a sub-shell
    if self.in_command_output && self.shell_depth > 0 {
        self.shell_depth += 1;
        self.terminal_events.push(TerminalEvent::SubShellDetected {
            depth: self.shell_depth,
            shell_type: None,
        });
    } else if self.shell_depth == 0 {
        self.shell_depth = 1;
    }
    self.in_command_output = false;
    // ... existing code ...
}
Some('C') => {
    self.in_command_output = true;
    // ... existing code ...
}
Some('D') => {
    self.in_command_output = false;
    if self.shell_depth > 1 {
        self.shell_depth -= 1;
        self.terminal_events.push(TerminalEvent::SubShellDetected {
            depth: self.shell_depth,
            shell_type: None,
        });
    }
    // ... existing code ...
}
```

**Step 4: Run compilation check**

Run: `cargo check --no-default-features --features pyo3/auto-initialize`

**Step 5: Commit**

```bash
git add src/terminal/mod.rs src/terminal/sequences/osc.rs
git commit -m "feat(events): emit environment, remote host transition, and sub-shell events"
```

---

### Task 5: Update Python bindings for new events

**Files:**
- Modify: `src/python_bindings/terminal.rs:2585-2736` (poll_events match arms)
- Modify: `src/python_bindings/terminal.rs:2746-2768` (set_event_subscription match arms)

**Step 1: Add new event dict conversions in poll_events()**

After the `ShellIntegrationEvent` arm (~line 2731), add:

```rust
TerminalEvent::ZoneOpened { zone_id, zone_type, abs_row_start } => {
    map.insert("type".to_string(), "zone_opened".to_string());
    map.insert("zone_id".to_string(), zone_id.to_string());
    map.insert("zone_type".to_string(), zone_type.to_string());
    map.insert("abs_row_start".to_string(), abs_row_start.to_string());
}
TerminalEvent::ZoneClosed { zone_id, zone_type, abs_row_start, abs_row_end, exit_code } => {
    map.insert("type".to_string(), "zone_closed".to_string());
    map.insert("zone_id".to_string(), zone_id.to_string());
    map.insert("zone_type".to_string(), zone_type.to_string());
    map.insert("abs_row_start".to_string(), abs_row_start.to_string());
    map.insert("abs_row_end".to_string(), abs_row_end.to_string());
    if let Some(code) = exit_code {
        map.insert("exit_code".to_string(), code.to_string());
    }
}
TerminalEvent::ZoneScrolledOut { zone_id, zone_type } => {
    map.insert("type".to_string(), "zone_scrolled_out".to_string());
    map.insert("zone_id".to_string(), zone_id.to_string());
    map.insert("zone_type".to_string(), zone_type.to_string());
}
TerminalEvent::EnvironmentChanged { key, value, old_value } => {
    map.insert("type".to_string(), "environment_changed".to_string());
    map.insert("key".to_string(), key.clone());
    map.insert("value".to_string(), value.clone());
    if let Some(old) = old_value {
        map.insert("old_value".to_string(), old.clone());
    }
}
TerminalEvent::RemoteHostTransition { hostname, username, old_hostname, old_username } => {
    map.insert("type".to_string(), "remote_host_transition".to_string());
    map.insert("hostname".to_string(), hostname.clone());
    if let Some(u) = username {
        map.insert("username".to_string(), u.clone());
    }
    if let Some(oh) = old_hostname {
        map.insert("old_hostname".to_string(), oh.clone());
    }
    if let Some(ou) = old_username {
        map.insert("old_username".to_string(), ou.clone());
    }
}
TerminalEvent::SubShellDetected { depth, shell_type } => {
    map.insert("type".to_string(), "sub_shell_detected".to_string());
    map.insert("depth".to_string(), depth.to_string());
    if let Some(st) = shell_type {
        map.insert("shell_type".to_string(), st.clone());
    }
}
```

**Step 2: Add new subscription filter kinds**

In `set_event_subscription` (~line 2751), add:

```rust
"zone_opened" => Some(TerminalEventKind::ZoneOpened),
"zone_closed" => Some(TerminalEventKind::ZoneClosed),
"zone_scrolled_out" => Some(TerminalEventKind::ZoneScrolledOut),
"environment_changed" => Some(TerminalEventKind::EnvironmentChanged),
"remote_host_transition" => Some(TerminalEventKind::RemoteHostTransition),
"sub_shell_detected" => Some(TerminalEventKind::SubShellDetected),
```

Also update the docstring for `set_event_subscription` to list the new kinds.

**Step 3: Run compilation check**

Run: `cargo check --no-default-features --features pyo3/auto-initialize`
Expected: PASS (all non-exhaustive match errors resolved)

**Step 4: Commit**

```bash
git add src/python_bindings/terminal.rs
git commit -m "feat(python): add bindings for zone, environment, host transition, and sub-shell events"
```

---

### Task 6: Update streaming protocol (protobuf + protocol.rs + proto.rs + server.rs)

**Files:**
- Modify: `proto/terminal.proto:25-45` (EventType enum)
- Modify: `proto/terminal.proto:51-78` (ServerMessage oneof)
- Modify: `proto/terminal.proto` (add new message definitions)
- Modify: `src/streaming/protocol.rs:83-403` (ServerMessage enum)
- Modify: `src/streaming/protocol.rs:494-536` (EventType enum)
- Modify: `src/streaming/proto.rs:164-438` (App→Proto conversion)
- Modify: `src/streaming/proto.rs:582-770` (Proto→App conversion)
- Modify: `src/streaming/proto.rs:509-531` (EventType App→Proto)
- Modify: `src/streaming/proto.rs:831-854` (EventType Proto→App)
- Modify: `src/streaming/server.rs:2710-2749` (should_send)
- Modify: `src/bin/streaming_server.rs:668-766` (poll_terminal_events dispatch)

**Step 1: Add protobuf messages and EventType values to terminal.proto**

After `EVENT_TYPE_SYSTEM_STATS = 18;` add:
```protobuf
EVENT_TYPE_ZONE = 19;
EVENT_TYPE_ENVIRONMENT = 20;
EVENT_TYPE_REMOTE_HOST = 21;
EVENT_TYPE_SUB_SHELL = 22;
```

In `ServerMessage` oneof, after `system_stats = 24`:
```protobuf
ZoneOpened zone_opened = 25;
ZoneClosed zone_closed = 26;
ZoneScrolledOut zone_scrolled_out = 27;
EnvironmentChanged environment_changed = 28;
RemoteHostTransition remote_host_transition = 29;
SubShellDetected sub_shell_detected = 30;
```

Add new message definitions after SystemStats:
```protobuf
// Zone opened (prompt, command, or output block started)
message ZoneOpened {
  uint64 zone_id = 1;
  string zone_type = 2;    // "prompt", "command", "output"
  uint64 abs_row_start = 3;
}

// Zone closed (prompt, command, or output block ended)
message ZoneClosed {
  uint64 zone_id = 1;
  string zone_type = 2;
  uint64 abs_row_start = 3;
  uint64 abs_row_end = 4;
  optional int32 exit_code = 5;
}

// Zone evicted from scrollback
message ZoneScrolledOut {
  uint64 zone_id = 1;
  string zone_type = 2;
}

// Environment variable changed
message EnvironmentChanged {
  string key = 1;
  string value = 2;
  optional string old_value = 3;
}

// Remote host transition detected
message RemoteHostTransition {
  string hostname = 1;
  optional string username = 2;
  optional string old_hostname = 3;
  optional string old_username = 4;
}

// Sub-shell detected
message SubShellDetected {
  uint64 depth = 1;
  optional string shell_type = 2;
}
```

**Step 2: Regenerate Rust protobuf code**

Run: `make proto-rust`

**Step 3: Add ServerMessage variants to protocol.rs**

After `SystemStats { ... }` add:
```rust
/// Zone opened (prompt, command, or output block started)
#[serde(rename = "zone_opened")]
ZoneOpened {
    zone_id: u64,
    zone_type: String,
    abs_row_start: u64,
},

/// Zone closed (prompt, command, or output block ended)
#[serde(rename = "zone_closed")]
ZoneClosed {
    zone_id: u64,
    zone_type: String,
    abs_row_start: u64,
    abs_row_end: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
},

/// Zone evicted from scrollback
#[serde(rename = "zone_scrolled_out")]
ZoneScrolledOut {
    zone_id: u64,
    zone_type: String,
},

/// Environment variable changed
#[serde(rename = "environment_changed")]
EnvironmentChanged {
    key: String,
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_value: Option<String>,
},

/// Remote host transition detected
#[serde(rename = "remote_host_transition")]
RemoteHostTransition {
    hostname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_username: Option<String>,
},

/// Sub-shell detected
#[serde(rename = "sub_shell_detected")]
SubShellDetected {
    depth: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_type: Option<String>,
},
```

Add EventType variants:
```rust
/// Zone events (opened, closed, scrolled out)
Zone,
/// Environment change events
Environment,
/// Remote host transition events
RemoteHost,
/// Sub-shell detection events
SubShell,
```

Add constructor methods on `impl ServerMessage`:
```rust
pub fn zone_opened(zone_id: u64, zone_type: String, abs_row_start: u64) -> Self {
    Self::ZoneOpened { zone_id, zone_type, abs_row_start }
}

pub fn zone_closed(zone_id: u64, zone_type: String, abs_row_start: u64, abs_row_end: u64, exit_code: Option<i32>) -> Self {
    Self::ZoneClosed { zone_id, zone_type, abs_row_start, abs_row_end, exit_code }
}

pub fn zone_scrolled_out(zone_id: u64, zone_type: String) -> Self {
    Self::ZoneScrolledOut { zone_id, zone_type }
}

pub fn environment_changed(key: String, value: String, old_value: Option<String>) -> Self {
    Self::EnvironmentChanged { key, value, old_value }
}

pub fn remote_host_transition(hostname: String, username: Option<String>, old_hostname: Option<String>, old_username: Option<String>) -> Self {
    Self::RemoteHostTransition { hostname, username, old_hostname, old_username }
}

pub fn sub_shell_detected(depth: u64, shell_type: Option<String>) -> Self {
    Self::SubShellDetected { depth, shell_type }
}
```

**Step 4: Update proto.rs conversions (App→Proto and Proto→App)**

In `From<&AppServerMessage> for pb::ServerMessage`, add arms for all 6 new variants.

In `TryFrom<pb::ServerMessage> for AppServerMessage`, add arms for all 6 new pb variants.

In `From<AppEventType> for i32`, add:
```rust
AppEventType::Zone => pb::EventType::Zone as i32,
AppEventType::Environment => pb::EventType::Environment as i32,
AppEventType::RemoteHost => pb::EventType::RemoteHost as i32,
AppEventType::SubShell => pb::EventType::SubShell as i32,
```

In `From<pb::EventType> for AppEventType`, add:
```rust
pb::EventType::Zone => AppEventType::Zone,
pb::EventType::Environment => AppEventType::Environment,
pb::EventType::RemoteHost => AppEventType::RemoteHost,
pb::EventType::SubShell => AppEventType::SubShell,
```

**Step 5: Update should_send in server.rs**

Add to the match in `should_send()`:
```rust
ServerMessage::ZoneOpened { .. } | ServerMessage::ZoneClosed { .. } | ServerMessage::ZoneScrolledOut { .. } => {
    subs.contains(&EventType::Zone)
}
ServerMessage::EnvironmentChanged { .. } => subs.contains(&EventType::Environment),
ServerMessage::RemoteHostTransition { .. } => subs.contains(&EventType::RemoteHost),
ServerMessage::SubShellDetected { .. } => subs.contains(&EventType::SubShell),
```

**Step 6: Update poll_terminal_events in streaming_server.rs**

Add dispatch arms in `src/bin/streaming_server.rs:682`:
```rust
TerminalEvent::ZoneOpened { zone_id, zone_type, abs_row_start } => {
    self.streaming_server.broadcast(
        ServerMessage::zone_opened(zone_id as u64, zone_type.to_string(), abs_row_start as u64),
    );
}
TerminalEvent::ZoneClosed { zone_id, zone_type, abs_row_start, abs_row_end, exit_code } => {
    self.streaming_server.broadcast(
        ServerMessage::zone_closed(zone_id as u64, zone_type.to_string(), abs_row_start as u64, abs_row_end as u64, exit_code),
    );
}
TerminalEvent::ZoneScrolledOut { zone_id, zone_type } => {
    self.streaming_server.broadcast(
        ServerMessage::zone_scrolled_out(zone_id as u64, zone_type.to_string()),
    );
}
TerminalEvent::EnvironmentChanged { key, value, old_value } => {
    self.streaming_server.broadcast(
        ServerMessage::environment_changed(key, value, old_value),
    );
}
TerminalEvent::RemoteHostTransition { hostname, username, old_hostname, old_username } => {
    self.streaming_server.broadcast(
        ServerMessage::remote_host_transition(hostname, username, old_hostname, old_username),
    );
}
TerminalEvent::SubShellDetected { depth, shell_type } => {
    self.streaming_server.broadcast(
        ServerMessage::sub_shell_detected(depth as u64, shell_type),
    );
}
```

**Step 7: Run compilation check**

Run: `cargo check --no-default-features --features pyo3/auto-initialize,streaming`
Expected: PASS

**Step 8: Commit**

```bash
git add proto/terminal.proto src/streaming/protocol.rs src/streaming/proto.rs src/streaming/server.rs src/bin/streaming_server.rs src/terminal.pb.rs
git commit -m "feat(streaming): add zone, environment, host, and sub-shell events to streaming protocol"
```

---

### Task 7: Regenerate TypeScript proto and rebuild web frontend

**Step 1: Regenerate TypeScript protobuf code**

Run: `make proto-typescript`

**Step 2: Build static web frontend**

Run: `make web-build-static`

**Step 3: Commit generated files**

```bash
git add web_term/ web_terminal/
git commit -m "chore: regenerate TypeScript proto and rebuild web frontend"
```

---

### Task 8: Add Rust tests for new events

**Files:**
- Modify: `src/terminal/sequences/osc.rs` (add tests at end of test module)
- Modify: `src/zone.rs` (update existing tests for new id field)
- Modify: `tests/test_streaming.rs` (add streaming protocol tests)

**Step 1: Write zone lifecycle event tests**

Add to the test module in `src/terminal/sequences/osc.rs`:

```rust
#[test]
fn test_zone_opened_events_emitted() {
    let mut term = Terminal::new(80, 24);
    // OSC 133 A (prompt start) should emit ZoneOpened
    term.process(b"\x1b]133;A\x1b\\");
    let events = term.poll_events();
    assert!(events.iter().any(|e| matches!(e, TerminalEvent::ZoneOpened {
        zone_type, ..
    } if *zone_type == crate::zone::ZoneType::Prompt)));
}

#[test]
fn test_zone_closed_events_emitted() {
    let mut term = Terminal::new(80, 24);
    // Full cycle: A -> B -> C -> D
    term.process(b"\x1b]133;A\x1b\\");
    term.poll_events(); // drain
    term.process(b"\x1b]133;B\x1b\\");
    let events = term.poll_events();
    // Should have ZoneClosed for prompt and ZoneOpened for command
    assert!(events.iter().any(|e| matches!(e, TerminalEvent::ZoneClosed {
        zone_type, ..
    } if *zone_type == crate::zone::ZoneType::Prompt)));
    assert!(events.iter().any(|e| matches!(e, TerminalEvent::ZoneOpened {
        zone_type, ..
    } if *zone_type == crate::zone::ZoneType::Command)));
}

#[test]
fn test_zone_closed_with_exit_code() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b]133;A\x1b\\");
    term.process(b"\x1b]133;B\x1b\\");
    term.process(b"\x1b]133;C\x1b\\");
    term.poll_events(); // drain
    term.process(b"\x1b]133;D;0\x1b\\");
    let events = term.poll_events();
    assert!(events.iter().any(|e| matches!(e, TerminalEvent::ZoneClosed {
        zone_type, exit_code: Some(0), ..
    } if *zone_type == crate::zone::ZoneType::Output)));
}

#[test]
fn test_zone_ids_monotonically_increase() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b]133;A\x1b\\"); // zone 0
    term.process(b"\x1b]133;B\x1b\\"); // zone 1
    term.process(b"\x1b]133;C\x1b\\"); // zone 2
    let events = term.poll_events();
    let zone_ids: Vec<usize> = events.iter().filter_map(|e| match e {
        TerminalEvent::ZoneOpened { zone_id, .. } => Some(*zone_id),
        _ => None,
    }).collect();
    assert_eq!(zone_ids, vec![0, 1, 2]);
}
```

**Step 2: Write environment change event tests**

```rust
#[test]
fn test_environment_changed_on_cwd_change() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b]7;file:///home/user/project\x1b\\");
    let events = term.poll_events();
    assert!(events.iter().any(|e| matches!(e, TerminalEvent::EnvironmentChanged {
        key, value, ..
    } if key == "cwd" && value == "/home/user/project")));
}

#[test]
fn test_remote_host_transition_from_osc7() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b]7;file://remotehost/home/user\x1b\\");
    let events = term.poll_events();
    assert!(events.iter().any(|e| matches!(e, TerminalEvent::RemoteHostTransition {
        hostname, old_hostname: None, ..
    } if hostname == "remotehost")));
}

#[test]
fn test_remote_host_transition_from_osc1337() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b]1337;RemoteHost=alice@server1\x1b\\");
    let events = term.poll_events();
    assert!(events.iter().any(|e| matches!(e, TerminalEvent::RemoteHostTransition {
        hostname, username: Some(u), ..
    } if hostname == "server1" && u == "alice")));
}
```

**Step 3: Write streaming protocol round-trip tests**

Add to `tests/test_streaming.rs`:

```rust
#[test]
fn test_zone_opened_round_trip() {
    let msg = ServerMessage::zone_opened(42, "prompt".to_string(), 100);
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"zone_opened"#));
    let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        ServerMessage::ZoneOpened { zone_id, zone_type, abs_row_start } => {
            assert_eq!(zone_id, 42);
            assert_eq!(zone_type, "prompt");
            assert_eq!(abs_row_start, 100);
        }
        _ => panic!("Wrong message type"),
    }
}
// Similar tests for ZoneClosed, ZoneScrolledOut, EnvironmentChanged,
// RemoteHostTransition, SubShellDetected
```

**Step 4: Run all tests**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming`
Expected: PASS

**Step 5: Commit**

```bash
git add src/terminal/sequences/osc.rs src/zone.rs tests/test_streaming.rs
git commit -m "test: add tests for zone lifecycle, environment, host, and sub-shell events"
```

---

### Task 9: Add Python integration tests

**Files:**
- Create: `tests/test_contextual_events.py`

**Step 1: Write Python tests for new event types**

```python
"""Tests for contextual awareness API events (issue #37)."""
import par_term_emu_core_rust as pte


def test_zone_opened_event():
    """ZoneOpened event fires on OSC 133 A."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\")
    events = term.poll_events()
    zone_opened = [e for e in events if e["type"] == "zone_opened"]
    assert len(zone_opened) >= 1
    assert zone_opened[0]["zone_type"] == "prompt"


def test_zone_closed_event():
    """ZoneClosed event fires when zone transitions."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\")
    term.poll_events()  # drain
    term.process(b"\x1b]133;B\x1b\\")
    events = term.poll_events()
    zone_closed = [e for e in events if e["type"] == "zone_closed"]
    assert len(zone_closed) >= 1
    assert zone_closed[0]["zone_type"] == "prompt"


def test_zone_closed_with_exit_code():
    """ZoneClosed for output zone includes exit code."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\")
    term.process(b"\x1b]133;B\x1b\\")
    term.process(b"\x1b]133;C\x1b\\")
    term.poll_events()
    term.process(b"\x1b]133;D;0\x1b\\")
    events = term.poll_events()
    zone_closed = [e for e in events if e["type"] == "zone_closed"]
    assert any(e.get("exit_code") == "0" for e in zone_closed)


def test_environment_changed_event():
    """EnvironmentChanged fires on CWD change."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]7;file:///home/user/project\x1b\\")
    events = term.poll_events()
    env_events = [e for e in events if e["type"] == "environment_changed"]
    cwd_events = [e for e in env_events if e["key"] == "cwd"]
    assert len(cwd_events) >= 1
    assert cwd_events[0]["value"] == "/home/user/project"


def test_remote_host_transition_event():
    """RemoteHostTransition fires on hostname change."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]1337;RemoteHost=alice@server1\x1b\\")
    events = term.poll_events()
    host_events = [e for e in events if e["type"] == "remote_host_transition"]
    assert len(host_events) >= 1
    assert host_events[0]["hostname"] == "server1"
    assert host_events[0].get("username") == "alice"


def test_event_subscription_new_types():
    """New event types work with subscription filtering."""
    term = pte.Terminal(80, 24)
    term.set_event_subscription(["zone_opened", "zone_closed"])
    term.process(b"\x1b]133;A\x1b\\")
    events = term.poll_subscribed_events()
    assert all(e["type"] in ("zone_opened", "zone_closed") for e in events)
```

**Step 2: Build Python bindings**

Run: `make dev`

**Step 3: Run Python tests**

Run: `make test-python` or `uv run pytest tests/test_contextual_events.py -v`
Expected: PASS

**Step 4: Commit**

```bash
git add tests/test_contextual_events.py
git commit -m "test(python): add integration tests for contextual awareness events"
```

---

### Task 10: Run full checkall and fix any issues

**Step 1: Run full quality checks**

Run: `make checkall`
Expected: PASS (all fmt, lint, clippy, pyright, tests)

**Step 2: Fix any issues found**

Address any clippy warnings, formatting issues, or test failures.

**Step 3: Final commit if needed**

```bash
git add -A
git commit -m "fix: address checkall issues for contextual awareness API"
```

---

### Task 11: Create PR

**Step 1: Push branch and create PR**

```bash
git push -u origin feat/contextual-awareness-api
gh pr create --title "feat: Contextual Awareness API - granular terminal state change notifications" --body "$(cat <<'EOF'
## Summary
- Adds 6 new `TerminalEvent` variants: `ZoneOpened`, `ZoneClosed`, `ZoneScrolledOut`, `EnvironmentChanged`, `RemoteHostTransition`, `SubShellDetected`
- Zone events fire alongside existing `ShellIntegrationEvent` (consumers choose abstraction level)
- Environment/host events derive from OSC 7 and OSC 1337 RemoteHost
- Sub-shell detection uses basic prompt nesting heuristic
- Full streaming protocol support (protobuf + proto conversion)
- Python bindings with subscription filtering for all new event types
- Comprehensive Rust and Python tests

Closes #37

## Test plan
- [x] Rust unit tests for zone lifecycle events
- [x] Rust unit tests for environment/host/sub-shell events
- [x] Streaming protocol round-trip tests
- [x] Python integration tests for all new event types
- [x] `make checkall` passes
EOF
)"
```
