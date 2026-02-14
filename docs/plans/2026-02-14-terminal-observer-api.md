# Terminal Observer API Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `TerminalObserver` trait for push-based event delivery, a C-compatible `SharedState` FFI view, and Python bindings with sync/async observer modes.

**Architecture:** Trait-based observer with deferred dispatch — events are buffered during `process()`, dispatched to observers after `process()` returns. SharedState is a `#[repr(C)]` snapshot struct with cell-level screen content. Python gets sync callbacks and asyncio queue modes.

**Tech Stack:** Rust (traits, Arc, repr(C)), PyO3 (Python bindings), asyncio (Python async)

---

### Task 1: Core Observer Trait and Registration

**Files:**
- Create: `src/observer.rs`
- Modify: `src/lib.rs:37-66` (add `pub mod observer;`)
- Modify: `src/terminal/mod.rs:1456` (add observer storage fields)

**Step 1: Create `src/observer.rs` with the trait and types**

```rust
//! Terminal observer trait for push-based event delivery
//!
//! Observers receive terminal events via trait callbacks after each `process()` call.
//! Events are dispatched after processing completes (deferred dispatch), ensuring
//! no internal mutexes are held during callbacks.

use std::collections::HashSet;
use std::sync::Arc;

use crate::terminal::{TerminalEvent, TerminalEventKind};

/// Unique identifier for a registered observer
pub type ObserverId = u64;

/// Terminal event observer trait
///
/// Implement this trait to receive push-based terminal events. All methods have
/// default no-op implementations, so you only need to override the ones you care about.
///
/// Events are dispatched in two phases:
/// 1. Category-specific method (`on_zone_event`, `on_command_event`, etc.)
/// 2. Catch-all `on_event` (always called for every event)
///
/// # Thread Safety
/// Observers must be `Send + Sync` since they may be called from different threads.
/// Dispatch happens after `process()` returns — no Terminal internal state is borrowed.
pub trait TerminalObserver: Send + Sync {
    /// Called for zone lifecycle events (ZoneOpened, ZoneClosed, ZoneScrolledOut)
    fn on_zone_event(&self, _event: &TerminalEvent) {}

    /// Called for command/shell integration events (ShellIntegrationEvent)
    fn on_command_event(&self, _event: &TerminalEvent) {}

    /// Called for environment changes (CwdChanged, EnvironmentChanged,
    /// RemoteHostTransition, SubShellDetected)
    fn on_environment_event(&self, _event: &TerminalEvent) {}

    /// Called for screen content events (BellRang, TitleChanged, SizeChanged,
    /// ModeChanged, GraphicsAdded, HyperlinkAdded, DirtyRegion, UserVarChanged,
    /// ProgressBarChanged, BadgeChanged, TriggerMatched)
    fn on_screen_event(&self, _event: &TerminalEvent) {}

    /// Called for ALL events (catch-all). Called after category-specific methods.
    fn on_event(&self, _event: &TerminalEvent) {}

    /// Which event kinds this observer is interested in (None = all)
    fn subscriptions(&self) -> Option<HashSet<TerminalEventKind>> {
        None
    }
}

/// Internal entry for a registered observer
pub(crate) struct ObserverEntry {
    pub id: ObserverId,
    pub observer: Arc<dyn TerminalObserver>,
}

/// Categorize an event for routing to the appropriate observer method
pub(crate) fn event_category(event: &TerminalEvent) -> EventCategory {
    match event {
        TerminalEvent::ZoneOpened { .. }
        | TerminalEvent::ZoneClosed { .. }
        | TerminalEvent::ZoneScrolledOut { .. } => EventCategory::Zone,

        TerminalEvent::ShellIntegrationEvent { .. } => EventCategory::Command,

        TerminalEvent::CwdChanged(_)
        | TerminalEvent::EnvironmentChanged { .. }
        | TerminalEvent::RemoteHostTransition { .. }
        | TerminalEvent::SubShellDetected { .. } => EventCategory::Environment,

        _ => EventCategory::Screen,
    }
}

/// Event category for routing
pub(crate) enum EventCategory {
    Zone,
    Command,
    Environment,
    Screen,
}
```

**Step 2: Register the module in `src/lib.rs`**

Add `pub mod observer;` after the existing module declarations (after line 51, before `pub mod pty_error;`):

```rust
pub mod observer;
```

**Step 3: Add observer storage to Terminal struct**

In `src/terminal/mod.rs`, add these fields to the `Terminal` struct (after `terminal_events` field at line 1456):

```rust
    /// Registered observers for push-based event delivery
    observers: Vec<crate::observer::ObserverEntry>,
    /// Next observer ID to assign
    next_observer_id: crate::observer::ObserverId,
```

**Step 4: Initialize observer fields in Terminal::new()**

Find the `Terminal::new()` constructor and add field initialization:

```rust
    observers: Vec::new(),
    next_observer_id: 1,
```

**Step 5: Add observer management methods to Terminal**

Add these methods to the Terminal impl block (near the existing `poll_events` at line 2971):

```rust
    /// Register an observer for push-based event delivery
    ///
    /// Returns a unique observer ID that can be used to remove the observer later.
    /// Observers receive events after each `process()` call via their trait methods.
    pub fn add_observer(&mut self, observer: Arc<dyn crate::observer::TerminalObserver>) -> crate::observer::ObserverId {
        let id = self.next_observer_id;
        self.next_observer_id += 1;
        self.observers.push(crate::observer::ObserverEntry { id, observer });
        id
    }

    /// Remove a previously registered observer
    ///
    /// Returns true if the observer was found and removed, false if the ID was not found.
    pub fn remove_observer(&mut self, id: crate::observer::ObserverId) -> bool {
        let len_before = self.observers.len();
        self.observers.retain(|entry| entry.id != id);
        self.observers.len() < len_before
    }

    /// Returns the number of currently registered observers
    pub fn observer_count(&self) -> usize {
        self.observers.len()
    }

    /// Dispatch pending events to all registered observers
    ///
    /// Called internally after `process()` returns. Events remain in the buffer
    /// for `poll_events()` backward compatibility.
    fn dispatch_to_observers(&self) {
        if self.observers.is_empty() {
            return;
        }

        for event in &self.terminal_events {
            let kind = Self::event_kind(event);
            let category = crate::observer::event_category(event);

            for entry in &self.observers {
                // Check subscription filter
                if let Some(subs) = entry.observer.subscriptions() {
                    if !subs.contains(&kind) {
                        continue;
                    }
                }

                // Call category-specific method
                match category {
                    crate::observer::EventCategory::Zone => entry.observer.on_zone_event(event),
                    crate::observer::EventCategory::Command => entry.observer.on_command_event(event),
                    crate::observer::EventCategory::Environment => entry.observer.on_environment_event(event),
                    crate::observer::EventCategory::Screen => entry.observer.on_screen_event(event),
                }

                // Always call catch-all
                entry.observer.on_event(event);
            }
        }
    }
```

**Step 6: Hook dispatch into process()**

In `src/terminal/mod.rs`, modify `process()` (line 3440) to call dispatch after processing:

Current:
```rust
    pub fn process(&mut self, data: &[u8]) {
        // ... existing body ...
        self.process_vte_data(data);
    }
```

Add dispatch call at the end of `process()`, just before the closing brace:

```rust
        self.dispatch_to_observers();
```

Also add dispatch at the end of the tmux path (after `return;` on line 3463, add it before the return):

