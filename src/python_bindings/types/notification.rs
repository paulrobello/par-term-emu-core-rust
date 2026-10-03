//! Desktop / tmux notification types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Tmux control protocol notification
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "TmuxNotification", from_py_object)]
#[derive(Clone)]
pub struct PyTmuxNotification {
    /// Notification type (e.g., "output", "window-add", "session-changed")
    pub notification_type: String,

    /// Provenance of an agent-state-changed state: "hook" (the agent claimed
    /// it) or "scrape" (a pattern matched pane content). None for every
    /// other notification type and for lines with no `source=` token.
    pub source: Option<String>,

    /// Pane ID (for notifications that involve a pane)
    pub pane_id: Option<String>,

    /// Window ID (for notifications that involve a window)
    pub window_id: Option<String>,

    /// Session ID (for notifications that involve a session)
    pub session_id: Option<String>,

    /// Name (for window/session rename notifications)
    pub name: Option<String>,

    /// Exit code of the pane's process (for `pane-exited`); None on signal
    /// death, when unreadable, and for every other notification type
    pub exit_code: Option<i32>,

    /// Client name (for client-related notifications)
    pub client: Option<String>,

    /// Output data (for output notifications, as bytes)
    pub data: Option<Vec<u8>>,

    /// Timestamp (for begin/end/error notifications)
    pub timestamp: Option<u64>,

    /// Command number (for begin/end/error notifications)
    pub command_number: Option<u32>,

    /// Flags (for begin/end/error notifications)
    pub flags: Option<String>,

    /// Delay in milliseconds (for extended-output notifications)
    pub delay_ms: Option<u64>,

    /// Subscription name (for subscription-changed notifications)
    pub subscription_name: Option<String>,

    /// Subscription value (for subscription-changed notifications)
    pub value: Option<String>,

    /// Window layout (for layout-change notifications)
    pub window_layout: Option<String>,

    /// Window visible layout (for layout-change notifications)
    pub window_visible_layout: Option<String>,

    /// Window raw flags (for layout-change notifications)
    pub window_raw_flags: Option<String>,

    /// Raw line (for unknown notifications)
    pub raw_line: Option<String>,
}

#[pymethods]
impl PyTmuxNotification {
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!("TmuxNotification(type={})", self.notification_type))
    }
}

/// Desktop notification from an OSC 9, OSC 777, or Kitty OSC 99 sequence.
///
/// The `id`, `urgency`, and `actions` fields carry Kitty OSC 99 metadata; for
/// OSC 9/777 notifications `id` is None, `urgency` is "normal", and `actions`
/// is empty.
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "Notification", from_py_object)]
#[derive(Clone)]
pub struct PyNotification {
    /// Notification title (empty for OSC 9)
    pub title: String,

    /// Notification message/body
    pub message: String,

    /// Kitty OSC 99 identifier used to group/update notifications; None otherwise
    pub id: Option<String>,

    /// Urgency: one of "low", "normal", "critical"
    pub urgency: String,

    /// Requested Kitty OSC 99 actions (e.g. "focus", "report", "close")
    pub actions: Vec<String>,
}

#[pymethods]
impl PyNotification {
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Notification(title={:?}, message={:?}, urgency={:?})",
            self.title, self.message, self.urgency
        ))
    }
}

impl From<&crate::terminal::Notification> for PyNotification {
    fn from(n: &crate::terminal::Notification) -> Self {
        let urgency = match n.urgency {
            crate::terminal::Urgency::Low => "low",
            crate::terminal::Urgency::Normal => "normal",
            crate::terminal::Urgency::Critical => "critical",
        };
        PyNotification {
            title: n.title.clone(),
            message: n.message.clone(),
            id: n.id.clone(),
            urgency: urgency.to_string(),
            actions: n.actions.clone(),
        }
    }
}

