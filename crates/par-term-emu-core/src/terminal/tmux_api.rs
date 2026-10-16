//! tmux control-mode toggles and notification buffer (ARC-003).

use super::Terminal;

impl Terminal {
    /// Enable or disable tmux control mode
    pub fn set_tmux_control_mode(&mut self, enabled: bool) {
        self.tmux.tmux_parser.set_control_mode(enabled);
    }

    /// Check if tmux control mode is enabled
    pub fn is_tmux_control_mode(&self) -> bool {
        self.tmux.tmux_parser.is_control_mode()
    }

    /// Enable or disable tmux control mode auto-detection
    pub fn set_tmux_auto_detect(&mut self, enabled: bool) {
        self.tmux.tmux_parser.set_auto_detect(enabled);
    }

    /// Check if tmux control mode auto-detection is enabled
    pub fn is_tmux_auto_detect(&self) -> bool {
        self.tmux.tmux_parser.is_auto_detect()
    }

    /// Get tmux control protocol notifications
    pub fn tmux_notifications(&self) -> &[crate::tmux_control::TmuxNotification] {
        &self.tmux.tmux_notifications
    }

    /// Drain and return tmux control protocol notifications
    pub fn drain_tmux_notifications(&mut self) -> Vec<crate::tmux_control::TmuxNotification> {
        std::mem::take(&mut self.tmux.tmux_notifications)
    }

    /// Check if there are pending tmux control protocol notifications
    pub fn has_tmux_notifications(&self) -> bool {
        !self.tmux.tmux_notifications.is_empty()
    }

    /// Clear the tmux control protocol notifications buffer
    pub fn clear_tmux_notifications(&mut self) {
        self.tmux.tmux_notifications.clear();
    }
}