```rust
            self.dispatch_to_observers();
            return;
```

**Step 7: Run `cargo check` to verify compilation**

Run: `cargo check --lib --no-default-features`
Expected: Compiles successfully

**Step 8: Commit**

```bash
git add src/observer.rs src/lib.rs src/terminal/mod.rs
git commit -m "feat(observer): add TerminalObserver trait with deferred dispatch"
```

---

### Task 2: Rust Unit Tests for Observer System

**Files:**
- Create: `src/tests/observer_tests.rs`
- Modify: `src/terminal/mod.rs:7695` (include test file)

**Step 1: Create test file with mock observer**

Create `src/tests/observer_tests.rs`:

```rust
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::observer::{ObserverId, TerminalObserver};
use crate::terminal::{TerminalEvent, TerminalEventKind};

/// Mock observer that records received events
struct MockObserver {
    zone_events: Mutex<Vec<String>>,
    command_events: Mutex<Vec<String>>,
    environment_events: Mutex<Vec<String>>,
    screen_events: Mutex<Vec<String>>,
    all_events: Mutex<Vec<String>>,
    subscriptions: Option<HashSet<TerminalEventKind>>,
}

impl MockObserver {
    fn new() -> Self {
        Self {
            zone_events: Mutex::new(Vec::new()),
            command_events: Mutex::new(Vec::new()),
            environment_events: Mutex::new(Vec::new()),
            screen_events: Mutex::new(Vec::new()),
            all_events: Mutex::new(Vec::new()),
            subscriptions: None,
        }
    }

    fn with_subscriptions(subs: HashSet<TerminalEventKind>) -> Self {
        Self {
            subscriptions: Some(subs),
            ..Self::new()
        }
    }

    fn event_type_str(event: &TerminalEvent) -> String {
        match event {
            TerminalEvent::BellRang(_) => "bell".to_string(),
            TerminalEvent::TitleChanged(t) => format!("title:{t}"),
            TerminalEvent::ZoneOpened { zone_id, .. } => format!("zone_opened:{zone_id}"),
            TerminalEvent::ZoneClosed { zone_id, .. } => format!("zone_closed:{zone_id}"),
            TerminalEvent::ZoneScrolledOut { zone_id, .. } => format!("zone_scrolled_out:{zone_id}"),
            TerminalEvent::ShellIntegrationEvent { event_type, .. } => {
                format!("shell:{event_type}")
            }
            TerminalEvent::CwdChanged(c) => format!("cwd:{}", c.new_cwd),
            TerminalEvent::EnvironmentChanged { key, value, .. } => {
                format!("env:{key}={value}")
            }
            TerminalEvent::SizeChanged(c, r) => format!("size:{c}x{r}"),
            _ => "other".to_string(),
        }
    }

    fn all_event_count(&self) -> usize {
        self.all_events.lock().unwrap().len()
    }

    fn zone_event_count(&self) -> usize {
        self.zone_events.lock().unwrap().len()
    }

    fn screen_event_count(&self) -> usize {
        self.screen_events.lock().unwrap().len()
    }

    fn command_event_count(&self) -> usize {
        self.command_events.lock().unwrap().len()
    }

    fn environment_event_count(&self) -> usize {
        self.environment_events.lock().unwrap().len()
    }
}

impl TerminalObserver for MockObserver {
    fn on_zone_event(&self, event: &TerminalEvent) {
        self.zone_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn on_command_event(&self, event: &TerminalEvent) {
        self.command_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn on_environment_event(&self, event: &TerminalEvent) {
        self.environment_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn on_screen_event(&self, event: &TerminalEvent) {
        self.screen_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn on_event(&self, event: &TerminalEvent) {
        self.all_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn subscriptions(&self) -> Option<HashSet<TerminalEventKind>> {
        self.subscriptions.clone()
    }
}

#[test]
fn test_observer_registration() {
    let mut term = Terminal::new(80, 24, 100);
    assert_eq!(term.observer_count(), 0);

    let obs = Arc::new(MockObserver::new());
    let id = term.add_observer(obs);
    assert_eq!(term.observer_count(), 1);
    assert!(id > 0);
}

#[test]
fn test_observer_removal() {
    let mut term = Terminal::new(80, 24, 100);
    let obs = Arc::new(MockObserver::new());
    let id = term.add_observer(obs);

    assert!(term.remove_observer(id));
    assert_eq!(term.observer_count(), 0);

    // Removing again returns false
    assert!(!term.remove_observer(id));
}

#[test]
fn test_observer_unique_ids() {
    let mut term = Terminal::new(80, 24, 100);
    let id1 = term.add_observer(Arc::new(MockObserver::new()));
    let id2 = term.add_observer(Arc::new(MockObserver::new()));
    let id3 = term.add_observer(Arc::new(MockObserver::new()));

    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);
}

#[test]
fn test_observer_receives_bell_event() {
    let mut term = Terminal::new(80, 24, 100);
    let obs = Arc::new(MockObserver::new());
    let obs_clone = obs.clone();
    term.add_observer(obs);

    // Bell character
    term.process(b"\x07");

    assert!(obs_clone.all_event_count() > 0);
    assert!(obs_clone.screen_event_count() > 0);
    assert_eq!(obs_clone.zone_event_count(), 0);
}

#[test]
fn test_observer_receives_title_change() {
    let mut term = Terminal::new(80, 24, 100);
    let obs = Arc::new(MockObserver::new());
    let obs_clone = obs.clone();
    term.add_observer(obs);

    // OSC 0 set title
    term.process(b"\x1b]0;Hello World\x07");

    let all = obs_clone.all_events.lock().unwrap();
    assert!(all.iter().any(|e| e.contains("title:Hello World")));
}

#[test]
fn test_observer_zone_routing() {
    let mut term = Terminal::new(80, 24, 100);
    let obs = Arc::new(MockObserver::new());
    let obs_clone = obs.clone();
    term.add_observer(obs);

    // Open a prompt zone (OSC 133;A)
    term.process(b"\x1b]133;A\x07");

    assert!(obs_clone.zone_event_count() > 0);
    let zones = obs_clone.zone_events.lock().unwrap();
    assert!(zones.iter().any(|e| e.contains("zone_opened")));
}

#[test]
fn test_observer_command_routing() {
    let mut term = Terminal::new(80, 24, 100);
    let obs = Arc::new(MockObserver::new());
    let obs_clone = obs.clone();
    term.add_observer(obs);

    // Full shell integration cycle: prompt start → command start
    term.process(b"\x1b]133;A\x07");
    term.process(b"\x1b]133;B\x07");
    term.process(b"\x1b]133;C\x07");
    term.process(b"\x1b]133;D;0\x07");

    assert!(obs_clone.command_event_count() > 0);
}

#[test]
fn test_observer_subscription_filter() {
    let mut term = Terminal::new(80, 24, 100);
    let mut subs = HashSet::new();
    subs.insert(TerminalEventKind::TitleChanged);
    let obs = Arc::new(MockObserver::with_subscriptions(subs));
    let obs_clone = obs.clone();
    term.add_observer(obs);

    // Bell (not subscribed)
    term.process(b"\x07");
    assert_eq!(obs_clone.all_event_count(), 0);

    // Title change (subscribed)
    term.process(b"\x1b]0;Filtered\x07");
    assert!(obs_clone.all_event_count() > 0);
}

#[test]
fn test_multiple_observers() {
    let mut term = Terminal::new(80, 24, 100);
    let obs1 = Arc::new(MockObserver::new());
    let obs2 = Arc::new(MockObserver::new());
    let obs1_clone = obs1.clone();
    let obs2_clone = obs2.clone();
    term.add_observer(obs1);
    term.add_observer(obs2);

    term.process(b"\x07");

    assert!(obs1_clone.all_event_count() > 0);
    assert!(obs2_clone.all_event_count() > 0);
}

#[test]
fn test_poll_events_still_works_with_observers() {
    let mut term = Terminal::new(80, 24, 100);
    let obs = Arc::new(MockObserver::new());
    term.add_observer(obs);

    term.process(b"\x07");

    // poll_events should still return events even with observers registered
    let events = term.poll_events();
    assert!(!events.is_empty());
}
```

