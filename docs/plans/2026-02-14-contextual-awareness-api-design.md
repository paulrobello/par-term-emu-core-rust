# Contextual Awareness API Design

**Issue:** #37 — feat: Contextual Awareness API - granular terminal state change notifications
**Date:** 2026-02-14
**Status:** Approved

## Summary

Expand the `TerminalEvent` system with dedicated event variants for zone lifecycle, sub-shell detection, environment changes, and remote host transitions. These coexist with the existing `ShellIntegrationEvent` — consumers choose their preferred abstraction level.

## New TerminalEvent Variants

### Zone Lifecycle Events

```rust
ZoneOpened { zone_id: usize, zone_type: ZoneType, abs_row_start: usize }
ZoneClosed { zone_id: usize, zone_type: ZoneType, abs_row_start: usize, abs_row_end: usize, exit_code: Option<i32> }
ZoneScrolledOut { zone_id: usize, zone_type: ZoneType }
```

- `zone_id`: monotonically increasing ID assigned at zone creation
- `ZoneOpened`: emitted after `push_zone()` in OSC 133 handler
- `ZoneClosed`: emitted after `close_current_zone()` in OSC 133 handler
- `ZoneScrolledOut`: emitted during `evict_zones()` — grid returns evicted zones, terminal queues events

### Environment Events

```rust
EnvironmentChanged { key: String, value: String, old_value: Option<String> }
RemoteHostTransition { hostname: String, username: Option<String>, old_hostname: Option<String>, old_username: Option<String> }
SubShellDetected { depth: usize, shell_type: Option<String> }
```

- `EnvironmentChanged`: fires for CWD, hostname, username changes (key = "cwd", "hostname", "username")
- `RemoteHostTransition`: fires when hostname changes, derived from OSC 7 and OSC 1337 `RemoteHost`/`CurrentDir`
- `SubShellDetected`: basic heuristic tracking prompt nesting depth via OSC 133;A

## Emission Points

| Event | Source |
|-------|--------|
| ZoneOpened | `osc.rs` OSC 133 handler, after `push_zone()` |
| ZoneClosed | `osc.rs` OSC 133 handler, after `close_current_zone()` |
| ZoneScrolledOut | `grid.rs` `evict_zones()` returns evicted zones → terminal queues events |
| EnvironmentChanged | `osc.rs` OSC 7 handler, new OSC 1337 handlers |
| RemoteHostTransition | `osc.rs` OSC 7 handler + new OSC 1337 RemoteHost handler |
| SubShellDetected | `osc.rs` OSC 133;A handler with depth counter |

## State Tracking (new Terminal fields)

```rust
last_hostname: Option<String>
last_username: Option<String>
shell_depth: usize
next_zone_id: usize
```

## Zone ID Assignment

Zones get a `zone_id` field (new) assigned from `next_zone_id` counter on Terminal. This provides stable identity across open/close/evict events. The `Zone` struct gains a `pub id: usize` field.

## Streaming Protocol

### New Protobuf Messages

```protobuf
message ZoneOpened { uint64 zone_id = 1; string zone_type = 2; uint64 abs_row_start = 3; }
message ZoneClosed { uint64 zone_id = 1; string zone_type = 2; uint64 abs_row_start = 3; uint64 abs_row_end = 4; optional int32 exit_code = 5; }
message ZoneScrolledOut { uint64 zone_id = 1; string zone_type = 2; }
message EnvironmentChanged { string key = 1; string value = 2; optional string old_value = 3; }
message RemoteHostTransition { string hostname = 1; optional string username = 2; optional string old_hostname = 3; optional string old_username = 4; }
message SubShellDetected { uint64 depth = 1; optional string shell_type = 2; }
```

### New EventType Values

```protobuf
EVENT_TYPE_ZONE = 19;
EVENT_TYPE_ENVIRONMENT = 20;
EVENT_TYPE_REMOTE_HOST = 21;
EVENT_TYPE_SUB_SHELL = 22;
```

## Python Bindings

### poll_events() Dict Formats

```python
{"type": "zone_opened", "zone_id": "0", "zone_type": "prompt", "abs_row_start": "5"}
{"type": "zone_closed", "zone_id": "0", "zone_type": "output", "abs_row_start": "5", "abs_row_end": "20", "exit_code": "0"}
{"type": "zone_scrolled_out", "zone_id": "0", "zone_type": "output"}
{"type": "environment_changed", "key": "cwd", "value": "/home/user", "old_value": "/tmp"}
{"type": "remote_host_transition", "hostname": "server.com", "username": "deploy", "old_hostname": "localhost", "old_username": "user"}
{"type": "sub_shell_detected", "depth": "2", "shell_type": "bash"}
```

### New Subscription Filter Kinds

`"zone_opened"`, `"zone_closed"`, `"zone_scrolled_out"`, `"environment_changed"`, `"remote_host_transition"`, `"sub_shell_detected"`

## Design Decisions

1. **Coexistence with ShellIntegrationEvent**: Both fire. ShellIntegrationEvent is the raw OSC 133 signal; zone events are the higher-level lifecycle abstraction.
2. **Sub-shell detection**: Basic heuristic using prompt nesting depth. Shell type from OSC 7 path or user vars when available.
3. **Remote host detection**: Multiple signals — OSC 7 hostname changes plus OSC 1337 RemoteHost/CurrentDir.
4. **Dedicated proto messages**: Each event gets its own message type and EventType value, consistent with existing patterns.

## Testing

1. Rust unit tests: verify event emission at correct lifecycle points
2. Python integration tests: verify `poll_events()` returns correct dicts
3. Streaming tests: verify proto round-trip serialization
