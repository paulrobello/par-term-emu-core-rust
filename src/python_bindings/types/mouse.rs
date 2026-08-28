//! Mouse event and position types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Mouse event
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "MouseEvent", from_py_object)]
#[derive(Clone)]
pub struct PyMouseEvent {
    /// Event kind: "press", "release", or "motion"
    pub event_type: String,
    /// Button name (e.g. "left", "right", "middle", "wheel_up", "none")
    pub button: String,
    /// Column (0-indexed)
    pub col: usize,
    /// Row (0-indexed)
    pub row: usize,
    /// X pixel coordinate, when the terminal reports one
    pub pixel_x: Option<u16>,
    /// Y pixel coordinate, when the terminal reports one
    pub pixel_y: Option<u16>,
    /// Modifier bitflags (shift=1, alt=2, ctrl=4, etc.)
    pub modifiers: u8,
    /// Unix epoch milliseconds when the event occurred
    pub timestamp: u64,
}

#[pymethods]
impl PyMouseEvent {
    fn __repr__(&self) -> String {
        format!(
            "MouseEvent(type={}, button={}, pos=({}, {}), timestamp={})",
            self.event_type, self.button, self.col, self.row, self.timestamp
        )
    }
}

impl From<&crate::mouse::MouseEventRecord> for PyMouseEvent {
    fn from(event: &crate::mouse::MouseEventRecord) -> Self {
        use crate::mouse::{MouseButton, MouseEventType};

        let event_type = match event.event_type {
            MouseEventType::Press => "press",
            MouseEventType::Release => "release",
            MouseEventType::Move => "move",
            MouseEventType::Drag => "drag",
            MouseEventType::ScrollUp => "scrollup",
            MouseEventType::ScrollDown => "scrolldown",
        }
        .to_string();

        let button = match event.button {
            MouseButton::Left => "left",
            MouseButton::Middle => "middle",
            MouseButton::Right => "right",
            MouseButton::None => "none",
        }
        .to_string();

        PyMouseEvent {
            event_type,
            button,
            col: event.col,
            row: event.row,
            pixel_x: event.pixel_x,
            pixel_y: event.pixel_y,
            modifiers: event.modifiers,
            timestamp: event.timestamp,
        }
    }
}

/// Mouse position
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "MousePosition", from_py_object)]
#[derive(Clone)]
pub struct PyMousePosition {
    /// Column (0-indexed)
    pub col: usize,
    /// Row (0-indexed)
    pub row: usize,
    /// Unix epoch milliseconds when the position was recorded
    pub timestamp: u64,
}

#[pymethods]
impl PyMousePosition {
    fn __repr__(&self) -> String {
        format!(
            "MousePosition(col={}, row={}, timestamp={})",
            self.col, self.row, self.timestamp
        )
    }
}

impl From<&crate::mouse::MousePosition> for PyMousePosition {
    fn from(pos: &crate::mouse::MousePosition) -> Self {
        PyMousePosition {
            col: pos.col,
            row: pos.row,
            timestamp: pos.timestamp,
        }
    }
}