**Step 2: Include the test file in the test module**

In `src/terminal/mod.rs` at line ~7695, add inside the `#[cfg(test)] mod tests` block:

```rust
    include!("../tests/observer_tests.rs");
```

**Step 3: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features -- observer`
Expected: All 9 tests pass

**Step 4: Commit**

```bash
git add src/tests/observer_tests.rs src/terminal/mod.rs
git commit -m "test(observer): add unit tests for observer registration and event dispatch"
```

---

### Task 3: SharedState FFI Types

**Files:**
- Create: `src/ffi.rs`
- Modify: `src/lib.rs` (add `pub mod ffi;`)

**Step 1: Create `src/ffi.rs` with repr(C) types**

```rust
//! C-compatible FFI types for terminal state access
//!
//! Provides `SharedState` and `SharedCell` as `#[repr(C)]` structs that can be
//! consumed by C/C++ code. The state is a snapshot — values are copied at call time
//! and remain valid until freed.

use std::ffi::{c_char, CString};
use std::os::raw::c_void;
use std::ptr;

use crate::cell::CellBitflags;
use crate::observer::{ObserverId, TerminalObserver};
use crate::terminal::{Terminal, TerminalEvent, TerminalEventKind};
use std::collections::HashSet;
use std::sync::Arc;

/// C-compatible cell representation
#[repr(C)]
#[derive(Debug, Clone)]
pub struct SharedCell {
    /// UTF-8 encoded character, null-padded (max 4 bytes for a single char)
    pub text: [u8; 4],
    /// Length of valid UTF-8 bytes in `text`
    pub text_len: u8,
    /// Foreground color (RGB)
    pub fg_r: u8,
    pub fg_g: u8,
    pub fg_b: u8,
    /// Background color (RGB)
    pub bg_r: u8,
    pub bg_g: u8,
    pub bg_b: u8,
    /// Attribute bitfield (bold=1, dim=2, italic=4, underline=8, blink=16,
    /// reverse=32, hidden=64, strikethrough=128, overline=256, wide=512)
    pub attrs: u16,
    /// Display width (1 or 2 for wide chars, 0 for spacer)
    pub width: u8,
}

/// C-compatible terminal state snapshot
#[repr(C)]
pub struct SharedState {
    /// Terminal column count
    pub cols: u32,
    /// Terminal row count
    pub rows: u32,
    /// Cursor column (0-indexed)
    pub cursor_col: u32,
    /// Cursor row (0-indexed)
    pub cursor_row: u32,
    /// Whether cursor is visible
    pub cursor_visible: bool,
    /// Whether alternate screen is active
    pub alt_screen_active: bool,
    /// Mouse mode (0=none, 1=x10, 2=normal, 3=button, 4=any)
    pub mouse_mode: u8,
    /// Terminal title (null-terminated C string, owned by this struct)
    pub title: *mut c_char,
    /// Length of title string (excluding null terminator)
    pub title_len: u32,
    /// Current working directory (null-terminated C string, owned by this struct)
    pub cwd: *mut c_char,
    /// Length of CWD string (excluding null terminator)
    pub cwd_len: u32,
    /// Pointer to row-major cell array (owned by this struct)
    pub cells: *mut SharedCell,
    /// Total number of cells (cols * rows)
    pub cell_count: u32,
    /// Number of scrollback lines
    pub scrollback_lines: u32,
    /// Total lines (scrollback + visible)
    pub total_lines: u32,
}

impl SharedState {
    /// Create a snapshot of the current terminal state
    pub fn from_terminal(term: &Terminal) -> Self {
        let grid = term.active_grid();
        let cols = grid.cols();
        let rows = grid.rows();
        let cursor = term.cursor();

        // Build title CString
        let title_cstr = CString::new(term.title()).unwrap_or_default();
        let title_len = title_cstr.as_bytes().len() as u32;
        let title = title_cstr.into_raw();

        // Build CWD CString
        let cwd_str = term.get_current_cwd().unwrap_or_default();
        let cwd_cstr = CString::new(cwd_str).unwrap_or_default();
        let cwd_len = cwd_cstr.as_bytes().len() as u32;
        let cwd = cwd_cstr.into_raw();

        // Build cell array
        let cell_count = cols * rows;
        let mut cells: Vec<SharedCell> = Vec::with_capacity(cell_count);

        for row_idx in 0..rows {
            if let Some(row) = grid.row(row_idx) {
                for cell in row.iter().take(cols) {
                    let (fg_r, fg_g, fg_b) = cell.fg.to_rgb();
                    let (bg_r, bg_g, bg_b) = cell.bg.to_rgb();

                    let mut text = [0u8; 4];
                    let encoded = cell.c.encode_utf8(&mut text);
                    let text_len = encoded.len() as u8;

                    let attrs = cell.flags.to_bitflags().bits();
                    let width = cell.width;

                    cells.push(SharedCell {
                        text,
                        text_len,
                        fg_r,
                        fg_g,
                        fg_b,
                        bg_r,
                        bg_g,
                        bg_b,
                        attrs,
                        width,
                    });
                }
            } else {
                // Fill with empty cells if row doesn't exist
                for _ in 0..cols {
                    cells.push(SharedCell {
                        text: [b' ', 0, 0, 0],
                        text_len: 1,
                        fg_r: 255,
                        fg_g: 255,
                        fg_b: 255,
                        bg_r: 0,
                        bg_g: 0,
                        bg_b: 0,
                        attrs: 0,
                        width: 1,
                    });
                }
            }
        }

        let cells_ptr = cells.as_mut_ptr();
        std::mem::forget(cells); // Prevent deallocation — caller must free

        let mouse_mode = match term.mouse_mode() {
            crate::mouse::MouseMode::Off => 0,
            crate::mouse::MouseMode::X10 => 1,
            crate::mouse::MouseMode::Normal => 2,
            crate::mouse::MouseMode::Button => 3,
            crate::mouse::MouseMode::Any => 4,
        };

        let scrollback_stats = term.scrollback_stats();

        SharedState {
            cols: cols as u32,
            rows: rows as u32,
            cursor_col: cursor.col as u32,
            cursor_row: cursor.row as u32,
            cursor_visible: cursor.visible,
            alt_screen_active: term.is_alt_screen_active(),
            mouse_mode,
            title,
            title_len,
            cwd,
            cwd_len,
            cells: cells_ptr,
            cell_count: cell_count as u32,
            scrollback_lines: scrollback_stats.total_lines as u32,
            total_lines: (scrollback_stats.total_lines + rows) as u32,
        }
    }
}

