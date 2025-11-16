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
