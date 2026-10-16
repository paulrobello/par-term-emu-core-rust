//! OSC 9/777 notification queue, OSC 9;4 / OSC 934 progress bars, and the
//! bell counter (ARC-003).

use super::{NamedProgressBar, Notification, ProgressBar, ProgressState, Terminal};
use std::collections::HashMap;

impl Terminal {
    /// Get pending notifications (OSC 9 / OSC 777)
    ///
    /// Returns a reference to the list of notifications that have been received
    /// but not yet retrieved.
    pub fn notifications(&self) -> &[Notification] {
        &self.notifications_state.notifications
    }

    /// Take all pending notifications
    ///
    /// Returns and clears the notification queue. Use this to poll for new notifications.
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        std::mem::take(&mut self.notifications_state.notifications)
    }

    /// Check if there are pending notifications
    pub fn has_notifications(&self) -> bool {
        !self.notifications_state.notifications.is_empty()
    }

    pub(super) fn enqueue_notification(&mut self, notification: Notification) {
        self.notifications_state.enqueue(notification);
    }

    /// Set maximum OSC 9/777 notifications retained (0 disables buffering)
    pub fn set_max_notifications(&mut self, max: usize) {
        self.notifications_state.max_notifications = max;
        if max == 0 {
            self.notifications_state.notifications.clear();
        } else if self.notifications_state.notifications.len() > max {
            let excess = self.notifications_state.notifications.len() - max;
            self.notifications_state.notifications.drain(0..excess);
        }
    }

    /// Get maximum OSC 9/777 notifications retained
    pub fn max_notifications(&self) -> usize {
        self.notifications_state.max_notifications
    }

    // === Progress Bar Methods (OSC 9;4) ===

    /// Get the current progress bar state
    ///
    /// Returns the progress bar state set via OSC 9;4 sequences.
    /// The progress bar has a state (hidden, normal, indeterminate, warning, error)
    /// and a percentage (0-100) for states that support it.
    pub fn progress_bar(&self) -> &ProgressBar {
        &self.progress_state.progress_bar
    }

    /// Check if the progress bar is currently active (visible)
    ///
    /// Returns true if the progress bar is in any state other than Hidden.
    pub fn has_progress(&self) -> bool {
        self.progress_state.progress_bar.is_active()
    }

    /// Get the current progress percentage (0-100)
    ///
    /// Returns the progress percentage. Only meaningful when the progress bar
    /// state is Normal, Warning, or Error.
    pub fn progress_value(&self) -> u8 {
        self.progress_state.progress_bar.progress
    }

    /// Get the current progress bar state
    ///
    /// Returns the state (Hidden, Normal, Indeterminate, Warning, Error).
    pub fn progress_state(&self) -> ProgressState {
        self.progress_state.progress_bar.state
    }

    /// Manually set the progress bar state
    ///
    /// This can be used to programmatically control the progress bar
    /// without receiving OSC 9;4 sequences.
    pub fn set_progress(&mut self, state: ProgressState, progress: u8) {
        self.progress_state.progress_bar = ProgressBar::new(state, progress);
    }

    /// Clear/hide the progress bar
    ///
    /// Equivalent to receiving OSC 9;4;0.
    pub fn clear_progress(&mut self) {
        self.progress_state.progress_bar = ProgressBar::hidden();
    }

    // Named progress bar methods (OSC 934)

    /// Get all named progress bars
    ///
    /// Returns the map of active named progress bars set via OSC 934 sequences.
    pub fn named_progress_bars(&self) -> &HashMap<String, NamedProgressBar> {
        &self.progress_state.named_progress_bars
    }

    /// Get a specific named progress bar by ID
    pub fn get_named_progress_bar(&self, id: &str) -> Option<&NamedProgressBar> {
        self.progress_state.named_progress_bars.get(id)
    }

    /// Set or update a named progress bar and emit an event
    pub fn set_named_progress_bar(&mut self, bar: NamedProgressBar) {
        self.progress_state.set_named(bar, &mut self.events);
    }

    /// Remove a named progress bar by ID and emit an event
    ///
    /// Returns true if the bar existed and was removed.
    pub fn remove_named_progress_bar(&mut self, id: &str) -> bool {
        self.progress_state.remove_named(id, &mut self.events)
    }

    /// Remove all named progress bars and emit an event
    pub fn remove_all_named_progress_bars(&mut self) {
        self.progress_state.remove_all_named(&mut self.events);
    }

    /// Get the current bell count
    ///
    /// This counter increments each time the terminal receives a bell character (BEL/\x07).
    /// Applications can poll this to detect bell events for visual bell implementations.
    ///
    /// Returns the total number of bell events received since terminal creation.
    pub fn bell_count(&self) -> u64 {
        self.progress_state.bell_count
    }
}
