# Observer System Guide

This guide explains how to use the terminal observer pattern for push-based event delivery across Rust, Python, and FFI contexts.

## Table of Contents

- [Overview](#overview)
- [Event Categories](#event-categories)
- [Rust Observer API](#rust-observer-api)
- [Python Observer API](#python-observer-api)
- [FFI/C Observer API](#ffic-observer-api)
- [Event Dict Reference](#event-dict-reference)
- [Event Lifecycle](#event-lifecycle)
- [Thread Safety](#thread-safety)
- [Best Practices](#best-practices)
- [Related Documentation](#related-documentation)

## Overview

The observer pattern allows your application to receive terminal events immediately after they occur, without polling. Observers are registered with the terminal and receive callbacks whenever events are emitted during `process()` calls.

### Key Concepts

- **Push-based delivery**: Events are delivered via callbacks, not polling
- **Deferred dispatch**: Callbacks are invoked after `process()` returns, ensuring no internal mutexes are held
- **Subscription filtering**: Observers can subscribe to specific event types
- **Thread-safe**: Observers must implement `Send + Sync` since they may be called from different threads
- **Dual delivery**: Both observers and `poll_events()` work simultaneously

## Event Categories

Events are routed to category-specific methods before the catch-all `on_event`:

### Zone Events

Lifecycle events for semantic zones (prompt, command, output blocks):

| Event | Description |
|-------|-------------|
| `ZoneOpened` | A zone was created |
| `ZoneClosed` | A zone was completed |
| `ZoneScrolledOut` | A zone was evicted from scrollback |

### Command Events

Shell integration events:

| Event | Description |
|-------|-------------|
| `ShellIntegrationEvent` | Prompt start, command start, command executed, command finished |

### Environment Events

Environment changes:

| Event | Description |
|-------|-------------|
| `CwdChanged` | Current working directory changed |
| `EnvironmentChanged` | Generic environment variable change |
| `RemoteHostTransition` | Hostname changed (SSH/remote session) |
| `SubShellDetected` | Shell nesting depth changed |

### Screen Events

Screen content and metadata changes:

| Event | Description |
|-------|-------------|
| `BellRang` | Bell event (visual, warning, margin) |
| `TitleChanged` | Terminal title changed |
| `SizeChanged` | Terminal resized |
| `ModeChanged` | Terminal mode toggled (e.g., DECCKM, DECAWM) |
| `GraphicsAdded` | Graphics image added |
| `HyperlinkAdded` | Hyperlink detected |
| `DirtyRegion` | Screen region needs redraw |
| `UserVarChanged` | User variable set via OSC 1337 |
| `ProgressBarChanged` | Progress bar updated via OSC 934 |
| `BadgeChanged` | Badge text changed via OSC 1337 |
| `TriggerMatched` | Output pattern matched (from `Trigger`) |

### File Transfer Events

File transfer lifecycle events:

| Event | Description |
|-------|-------------|
| `FileTransferStarted` | A file transfer (download/upload) has started |
| `FileTransferProgress` | Progress update for an active transfer |
| `FileTransferCompleted` | Transfer completed successfully |
| `FileTransferFailed` | Transfer failed with an error |
| `UploadRequested` | Remote application requested an upload |

## Rust Observer API

Implement the `TerminalObserver` trait. All methods have default no-op implementations, so you only need to override the ones you care about.

```rust
use par_term_emu_core_rust::observer::TerminalObserver;
use par_term_emu_core_rust::terminal::TerminalEvent;
use std::sync::Arc;

struct MyObserver;

impl TerminalObserver for MyObserver {
    fn on_zone_event(&self, event: &TerminalEvent) {
        println!("Zone event: {:?}", event);
    }

    fn on_command_event(&self, event: &TerminalEvent) {
        println!("Command event: {:?}", event);
    }

    fn on_environment_event(&self, event: &TerminalEvent) {
        println!("Environment event: {:?}", event);
    }

    fn on_screen_event(&self, event: &TerminalEvent) {
        println!("Screen event: {:?}", event);
    }

    fn on_event(&self, event: &TerminalEvent) {
        // Catch-all for every event (called after category-specific methods)
        println!("Event: {:?}", event);
    }
}

// Register the observer
let mut term = Terminal::new(80, 24);
let observer_id = term.add_observer(Arc::new(MyObserver));

// Check observer count
assert_eq!(term.observer_count(), 1);

// Later: remove the observer
term.remove_observer(observer_id);
```

### Subscription Filtering (Rust)

Override `subscriptions()` to receive only specific event types:

```rust
use std::collections::HashSet;
use par_term_emu_core_rust::terminal::TerminalEventKind;

impl TerminalObserver for MyObserver {
    fn on_screen_event(&self, event: &TerminalEvent) {
        // Only called for title/bell events (due to filter)
        println!("Screen event: {:?}", event);
    }

    fn subscriptions(&self) -> Option<&HashSet<TerminalEventKind>> {
        static KINDS: std::sync::LazyLock<HashSet<TerminalEventKind>> =
            std::sync::LazyLock::new(|| {
                let mut set = HashSet::new();
                set.insert(TerminalEventKind::TitleChanged);
                set.insert(TerminalEventKind::BellRang);
                set
            });
        Some(&KINDS)
    }
}
```

Returning `None` from `subscriptions()` means "receive all events" (default behavior).

## Python Observer API

Python observers are registered via callbacks or async queues.

### Synchronous Observers

```python
from par_term_emu_core_rust import Terminal

term = Terminal(80, 24, scrollback=100)

def my_callback(event: dict) -> None:
    print(f"Event: {event['type']}")
    if event['type'] == 'title_changed':
        print(f"New title: {event['title']}")

# Add observer (receives all events)
observer_id = term.add_observer(my_callback)

# Add observer with filter (receives only specific event types)
observer_id = term.add_observer(my_callback, kinds=["title_changed", "bell"])

# Check observer count
print(f"Observers: {term.observer_count()}")

# Process input
term.process(b"\x1b]0;New Title\x07")

# Remove observer
term.remove_observer(observer_id)
```

### Asynchronous Observers

For async applications, use `add_async_observer()` to receive events via a queue:

```python
import asyncio
from par_term_emu_core_rust import Terminal

async def watch_events():
    term = Terminal(80, 24, scrollback=100)
    observer_id, queue = term.add_async_observer()

    # Process input in background
    term.process(b"\x1b]0;Async Title\x07")

    # Consume events from queue
    while not queue.empty():
        event = queue.get_nowait()
        print(f"Async event: {event['type']}")

    term.remove_observer(observer_id)

asyncio.run(watch_events())
```

### Convenience Wrappers

Python provides convenience wrappers for common event types:

```python
from par_term_emu_core_rust.observers import (
    on_bell,
    on_command_complete,
    on_cwd_change,
    on_title_change,
    on_zone_change,
)

term = Terminal(80, 24, scrollback=100)

# Register specific observers
on_bell(term, lambda e: print("Bell rang!"))
on_title_change(term, lambda e: print(f"Title: {e['title']}"))
on_cwd_change(term, lambda e: print(f"CWD: {e['new_cwd']}"))
on_command_complete(term, lambda e: print(f"Exit code: {e.get('exit_code')}"))
on_zone_change(term, lambda e: print(f"Zone {e['type']}: {e.get('zone_type')}"))
```

### Registration Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `add_observer(callback, kinds=None)` | `int` | Register synchronous callback |
| `add_async_observer(kinds=None)` | `tuple[int, asyncio.Queue]` | Register async queue observer |
| `remove_observer(observer_id)` | `bool` | Remove observer (True if found) |
| `observer_count()` | `int` | Get number of registered observers |

## FFI/C Observer API

For embedding in Swift, Kotlin/JNI, C/C++, and other languages, use the FFI observer interface defined in `src/ffi.rs`.

### TerminalObserverVtable

A C-compatible vtable for terminal event observation:

```c
typedef struct {
    // Called for zone lifecycle events
    void (*on_zone_event)(void* user_data, const char* event_json);
    // Called for command/shell integration events
    void (*on_command_event)(void* user_data, const char* event_json);
    // Called for environment change events
    void (*on_environment_event)(void* user_data, const char* event_json);
    // Called for screen content events
    void (*on_screen_event)(void* user_data, const char* event_json);
    // Called for ALL events (catch-all)
    void (*on_event)(void* user_data, const char* event_json);
    // Opaque pointer passed to every callback
    void* user_data;
} TerminalObserverVtable;
```

Each callback receives a JSON-encoded event description as a NUL-terminated C string. The callee must NOT free the event string - it is owned by the caller and valid only for the duration of the callback.

### FFI Functions

```c
// Register an FFI observer on the terminal
// Returns an observer ID for later removal
uint64_t terminal_add_observer(Terminal* term, TerminalObserverVtable vtable);

// Remove a previously registered observer
// Returns true if the observer was found and removed
bool terminal_remove_observer(Terminal* term, uint64_t id);
```

### Example (C)

```c
void on_event_callback(void* user_data, const char* event_json) {
    printf("Event: %s\n", event_json);
}

TerminalObserverVtable vtable = {
    .on_zone_event = NULL,
    .on_command_event = NULL,
    .on_environment_event = NULL,
    .on_screen_event = NULL,
    .on_event = on_event_callback,
    .user_data = my_context
};

uint64_t observer_id = terminal_add_observer(term, vtable);

// Later...
terminal_remove_observer(term, observer_id);
```

## Event Dict Reference

All observer events are delivered as Python dicts with a `"type"` key identifying the event kind.

### Supported Event Types

| Type String | Rust Kind | Category |
|-------------|-----------|----------|
| `bell` | `BellRang` | Screen |
| `title_changed` | `TitleChanged` | Screen |
| `size_changed` | `SizeChanged` | Screen |
| `mode_changed` | `ModeChanged` | Screen |
| `graphics_added` | `GraphicsAdded` | Screen |
| `hyperlink_added` | `HyperlinkAdded` | Screen |
| `dirty_region` | `DirtyRegion` | Screen |
| `cwd_changed` | `CwdChanged` | Environment |
| `trigger_matched` | `TriggerMatched` | Screen |
| `user_var_changed` | `UserVarChanged` | Screen |
| `progress_bar_changed` | `ProgressBarChanged` | Screen |
| `badge_changed` | `BadgeChanged` | Screen |
| `shell_integration` | `ShellIntegrationEvent` | Command |
| `zone_opened` | `ZoneOpened` | Zone |
| `zone_closed` | `ZoneClosed` | Zone |
| `zone_scrolled_out` | `ZoneScrolledOut` | Zone |
| `environment_changed` | `EnvironmentChanged` | Environment |
| `remote_host_transition` | `RemoteHostTransition` | Environment |
| `sub_shell_detected` | `SubShellDetected` | Environment |
| `file_transfer_started` | `FileTransferStarted` | Screen |
| `file_transfer_progress` | `FileTransferProgress` | Screen |
| `file_transfer_completed` | `FileTransferCompleted` | Screen |
| `file_transfer_failed` | `FileTransferFailed` | Screen |
| `upload_requested` | `UploadRequested` | Screen |

### Event Fields

#### Bell Event

```python
{
    "type": "bell",
    "bell_type": "visual" | "warning" | "margin",
    "volume": "5"  # Only for warning/margin bells (0-8)
}
```

#### Title Changed

```python
{
    "type": "title_changed",
    "title": "New Terminal Title"
}
```

#### Size Changed

```python
{
    "type": "size_changed",
    "cols": "120",
    "rows": "40"
}
```

#### Mode Changed

```python
{
    "type": "mode_changed",
    "mode": "DECCKM",
    "enabled": "true"
}
```

#### CWD Changed

```python
{
    "type": "cwd_changed",
    "new_cwd": "/home/user/project",
    "old_cwd": "/home/user",  # Optional
    "hostname": "server.example.com",  # Optional
    "username": "user",  # Optional
    "timestamp": "1708400000000"
}
```

#### Shell Integration

```python
{
    "type": "shell_integration",
    "event_type": "prompt_start" | "command_start" | "command_executed" | "command_finished",
    "command": "ls -la",  # Optional
    "exit_code": "0",  # Optional
    "timestamp": "1708400000000",  # Optional
    "cursor_line": "42"  # Optional
}
```

#### Zone Events

```python
# Zone Opened
{
    "type": "zone_opened",
    "zone_id": "1",
    "zone_type": "prompt" | "command" | "output",
    "abs_row_start": "10"
}

# Zone Closed
{
    "type": "zone_closed",
    "zone_id": "1",
    "zone_type": "output",
    "abs_row_start": "10",
    "abs_row_end": "25",
    "exit_code": "0"  # Optional
}

# Zone Scrolled Out
{
    "type": "zone_scrolled_out",
    "zone_id": "1",
    "zone_type": "command"
}
```

#### File Transfer Events

```python
# Transfer Started
{
    "type": "file_transfer_started",
    "id": "123",
    "direction": "download" | "upload",
    "filename": "file.txt",  # Optional
    "total_bytes": "1024"  # Optional
}

# Transfer Progress
{
    "type": "file_transfer_progress",
    "id": "123",
    "bytes_transferred": "512",
    "total_bytes": "1024"  # Optional
}

# Transfer Completed
{
    "type": "file_transfer_completed",
    "id": "123",
    "filename": "file.txt",  # Optional
    "size": "1024"
}

# Transfer Failed
{
    "type": "file_transfer_failed",
    "id": "123",
    "reason": "Network error"
}

# Upload Requested
{
    "type": "upload_requested",
    "format": "base64"
}
```

#### Other Events

```python
# Trigger Matched
{
    "type": "trigger_matched",
    "trigger_id": "1",
    "row": "10",
    "col": "0",
    "end_col": "15",
    "text": "error: something failed",
    "timestamp": "1708400000000"
}

# Progress Bar Changed
{
    "type": "progress_bar_changed",
    "action": "set" | "remove" | "remove_all",
    "id": "download",
    "state": "normal",  # Optional, for "set" action
    "percent": "50",  # Optional, for "set" action
    "label": "Downloading..."  # Optional
}

# Badge Changed
{
    "type": "badge_changed",
    "badge": "Important"  # Optional, None if cleared
}

# Hyperlink Added
{
    "type": "hyperlink_added",
    "url": "https://example.com",
    "row": "5",
    "col": "10",
    "id": "42"  # Optional
}

# User Var Changed
{
    "type": "user_var_changed",
    "name": "MY_VAR",
    "value": "new_value",
    "old_value": "old_value"  # Optional
}

# Environment Changed
{
    "type": "environment_changed",
    "key": "cwd",
    "value": "/new/path",
    "old_value": "/old/path"  # Optional
}

# Remote Host Transition
{
    "type": "remote_host_transition",
    "hostname": "server.example.com",
    "username": "user",  # Optional
    "old_hostname": "localhost",  # Optional
    "old_username": "localuser"  # Optional
}

# Sub Shell Detected
{
    "type": "sub_shell_detected",
    "depth": "2",
    "shell_type": "bash"  # Optional
}

# Dirty Region
{
    "type": "dirty_region",
    "first_row": "0",
    "last_row": "23"
}

# Graphics Added
{
    "type": "graphics_added",
    "row": "10"
}
```

## Event Lifecycle

### Dispatch vs Poll

The terminal supports two event delivery mechanisms:

1. **Observers (push)**: Events are delivered via callbacks immediately after `process()` returns
2. **Polling (pull)**: Events are queued in a buffer and retrieved via `poll_events()`

Both mechanisms work simultaneously:
- Observers receive events via callbacks
- `poll_events()` returns queued events and clears the queue

### Dispatch Order

When `process()` emits events:

1. Events are added to the internal queue
2. After `process()` returns, observers are notified in order:
   - Category-specific method (`on_zone_event`, `on_command_event`, etc.)
   - Catch-all `on_event` (always called)
3. Application calls `poll_events()` to retrieve queued events (optional)

### Example: Mixed Usage

```python
term = Terminal(80, 24, scrollback=100)

# Add observer for push-based delivery
term.add_observer(lambda e: print(f"Observer: {e['type']}"))

# Process input
term.process(b"\x1b]0;Test\x07")

# Also retrieve events via polling (same events)
events = term.poll_events()
print(f"Polled {len(events)} events")
```

## Thread Safety

Observers are invoked **after** `process()` returns, ensuring no internal mutexes are held during callbacks. This prevents deadlocks when observers call back into the terminal.

However, observers must be `Send + Sync` since they may be called from different threads (e.g., in a PTY background reader thread).

## Best Practices

- **Avoid blocking**: Observers should return quickly to avoid delaying subsequent events
- **Error handling**: Observer callbacks should catch and log exceptions; uncaught exceptions may terminate the application
- **Subscription filtering**: Use filters to reduce overhead when you only need specific events
- **Observer lifetime**: Remove observers before dropping the terminal to avoid dangling callbacks
- **Deferred work**: If an observer needs to perform heavy work, queue it for later processing rather than blocking the callback

## Related Documentation

- [API Reference - Observer API](API_REFERENCE.md#observer-api) - Complete API method signatures
- [FFI Guide](FFI_GUIDE.md) - C API for embedding in native applications
- [Architecture](ARCHITECTURE.md) - Internal design of the event system
- [Shell Integration](ADVANCED_FEATURES.md#shell-integration) - OSC 133 semantic markers
