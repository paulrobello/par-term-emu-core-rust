//! Notification support for OSC 9, OSC 777, and OSC 99 (Kitty) sequences

/// Notification data from OSC 9, OSC 777, or OSC 99 sequences
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Notification title (may be empty for OSC 9)
    pub title: String,
    /// Notification message/body
    pub message: String,
    /// Notification identifier from Kitty OSC 99 `i=` key, used to group/update
    /// notifications. `None` for OSC 9/777 or an id-less OSC 99 notification.
    pub id: Option<String>,
    /// Urgency level from Kitty OSC 99 `u=` key
    pub urgency: Urgency,
    /// Requested actions from Kitty OSC 99 `a=` key (e.g. "focus", "report", "close")
    pub actions: Vec<String>,
}

impl Notification {
    /// Create a new notification (OSC 9 / OSC 777 style)
    pub fn new(title: String, message: String) -> Self {
        Self {
            title,
            message,
            id: None,
            urgency: Urgency::default(),
            actions: Vec::new(),
        }
    }

    /// Create a notification carrying Kitty OSC 99 metadata
    pub fn with_metadata(
        title: String,
        message: String,
        id: Option<String>,
        urgency: Urgency,
        actions: Vec<String>,
    ) -> Self {
        Self {
            title,
            message,
            id,
            urgency,
            actions,
        }
    }
}

/// Notification urgency level (Kitty OSC 99 `u=` key)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    /// Low urgency (`u=0`)
    Low,
    /// Normal urgency (`u=1`, the default)
    #[default]
    Normal,
    /// Critical urgency (`u=2`)
    Critical,
}

impl Urgency {
    /// Parse a Kitty OSC 99 `u=` parameter value, defaulting to `Normal` for
    /// anything other than "0" or "2".
    pub(crate) fn from_param(value: &str) -> Self {
        match value {
            "0" => Urgency::Low,
            "2" => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }
}

/// Accumulator for an in-progress Kitty OSC 99 notification. Chunks arrive as
/// separate OSC 99 escapes sharing the same `i=` id (or the empty-string key
/// when no id is given) until a chunk with `d=1` (the default) completes it.
#[derive(Debug, Clone, Default)]
pub(crate) struct PartialNotification {
    pub(crate) id: Option<String>,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) urgency: Urgency,
    pub(crate) actions: Vec<String>,
}

/// Notification trigger type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationTrigger {
    /// Terminal bell rang
    Bell,
    /// Terminal activity detected
    Activity,
    /// Silence detected (no activity for duration)
    Silence,
    /// Custom trigger with ID
    Custom(u32),
}

/// Notification alert type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationAlert {
    /// Desktop/system notification
    Desktop,
    /// Sound alert with volume (0-100)
    Sound(u8),
    /// Visual alert (flash, border, etc.)
    Visual,
}

/// Notification event record
#[derive(Debug, Clone)]
pub struct NotificationEvent {
    /// What triggered the notification
    pub trigger: NotificationTrigger,
    /// Type of alert
    pub alert: NotificationAlert,
    /// Optional message
    pub message: Option<String>,
    /// Timestamp when event occurred
    pub timestamp: u64,
    /// Whether notification was delivered
    pub delivered: bool,
}

/// Notification configuration
#[derive(Debug, Clone)]
pub struct NotificationConfig {
    /// Enable desktop notifications on bell
    pub bell_desktop: bool,
    /// Enable sound on bell (0 = disabled, 1-100 = volume)
    pub bell_sound: u8,
    /// Enable visual alert on bell
    pub bell_visual: bool,
    /// Enable notifications on activity
    pub activity_enabled: bool,
    /// Activity threshold (seconds of inactivity before triggering)
    pub activity_threshold: u64,
    /// Enable notifications on silence
    pub silence_enabled: bool,
    /// Silence threshold (seconds of activity before silence notification)
    pub silence_threshold: u64,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            bell_desktop: false,
            bell_sound: 0,
            bell_visual: true,
            activity_enabled: false,
            activity_threshold: 10,
            silence_enabled: false,
            silence_threshold: 300,
        }
    }
}

