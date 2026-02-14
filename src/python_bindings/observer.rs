//! Python observer bindings for push-based event delivery
//!
//! Provides `PyCallbackObserver` (sync callback) and `PyQueueObserver` (asyncio.Queue)
//! that bridge the Rust `TerminalObserver` trait to Python callables.

use std::collections::{HashMap, HashSet};

use pyo3::prelude::*;
use pyo3::types::PyAny;

use crate::observer::TerminalObserver;
use crate::terminal::{TerminalEvent, TerminalEventKind};

/// Convert a `TerminalEvent` to a Python-friendly dictionary.
///
/// This is the single source of truth for event-to-dict conversion, shared by
/// `poll_events()`, `poll_subscribed_events()`, and observer dispatch.
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
            map.insert(
                "trigger_id".to_string(),
                trigger_match.trigger_id.to_string(),
            );
            map.insert("row".to_string(), trigger_match.row.to_string());
            map.insert("col".to_string(), trigger_match.col.to_string());
            map.insert("end_col".to_string(), trigger_match.end_col.to_string());
            map.insert("text".to_string(), trigger_match.text.clone());
            map.insert("timestamp".to_string(), trigger_match.timestamp.to_string());
        }
        TerminalEvent::UserVarChanged {
            name,
            value,
            old_value,
        } => {
            map.insert("type".to_string(), "user_var_changed".to_string());
            map.insert("name".to_string(), name.clone());
            map.insert("value".to_string(), value.clone());
            if let Some(old) = old_value {
                map.insert("old_value".to_string(), old.clone());
            }
        }
        TerminalEvent::ProgressBarChanged {
            action,
            id,
            state,
            percent,
            label,
        } => {
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
        TerminalEvent::ShellIntegrationEvent {
            event_type,
            command,
            exit_code,
            timestamp,
            cursor_line,
        } => {
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
        TerminalEvent::ZoneOpened {
            zone_id,
            zone_type,
            abs_row_start,
        } => {
            map.insert("type".to_string(), "zone_opened".to_string());
            map.insert("zone_id".to_string(), zone_id.to_string());
            map.insert("zone_type".to_string(), zone_type.to_string());
            map.insert("abs_row_start".to_string(), abs_row_start.to_string());
        }
        TerminalEvent::ZoneClosed {
            zone_id,
            zone_type,
            abs_row_start,
            abs_row_end,
            exit_code,
        } => {
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
        TerminalEvent::EnvironmentChanged {
            key,
            value,
            old_value,
        } => {
            map.insert("type".to_string(), "environment_changed".to_string());
            map.insert("key".to_string(), key.clone());
            map.insert("value".to_string(), value.clone());
            if let Some(old) = old_value {
                map.insert("old_value".to_string(), old.clone());
            }
        }
        TerminalEvent::RemoteHostTransition {
            hostname,
            username,
            old_hostname,
            old_username,
        } => {
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
        let dict = event_to_dict(event);
        Python::attach(|py| {
            if let Err(e) = self.callback.call1(py, (dict,)) {
                eprintln!("Observer callback error: {e}");
            }
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
        let dict = event_to_dict(event);
        Python::attach(|py| {
            if let Err(e) = self.queue.call_method1(py, "put_nowait", (dict,)) {
                eprintln!("Observer queue.put_nowait error: {e}");
            }
        });
    }

    fn subscriptions(&self) -> Option<&HashSet<TerminalEventKind>> {
        self.subscriptions.as_ref()
    }
}
