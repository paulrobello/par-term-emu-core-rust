# Terminal Observer API Design

**Issue:** #39 — Python/Scripting API: extensibility hooks and stable FFI terminal state
**Date:** 2026-02-14

## Summary

Add a `TerminalObserver` trait enabling push-based event delivery, a C-compatible `SharedState` FFI view for external consumers, and Python bindings supporting both synchronous callbacks and asyncio queue-based async consumption.

## Architecture: Trait-based Observer with Deferred Dispatch

### Core Trait

```rust
pub type ObserverId = u64;

pub trait TerminalObserver: Send + Sync {
    fn on_zone_event(&self, event: &TerminalEvent) {}
    fn on_command_event(&self, event: &TerminalEvent) {}
    fn on_environment_event(&self, event: &TerminalEvent) {}
    fn on_screen_event(&self, event: &TerminalEvent) {}
    fn on_event(&self, event: &TerminalEvent) {}
    fn subscriptions(&self) -> Option<HashSet<TerminalEventKind>> { None }
}
```

Category methods receive only events of their type. `on_event()` receives all events as a catch-all, called after the category method.

### Registration

```rust
impl Terminal {
    pub fn add_observer(&mut self, observer: Arc<dyn TerminalObserver>) -> ObserverId;
    pub fn remove_observer(&mut self, id: ObserverId) -> bool;
}
```

Observers stored as `Vec<ObserverEntry>` with monotonic ID assignment.

### Dispatch Flow

1. `process(data)` processes VT sequences, pushes events to `terminal_events` Vec (unchanged).
2. After `process()` returns, `dispatch_to_observers()` is called.
3. For each event, for each observer: check subscription filter, call category method, call `on_event()`.
4. Events remain in `terminal_events` — `poll_events()` still works (backward compatible).

Key invariant: no internal mutexes held during observer callbacks.

## C-Compatible SharedState FFI

### Snapshot Structs

```rust
#[repr(C)]
pub struct SharedState {
    pub cols: u32,
    pub rows: u32,
    pub cursor_col: u32,
    pub cursor_row: u32,
    pub cursor_visible: bool,
    pub alt_screen_active: bool,
    pub mouse_mode: u8,
    pub title: *const c_char,
    pub title_len: u32,
    pub cwd: *const c_char,
    pub cwd_len: u32,
    pub cells: *const SharedCell,
    pub cell_count: u32,
    pub scrollback_lines: u32,
    pub total_lines: u32,
}

#[repr(C)]
pub struct SharedCell {
    pub text: [u8; 4],
    pub text_len: u8,
    pub fg_r: u8, pub fg_g: u8, pub fg_b: u8,
    pub bg_r: u8, pub bg_g: u8, pub bg_b: u8,
    pub attrs: u16,
    pub width: u8,
}
```

### C API

```c
SharedState* terminal_get_state(TerminalHandle handle);
void terminal_free_state(SharedState* state);
ObserverId terminal_add_observer(TerminalHandle handle, TerminalObserverVtable* vtable, void* user_data);
bool terminal_remove_observer(TerminalHandle handle, ObserverId id);
```

`TerminalObserverVtable` is a struct with function pointers matching trait methods, plus `user_data` context pointer.

## Python Bindings

### Sync Mode — Direct Callback

```python
def handler(event: dict):
    print(event["type"], event)

observer_id = terminal.add_observer(handler)
terminal.remove_observer(observer_id)
```

`PyCallbackObserver` wraps the callable, acquires GIL during dispatch, converts events to dicts using existing conversion code.

### Async Mode — Queue-Based

```python
observer_id, queue = terminal.add_async_observer()

async def consume():
    while True:
        event = await queue.get()
        handle(event)

terminal.remove_observer(observer_id)
```

`QueueObserver` holds a reference to an `asyncio.Queue`, uses `put_nowait()` during dispatch.

### Convenience Wrappers

Located in `python/par_term_emu_core_rust/observers.py`:

- `on_command_complete(terminal, callback)` — fires on shell_integration command_finished events
- `on_zone_change(terminal, callback)` — fires on zone_opened/closed/scrolled_out events

## Thread Safety

1. `TerminalObserver` requires `Send + Sync`.
2. Dispatch happens after `process()` returns — no Rust mutexes held.
3. Python sync callbacks acquire GIL only during dispatch.
4. Python async queue uses non-blocking `put_nowait()`.
5. C FFI callbacks invoked via function pointer — caller manages their own synchronization.

## Testing Plan

### Rust Tests (9)
- Observer registration/removal and ID management
- Event delivery to observers
- Subscription filter correctness
- Category routing (zone → on_zone_event, etc.)
- Multiple observer support
- Observer removal safety
- Backward compatibility with poll_events()
- SharedState snapshot correctness
- SharedCell content verification

### Python Tests (5)
- Sync observer callback receives events as dicts
- Async observer queue receives events
- Observer cleanup stops delivery
- Convenience wrappers (on_command_complete, on_zone_change)
- Observer thread safety

### C FFI Tests (3)
- SharedState populated correctly
- Memory cleanup (no leaks)
- Observer vtable function pointer callbacks

## Files to Create/Modify

### New Files
- `src/observer.rs` — TerminalObserver trait, ObserverEntry, dispatch logic
- `src/ffi.rs` — SharedState, SharedCell, C API functions
- `src/python_bindings/observer.rs` — PyCallbackObserver, QueueObserver, Python API
- `python/par_term_emu_core_rust/observers.py` — convenience wrappers
- `tests/test_observer.py` — Python integration tests

### Modified Files
- `src/terminal/mod.rs` — observer storage, dispatch_to_observers() call after process()
- `src/lib.rs` — register new modules
- `src/python_bindings/mod.rs` — register observer module
- `src/python_bindings/terminal.rs` — add_observer/remove_observer methods
- `docs/API_REFERENCE.md` — document new APIs
- `README.md` — document observer feature
