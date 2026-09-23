//! Python observer bindings for push-based event delivery
//!
//! Provides `PyCallbackObserver` (sync callback) and `PyQueueObserver` (asyncio.Queue)
//! that bridge the Rust `TerminalObserver` trait to Python callables.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

use crate::observer::TerminalObserver;
use crate::terminal::{TerminalEvent, TerminalEventKind};

/// A field value in an event dictionary, before it is rendered to Python.
///
/// `event_fields` is the single source of truth for event-to-dict conversion,
/// shared by `poll_events()`, `poll_subscribed_events()`, and observer
/// dispatch. Numeric, boolean, and optional Rust fields map to `Int`/`Bool`/
/// `None` so the Python-facing dicts carry native types; the legacy renderer
/// stringifies them back to the pre-0.51 shape.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EventField {
    Str(String),
    Int(i64),
    Bool(bool),
    None,
}

/// Collect an event's dictionary fields with native Rust types.
///
/// Optional fields are always present, as `EventField::None` when unset — the
/// legacy renderer omits them instead.
pub(crate) fn event_fields(event: &TerminalEvent) -> Vec<(String, EventField)> {
    let mut fields: Vec<(String, EventField)> = Vec::new();
    macro_rules! put {
        ($key:expr, $val:expr) => {
            fields.push(($key.to_string(), $val))
        };
    }
    match event {
        TerminalEvent::BellRang(bell) => {
            put!("type", EventField::Str("bell".to_string()));
            match bell {
                crate::terminal::BellEvent::VisualBell => {
                    put!("bell_type", EventField::Str("visual".to_string()));
                }
                crate::terminal::BellEvent::WarningBell(vol) => {
                    put!("bell_type", EventField::Str("warning".to_string()));
                    put!("volume", EventField::Int(*vol as i64));
                }
                crate::terminal::BellEvent::MarginBell(vol) => {
                    put!("bell_type", EventField::Str("margin".to_string()));
                    put!("volume", EventField::Int(*vol as i64));
                }
            }
        }
        TerminalEvent::TitleChanged(title) => {
            put!("type", EventField::Str("title_changed".to_string()));
            put!("title", EventField::Str(title.clone()));
        }
        TerminalEvent::SizeChanged(cols, rows) => {
            put!("type", EventField::Str("size_changed".to_string()));
            put!("cols", EventField::Int(*cols as i64));
            put!("rows", EventField::Int(*rows as i64));
        }
        TerminalEvent::ModeChanged(mode, enabled) => {
            put!("type", EventField::Str("mode_changed".to_string()));
            put!("mode", EventField::Str(mode.clone()));
            put!("enabled", EventField::Bool(*enabled));
        }
        TerminalEvent::GraphicsAdded(row) => {
            put!("type", EventField::Str("graphics_added".to_string()));
            put!("row", EventField::Int(*row as i64));
        }
        TerminalEvent::HyperlinkAdded { url, row, col, id } => {
            put!("type", EventField::Str("hyperlink_added".to_string()));
            put!("url", EventField::Str(url.clone()));
            put!("row", EventField::Int(*row as i64));
            put!("col", EventField::Int(*col as i64));
            put!(
                "id",
                id.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::DirtyRegion(first, last) => {
            put!("type", EventField::Str("dirty_region".to_string()));
            put!("first_row", EventField::Int(*first as i64));
            put!("last_row", EventField::Int(*last as i64));
        }
        TerminalEvent::CwdChanged(change) => {
            put!("type", EventField::Str("cwd_changed".to_string()));
            put!(
                "old_cwd",
                change
                    .old_cwd
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
            put!("new_cwd", EventField::Str(change.new_cwd.clone()));
            put!(
                "hostname",
                change
                    .hostname
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
            put!(
                "username",
                change
                    .username
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
            put!("timestamp", EventField::Int(change.timestamp as i64));
        }
        TerminalEvent::TriggerMatched(trigger_match) => {
            put!("type", EventField::Str("trigger_matched".to_string()));
            put!(
                "trigger_id",
                EventField::Int(trigger_match.trigger_id as i64)
            );
            put!("row", EventField::Int(trigger_match.row as i64));
            put!("col", EventField::Int(trigger_match.col as i64));
            put!("end_col", EventField::Int(trigger_match.end_col as i64));
            put!("text", EventField::Str(trigger_match.text.clone()));
            put!("timestamp", EventField::Int(trigger_match.timestamp as i64));
        }
        TerminalEvent::UserVarChanged {
            name,
            value,
            old_value,
        } => {
            put!("type", EventField::Str("user_var_changed".to_string()));
            put!("name", EventField::Str(name.clone()));
            put!("value", EventField::Str(value.clone()));
            put!(
                "old_value",
                old_value.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::ProgressBarChanged {
            action,
            id,
            state,
            percent,
            label,
        } => {
            put!("type", EventField::Str("progress_bar_changed".to_string()));
            let action_str = match action {
                crate::terminal::ProgressBarAction::Set => "set",
                crate::terminal::ProgressBarAction::Remove => "remove",
                crate::terminal::ProgressBarAction::RemoveAll => "remove_all",
            };
            put!("action", EventField::Str(action_str.to_string()));
            put!("id", EventField::Str(id.clone()));
            put!(
                "state",
                state
                    .as_ref()
                    .map(|s| EventField::Str(s.description().to_string()))
                    .unwrap_or(EventField::None)
            );
            put!(
                "percent",
                percent.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
            put!(
                "label",
                label.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::BadgeChanged(badge) => {
            put!("type", EventField::Str("badge_changed".to_string()));
            put!(
                "badge",
                badge.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::ShellIntegrationEvent {
            event_type,
            command,
            exit_code,
            timestamp,
            cursor_line,
        } => {
            put!("type", EventField::Str("shell_integration".to_string()));
            put!("event_type", EventField::Str(event_type.clone()));
            put!(
                "command",
                command.clone().map_or(EventField::None, EventField::Str)
            );
            put!(
                "exit_code",
                exit_code.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
            put!(
                "cursor_line",
                cursor_line.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
            put!(
                "timestamp",
                timestamp.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::ZoneOpened {
            zone_id,
            zone_type,
            abs_row_start,
        } => {
            put!("type", EventField::Str("zone_opened".to_string()));
            put!("zone_id", EventField::Int(*zone_id as i64));
            put!("zone_type", EventField::Str(zone_type.to_string()));
            put!("abs_row_start", EventField::Int(*abs_row_start as i64));
        }
        TerminalEvent::ZoneClosed {
            zone_id,
            zone_type,
            abs_row_start,
            abs_row_end,
            exit_code,
        } => {
            put!("type", EventField::Str("zone_closed".to_string()));
            put!("zone_id", EventField::Int(*zone_id as i64));
            put!("zone_type", EventField::Str(zone_type.to_string()));
            put!("abs_row_start", EventField::Int(*abs_row_start as i64));
            put!("abs_row_end", EventField::Int(*abs_row_end as i64));
            put!(
                "exit_code",
                exit_code.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::ZoneScrolledOut { zone_id, zone_type } => {
            put!("type", EventField::Str("zone_scrolled_out".to_string()));
            put!("zone_id", EventField::Int(*zone_id as i64));
            put!("zone_type", EventField::Str(zone_type.to_string()));
        }
        TerminalEvent::EnvironmentChanged {
            key,
            value,
            old_value,
        } => {
            put!("type", EventField::Str("environment_changed".to_string()));
            put!("key", EventField::Str(key.clone()));
            put!("value", EventField::Str(value.clone()));
            put!(
                "old_value",
                old_value.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::RemoteHostTransition {
            hostname,
            username,
            old_hostname,
            old_username,
        } => {
            put!(
                "type",
                EventField::Str("remote_host_transition".to_string())
            );
            put!("hostname", EventField::Str(hostname.clone()));
            put!(
                "username",
                username.clone().map_or(EventField::None, EventField::Str)
            );
            put!(
                "old_hostname",
                old_hostname
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
            put!(
                "old_username",
                old_username
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::SubShellDetected { depth, shell_type } => {
            put!("type", EventField::Str("sub_shell_detected".to_string()));
            put!("depth", EventField::Int(*depth as i64));
            put!(
                "shell_type",
                shell_type.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::FileTransferStarted {
            id,
            direction,
            filename,
            total_bytes,
        } => {
            put!("type", EventField::Str("file_transfer_started".to_string()));
            put!("id", EventField::Int(*id as i64));
            let dir_str = match direction {
                crate::terminal::TransferDirection::Download => "download",
                crate::terminal::TransferDirection::Upload => "upload",
            };
            put!("direction", EventField::Str(dir_str.to_string()));
            put!(
                "filename",
                filename.clone().map_or(EventField::None, EventField::Str)
            );
            put!(
                "total_bytes",
                total_bytes.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::FileTransferProgress {
            id,
            bytes_transferred,
            total_bytes,
        } => {
            put!(
                "type",
                EventField::Str("file_transfer_progress".to_string())
            );
            put!("id", EventField::Int(*id as i64));
            put!(
                "bytes_transferred",
                EventField::Int(*bytes_transferred as i64)
            );
            put!(
                "total_bytes",
                total_bytes.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::FileTransferCompleted { id, filename, size } => {
            put!(
                "type",
                EventField::Str("file_transfer_completed".to_string())
            );
            put!("id", EventField::Int(*id as i64));
            put!(
                "filename",
                filename.clone().map_or(EventField::None, EventField::Str)
            );
            put!("size", EventField::Int(*size as i64));
        }
        TerminalEvent::FileTransferFailed { id, reason } => {
            put!("type", EventField::Str("file_transfer_failed".to_string()));
            put!("id", EventField::Int(*id as i64));
            put!("reason", EventField::Str(reason.clone()));
        }
        TerminalEvent::UploadRequested { format } => {
            put!("type", EventField::Str("upload_requested".to_string()));
            put!("format", EventField::Str(format.clone()));
        }
        TerminalEvent::ScreenCleared { include_scrollback } => {
            put!("type", EventField::Str("screen_cleared".to_string()));
            put!("include_scrollback", EventField::Bool(*include_scrollback));
        }
        TerminalEvent::InlineImageDropped { reason } => {
            put!("type", EventField::Str("inline_image_dropped".to_string()));
            put!("reason", EventField::Str(reason.clone()));
        }
    }
    fields
}

/// Convert a `TerminalEvent` to a Python dictionary with native value types
/// (`int` for numeric fields, `bool` for flags, `None` for unset optional
/// fields, `str` for text). Shared by `poll_events()`,
/// `poll_subscribed_events()`, and observer dispatch.
pub(crate) fn event_to_dict<'py>(py: Python<'py>, event: &TerminalEvent) -> Bound<'py, PyDict> {
    let dict = PyDict::new(py);
    for (key, field) in event_fields(event) {
        match field {
            EventField::Str(s) => dict.set_item(key, s),
            EventField::Int(i) => dict.set_item(key, i),
            EventField::Bool(b) => dict.set_item(key, b),
            EventField::None => dict.set_item(key, py.None()),
        }
        .expect("PyDict::set_item with a str key and a native value cannot fail");
    }
    dict
}

/// Convert a `TerminalEvent` to the legacy stringly-typed dictionary — every
/// value a `str`, optional fields omitted when unset — that `poll_events()`
/// returned before 0.51. Kept for one release behind `poll_events_legacy()`
/// and `poll_subscribed_events_legacy()`.
pub(crate) fn event_to_dict_legacy(event: &TerminalEvent) -> HashMap<String, String> {
    event_fields(event)
        .into_iter()
        .filter_map(|(key, field)| {
            let value = match field {
                EventField::Str(s) => s,
                EventField::Int(i) => i.to_string(),
                EventField::Bool(b) => b.to_string(),
                EventField::None => return None,
            };
            Some((key, value))
        })
        .collect()
}

thread_local! {
    /// Reentrancy guard for Python observer callbacks (ARC-016).
    ///
    /// While a Python observer callback runs on this thread, further observer
    /// events are dropped instead of dispatched. This breaks the reentrant
    /// cycle — `process()` → dispatch → Python callback → terminal method →
    /// `process()` again — that would otherwise deadlock on the non-reentrant
    /// `parking_lot` Terminal mutex already held by the calling thread.
    static IN_PY_OBSERVER_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

/// Clears the reentrancy flag on drop, including when the callback panics.
struct PyCallbackReentrancyGuard;
impl Drop for PyCallbackReentrancyGuard {
    fn drop(&mut self) {
        IN_PY_OBSERVER_CALLBACK.set(false);
    }
}

/// Run `f` only if this thread is not already inside a Python observer
/// callback; arm the reentrancy guard while it runs. Returns `None` (and skips
/// dispatch) on reentrant entry. See ARC-016.
fn with_py_callback_reentrancy_guard<R>(f: impl FnOnce() -> R) -> Option<R> {
    let already_inside = IN_PY_OBSERVER_CALLBACK.replace(true);
    if already_inside {
        // Nested entry: the outer callback is still running (flag stays true).
        return None;
    }
    let _guard = PyCallbackReentrancyGuard;
    Some(f())
}

/// Observer that calls a Python callable for each event
pub(crate) struct PyCallbackObserver {
    callback: Py<PyAny>,
    subscriptions: Option<HashSet<TerminalEventKind>>,
}

impl PyCallbackObserver {
    pub fn new(callback: Py<PyAny>, subscriptions: Option<HashSet<TerminalEventKind>>) -> Self {
        Self {
            callback,
            subscriptions,
        }
    }
}

// Safety: Py<PyAny> is Send+Sync when we acquire the GIL before use.
// We only access the callback inside `Python::attach`.
unsafe impl Send for PyCallbackObserver {}
unsafe impl Sync for PyCallbackObserver {}

impl TerminalObserver for PyCallbackObserver {
    fn on_event(&self, event: &TerminalEvent) {
        // Guard against reentrant dispatch deadlocking on the Terminal mutex
        // (ARC-016): drop the event if this thread is already inside a Python
        // observer callback.
        let _ = with_py_callback_reentrancy_guard(|| {
            Python::attach(|py| {
                let dict = event_to_dict(py, event);
                if let Err(e) = self.callback.call1(py, (dict,)) {
                    log::error!("Observer callback error: {e}");
                }
            });
        });
    }

    fn subscriptions(&self) -> Option<&HashSet<TerminalEventKind>> {
        self.subscriptions.as_ref()
    }
}

/// Observer that pushes events into a Python asyncio.Queue via put_nowait
pub(crate) struct PyQueueObserver {
    queue: Py<PyAny>,
    subscriptions: Option<HashSet<TerminalEventKind>>,
}

impl PyQueueObserver {
    pub fn new(queue: Py<PyAny>, subscriptions: Option<HashSet<TerminalEventKind>>) -> Self {
        Self {
            queue,
            subscriptions,
        }
    }
}

// Safety: Py<PyAny> is Send+Sync when GIL is acquired before use.
// We only access the queue inside `Python::attach`.
unsafe impl Send for PyQueueObserver {}
unsafe impl Sync for PyQueueObserver {}

impl TerminalObserver for PyQueueObserver {
    fn on_event(&self, event: &TerminalEvent) {
        // Same reentrancy guard as the sync callback (ARC-016).
        let _ = with_py_callback_reentrancy_guard(|| {
            Python::attach(|py| {
                let dict = event_to_dict(py, event);
                if let Err(e) = self.queue.call_method1(py, "put_nowait", (dict,)) {
                    log::error!("Observer queue.put_nowait error: {e}");
                }
            });
        });
    }

    fn subscriptions(&self) -> Option<&HashSet<TerminalEventKind>> {
        self.subscriptions.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reentrancy_guard_blocks_nested_dispatch_and_clears_on_exit() {
        // Start from a known state on this test thread.
        IN_PY_OBSERVER_CALLBACK.with(|c| c.set(false));

        let mut outer_ran = false;
        let mut nested_result: Option<()> = None;

        let outer = with_py_callback_reentrancy_guard(|| {
            outer_ran = true;
            // Simulate a callback re-entering dispatch on the same thread:
            nested_result = with_py_callback_reentrancy_guard(|| {
                panic!("nested dispatch must not run");
            });
            IN_PY_OBSERVER_CALLBACK.with(|c| {
                assert!(c.get(), "flag must remain true while outer callback runs");
            });
        });

        assert!(outer.is_some(), "outer (first) dispatch should run");
        assert!(outer_ran);
        assert_eq!(nested_result, None, "nested dispatch must be suppressed");
        assert!(
            !IN_PY_OBSERVER_CALLBACK.with(Cell::get),
            "flag must be cleared once the callback returns"
        );
    }

    #[test]
    fn legacy_renderer_reproduces_pre_051_stringly_shape() {
        // Numeric fields stringify, bools render lowercase like Rust's
        // bool::to_string always did.
        let legacy = event_to_dict_legacy(&TerminalEvent::ModeChanged("insert".to_string(), false));
        assert_eq!(legacy.get("enabled").map(String::as_str), Some("false"));

        let legacy = event_to_dict_legacy(&TerminalEvent::SizeChanged(80, 24));
        assert_eq!(legacy.get("cols").map(String::as_str), Some("80"));
        assert_eq!(legacy.get("rows").map(String::as_str), Some("24"));

        // Optional fields are omitted when unset (the native renderer keeps
        // the key with None instead).
        let legacy = event_to_dict_legacy(&TerminalEvent::HyperlinkAdded {
            url: "https://example.com".to_string(),
            row: 1,
            col: 2,
            id: None,
        });
        assert!(!legacy.contains_key("id"));

        let fields = event_fields(&TerminalEvent::HyperlinkAdded {
            url: "https://example.com".to_string(),
            row: 1,
            col: 2,
            id: None,
        });
        let id_field = fields
            .iter()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value);
        assert_eq!(id_field, Some(&EventField::None));
    }
}
