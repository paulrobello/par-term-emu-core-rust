//! Pane, window-layout, and session-state types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Pane state
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "PaneState", from_py_object)]
#[derive(Clone)]
pub struct PyPaneState {
    /// Pane identifier
    pub id: String,
    /// Pane title
    pub title: String,
    /// Pane size as (cols, rows)
    pub size: (usize, usize),
    /// Pane position as (x, y)
    pub position: (usize, usize),
    /// Working directory reported by the pane, if known
    pub cwd: Option<String>,
    /// Visible pane content, one string per row
    pub content: Vec<String>,
    /// Cursor position as (col, row)
    pub cursor: (usize, usize),
    /// Whether the pane is showing the alternate screen
    pub alt_screen: bool,
    /// Scroll offset into the pane's scrollback (0 = bottom)
    pub scroll_offset: usize,
    /// Unix epoch milliseconds when the pane was created
    pub created_at: u64,
    /// Unix epoch milliseconds of the last activity
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
    /// Layout identifier
    pub id: String,
    /// Layout name
    pub name: String,
    /// Split direction ("horizontal" or "vertical")
    pub direction: String,
    /// Pane IDs participating in this layout
    pub panes: Vec<String>,
    /// Relative pane sizes (percentages)
    pub sizes: Vec<u8>,
    /// Index of the active pane
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
    /// Session identifier
    pub id: String,
    /// Session name
    pub name: String,
    /// Panes belonging to this session
    pub panes: Vec<PyPaneState>,
    /// Window layouts of this session
    pub layouts: Vec<PyWindowLayout>,
    /// Index of the active layout
    pub active_layout: usize,
    /// Unix epoch milliseconds when the session was created
    pub created_at: u64,
    /// Unix epoch milliseconds when the session was last saved
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