impl Drop for SharedState {
    fn drop(&mut self) {
        unsafe {
            if !self.title.is_null() {
                let _ = CString::from_raw(self.title);
            }
            if !self.cwd.is_null() {
                let _ = CString::from_raw(self.cwd);
            }
            if !self.cells.is_null() {
                let _ = Vec::from_raw_parts(
                    self.cells,
                    self.cell_count as usize,
                    self.cell_count as usize,
                );
            }
        }
    }
}

// ========== C FFI Functions ==========

/// Opaque handle for Terminal
pub type TerminalHandle = *mut c_void;

/// C-compatible observer vtable
#[repr(C)]
pub struct TerminalObserverVtable {
    /// Called for zone events. event_json is a null-terminated JSON string.
    pub on_zone_event: Option<unsafe extern "C" fn(user_data: *mut c_void, event_json: *const c_char)>,
    /// Called for command events
    pub on_command_event: Option<unsafe extern "C" fn(user_data: *mut c_void, event_json: *const c_char)>,
    /// Called for environment events
    pub on_environment_event: Option<unsafe extern "C" fn(user_data: *mut c_void, event_json: *const c_char)>,
    /// Called for screen events
    pub on_screen_event: Option<unsafe extern "C" fn(user_data: *mut c_void, event_json: *const c_char)>,
    /// Called for all events
    pub on_event: Option<unsafe extern "C" fn(user_data: *mut c_void, event_json: *const c_char)>,
    /// User data pointer passed to all callbacks
    pub user_data: *mut c_void,
}

unsafe impl Send for TerminalObserverVtable {}
unsafe impl Sync for TerminalObserverVtable {}

/// FFI observer implementation that delegates to C function pointers
struct FfiObserver {
    vtable: TerminalObserverVtable,
}

unsafe impl Send for FfiObserver {}
unsafe impl Sync for FfiObserver {}

impl FfiObserver {
    fn event_to_json(event: &TerminalEvent) -> CString {
        // Simple JSON serialization for events
        let json = format!("{:?}", event);
        CString::new(json).unwrap_or_default()
    }
}

impl TerminalObserver for FfiObserver {
    fn on_zone_event(&self, event: &TerminalEvent) {
        if let Some(cb) = self.vtable.on_zone_event {
            let json = Self::event_to_json(event);
            unsafe { cb(self.vtable.user_data, json.as_ptr()) }
        }
    }

    fn on_command_event(&self, event: &TerminalEvent) {
        if let Some(cb) = self.vtable.on_command_event {
            let json = Self::event_to_json(event);
            unsafe { cb(self.vtable.user_data, json.as_ptr()) }
        }
    }

    fn on_environment_event(&self, event: &TerminalEvent) {
        if let Some(cb) = self.vtable.on_environment_event {
            let json = Self::event_to_json(event);
            unsafe { cb(self.vtable.user_data, json.as_ptr()) }
        }
    }

    fn on_screen_event(&self, event: &TerminalEvent) {
        if let Some(cb) = self.vtable.on_screen_event {
            let json = Self::event_to_json(event);
            unsafe { cb(self.vtable.user_data, json.as_ptr()) }
        }
    }

    fn on_event(&self, event: &TerminalEvent) {
        if let Some(cb) = self.vtable.on_event {
            let json = Self::event_to_json(event);
            unsafe { cb(self.vtable.user_data, json.as_ptr()) }
        }
    }
}

/// Get a snapshot of terminal state. Caller must free with `terminal_free_state`.
///
/// # Safety
/// `handle` must be a valid pointer to a `Terminal` instance.
#[no_mangle]
pub unsafe extern "C" fn terminal_get_state(handle: TerminalHandle) -> *mut SharedState {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let term = &*(handle as *const Terminal);
    let state = Box::new(SharedState::from_terminal(term));
    Box::into_raw(state)
}

/// Free a previously allocated SharedState
///
/// # Safety
/// `state` must be a pointer returned by `terminal_get_state`, or null.
#[no_mangle]
pub unsafe extern "C" fn terminal_free_state(state: *mut SharedState) {
    if !state.is_null() {
        let _ = Box::from_raw(state);
    }
}

/// Register a C observer via vtable. Returns observer ID (0 on failure).
///
/// # Safety
/// `handle` must be a valid pointer to a mutable `Terminal`.
/// `vtable` must be a valid pointer to a `TerminalObserverVtable`.
#[no_mangle]
pub unsafe extern "C" fn terminal_add_observer(
    handle: TerminalHandle,
    vtable: *const TerminalObserverVtable,
) -> ObserverId {
    if handle.is_null() || vtable.is_null() {
        return 0;
    }
    let term = &mut *(handle as *mut Terminal);
    let vtable = ptr::read(vtable);
    let observer = Arc::new(FfiObserver { vtable });
    term.add_observer(observer)
}

/// Remove a previously registered observer. Returns true if found and removed.
///
/// # Safety
/// `handle` must be a valid pointer to a mutable `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn terminal_remove_observer(
    handle: TerminalHandle,
    id: ObserverId,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let term = &mut *(handle as *mut Terminal);
    term.remove_observer(id)
}
```

**Step 2: Register ffi module in `src/lib.rs`**

Add after the `observer` module:
```rust
pub mod ffi;
```

**Step 3: Check that CellFlags has a `to_bitflags` method**

We need to expose the internal CellBitflags from CellFlags. Check `src/cell.rs` — if `to_bitflags()` doesn't exist, we need to add it. Add this method to the `CellFlags` impl:

```rust
    /// Get the raw bitflags value
    pub fn to_bitflags(&self) -> CellBitflags {
        self.bits
    }
```

**Step 4: Check Terminal has `is_alt_screen_active()` and `get_current_cwd()` public methods**

Search for these and verify they exist. If `is_alt_screen_active()` doesn't exist, add:

```rust
    pub fn is_alt_screen_active(&self) -> bool {
        self.alt_screen_active
    }
```

**Step 5: Run `cargo check`**

Run: `cargo check --lib --no-default-features`
Expected: Compiles successfully

**Step 6: Commit**

```bash
git add src/ffi.rs src/lib.rs src/cell.rs src/terminal/mod.rs
git commit -m "feat(ffi): add SharedState/SharedCell repr(C) types and C API functions"
```

---

### Task 4: FFI Unit Tests

**Files:**
- Create: `src/tests/ffi_tests.rs`
- Modify: `src/terminal/mod.rs` (include test file)

**Step 1: Create FFI test file**

Create `src/tests/ffi_tests.rs`:

```rust
use crate::ffi::{SharedState, SharedCell};
use std::ffi::CStr;

#[test]
fn test_shared_state_dimensions() {
    let term = Terminal::new(80, 24, 100);
    let state = SharedState::from_terminal(&term);

    assert_eq!(state.cols, 80);
    assert_eq!(state.rows, 24);
    assert_eq!(state.cell_count, 80 * 24);
}

#[test]
fn test_shared_state_cursor() {
    let mut term = Terminal::new(80, 24, 100);
    // Move cursor to position (10, 5) using CUP sequence
    term.process(b"\x1b[6;11H"); // 1-indexed: row 6, col 11 = (10, 5) 0-indexed

    let state = SharedState::from_terminal(&term);
    assert_eq!(state.cursor_col, 10);
    assert_eq!(state.cursor_row, 5);
    assert!(state.cursor_visible);
}

