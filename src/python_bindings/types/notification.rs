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

    /// Pane ID (for notifications that involve a pane)
    pub pane_id: Option<String>,

    /// Window ID (for notifications that involve a window)
    pub window_id: Option<String>,

    /// Session ID (for notifications that involve a session)
    pub session_id: Option<String>,

    /// Name (for window/session rename notifications)
    pub name: Option<String>,

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

impl From<&crate::tmux_control::TmuxNotification> for PyTmuxNotification {
    fn from(notif: &crate::tmux_control::TmuxNotification) -> Self {
        use crate::tmux_control::TmuxNotification;

        match notif {
            TmuxNotification::Begin {
                timestamp,
                command_number,
                flags,
            } => PyTmuxNotification {
                notification_type: "begin".to_string(),
                timestamp: Some(*timestamp),
                command_number: Some(*command_number),
                flags: Some(flags.clone()),
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::End {
                timestamp,
                command_number,
                flags,
            } => PyTmuxNotification {
                notification_type: "end".to_string(),
                timestamp: Some(*timestamp),
                command_number: Some(*command_number),
                flags: Some(flags.clone()),
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Error {
                timestamp,
                command_number,
                flags,
            } => PyTmuxNotification {
                notification_type: "error".to_string(),
                timestamp: Some(*timestamp),
                command_number: Some(*command_number),
                flags: Some(flags.clone()),
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Output { pane_id, data } => PyTmuxNotification {
                notification_type: "output".to_string(),
                pane_id: Some(pane_id.clone()),
                data: Some(data.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::PaneModeChanged { pane_id } => PyTmuxNotification {
                notification_type: "pane-mode-changed".to_string(),
                pane_id: Some(pane_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::WindowPaneChanged { window_id, pane_id } => PyTmuxNotification {
                notification_type: "window-pane-changed".to_string(),
                window_id: Some(window_id.clone()),
                pane_id: Some(pane_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::WindowClose { window_id } => PyTmuxNotification {
                notification_type: "window-close".to_string(),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::UnlinkedWindowClose { window_id } => PyTmuxNotification {
                notification_type: "unlinked-window-close".to_string(),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::WindowAdd { window_id } => PyTmuxNotification {
                notification_type: "window-add".to_string(),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::UnlinkedWindowAdd { window_id } => PyTmuxNotification {
                notification_type: "unlinked-window-add".to_string(),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::WindowRenamed { window_id, name } => PyTmuxNotification {
                notification_type: "window-renamed".to_string(),
                window_id: Some(window_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::UnlinkedWindowRenamed { window_id, name } => PyTmuxNotification {
                notification_type: "unlinked-window-renamed".to_string(),
                window_id: Some(window_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SessionChanged { session_id, name } => PyTmuxNotification {
                notification_type: "session-changed".to_string(),
                session_id: Some(session_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::ClientSessionChanged {
                client,
                session_id,
                name,
            } => PyTmuxNotification {
                notification_type: "client-session-changed".to_string(),
                client: Some(client.clone()),
                session_id: Some(session_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SessionRenamed { session_id, name } => PyTmuxNotification {
                notification_type: "session-renamed".to_string(),
                session_id: Some(session_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SessionsChanged => PyTmuxNotification {
                notification_type: "sessions-changed".to_string(),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SessionWindowChanged {
                session_id,
                window_id,
            } => PyTmuxNotification {
                notification_type: "session-window-changed".to_string(),
                session_id: Some(session_id.clone()),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::ClientDetached { client } => PyTmuxNotification {
                notification_type: "client-detached".to_string(),
                client: Some(client.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Exit => PyTmuxNotification {
                notification_type: "exit".to_string(),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Pause { pane_id } => PyTmuxNotification {
                notification_type: "pause".to_string(),
                pane_id: Some(pane_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::ExtendedOutput {
                pane_id,
                delay_ms,
                data,
            } => PyTmuxNotification {
                notification_type: "extended-output".to_string(),
                pane_id: Some(pane_id.clone()),
                delay_ms: Some(*delay_ms),
                data: Some(data.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Continue => PyTmuxNotification {
                notification_type: "continue".to_string(),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SubscriptionChanged { name, value } => PyTmuxNotification {
                notification_type: "subscription-changed".to_string(),
                subscription_name: Some(name.clone()),
                value: Some(value.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::LayoutChange {
                window_id,
                window_layout,
                window_visible_layout,
                window_raw_flags,
            } => PyTmuxNotification {
                notification_type: "layout-change".to_string(),
                window_id: Some(window_id.clone()),
                window_layout: Some(window_layout.clone()),
                window_visible_layout: Some(window_visible_layout.clone()),
                window_raw_flags: Some(window_raw_flags.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                raw_line: None,
            },
            TmuxNotification::PasteBufferChanged { name } => PyTmuxNotification {
                notification_type: "paste-buffer-changed".to_string(),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::PasteBufferDeleted { name } => PyTmuxNotification {
                notification_type: "paste-buffer-deleted".to_string(),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Unknown { line } => PyTmuxNotification {
                notification_type: "unknown".to_string(),
                raw_line: Some(line.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
            },
            TmuxNotification::TerminalOutput { data } => PyTmuxNotification {
                notification_type: "terminal-output".to_string(),
                data: Some(data.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
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
