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