#[test]
fn test_shared_state_title() {
    let mut term = Terminal::new(80, 24, 100);
    term.process(b"\x1b]0;Test Title\x07");

    let state = SharedState::from_terminal(&term);
    let title = unsafe { CStr::from_ptr(state.title) };
    assert_eq!(title.to_str().unwrap(), "Test Title");
    assert_eq!(state.title_len, 10);
}

#[test]
fn test_shared_state_cell_content() {
    let mut term = Terminal::new(80, 24, 100);
    term.process(b"ABC");

    let state = SharedState::from_terminal(&term);
    assert!(!state.cells.is_null());

    unsafe {
        let cell0 = &*state.cells;
        let text = std::str::from_utf8(&cell0.text[..cell0.text_len as usize]).unwrap();
        assert_eq!(text, "A");
        assert_eq!(cell0.width, 1);

        let cell1 = &*state.cells.add(1);
        let text1 = std::str::from_utf8(&cell1.text[..cell1.text_len as usize]).unwrap();
        assert_eq!(text1, "B");

        let cell2 = &*state.cells.add(2);
        let text2 = std::str::from_utf8(&cell2.text[..cell2.text_len as usize]).unwrap();
        assert_eq!(text2, "C");
    }
}

#[test]
fn test_shared_state_alt_screen() {
    let mut term = Terminal::new(80, 24, 100);
    assert!(!SharedState::from_terminal(&term).alt_screen_active);

    // Enter alt screen
    term.process(b"\x1b[?1049h");
    assert!(SharedState::from_terminal(&term).alt_screen_active);
}

#[test]
fn test_shared_state_drop_frees_memory() {
    let term = Terminal::new(80, 24, 100);
    let state = SharedState::from_terminal(&term);
    // Drop should not panic or leak
    drop(state);
}
```

**Step 2: Include in test module**

In `src/terminal/mod.rs` test module, add:
```rust
    include!("../tests/ffi_tests.rs");
```

**Step 3: Run tests**

Run: `cargo test --lib --no-default-features -- ffi`
Expected: All 6 tests pass

**Step 4: Commit**

```bash
git add src/tests/ffi_tests.rs src/terminal/mod.rs
git commit -m "test(ffi): add unit tests for SharedState snapshot"
```

---

### Task 5: Python Bindings for Observer API

**Files:**
- Create: `src/python_bindings/observer.rs`
- Modify: `src/python_bindings/mod.rs` (add `pub mod observer;`)
- Modify: `src/python_bindings/terminal.rs` (add `add_observer`, `add_async_observer`, `remove_observer` methods)
- Modify: `src/lib.rs` (re-export new types)

**Step 1: Create `src/python_bindings/observer.rs`**

```rust
//! Python bindings for terminal observer API
//!
//! Provides PyCallbackObserver (sync) and PyQueueObserver (async) implementations
//! of the TerminalObserver trait for Python consumers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pyo3::prelude::*;

use crate::observer::TerminalObserver;
use crate::terminal::{TerminalEvent, TerminalEventKind};

/// Convert a TerminalEvent to a Python dict (HashMap<String, String>)
///
/// This is extracted from PyTerminal::poll_events() to be reusable by observers.
pub(crate) fn event_to_dict(event: &TerminalEvent) -> HashMap<String, String> {
    let mut map = HashMap::new();
    match event {
        TerminalEvent::BellRang(bell) => {
            map.insert("type".to_string(), "bell".to_string());
            match bell {
                crate::terminal::BellEvent::VisualBell => {
                    map.insert("bell_type".to_string(), "visual".to_string());
                }
                crate::terminal::BellEvent::WarningBell(vol) => {
                    map.insert("bell_type".to_string(), "warning".to_string());
                    map.insert("volume".to_string(), vol.to_string());
                }
                crate::terminal::BellEvent::MarginBell(vol) => {
                    map.insert("bell_type".to_string(), "margin".to_string());
                    map.insert("volume".to_string(), vol.to_string());
                }
            }
        }
        TerminalEvent::TitleChanged(title) => {
            map.insert("type".to_string(), "title_changed".to_string());
            map.insert("title".to_string(), title.clone());
        }
        TerminalEvent::SizeChanged(cols, rows) => {
            map.insert("type".to_string(), "size_changed".to_string());
            map.insert("cols".to_string(), cols.to_string());
            map.insert("rows".to_string(), rows.to_string());
        }
        TerminalEvent::ModeChanged(mode, enabled) => {
            map.insert("type".to_string(), "mode_changed".to_string());
            map.insert("mode".to_string(), mode.clone());
            map.insert("enabled".to_string(), enabled.to_string());
        }
        TerminalEvent::GraphicsAdded(row) => {
            map.insert("type".to_string(), "graphics_added".to_string());
            map.insert("row".to_string(), row.to_string());
        }
        TerminalEvent::HyperlinkAdded { url, row, col, id } => {
            map.insert("type".to_string(), "hyperlink_added".to_string());
            map.insert("url".to_string(), url.clone());
            map.insert("row".to_string(), row.to_string());
            map.insert("col".to_string(), col.to_string());
            if let Some(id) = id {
                map.insert("id".to_string(), id.to_string());
            }
        }
        TerminalEvent::DirtyRegion(first, last) => {
            map.insert("type".to_string(), "dirty_region".to_string());
            map.insert("first_row".to_string(), first.to_string());
            map.insert("last_row".to_string(), last.to_string());
        }
        TerminalEvent::CwdChanged(change) => {
            map.insert("type".to_string(), "cwd_changed".to_string());
            if let Some(old) = &change.old_cwd {
                map.insert("old_cwd".to_string(), old.clone());
            }
            map.insert("new_cwd".to_string(), change.new_cwd.clone());
            if let Some(host) = &change.hostname {
                map.insert("hostname".to_string(), host.clone());
            }
            if let Some(user) = &change.username {
                map.insert("username".to_string(), user.clone());
            }
            map.insert("timestamp".to_string(), change.timestamp.to_string());
        }
        TerminalEvent::TriggerMatched(trigger_match) => {
            map.insert("type".to_string(), "trigger_matched".to_string());
            map.insert("trigger_id".to_string(), trigger_match.trigger_id.to_string());
            map.insert("row".to_string(), trigger_match.row.to_string());
            map.insert("col".to_string(), trigger_match.col.to_string());
            map.insert("end_col".to_string(), trigger_match.end_col.to_string());
            map.insert("text".to_string(), trigger_match.text.clone());
            map.insert("timestamp".to_string(), trigger_match.timestamp.to_string());
        }
        TerminalEvent::UserVarChanged { name, value, old_value } => {
            map.insert("type".to_string(), "user_var_changed".to_string());
            map.insert("name".to_string(), name.clone());
            map.insert("value".to_string(), value.clone());
            if let Some(old) = old_value {
                map.insert("old_value".to_string(), old.clone());
            }
        }
        TerminalEvent::ProgressBarChanged { action, id, state, percent, label } => {
            map.insert("type".to_string(), "progress_bar_changed".to_string());
            let action_str = match action {
                crate::terminal::ProgressBarAction::Set => "set",
                crate::terminal::ProgressBarAction::Remove => "remove",
                crate::terminal::ProgressBarAction::RemoveAll => "remove_all",
            };
            map.insert("action".to_string(), action_str.to_string());
            map.insert("id".to_string(), id.clone());
            if let Some(s) = state {
                map.insert("state".to_string(), s.description().to_string());
            }
            if let Some(p) = percent {
                map.insert("percent".to_string(), p.to_string());
            }
            if let Some(l) = label {
                map.insert("label".to_string(), l.clone());
            }
        }
        TerminalEvent::BadgeChanged(badge) => {
            map.insert("type".to_string(), "badge_changed".to_string());
            if let Some(b) = badge {
                map.insert("badge".to_string(), b.clone());
            }
        }
        TerminalEvent::ShellIntegrationEvent { event_type, command, exit_code, timestamp, cursor_line } => {
            map.insert("type".to_string(), "shell_integration".to_string());
            map.insert("event_type".to_string(), event_type.clone());
            if let Some(cmd) = command {
                map.insert("command".to_string(), cmd.clone());
            }
            if let Some(code) = exit_code {
                map.insert("exit_code".to_string(), code.to_string());
            }
            if let Some(line) = cursor_line {
                map.insert("cursor_line".to_string(), line.to_string());
            }
            if let Some(ts) = timestamp {
                map.insert("timestamp".to_string(), ts.to_string());
            }
        }
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
    }
    map
}

