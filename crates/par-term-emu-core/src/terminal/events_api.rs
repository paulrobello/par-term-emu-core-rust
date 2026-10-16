//! Observer registry and event polling/subscription (ARC-003).

use super::{BellEvent, CwdChange, ShellEvent, Terminal, TerminalEvent, TerminalEventKind};
use std::collections::HashSet;

impl Terminal {
    // === Observer Management ===

    /// Add an observer for push-based event delivery
    pub fn add_observer(
        &mut self,
        observer: std::sync::Arc<dyn crate::observer::TerminalObserver>,
    ) -> crate::observer::ObserverId {
        self.events.add_observer(observer)
    }

    /// Remove an observer by ID
    pub fn remove_observer(&mut self, id: crate::observer::ObserverId) -> bool {
        self.events.remove_observer(id)
    }

    /// Get the number of registered observers
    pub fn observer_count(&self) -> usize {
        self.events.observer_count()
    }

    /// Move zones evicted from either grid onto the event queue as
    /// `ZoneScrolledOut` events. Runs before observer dispatch in
    /// `process`/`process_deferred`/`apply_action(s)` so observers and
    /// filtered polls see them; `poll_events` runs it too, for zones evicted
    /// by a `Grid` mutated directly rather than through the parser.
    pub(super) fn flush_evicted_zones(&mut self) {
        let evicted = self.grid.drain_evicted_zones();
        let alt_evicted = self.alt_grid.drain_evicted_zones();
        for zone in evicted.into_iter().chain(alt_evicted) {
            self.events.push(TerminalEvent::ZoneScrolledOut {
                zone_id: zone.id,
                zone_type: zone.zone_type,
            });
        }
    }

    /// Poll for pending events
    pub fn poll_events(&mut self) -> Vec<TerminalEvent> {
        self.flush_evicted_zones();
        self.events.take_all()
    }

    /// Drain pending bell events
    pub fn drain_bell_events(&mut self) -> Vec<BellEvent> {
        self.events.drain_bells()
    }

    // === Event Subscription ===

    /// Set the event subscription filter
    pub fn set_event_subscription(&mut self, filter: HashSet<TerminalEventKind>) {
        self.events.set_subscription(Some(filter));
    }

    /// Clear the event subscription filter (subscribe to all events)
    pub fn clear_event_subscription(&mut self) {
        self.events.set_subscription(None);
    }

    /// Poll for events that match the current subscription filter
    pub fn poll_subscribed_events(&mut self) -> Vec<TerminalEvent> {
        self.flush_evicted_zones();
        // No filter: the full drain.
        self.events
            .extract_subscribed()
            .unwrap_or_else(|| self.poll_events())
    }

    /// Poll for CWD change events
    pub fn poll_cwd_events(&mut self) -> Vec<CwdChange> {
        self.events.extract(|event| match event {
            TerminalEvent::CwdChanged(change) => Ok(change),
            other => Err(other),
        })
    }

    /// Poll for upload request events
    ///
    /// Returns all pending UploadRequested events and removes them from the queue.
    pub fn poll_upload_requests(&mut self) -> Vec<String> {
        self.events.extract(|event| match event {
            TerminalEvent::UploadRequested { format } => Ok(format),
            other => Err(other),
        })
    }

    /// Poll for shell integration events
    pub fn poll_shell_integration_events(&mut self) -> Vec<ShellEvent> {
        self.events.extract(|event| match event {
            TerminalEvent::ShellIntegrationEvent {
                event_type,
                command,
                exit_code,
                timestamp,
                cursor_line,
            } => Ok((event_type, command, exit_code, timestamp, cursor_line)),
            other => Err(other),
        })
    }

    /// Drain any pending `ScreenCleared` events.
    ///
    /// Returns a `Vec<bool>` where each element corresponds to one clear event;
    /// `true` means the scrollback was also cleared (ESC[3J), `false` means
    /// only the visible screen was cleared (ESC[2J).
    pub fn poll_screen_cleared_events(&mut self) -> Vec<bool> {
        self.events.extract(|event| match event {
            TerminalEvent::ScreenCleared { include_scrollback } => Ok(include_scrollback),
            other => Err(other),
        })
    }
}
