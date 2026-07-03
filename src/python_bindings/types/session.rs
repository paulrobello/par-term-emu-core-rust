//! Pane, window-layout, and session-state types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Pane state
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "PaneState", from_py_object)]
#[derive(Clone)]
pub struct PyPaneState {
    pub id: String,
    pub title: String,
    pub size: (usize, usize),
    pub position: (usize, usize),
    pub cwd: Option<String>,
    pub content: Vec<String>,
    pub cursor: (usize, usize),
    pub alt_screen: bool,
    pub scroll_offset: usize,
    pub created_at: u64,
    pub last_activity: u64,
}

#[pymethods]
impl PyPaneState {
    fn __repr__(&self) -> String {
        format!(
            "PaneState(id={}, title={}, size={}x{})",
            self.id, self.title, self.size.0, self.size.1
        )
    }
}

impl From<&crate::terminal::PaneState> for PyPaneState {
    fn from(state: &crate::terminal::PaneState) -> Self {
        PyPaneState {
            id: state.id.clone(),
            title: state.title.clone(),
            size: state.size,
            position: state.position,
            cwd: state.cwd.clone(),
            content: state.content.clone(),
            cursor: state.cursor,
            alt_screen: state.alt_screen,
            scroll_offset: state.scroll_offset,
            created_at: state.created_at,
            last_activity: state.last_activity,
        }
    }
}

/// Window layout
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "WindowLayout", from_py_object)]
#[derive(Clone)]
pub struct PyWindowLayout {
    pub id: String,
    pub name: String,
    pub direction: String,
    pub panes: Vec<String>,
    pub sizes: Vec<u8>,
    pub active_pane: usize,
}

#[pymethods]
impl PyWindowLayout {
    fn __repr__(&self) -> String {
        format!(
            "WindowLayout(id={}, name={}, panes={})",
            self.id,
            self.name,
            self.panes.len()
        )
    }
}

impl From<&crate::terminal::WindowLayout> for PyWindowLayout {
    fn from(layout: &crate::terminal::WindowLayout) -> Self {
        use crate::terminal::LayoutDirection;

        let direction = match layout.direction {
            LayoutDirection::Horizontal => "horizontal",
            LayoutDirection::Vertical => "vertical",
        }
        .to_string();

        PyWindowLayout {
            id: layout.id.clone(),
            name: layout.name.clone(),
            direction,
            panes: layout.panes.clone(),
            sizes: layout.sizes.clone(),
            active_pane: layout.active_pane,
        }
    }
}

/// Session state
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "SessionState", from_py_object)]
#[derive(Clone)]
pub struct PySessionState {
    pub id: String,
    pub name: String,
    pub panes: Vec<PyPaneState>,
    pub layouts: Vec<PyWindowLayout>,
    pub active_layout: usize,
    pub created_at: u64,
    pub last_saved: u64,
}

#[pymethods]
impl PySessionState {
    fn __repr__(&self) -> String {
        format!(
            "SessionState(id={}, name={}, panes={}, layouts={})",
            self.id,
            self.name,
            self.panes.len(),
            self.layouts.len()
        )
    }
}

impl From<&crate::terminal::SessionState> for PySessionState {
    fn from(session: &crate::terminal::SessionState) -> Self {
        PySessionState {
            id: session.id.clone(),
            name: session.name.clone(),
            panes: session.panes.iter().map(PyPaneState::from).collect(),
            layouts: session.layouts.iter().map(PyWindowLayout::from).collect(),
            active_layout: session.active_layout,
            created_at: session.created_at,
            last_saved: session.last_saved,
        }
    }
}