/// Synchronous Python callback observer
///
/// Wraps a Python callable and calls it with event dicts during dispatch.
/// GIL is acquired only during the callback, never while Rust state is borrowed.
pub(crate) struct PyCallbackObserver {
    callback: PyObject,
    subscriptions: Option<HashSet<TerminalEventKind>>,
}

impl PyCallbackObserver {
    pub fn new(callback: PyObject, subscriptions: Option<HashSet<TerminalEventKind>>) -> Self {
        Self { callback, subscriptions }
    }
}

unsafe impl Send for PyCallbackObserver {}
unsafe impl Sync for PyCallbackObserver {}

impl TerminalObserver for PyCallbackObserver {
    fn on_event(&self, event: &TerminalEvent) {
        let dict = event_to_dict(event);
        Python::with_gil(|py| {
            if let Err(e) = self.callback.call1(py, (dict,)) {
                // Log error but don't panic — observer errors shouldn't crash the terminal
                eprintln!("Observer callback error: {e}");
            }
        });
    }

    fn subscriptions(&self) -> Option<HashSet<TerminalEventKind>> {
        self.subscriptions.clone()
    }
}

/// Async queue-based observer
///
/// Pushes event dicts to a Python asyncio.Queue using put_nowait.
/// The Python consumer awaits queue.get() in their event loop.
pub(crate) struct PyQueueObserver {
    queue: PyObject,
    subscriptions: Option<HashSet<TerminalEventKind>>,
}

impl PyQueueObserver {
    pub fn new(queue: PyObject, subscriptions: Option<HashSet<TerminalEventKind>>) -> Self {
        Self { queue, subscriptions }
    }
}

unsafe impl Send for PyQueueObserver {}
unsafe impl Sync for PyQueueObserver {}

impl TerminalObserver for PyQueueObserver {
    fn on_event(&self, event: &TerminalEvent) {
        let dict = event_to_dict(event);
        Python::with_gil(|py| {
            if let Err(e) = self.queue.call_method1(py, "put_nowait", (dict,)) {
                eprintln!("Observer queue error: {e}");
            }
        });
    }

    fn subscriptions(&self) -> Option<HashSet<TerminalEventKind>> {
        self.subscriptions.clone()
    }
}
```

**Step 2: Register module in `src/python_bindings/mod.rs`**

Add after existing module declarations:
```rust
pub mod observer;
```

**Step 3: Add observer methods to PyTerminal in `src/python_bindings/terminal.rs`**

Add these methods to the `#[pymethods]` impl block of `PyTerminal`. Place them near the existing `poll_events` method (around line 2804):

```rust
    /// Register a synchronous observer callback
    ///
    /// The callback receives event dicts (same format as poll_events) after each process() call.
    ///
    /// Args:
    ///     callback: A callable that takes a single dict argument
    ///     kinds: Optional list of event kind strings to filter (same values as set_event_subscription)
    ///
    /// Returns:
    ///     int: Observer ID for later removal
    ///
    /// Example:
    ///     >>> def handler(event):
    ///     ...     print(event["type"])
    ///     >>> observer_id = terminal.add_observer(handler)
    #[pyo3(signature = (callback, kinds=None))]
    fn add_observer(&mut self, callback: PyObject, kinds: Option<Vec<String>>) -> PyResult<u64> {
        use crate::python_bindings::observer::PyCallbackObserver;
        let subs = kinds.map(|items| {
            items.into_iter().filter_map(|k| Self::parse_event_kind(&k)).collect()
        });
        let observer = std::sync::Arc::new(PyCallbackObserver::new(callback, subs));
        Ok(self.inner.add_observer(observer))
    }

    /// Register an async observer using an asyncio.Queue
    ///
    /// Returns an (observer_id, queue) tuple. Events are pushed to the queue
    /// using put_nowait(). The caller should await queue.get() in their event loop.
    ///
    /// Args:
    ///     kinds: Optional list of event kind strings to filter
    ///
    /// Returns:
    ///     tuple[int, asyncio.Queue]: Observer ID and the queue to consume events from
    ///
    /// Example:
    ///     >>> observer_id, queue = terminal.add_async_observer()
    ///     >>> event = await queue.get()
    #[pyo3(signature = (kinds=None))]
    fn add_async_observer(&mut self, py: Python<'_>, kinds: Option<Vec<String>>) -> PyResult<(u64, PyObject)> {
        use crate::python_bindings::observer::PyQueueObserver;
        let asyncio = py.import("asyncio")?;
        let queue = asyncio.call_method0("Queue")?;
        let queue_obj = queue.into_pyobject(py)?.unbind();

        let subs = kinds.map(|items| {
            items.into_iter().filter_map(|k| Self::parse_event_kind(&k)).collect()
        });
        let observer = std::sync::Arc::new(PyQueueObserver::new(queue_obj.clone(), subs));
        let id = self.inner.add_observer(observer);
        Ok((id, queue_obj))
    }

    /// Remove a previously registered observer
    ///
    /// Args:
    ///     observer_id: The ID returned by add_observer or add_async_observer
    ///
    /// Returns:
    ///     bool: True if the observer was found and removed
    ///
    /// Example:
    ///     >>> terminal.remove_observer(observer_id)
    ///     True
    fn remove_observer(&mut self, observer_id: u64) -> PyResult<bool> {
        Ok(self.inner.remove_observer(observer_id))
    }

    /// Get the number of currently registered observers
    ///
    /// Returns:
    ///     int: Number of active observers
    fn observer_count(&self) -> PyResult<usize> {
        Ok(self.inner.observer_count())
    }
```

**Step 4: Add `parse_event_kind` helper to PyTerminal**

Add this private helper method to the PyTerminal impl (can be non-pymethods):

