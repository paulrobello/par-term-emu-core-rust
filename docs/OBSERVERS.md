# Observer System Guide

This guide explains how to use the terminal observer pattern for push-based event delivery.

## Overview

The observer pattern allows your application to receive terminal events immediately after they occur, without polling. Observers are registered with the terminal and receive callbacks whenever events are emitted during `process()` calls.

### Key Concepts

- **Push-based delivery**: Events are delivered via callbacks, not polling
- **Deferred dispatch**: Callbacks are invoked after `process()` returns, ensuring no internal mutexes are held
- **Subscription filtering**: Observers can subscribe to specific event types
- **Thread-safe**: Observers must implement `Send + Sync` since they may be called from different threads

## Implementing an Observer (Rust)

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

// Later: remove the observer
term.remove_observer(observer_id);
```

## Event Categories

Events are routed to category-specific methods before the catch-all `on_event`:

### Zone Events
Lifecycle events for semantic zones (prompt, command, output blocks):
- `ZoneOpened` - A zone was created
- `ZoneClosed` - A zone was completed
- `ZoneScrolledOut` - A zone was evicted from scrollback

### Command Events
Shell integration events:
- `ShellIntegrationEvent` - Prompt start, command start, command executed, command finished

### Environment Events
Environment changes:
- `CwdChanged` - Current working directory changed
- `EnvironmentChanged` - Generic environment variable change
- `RemoteHostTransition` - Hostname changed (SSH/remote session)
- `SubShellDetected` - Shell nesting depth changed

### Screen Events
Screen content and metadata changes:
- `BellRang` - Bell event (visual, warning, margin)
- `TitleChanged` - Terminal title changed
- `SizeChanged` - Terminal resized
- `ModeChanged` - Terminal mode toggled (e.g., DECCKM, DECAWM)
- `GraphicsAdded` - Graphics image added
- `HyperlinkAdded` - Hyperlink detected
- `DirtyRegion` - Screen region needs redraw
- `UserVarChanged` - User variable set via OSC 1337
- `ProgressBarChanged` - Progress bar updated via OSC 934
- `BadgeChanged` - Badge text changed via OSC 1337
- `TriggerMatched` - Output pattern matched (from `Trigger`)

## Subscription Filtering (Rust)

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