impl PyTmuxNotification {
    /// Every optional field unset; each `From` arm fills in only the fields
    /// its `TmuxNotification` variant carries. Private and not a
    /// `#[pymethods]` item, so the Python surface is unchanged.
    fn empty(kind: &str) -> Self {
        Self {
            notification_type: kind.to_string(),
            source: None,
            pane_id: None,
            window_id: None,
            session_id: None,
            name: None,
            exit_code: None,
            client: None,
            data: None,
            timestamp: None,
            command_number: None,
            flags: None,
            delay_ms: None,
            subscription_name: None,
            value: None,
            window_layout: None,
            window_visible_layout: None,
            window_raw_flags: None,
            raw_line: None,
        }
    }
}

impl From<&crate::tmux_control::TmuxNotification> for PyTmuxNotification {
    fn from(notif: &crate::tmux_control::TmuxNotification) -> Self {
        use crate::tmux_control::TmuxNotification;

        // notification_type() is the one source of truth for kind strings;
        // keeping the match exhaustive (no `_` arm) turns a new variant
        // into a compile error here.
        let kind = notif.notification_type();

        match notif {
            TmuxNotification::Begin {
                timestamp,
                command_number,
                flags,
            } => Self {
                timestamp: Some(*timestamp),
                command_number: Some(*command_number),
                flags: Some(flags.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::End {
                timestamp,
                command_number,
                flags,
            } => Self {
                timestamp: Some(*timestamp),
                command_number: Some(*command_number),
                flags: Some(flags.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::Error {
                timestamp,
                command_number,
                flags,
            } => Self {
                timestamp: Some(*timestamp),
                command_number: Some(*command_number),
                flags: Some(flags.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::Output { pane_id, data } => Self {
                pane_id: Some(pane_id.clone()),
                data: Some(data.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::PaneModeChanged { pane_id } => Self {
                pane_id: Some(pane_id.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::WindowPaneChanged { window_id, pane_id } => Self {
                window_id: Some(window_id.clone()),
                pane_id: Some(pane_id.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::WindowClose { window_id } => Self {
                window_id: Some(window_id.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::UnlinkedWindowClose { window_id } => Self {
                window_id: Some(window_id.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::WindowAdd {
                window_id,
                window_layout,
                window_visible_layout,
                window_raw_flags,
            } => Self {
                window_id: Some(window_id.clone()),
                // The triple rides on newer emitters; a bare-id line (real
                // tmux's shape) leaves the fields None.
                window_layout: (!window_layout.is_empty()).then(|| window_layout.clone()),
                window_visible_layout: (!window_visible_layout.is_empty())
                    .then(|| window_visible_layout.clone()),
                window_raw_flags: (!window_raw_flags.is_empty()).then(|| window_raw_flags.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::UnlinkedWindowAdd { window_id } => Self {
                window_id: Some(window_id.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::WindowRenamed { window_id, name } => Self {
                window_id: Some(window_id.clone()),
                name: Some(name.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::UnlinkedWindowRenamed { window_id, name } => Self {
                window_id: Some(window_id.clone()),
                name: Some(name.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::SessionChanged { session_id, name } => Self {
                session_id: Some(session_id.clone()),
                name: Some(name.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::ClientSessionChanged {
                client,
                session_id,
                name,
            } => Self {
                client: Some(client.clone()),
                session_id: Some(session_id.clone()),
                name: Some(name.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::SessionRenamed { session_id, name } => Self {
                session_id: Some(session_id.clone()),
                name: Some(name.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::WorkspacesChanged => Self {
                ..Self::empty(kind)
            },
            TmuxNotification::SessionsChanged => Self {
                ..Self::empty(kind)
            },
            TmuxNotification::SessionWindowChanged {
                session_id,
                window_id,
            } => Self {
                session_id: Some(session_id.clone()),
                window_id: Some(window_id.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::ClientDetached { client } => Self {
                client: Some(client.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::Exit => Self {
                ..Self::empty(kind)
            },
            TmuxNotification::AgentStateChanged {
                pane_id,
                agent,
                state,
                source,
            } => Self {
                source: Some(source.clone()),
                pane_id: Some(pane_id.clone()),
                name: Some(agent.clone()),
                value: Some(state.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::AgentReleased { pane_id, agent } => Self {
                pane_id: Some(pane_id.clone()),
                name: Some(agent.clone()),
                ..Self::empty(kind)
            },
            // Identity only — the fresh telemetry blob lives on the
            // list-agents roster, so clients re-query.
            TmuxNotification::AgentTelemetryChanged { pane_id, agent } => Self {
                pane_id: Some(pane_id.clone()),
                name: Some(agent.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::PaneTitleChanged { pane_id, title } => Self {
                pane_id: Some(pane_id.clone()),
                name: Some(title.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::PaneExited { pane_id, exit_code } => Self {
                pane_id: Some(pane_id.clone()),
                // Deprecated: name-as-exit-string; clients read exit_code.
                name: exit_code.map(|code| code.to_string()),
                exit_code: *exit_code,
                ..Self::empty(kind)
            },
            TmuxNotification::PaneRespawned { pane_id } => Self {
                pane_id: Some(pane_id.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::Pause { pane_id } => Self {
                pane_id: Some(pane_id.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::ExtendedOutput {
                pane_id,
                delay_ms,
                data,
            } => Self {
                pane_id: Some(pane_id.clone()),
                delay_ms: Some(*delay_ms),
                data: Some(data.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::Continue => Self {
                ..Self::empty(kind)
            },
            TmuxNotification::SubscriptionChanged { name, value } => Self {
                subscription_name: Some(name.clone()),
                value: Some(value.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::LayoutChange {
                window_id,
                window_layout,
                window_visible_layout,
                window_raw_flags,
            } => Self {
                window_id: Some(window_id.clone()),
                window_layout: Some(window_layout.clone()),
                window_visible_layout: Some(window_visible_layout.clone()),
                window_raw_flags: Some(window_raw_flags.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::PasteBufferChanged { name } => Self {
                name: Some(name.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::PasteBufferDeleted { name } => Self {
                name: Some(name.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::Unknown { line } => Self {
                raw_line: Some(line.clone()),
                ..Self::empty(kind)
            },
            TmuxNotification::TerminalOutput { data } => Self {
                data: Some(data.clone()),
                ..Self::empty(kind)
            },
        }
    }
}
impl From<crate::tmux_control::TmuxNotification> for PyTmuxNotification {
    fn from(notif: crate::tmux_control::TmuxNotification) -> Self {
        (&notif).into()
    }
}

/// Notification event
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "NotificationEvent", from_py_object)]
#[derive(Clone)]
pub struct PyNotificationEvent {
    /// What triggered the notification (e.g. "bell", "activity", "silence")
    pub trigger: String,
    /// Alert kind (e.g. "desktop", "sound", "visual")
    pub alert: String,
    /// Human-readable notification text, when present
    pub message: Option<String>,
    /// Unix epoch milliseconds when the event occurred
    pub timestamp: u64,
    /// Whether the notification was delivered to the host
    pub delivered: bool,
}

#[pymethods]
impl PyNotificationEvent {
    fn __repr__(&self) -> String {
        format!(
            "NotificationEvent(trigger={}, alert={}, delivered={})",
            self.trigger, self.alert, self.delivered
        )
    }
}

impl From<&crate::terminal::NotificationEvent> for PyNotificationEvent {
    fn from(event: &crate::terminal::NotificationEvent) -> Self {
        let trigger = match event.trigger {
            crate::terminal::NotificationTrigger::Bell => "Bell".to_string(),
            crate::terminal::NotificationTrigger::Activity => "Activity".to_string(),
            crate::terminal::NotificationTrigger::Silence => "Silence".to_string(),
            crate::terminal::NotificationTrigger::Custom(id) => format!("Custom({})", id),
        };

        let alert = match event.alert {
            crate::terminal::NotificationAlert::Desktop => "Desktop".to_string(),
            crate::terminal::NotificationAlert::Sound(vol) => format!("Sound({})", vol),
            crate::terminal::NotificationAlert::Visual => "Visual".to_string(),
        };

        PyNotificationEvent {
            trigger,
            alert,
            message: event.message.clone(),
            timestamp: event.timestamp,
            delivered: event.delivered,
        }
    }
}

/// Notification configuration
#[pyclass(name = "NotificationConfig", from_py_object)]
#[derive(Clone)]
pub struct PyNotificationConfig {
    /// Whether BEL triggers a desktop notification
    #[pyo3(get, set)]
    pub bell_desktop: bool,
    /// BEL sound (0 = disabled, 1-100 = volume)
    #[pyo3(get, set)]
    pub bell_sound: u8,
    /// Whether BEL triggers a visual bell flash
    #[pyo3(get, set)]
    pub bell_visual: bool,
    /// Whether activity notifications are enabled
    #[pyo3(get, set)]
    pub activity_enabled: bool,
    /// Seconds of inactivity before an activity notification fires
    #[pyo3(get, set)]
    pub activity_threshold: u64,
    /// Whether silence notifications are enabled
    #[pyo3(get, set)]
    pub silence_enabled: bool,
    /// Seconds of silence before a silence notification fires
    #[pyo3(get, set)]
    pub silence_threshold: u64,
}

#[pymethods]
impl PyNotificationConfig {
    #[new]
    fn new() -> Self {
        PyNotificationConfig::default()
    }

    fn __repr__(&self) -> String {
        format!(
            "NotificationConfig(bell_desktop={}, bell_visual={}, activity={}, silence={})",
            self.bell_desktop, self.bell_visual, self.activity_enabled, self.silence_enabled
        )
    }
}

impl Default for PyNotificationConfig {
    fn default() -> Self {
        let config = crate::terminal::NotificationConfig::default();
        PyNotificationConfig {
            bell_desktop: config.bell_desktop,
            bell_sound: config.bell_sound,
            bell_visual: config.bell_visual,
            activity_enabled: config.activity_enabled,
            activity_threshold: config.activity_threshold,
            silence_enabled: config.silence_enabled,
            silence_threshold: config.silence_threshold,
        }
    }
}

impl From<&crate::terminal::NotificationConfig> for PyNotificationConfig {
    fn from(config: &crate::terminal::NotificationConfig) -> Self {
        PyNotificationConfig {
            bell_desktop: config.bell_desktop,
            bell_sound: config.bell_sound,
            bell_visual: config.bell_visual,
            activity_enabled: config.activity_enabled,
            activity_threshold: config.activity_threshold,
            silence_enabled: config.silence_enabled,
            silence_threshold: config.silence_threshold,
        }
    }
}

impl From<&PyNotificationConfig> for crate::terminal::NotificationConfig {
    fn from(config: &PyNotificationConfig) -> Self {
        crate::terminal::NotificationConfig {
            bell_desktop: config.bell_desktop,
            bell_sound: config.bell_sound,
            bell_visual: config.bell_visual,
            activity_enabled: config.activity_enabled,
            activity_threshold: config.activity_threshold,
            silence_enabled: config.silence_enabled,
            silence_threshold: config.silence_threshold,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux_control::TmuxNotification;

    const GOLDEN: &str = concat!(
        "begin | None | None | None | None | None | None | None | None | Some(101) | Some(201) | Some(\"flags-begin\") | None | None | None | None | None | None | None\n",
        "end | None | None | None | None | None | None | None | None | Some(102) | Some(202) | Some(\"flags-end\") | None | None | None | None | None | None | None\n",
        "error | None | None | None | None | None | None | None | None | Some(103) | Some(203) | Some(\"flags-error\") | None | None | None | None | None | None | None\n",
        "output | None | Some(\"p-output\") | None | None | None | None | None | Some([111, 117, 116, 45, 100, 97, 116, 97]) | None | None | None | None | None | None | None | None | None | None\n",
        "pane-mode-changed | None | Some(\"p-pane-mode\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "window-pane-changed | None | Some(\"p-window-pane\") | Some(\"w-pane-changed\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "window-close | None | None | Some(\"w-close\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "unlinked-window-close | None | None | Some(\"w-unlinked-close\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "window-add | None | None | Some(\"w-add\") | None | None | None | None | None | None | None | None | None | None | None | Some(\"layout-add\") | Some(\"visible-add\") | Some(\"flags-add\") | None\n",
        "window-add | None | None | Some(\"w-add-bare\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "unlinked-window-add | None | None | Some(\"w-unlinked-add\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "window-renamed | None | None | Some(\"w-renamed\") | None | Some(\"n-window-renamed\") | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "unlinked-window-renamed | None | None | Some(\"w-unlink-renamed\") | None | Some(\"n-unlink-renamed\") | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "session-changed | None | None | None | Some(\"s-changed\") | Some(\"n-session-changed\") | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "client-session-changed | None | None | None | Some(\"s-client-session\") | Some(\"n-client-session\") | None | Some(\"c-client\") | None | None | None | None | None | None | None | None | None | None | None\n",
        "session-renamed | None | None | None | Some(\"s-renamed\") | Some(\"n-session-renamed\") | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "sessions-changed | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "session-window-changed | None | None | Some(\"w-session-window\") | Some(\"s-session-window\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "client-detached | None | None | None | None | None | None | Some(\"c-detached\") | None | None | None | None | None | None | None | None | None | None | None\n",
        "exit | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "agent-state-changed | Some(\"hook\") | Some(\"p-agent-state\") | None | None | Some(\"agent-state\") | None | None | None | None | None | None | None | None | Some(\"working\") | None | None | None | None\n",
        "agent-released | None | Some(\"p-agent-released\") | None | None | Some(\"agent-released\") | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "agent-telemetry-changed | None | Some(\"p-agent-telemetry\") | None | None | Some(\"agent-telemetry\") | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "pane-title-changed | None | Some(\"p-title\") | None | None | Some(\"pane-title\") | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "pane-exited | None | Some(\"p-exited\") | None | None | Some(\"3\") | Some(3) | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "pane-exited | None | Some(\"p-exited-signal\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "pane-respawned | None | Some(\"p-respawned\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "pause | None | Some(\"p-pause\") | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "extended-output | None | Some(\"p-extended\") | None | None | None | None | None | Some([101, 120, 116, 101, 110, 100, 101, 100, 45, 100, 97, 116, 97]) | None | None | None | Some(777) | None | None | None | None | None | None\n",
        "continue | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "subscription-changed | None | None | None | None | None | None | None | None | None | None | None | None | Some(\"sub-name\") | Some(\"sub-value\") | None | None | None | None\n",
        "layout-change | None | None | Some(\"w-layout\") | None | None | None | None | None | None | None | None | None | None | None | Some(\"layout-main\") | Some(\"layout-visible\") | Some(\"flags-raw\") | None\n",
        "paste-buffer-changed | None | None | None | None | Some(\"buf-changed\") | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "paste-buffer-deleted | None | None | None | None | Some(\"buf-deleted\") | None | None | None | None | None | None | None | None | None | None | None | None | None\n",
        "unknown | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | None | Some(\"%begin 101 201 1\")\n",
        "terminal-output | None | None | None | None | None | None | None | Some([116, 101, 114, 109, 105, 110, 97, 108, 45, 100, 97, 116, 97]) | None | None | None | None | None | None | None | None | None | None\n",
    );

    fn all_variants() -> Vec<TmuxNotification> {
        vec![
            TmuxNotification::Begin {
                timestamp: 101,
                command_number: 201,
                flags: "flags-begin".to_string(),
            },
            TmuxNotification::End {
                timestamp: 102,
                command_number: 202,
                flags: "flags-end".to_string(),
            },
            TmuxNotification::Error {
                timestamp: 103,
                command_number: 203,
                flags: "flags-error".to_string(),
            },
            TmuxNotification::Output {
                pane_id: "p-output".to_string(),
                data: b"out-data".to_vec(),
            },
            TmuxNotification::PaneModeChanged {
                pane_id: "p-pane-mode".to_string(),
            },
            TmuxNotification::WindowPaneChanged {
                window_id: "w-pane-changed".to_string(),
                pane_id: "p-window-pane".to_string(),
            },
            TmuxNotification::WindowClose {
                window_id: "w-close".to_string(),
            },
            TmuxNotification::UnlinkedWindowClose {
                window_id: "w-unlinked-close".to_string(),
            },
            TmuxNotification::WindowAdd {
                window_id: "w-add".to_string(),
                window_layout: "layout-add".to_string(),
                window_visible_layout: "visible-add".to_string(),
                window_raw_flags: "flags-add".to_string(),
            },
            TmuxNotification::WindowAdd {
                window_id: "w-add-bare".to_string(),
                window_layout: String::new(),
                window_visible_layout: String::new(),
                window_raw_flags: String::new(),
            },
            TmuxNotification::UnlinkedWindowAdd {
                window_id: "w-unlinked-add".to_string(),
            },
            TmuxNotification::WindowRenamed {
                window_id: "w-renamed".to_string(),
                name: "n-window-renamed".to_string(),
            },
            TmuxNotification::UnlinkedWindowRenamed {
                window_id: "w-unlink-renamed".to_string(),
                name: "n-unlink-renamed".to_string(),
            },
            TmuxNotification::SessionChanged {
                session_id: "s-changed".to_string(),
                name: "n-session-changed".to_string(),
            },
            TmuxNotification::ClientSessionChanged {
                client: "c-client".to_string(),
                session_id: "s-client-session".to_string(),
                name: "n-client-session".to_string(),
            },
            TmuxNotification::SessionRenamed {
                session_id: "s-renamed".to_string(),
                name: "n-session-renamed".to_string(),
            },
            TmuxNotification::SessionsChanged,
            TmuxNotification::SessionWindowChanged {
                session_id: "s-session-window".to_string(),
                window_id: "w-session-window".to_string(),
            },
            TmuxNotification::ClientDetached {
                client: "c-detached".to_string(),
            },
            TmuxNotification::Exit,
            TmuxNotification::AgentStateChanged {
                pane_id: "p-agent-state".to_string(),
                agent: "agent-state".to_string(),
                state: "working".to_string(),
                source: "hook".to_string(),
            },
            TmuxNotification::AgentReleased {
                pane_id: "p-agent-released".to_string(),
                agent: "agent-released".to_string(),
            },
            TmuxNotification::AgentTelemetryChanged {
                pane_id: "p-agent-telemetry".to_string(),
                agent: "agent-telemetry".to_string(),
            },
            TmuxNotification::PaneTitleChanged {
                pane_id: "p-title".to_string(),
                title: "pane-title".to_string(),
            },
            TmuxNotification::PaneExited {
                pane_id: "p-exited".to_string(),
                exit_code: Some(3),
            },
            TmuxNotification::PaneExited {
                pane_id: "p-exited-signal".to_string(),
                exit_code: None,
            },
            TmuxNotification::PaneRespawned {
                pane_id: "p-respawned".to_string(),
            },
            TmuxNotification::Pause {
                pane_id: "p-pause".to_string(),
            },
            TmuxNotification::ExtendedOutput {
                pane_id: "p-extended".to_string(),
                delay_ms: 777,
                data: b"extended-data".to_vec(),
            },
            TmuxNotification::Continue,
            TmuxNotification::SubscriptionChanged {
                name: "sub-name".to_string(),
                value: "sub-value".to_string(),
            },
            TmuxNotification::LayoutChange {
                window_id: "w-layout".to_string(),
                window_layout: "layout-main".to_string(),
                window_visible_layout: "layout-visible".to_string(),
                window_raw_flags: "flags-raw".to_string(),
            },
            TmuxNotification::PasteBufferChanged {
                name: "buf-changed".to_string(),
            },
            TmuxNotification::PasteBufferDeleted {
                name: "buf-deleted".to_string(),
            },
            TmuxNotification::Unknown {
                line: "%begin 101 201 1".to_string(),
            },
            TmuxNotification::TerminalOutput {
                data: b"terminal-data".to_vec(),
            },
        ]
    }

    /// All 19 `PyTmuxNotification` fields, in declaration order.
    fn render(n: &PyTmuxNotification) -> String {
        format!(
            "{} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?}",
            n.notification_type, n.source, n.pane_id, n.window_id, n.session_id,
            n.name, n.exit_code, n.client, n.data, n.timestamp, n.command_number,
            n.flags, n.delay_ms, n.subscription_name, n.value, n.window_layout,
            n.window_visible_layout, n.window_raw_flags, n.raw_line
        )
    }

    #[test]
    fn variants_are_exhaustively_converted() {
        // Completeness guard: a new `TmuxNotification` variant fails this
        // wildcard-free match until it joins `all_variants()`.
        for notif in all_variants() {
            match &notif {
                TmuxNotification::Begin { .. } => (),
                TmuxNotification::End { .. } => (),
                TmuxNotification::Error { .. } => (),
                TmuxNotification::Output { .. } => (),
                TmuxNotification::PaneModeChanged { .. } => (),
                TmuxNotification::WindowPaneChanged { .. } => (),
                TmuxNotification::WindowClose { .. } => (),
                TmuxNotification::UnlinkedWindowClose { .. } => (),
                TmuxNotification::WindowAdd { .. } => (),
                TmuxNotification::UnlinkedWindowAdd { .. } => (),
                TmuxNotification::WindowRenamed { .. } => (),
                TmuxNotification::UnlinkedWindowRenamed { .. } => (),
                TmuxNotification::SessionChanged { .. } => (),
                TmuxNotification::ClientSessionChanged { .. } => (),
                TmuxNotification::SessionRenamed { .. } => (),
                TmuxNotification::SessionsChanged => (),
                TmuxNotification::SessionWindowChanged { .. } => (),
                TmuxNotification::ClientDetached { .. } => (),
                TmuxNotification::Exit => (),
                TmuxNotification::AgentStateChanged { .. } => (),
                TmuxNotification::AgentReleased { .. } => (),
                TmuxNotification::AgentTelemetryChanged { .. } => (),
                TmuxNotification::PaneTitleChanged { .. } => (),
                TmuxNotification::PaneExited { .. } => (),
                TmuxNotification::PaneRespawned { .. } => (),
                TmuxNotification::Pause { .. } => (),
                TmuxNotification::ExtendedOutput { .. } => (),
                TmuxNotification::Continue => (),
                TmuxNotification::SubscriptionChanged { .. } => (),
                TmuxNotification::LayoutChange { .. } => (),
                TmuxNotification::PasteBufferChanged { .. } => (),
                TmuxNotification::PasteBufferDeleted { .. } => (),
                TmuxNotification::Unknown { .. } => (),
                TmuxNotification::TerminalOutput { .. } => (),
                TmuxNotification::WorkspacesChanged => (),
            }
            let _ = PyTmuxNotification::from(&notif);
        }
    }

    #[test]
    fn golden_matches_pre_refactor_output() {
        let rendered: Vec<String> = all_variants()
            .iter()
            .map(|n| render(&PyTmuxNotification::from(n)))
            .collect();
        assert_eq!(rendered.join("\n") + "\n", GOLDEN);
    }
}