```rust
impl PyTerminal {
    /// Parse a string event kind name to TerminalEventKind
    fn parse_event_kind(kind: &str) -> Option<TerminalEventKind> {
        use crate::terminal::TerminalEventKind;
        match kind {
            "bell" => Some(TerminalEventKind::BellRang),
            "title_changed" => Some(TerminalEventKind::TitleChanged),
            "size_changed" => Some(TerminalEventKind::SizeChanged),
            "mode_changed" => Some(TerminalEventKind::ModeChanged),
            "graphics_added" => Some(TerminalEventKind::GraphicsAdded),
            "hyperlink_added" => Some(TerminalEventKind::HyperlinkAdded),
            "dirty_region" => Some(TerminalEventKind::DirtyRegion),
            "cwd_changed" => Some(TerminalEventKind::CwdChanged),
            "trigger_matched" => Some(TerminalEventKind::TriggerMatched),
            "user_var_changed" => Some(TerminalEventKind::UserVarChanged),
            "progress_bar_changed" => Some(TerminalEventKind::ProgressBarChanged),
            "badge_changed" => Some(TerminalEventKind::BadgeChanged),
            "shell_integration" => Some(TerminalEventKind::ShellIntegrationEvent),
            "zone_opened" => Some(TerminalEventKind::ZoneOpened),
            "zone_closed" => Some(TerminalEventKind::ZoneClosed),
            "zone_scrolled_out" => Some(TerminalEventKind::ZoneScrolledOut),
            "environment_changed" => Some(TerminalEventKind::EnvironmentChanged),
            "remote_host_transition" => Some(TerminalEventKind::RemoteHostTransition),
            "sub_shell_detected" => Some(TerminalEventKind::SubShellDetected),
            _ => None,
        }
    }
}
```

**Step 5: Refactor existing `poll_events` to use shared `event_to_dict`**

Replace the body of `PyTerminal::poll_events()` (lines 2585-2804) to use the shared function:

```rust
    fn poll_events(&mut self) -> PyResult<Vec<HashMap<String, String>>> {
        use crate::python_bindings::observer::event_to_dict;
        let events = self.inner.poll_events();
        Ok(events.iter().map(event_to_dict).collect())
    }
```

**Step 6: Build with maturin**

Run: `make dev`
Expected: Build succeeds

**Step 7: Commit**

```bash
git add src/python_bindings/observer.rs src/python_bindings/mod.rs src/python_bindings/terminal.rs src/lib.rs
git commit -m "feat(python): add observer bindings with sync callback and async queue modes"
```

---

### Task 6: Python Convenience Wrappers

**Files:**
- Create: `python/par_term_emu_core_rust/observers.py`
- Modify: `python/par_term_emu_core_rust/__init__.py` (add import)

**Step 1: Create `python/par_term_emu_core_rust/observers.py`**

```python
"""Convenience wrappers for terminal observer patterns.

Provides easy-to-use functions for common observer use cases,
built on top of Terminal.add_observer().
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any


def on_command_complete(
    terminal: Any,
    callback: Callable[[dict[str, str]], None],
) -> int:
    """Register a callback for command completion events.

    Fires when a shell integration command_finished event occurs,
    providing the exit code and command details.

    Args:
        terminal: Terminal instance to observe.
        callback: Callable receiving event dict with keys:
            type, event_type, exit_code, timestamp.

    Returns:
        Observer ID for later removal via terminal.remove_observer().

    Example:
        >>> def on_done(event):
        ...     print(f"Command exited with code {event.get('exit_code')}")
        >>> observer_id = on_command_complete(terminal, on_done)
    """

    def handler(event: dict[str, str]) -> None:
        if (
            event.get("type") == "shell_integration"
            and event.get("event_type") == "command_finished"
        ):
            callback(event)

    return terminal.add_observer(handler, kinds=["shell_integration"])


def on_zone_change(
    terminal: Any,
    callback: Callable[[dict[str, str]], None],
) -> int:
    """Register a callback for zone lifecycle events.

    Fires when zones are opened, closed, or scrolled out.

    Args:
        terminal: Terminal instance to observe.
        callback: Callable receiving event dict with keys:
            type, zone_id, zone_type, and position info.

    Returns:
        Observer ID for later removal via terminal.remove_observer().

    Example:
        >>> def on_zone(event):
        ...     print(f"Zone {event['zone_id']} {event['type']}")
        >>> observer_id = on_zone_change(terminal, on_zone)
    """
    return terminal.add_observer(
        callback,
        kinds=["zone_opened", "zone_closed", "zone_scrolled_out"],
    )


def on_cwd_change(
    terminal: Any,
    callback: Callable[[dict[str, str]], None],
) -> int:
    """Register a callback for working directory changes.

    Fires when the terminal's current working directory changes
    (typically via OSC 7).

    Args:
        terminal: Terminal instance to observe.
        callback: Callable receiving event dict with keys:
            type, new_cwd, old_cwd, hostname, username, timestamp.

    Returns:
        Observer ID for later removal via terminal.remove_observer().

    Example:
        >>> def on_cwd(event):
        ...     print(f"CWD changed to {event['new_cwd']}")
        >>> observer_id = on_cwd_change(terminal, on_cwd)
    """
    return terminal.add_observer(callback, kinds=["cwd_changed"])


def on_title_change(
    terminal: Any,
    callback: Callable[[dict[str, str]], None],
) -> int:
    """Register a callback for terminal title changes.

    Args:
        terminal: Terminal instance to observe.
        callback: Callable receiving event dict with keys: type, title.

    Returns:
        Observer ID for later removal via terminal.remove_observer().

    Example:
        >>> def on_title(event):
        ...     print(f"Title: {event['title']}")
        >>> observer_id = on_title_change(terminal, on_title)
    """
    return terminal.add_observer(callback, kinds=["title_changed"])


def on_bell(
    terminal: Any,
    callback: Callable[[dict[str, str]], None],
) -> int:
    """Register a callback for bell events.

    Args:
        terminal: Terminal instance to observe.
        callback: Callable receiving event dict with keys:
            type, bell_type, volume (for warning/margin bells).

    Returns:
        Observer ID for later removal via terminal.remove_observer().
    """
    return terminal.add_observer(callback, kinds=["bell"])
```

**Step 2: Add import to `__init__.py`**

Add at the end of `python/par_term_emu_core_rust/__init__.py`:

```python
from .observers import (
    on_bell,
    on_command_complete,
    on_cwd_change,
    on_title_change,
    on_zone_change,
)
```

**Step 3: Commit**

```bash
git add python/par_term_emu_core_rust/observers.py python/par_term_emu_core_rust/__init__.py
git commit -m "feat(python): add convenience observer wrappers (on_command_complete, on_zone_change, etc.)"
```

---

### Task 7: Python Integration Tests

**Files:**
- Create: `tests/test_observer.py`

**Step 1: Create `tests/test_observer.py`**

