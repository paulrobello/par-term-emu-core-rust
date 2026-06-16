//! Mouse API methods for `PyTerminal` (ARC-002: split out of the monolithic
//! `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API or
//! behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 17: Advanced Mouse Support ===

    /// Record a mouse event
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

    /// Get mouse events
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

    /// Get mouse positions
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

    /// Get last mouse position
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
