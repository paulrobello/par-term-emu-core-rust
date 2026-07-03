//! Mouse API methods for `PyTerminal` (ARC-002: split out of the monolithic
//! `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API or
//! behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 17: Advanced Mouse Support ===

    /// Record a mouse event in the terminal's mouse history
    ///
    /// The event is appended to both the event history (`get_mouse_events()`)
    /// and the position history (`get_mouse_positions()`), each trimmed to
    /// `get_max_mouse_history()` entries. The event's stored timestamp is
    /// generated internally (microseconds since epoch) rather than taken from
    /// the `timestamp` argument.
    ///
    /// Args:
    ///     event_type: One of "press", "release", "move", "drag", "scrollup",
    ///         "scrolldown" (case-insensitive)
    ///     button: One of "left", "middle", "right", "none" (case-insensitive)
    ///     col: Column position, 0-indexed
    ///     row: Row position, 0-indexed
    ///     pixel_x: Optional pixel X position; accepted for API compatibility
    ///         but currently not stored (reserved for future SGR 1016 support)
    ///     pixel_y: Optional pixel Y position; accepted for API compatibility
    ///         but currently not stored (reserved for future SGR 1016 support)
    ///     modifiers: Modifier key bitmask (shift/alt/ctrl)
    ///     timestamp: Accepted for API compatibility but currently ignored;
    ///         the recorded event always uses the current time
    ///
    /// Raises:
    ///     ValueError: If `event_type` or `button` is not one of the supported
    ///         values above
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.record_mouse_event("press", "left", 10, 5, None, None, 0, 0)
    ///     ```
    #[allow(clippy::too_many_arguments, unused_variables)]
    fn record_mouse_event(
        &mut self,
        event_type: &str,
        button: &str,
        col: usize,
        row: usize,
        pixel_x: Option<u16>,
        pixel_y: Option<u16>,
        modifiers: u8,
        timestamp: u64,
    ) -> PyResult<()> {
        use crate::mouse::{MouseButton, MouseEventType};

        let event_type = match event_type.to_lowercase().as_str() {
            "press" => MouseEventType::Press,
            "release" => MouseEventType::Release,
            "move" => MouseEventType::Move,
            "drag" => MouseEventType::Drag,
            "scrollup" => MouseEventType::ScrollUp,
            "scrolldown" => MouseEventType::ScrollDown,
            _ => return Err(PyValueError::new_err("Invalid mouse event type")),
        };

        let button = match button.to_lowercase().as_str() {
            "left" => MouseButton::Left,
            "middle" => MouseButton::Middle,
            "right" => MouseButton::Right,
            "none" => MouseButton::None,
            _ => return Err(PyValueError::new_err("Invalid mouse button")),
        };

        self.inner
            .record_mouse_event(event_type, button, col, row, modifiers);
        Ok(())
    }

    /// Get recorded mouse events, most recent last
    ///
    /// Args:
    ///     count: If given, return only the last `count` events; if None,
    ///         return the full history (default: None)
    ///
    /// Returns:
    ///     list[MouseEvent]: Each event has `event_type`, `button`, `col`,
    ///     `row`, `pixel_x`, `pixel_y`, `modifiers`, `timestamp` (microseconds)
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.record_mouse_event("press", "left", 10, 5, None, None, 0, 0)
    ///     events = term.get_mouse_events(count=10)
    ///     ```
    #[pyo3(signature = (count=None))]
    fn get_mouse_events(
        &self,
        count: Option<usize>,
    ) -> PyResult<Vec<crate::python_bindings::types::PyMouseEvent>> {
        let all_events = self.inner.get_mouse_history();
        let events = match count {
            Some(n) => &all_events[all_events.len().saturating_sub(n)..],
            None => all_events,
        };
        Ok(events
            .iter()
            .map(crate::python_bindings::types::PyMouseEvent::from)
            .collect())
    }

    /// Get recorded mouse cursor positions, most recent last
    ///
    /// Args:
    ///     count: If given, return only the last `count` positions; if None,
    ///         return the full history (default: None)
    ///
    /// Returns:
    ///     list[MousePosition]: Each position has `col`, `row`, and
    ///     `timestamp` (microseconds since epoch)
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.record_mouse_event("move", "none", 10, 5, None, None, 0, 0)
    ///     positions = term.get_mouse_positions(count=5)
    ///     ```
    #[pyo3(signature = (count=None))]
    fn get_mouse_positions(
        &self,
        count: Option<usize>,
    ) -> PyResult<Vec<crate::python_bindings::types::PyMousePosition>> {
        let all_positions = self.inner.get_mouse_positions();
        let positions = match count {
            Some(n) => &all_positions[all_positions.len().saturating_sub(n)..],
            None => all_positions,
        };
        Ok(positions
            .iter()
            .map(crate::python_bindings::types::PyMousePosition::from)
            .collect())
    }

    /// Get the most recently recorded mouse position
    ///
    /// Returns:
    ///     MousePosition | None: The last position recorded via
    ///     `record_mouse_event()`, or None if no events have been recorded
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.record_mouse_event("move", "none", 10, 5, None, None, 0, 0)
    ///     pos = term.get_last_mouse_position()
    ///     if pos:
    ///         print(pos.col, pos.row)
    ///     ```
    fn get_last_mouse_position(
        &self,
    ) -> PyResult<Option<crate::python_bindings::types::PyMousePosition>> {
        Ok(self
            .inner
            .get_mouse_positions()
            .last()
            .map(crate::python_bindings::types::PyMousePosition::from))
    }

    /// Clear mouse history
    fn clear_mouse_history(&mut self) -> PyResult<()> {
        self.inner.clear_mouse_history();
        Ok(())
    }

    /// Set maximum mouse history size
    fn set_max_mouse_history(&mut self, max: usize) -> PyResult<()> {
        self.inner.set_max_mouse_history(max);
        Ok(())
    }

    /// Get maximum mouse history size
    fn get_max_mouse_history(&self) -> PyResult<usize> {
        Ok(self.inner.get_max_mouse_history())
    }
}