use crate::terminal::Terminal;

impl Terminal {
    // === Feature 37: Terminal Notifications ===

    /// Add a notification event
    pub fn add_notification_event(
        &mut self,
        trigger: NotificationTrigger,
        alert: NotificationAlert,
        message: Option<String>,
    ) {
        let event = NotificationEvent {
            trigger,
            alert,
            message,
            timestamp: crate::terminal::unix_millis(),
            delivered: false,
        };

        self.notifications_state.notification_events.push(event);
        if self.notifications_state.notification_events.len()
            > self.notifications_state.max_notifications
        {
            self.notifications_state.notification_events.remove(0);
        }
    }

    /// Get notification configuration
    pub fn notification_config(&self) -> &NotificationConfig {
        &self.notifications_state.notification_config
    }

    /// Get mutable access to notification configuration
    pub fn notification_config_mut(&mut self) -> &mut NotificationConfig {
        &mut self.notifications_state.notification_config
    }

    /// Get notification configuration
    pub fn get_notification_config(&self) -> NotificationConfig {
        self.notifications_state.notification_config.clone()
    }

    /// Set notification configuration
    pub fn set_notification_config(&mut self, config: NotificationConfig) {
        self.notifications_state.notification_config = config;
    }

    /// Get all notification events
    pub fn get_notification_events(&self) -> &[NotificationEvent] {
        &self.notifications_state.notification_events
    }

    /// Clear all notification events
    pub fn clear_notification_events(&mut self) {
        self.notifications_state.notification_events.clear();
    }

    /// Mark a notification as delivered by index
    pub fn mark_notification_delivered(&mut self, index: usize) {
        if let Some(event) = self.notifications_state.notification_events.get_mut(index) {
            event.delivered = true;
        }
    }

    /// Update last activity timestamp
    pub fn update_activity(&mut self) {
        self.notifications_state.last_activity_time = crate::terminal::unix_millis();
    }

    /// Check for silence notification trigger
    pub fn check_silence(&mut self) {
        if !self.notifications_state.notification_config.silence_enabled {
            return;
        }
        let now = crate::terminal::unix_millis();
        if now - self.notifications_state.last_activity_time
            > self
                .notifications_state
                .notification_config
                .silence_threshold
                * 1000
            && now - self.notifications_state.last_silence_check
                > self
                    .notifications_state
                    .notification_config
                    .silence_threshold
                    * 1000
        {
            self.add_notification_event(
                NotificationTrigger::Silence,
                NotificationAlert::Visual,
                Some("Terminal is silent".to_string()),
            );
            self.notifications_state.last_silence_check = now;
        }
    }

    /// Check for activity notification trigger
    pub fn check_activity(&mut self) {
        if self
            .notifications_state
            .notification_config
            .activity_enabled
        {
            // Implementation for activity check
        }
    }

    /// Register a custom notification trigger
    pub fn register_custom_trigger(&mut self, id: u32, message: String) {
        self.notifications_state.custom_triggers.insert(id, message);
    }

    /// Trigger a custom notification by ID
    pub fn trigger_custom_notification(&mut self, id: u32, alert: NotificationAlert) {
        let message = self.notifications_state.custom_triggers.get(&id).cloned();
        self.add_notification_event(NotificationTrigger::Custom(id), alert, message);
    }

    /// Handle a bell notification
    pub fn handle_bell_notification(&mut self) {
        let alert = if self.notifications_state.notification_config.bell_desktop {
            NotificationAlert::Desktop
        } else if self.notifications_state.notification_config.bell_sound > 0 {
            NotificationAlert::Sound(self.notifications_state.notification_config.bell_sound)
        } else {
            NotificationAlert::Visual
        };
        self.add_notification_event(
            NotificationTrigger::Bell,
            alert,
            Some("Bell rang".to_string()),
        );
    }

