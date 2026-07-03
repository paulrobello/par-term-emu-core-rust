//! Mouse event and position types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Mouse event
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "MouseEvent", from_py_object)]
#[derive(Clone)]
pub struct PyMouseEvent {
    pub event_type: String,
    pub button: String,
    pub col: usize,
    pub row: usize,
    pub pixel_x: Option<u16>,
    pub pixel_y: Option<u16>,
    pub modifiers: u8,
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
    pub col: usize,
    pub row: usize,
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
