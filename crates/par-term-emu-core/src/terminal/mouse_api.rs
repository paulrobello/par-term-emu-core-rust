//! Terminal mouse-history methods (Feature 17).
//!
//! `crate::mouse` stays a leaf module of mouse wire-format types; these
//! `impl Terminal` methods are the terminal-side counterpart and live in
//! the terminal layer so the leaf never names `Terminal` (ARC-108).

use crate::mouse::{MouseButton, MouseEventRecord, MouseEventType, MousePosition};
use crate::terminal::Terminal;

impl Terminal {
    // === Feature 17: Advanced Mouse Support ===

    /// Record a mouse event in history
    pub fn record_mouse_event(
        &mut self,
        event_type: MouseEventType,
        button: MouseButton,
        col: usize,
        row: usize,
        modifiers: u8,
    ) {
        let record = MouseEventRecord {
            event_type,
            button,
            col,
            row,
            pixel_x: None, // Could be populated if cell size known
            pixel_y: None,
            modifiers,
            timestamp: crate::terminal::get_timestamp_us(),
        };

        self.mouse_history.mouse_events.push(record);
        if self.mouse_history.mouse_events.len() > self.host.max_mouse_history {
            self.mouse_history.mouse_events.remove(0);
        }

        // Also record position history
        self.mouse_history.mouse_positions.push(MousePosition {
            col,
            row,
            timestamp: crate::terminal::get_timestamp_us(),
        });
        if self.mouse_history.mouse_positions.len() > self.host.max_mouse_history {
            self.mouse_history.mouse_positions.remove(0);
        }
    }

    /// Get mouse event history
    pub fn get_mouse_history(&self) -> &[MouseEventRecord] {
        &self.mouse_history.mouse_events
    }

    /// Get recent mouse positions
    pub fn get_mouse_positions(&self) -> &[MousePosition] {
        &self.mouse_history.mouse_positions
    }

    /// Clear mouse history
    pub fn clear_mouse_history(&mut self) {
        self.mouse_history.mouse_events.clear();
        self.mouse_history.mouse_positions.clear();
    }

    /// Set the maximum number of mouse events to retain
    pub fn set_max_mouse_history(&mut self, max: usize) {
        self.host.max_mouse_history = max;
        if self.mouse_history.mouse_events.len() > max {
            self.mouse_history
                .mouse_events
                .drain(0..self.mouse_history.mouse_events.len() - max);
        }
        if self.mouse_history.mouse_positions.len() > max {
            self.mouse_history
                .mouse_positions
                .drain(0..self.mouse_history.mouse_positions.len() - max);
        }
    }

    /// Get the maximum number of mouse events to retain
    pub fn get_max_mouse_history(&self) -> usize {
        self.host.max_mouse_history
    }
}