    /// Explicitly trigger a notification
    pub fn trigger_notification(
        &mut self,
        trigger: NotificationTrigger,
        alert: NotificationAlert,
        message: Option<String>,
    ) {
        self.add_notification_event(trigger, alert, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_new() {
        let notif = Notification::new("Title".to_string(), "Message".to_string());
        assert_eq!(notif.title, "Title");
        assert_eq!(notif.message, "Message");
    }

    #[test]
    fn test_notification_empty_title() {
        let notif = Notification::new("".to_string(), "Message".to_string());
        assert_eq!(notif.title, "");
        assert_eq!(notif.message, "Message");
    }

    #[test]
    fn test_notification_empty_message() {
        let notif = Notification::new("Title".to_string(), "".to_string());
        assert_eq!(notif.title, "Title");
        assert_eq!(notif.message, "");
    }

    #[test]
    fn test_notification_both_empty() {
        let notif = Notification::new("".to_string(), "".to_string());
        assert_eq!(notif.title, "");
        assert_eq!(notif.message, "");
    }

    #[test]
    fn test_notification_clone() {
        let notif1 = Notification::new("Title".to_string(), "Message".to_string());
        let notif2 = notif1.clone();
        assert_eq!(notif1, notif2);
    }

    #[test]
    fn test_notification_equality() {
        let notif1 = Notification::new("Title".to_string(), "Message".to_string());
        let notif2 = Notification::new("Title".to_string(), "Message".to_string());
        assert_eq!(notif1, notif2);
    }

    #[test]
    fn test_notification_inequality_title() {
        let notif1 = Notification::new("Title1".to_string(), "Message".to_string());
        let notif2 = Notification::new("Title2".to_string(), "Message".to_string());
        assert_ne!(notif1, notif2);
    }

    #[test]
    fn test_notification_inequality_message() {
        let notif1 = Notification::new("Title".to_string(), "Message1".to_string());
        let notif2 = Notification::new("Title".to_string(), "Message2".to_string());
        assert_ne!(notif1, notif2);
    }

    #[test]
    fn test_notification_debug() {
        let notif = Notification::new("Title".to_string(), "Message".to_string());
        let debug_str = format!("{:?}", notif);
        assert!(debug_str.contains("Title"));
        assert!(debug_str.contains("Message"));
    }

    #[test]
    fn test_notification_with_unicode() {
        let notif = Notification::new("📢 Alert".to_string(), "Message with emoji 🎉".to_string());
        assert_eq!(notif.title, "📢 Alert");
        assert_eq!(notif.message, "Message with emoji 🎉");
    }

    #[test]
    fn test_notification_with_newlines() {
        let notif = Notification::new(
            "Multi\nLine\nTitle".to_string(),
            "Multi\nLine\nMessage".to_string(),
        );
        assert!(notif.title.contains('\n'));
        assert!(notif.message.contains('\n'));
    }

    #[test]
    fn test_notification_with_special_chars() {
        let notif = Notification::new(
            "Title with \"quotes\" and 'apostrophes'".to_string(),
            "Message with <tags> & symbols".to_string(),
        );
        assert!(notif.title.contains('"'));
        assert!(notif.message.contains('<'));
    }

    #[test]
    fn test_notification_new_defaults_metadata() {
        let notif = Notification::new("Title".to_string(), "Message".to_string());
        assert_eq!(notif.id, None);
        assert_eq!(notif.urgency, Urgency::Normal);
        assert!(notif.actions.is_empty());
    }

    #[test]
    fn test_notification_with_metadata() {
        let notif = Notification::with_metadata(
            "Title".to_string(),
            "Message".to_string(),
            Some("id1".to_string()),
            Urgency::Critical,
            vec!["focus".to_string()],
        );
        assert_eq!(notif.id.as_deref(), Some("id1"));
        assert_eq!(notif.urgency, Urgency::Critical);
        assert_eq!(notif.actions, vec!["focus".to_string()]);
    }

    #[test]
    fn test_urgency_from_param() {
        assert_eq!(Urgency::from_param("0"), Urgency::Low);
        assert_eq!(Urgency::from_param("1"), Urgency::Normal);
        assert_eq!(Urgency::from_param("2"), Urgency::Critical);
        assert_eq!(Urgency::from_param("bogus"), Urgency::Normal);
    }

    #[test]
    fn test_urgency_default() {
        assert_eq!(Urgency::default(), Urgency::Normal);
    }
}