```python
"""Tests for terminal observer API (sync callbacks, async queue, convenience wrappers)."""

from __future__ import annotations

import asyncio

from par_term_emu_core_rust import Terminal
from par_term_emu_core_rust.observers import (
    on_bell,
    on_command_complete,
    on_cwd_change,
    on_title_change,
    on_zone_change,
)


class TestSyncObserver:
    """Test synchronous callback observers."""

    def test_add_and_remove_observer(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = term.add_observer(lambda e: events.append(e))
        assert term.observer_count() == 1
        assert term.remove_observer(observer_id)
        assert term.observer_count() == 0

    def test_observer_receives_bell(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        term.add_observer(lambda e: events.append(e))
        term.process(b"\x07")
        assert any(e["type"] == "bell" for e in events)

    def test_observer_receives_title_change(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        term.add_observer(lambda e: events.append(e))
        term.process(b"\x1b]0;Test Title\x07")
        assert any(
            e["type"] == "title_changed" and e["title"] == "Test Title" for e in events
        )

    def test_observer_with_filter(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        term.add_observer(lambda e: events.append(e), kinds=["title_changed"])
        # Bell should be filtered out
        term.process(b"\x07")
        assert not any(e["type"] == "bell" for e in events)
        # Title should come through
        term.process(b"\x1b]0;Filtered\x07")
        assert any(e["type"] == "title_changed" for e in events)

    def test_multiple_observers(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events1: list[dict[str, str]] = []
        events2: list[dict[str, str]] = []
        term.add_observer(lambda e: events1.append(e))
        term.add_observer(lambda e: events2.append(e))
        term.process(b"\x07")
        assert len(events1) > 0
        assert len(events2) > 0

    def test_observer_removal_stops_delivery(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = term.add_observer(lambda e: events.append(e))
        term.process(b"\x07")
        count_after_first = len(events)
        term.remove_observer(observer_id)
        term.process(b"\x07")
        assert len(events) == count_after_first

    def test_poll_events_still_works_with_observer(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        term.add_observer(lambda e: events.append(e))
        term.process(b"\x07")
        # poll_events should still return events
        polled = term.poll_events()
        assert len(polled) > 0


class TestAsyncObserver:
    """Test async queue-based observers."""

    def test_async_observer_returns_queue(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        observer_id, queue = term.add_async_observer()
        assert observer_id > 0
        assert hasattr(queue, "get")
        assert hasattr(queue, "put_nowait")
        term.remove_observer(observer_id)

    def test_async_observer_receives_events(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        observer_id, queue = term.add_async_observer()
        term.process(b"\x1b]0;Async Test\x07")
        # Queue should have events (non-async get_nowait)
        events = []
        while not queue.empty():
            events.append(queue.get_nowait())
        assert any(
            e["type"] == "title_changed" and e["title"] == "Async Test" for e in events
        )
        term.remove_observer(observer_id)

    def test_async_observer_with_filter(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        observer_id, queue = term.add_async_observer(kinds=["title_changed"])
        term.process(b"\x07")  # Bell - should be filtered
        term.process(b"\x1b]0;Filtered Async\x07")  # Title - should come through
        events = []
        while not queue.empty():
            events.append(queue.get_nowait())
        assert not any(e["type"] == "bell" for e in events)
        assert any(e["type"] == "title_changed" for e in events)
        term.remove_observer(observer_id)

    def test_async_observer_with_asyncio_loop(self) -> None:
        """Test that async observer works with a real asyncio event loop."""

        async def run_test() -> list[dict[str, str]]:
            term = Terminal(80, 24, scrollback=100)
            observer_id, queue = term.add_async_observer()
            term.process(b"\x1b]0;Async Loop\x07")
            events = []
            while not queue.empty():
                event = queue.get_nowait()
                events.append(event)
            term.remove_observer(observer_id)
            return events

        events = asyncio.run(run_test())
        assert any(e["type"] == "title_changed" for e in events)


class TestConvenienceWrappers:
    """Test convenience observer functions."""

    def test_on_command_complete(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_command_complete(term, lambda e: events.append(e))
        # Full shell integration cycle
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"\x1b]133;D;0\x07")
        # Should only get command_finished events
        assert len(events) > 0
        assert all(e.get("event_type") == "command_finished" for e in events)
        term.remove_observer(observer_id)

    def test_on_zone_change(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_zone_change(term, lambda e: events.append(e))
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        assert any(e["type"] == "zone_opened" for e in events)
        term.remove_observer(observer_id)

    def test_on_cwd_change(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_cwd_change(term, lambda e: events.append(e))
        term.process(b"\x1b]7;file:///home/user/test\x07")
        assert any(e["type"] == "cwd_changed" for e in events)
        term.remove_observer(observer_id)

    def test_on_title_change(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_title_change(term, lambda e: events.append(e))
        term.process(b"\x1b]0;New Title\x07")
        assert any(
            e["type"] == "title_changed" and e["title"] == "New Title" for e in events
        )
        term.remove_observer(observer_id)

    def test_on_bell(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_bell(term, lambda e: events.append(e))
        term.process(b"\x07")
        assert any(e["type"] == "bell" for e in events)
        term.remove_observer(observer_id)
```

**Step 2: Build the Python package**

Run: `make dev`
Expected: Build succeeds

**Step 3: Run Python tests**

Run: `uv run pytest tests/test_observer.py -v`
Expected: All tests pass

**Step 4: Commit**

```bash
git add tests/test_observer.py
git commit -m "test(python): add integration tests for observer API"
```

---

### Task 8: Documentation Updates

**Files:**
- Modify: `docs/API_REFERENCE.md` (add observer API section)
- Modify: `README.md` (add observer feature to "What's New" and feature list)

**Step 1: Add Observer API section to API_REFERENCE.md**

Add a new section for the Observer API documenting:
- `Terminal.add_observer(callback, kinds=None) -> int`
- `Terminal.add_async_observer(kinds=None) -> tuple[int, asyncio.Queue]`
- `Terminal.remove_observer(observer_id) -> bool`
- `Terminal.observer_count() -> int`
- Convenience functions from `observers.py`
- Event dict format reference
- Usage examples for sync, async, and convenience patterns

**Step 2: Update README.md**

Add observer/scripting API to the feature list and "What's New" section.

**Step 3: Commit**

```bash
git add docs/API_REFERENCE.md README.md
git commit -m "docs: add observer API to reference documentation and README"
```

---

### Task 9: Run Full Quality Checks

**Step 1: Run make checkall**

Run: `make checkall`
Expected: All checks pass (fmt, lint, clippy, pyright, tests)

**Step 2: Fix any issues found**

If any checks fail, fix them and re-run until clean.

**Step 3: Final commit if fixes were needed**

```bash
git add -A
git commit -m "fix: address quality check issues for observer API"
```

---

### Task 10: Create Pull Request

**Step 1: Push branch**

```bash
git push -u origin feat/terminal-observer-api
```

**Step 2: Create PR**

```bash
gh pr create --title "feat: add TerminalObserver API with FFI and Python bindings" --body "$(cat <<'EOF'
## Summary

Closes #39

- Adds `TerminalObserver` trait with deferred dispatch (events pushed after `process()` returns)
- Category-specific callbacks: `on_zone_event`, `on_command_event`, `on_environment_event`, `on_screen_event`, plus catch-all `on_event`
- Subscription filtering via `TerminalEventKind` sets
- C-compatible `SharedState`/`SharedCell` `#[repr(C)]` snapshot types with full screen content
- C FFI API: `terminal_get_state`, `terminal_free_state`, `terminal_add_observer`, `terminal_remove_observer`
- Python sync observer: `Terminal.add_observer(callback, kinds=None)`
- Python async observer: `Terminal.add_async_observer(kinds=None)` returns `(id, asyncio.Queue)`
- Convenience wrappers: `on_command_complete`, `on_zone_change`, `on_cwd_change`, `on_title_change`, `on_bell`
- Backward compatible: `poll_events()` continues to work with observers registered

## Test plan

- [ ] Rust unit tests: observer registration/removal, event dispatch, subscription filtering, category routing, multiple observers, backward compatibility
- [ ] Rust FFI tests: SharedState dimensions, cursor, title, cell content, alt screen, memory cleanup
- [ ] Python integration tests: sync observer, async observer, filtered observers, convenience wrappers, observer cleanup
- [ ] `make checkall` passes
EOF
)"
```
