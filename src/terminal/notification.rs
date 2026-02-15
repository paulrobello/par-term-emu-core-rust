//! Notification support for OSC 9 and OSC 777 sequences

/// Notification data from OSC 9 or OSC 777 sequences
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Notification title (may be empty for OSC 9)
    pub title: String,
    /// Notification message/body
    pub message: String,
}

impl Notification {
    /// Create a new notification
    pub fn new(title: String, message: String) -> Self {
        Self { title, message }
    }
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

        self.notification_events.push(event);
        if self.notification_events.len() > self.max_notifications {
            self.notification_events.remove(0);
        }
    }

    /// Get notification configuration
    pub fn notification_config(&self) -> &NotificationConfig {
        &self.notification_config
    }

    /// Get mutable access to notification configuration
    pub fn notification_config_mut(&mut self) -> &mut NotificationConfig {
        &mut self.notification_config
    }

    /// Get notification configuration
    pub fn get_notification_config(&self) -> NotificationConfig {
        self.notification_config.clone()
    }

    /// Set notification configuration
    pub fn set_notification_config(&mut self, config: NotificationConfig) {
        self.notification_config = config;
    }

    /// Get all notification events
    pub fn get_notification_events(&self) -> &[NotificationEvent] {
        &self.notification_events
    }

    /// Clear all notification events
    pub fn clear_notification_events(&mut self) {
        self.notification_events.clear();
    }

    /// Mark a notification as delivered by index
    pub fn mark_notification_delivered(&mut self, index: usize) {
        if let Some(event) = self.notification_events.get_mut(index) {
            event.delivered = true;
        }
    }

    /// Update last activity timestamp
    pub fn update_activity(&mut self) {
        self.last_activity_time = crate::terminal::unix_millis();
    }

    /// Check for silence notification trigger
    pub fn check_silence(&mut self) {
        if !self.notification_config.silence_enabled {
            return;
        }
        let now = crate::terminal::unix_millis();
        if now - self.last_activity_time > self.notification_config.silence_threshold * 1000
            && now - self.last_silence_check > self.notification_config.silence_threshold * 1000
        {
            self.add_notification_event(
                NotificationTrigger::Silence,
                NotificationAlert::Visual,
                Some("Terminal is silent".to_string()),
            );
            self.last_silence_check = now;
        }
    }

    /// Check for activity notification trigger
    pub fn check_activity(&mut self) {
        if self.notification_config.activity_enabled {
            // Implementation for activity check
        }
    }

    /// Register a custom notification trigger
    pub fn register_custom_trigger(&mut self, id: u32, message: String) {
        self.custom_triggers.insert(id, message);
    }

    /// Trigger a custom notification by ID
    pub fn trigger_custom_notification(&mut self, id: u32, alert: NotificationAlert) {
        let message = self.custom_triggers.get(&id).cloned();
        self.add_notification_event(NotificationTrigger::Custom(id), alert, message);
    }

    /// Handle a bell notification
    pub fn handle_bell_notification(&mut self) {
        let alert = if self.notification_config.bell_desktop {
            NotificationAlert::Desktop
        } else if self.notification_config.bell_sound > 0 {
            NotificationAlert::Sound(self.notification_config.bell_sound)
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
}
