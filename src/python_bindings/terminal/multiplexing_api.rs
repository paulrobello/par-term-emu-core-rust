//! Multiplexing API methods for `PyTerminal` (ARC-002: split out of the monolithic
//! `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API or
//! behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 13: Terminal Multiplexing ===

    /// Capture current pane state
    #[pyo3(signature = (id, cwd=None))]
    fn capture_pane_state(
        &self,
        id: String,
        cwd: Option<String>,
    ) -> PyResult<crate::python_bindings::types::PyPaneState> {
        let state = self.inner.capture_pane_state(id, cwd);
        Ok(crate::python_bindings::types::PyPaneState::from(&state))
    }

    /// Restore pane state
    fn restore_pane_state(
        &mut self,
        state: &crate::python_bindings::types::PyPaneState,
    ) -> PyResult<()> {
        // Convert Python state to Rust state
        use crate::terminal::PaneState;

        let rust_state = PaneState {
            id: state.id.clone(),
            title: state.title.clone(),
            size: state.size,
            position: state.position,
            cwd: state.cwd.clone(),
            env: std::collections::HashMap::new(), // Not exposed in Python for now
            content: state.content.clone(),
            cursor: state.cursor,
            alt_screen: state.alt_screen,
            scroll_offset: state.scroll_offset,
            created_at: state.created_at,
            last_activity: state.last_activity,
        };

        self.inner.restore_pane_state(&rust_state);
        Ok(())
    }

    /// Set pane state
    fn set_pane_state(
        &mut self,
        state: &crate::python_bindings::types::PyPaneState,
    ) -> PyResult<()> {
        // Convert Python state to Rust state
        use crate::terminal::PaneState;

        let rust_state = PaneState {
            id: state.id.clone(),
            title: state.title.clone(),
            size: state.size,
            position: state.position,
            cwd: state.cwd.clone(),
            env: std::collections::HashMap::new(),
            content: state.content.clone(),
            cursor: state.cursor,
            alt_screen: state.alt_screen,
            scroll_offset: state.scroll_offset,
            created_at: state.created_at,
            last_activity: state.last_activity,
        };

        self.inner.set_pane_state(rust_state);
        Ok(())
    }

    /// Get pane state
    fn get_pane_state(&self) -> PyResult<Option<crate::python_bindings::types::PyPaneState>> {
        Ok(self
            .inner
            .get_pane_state()
            .map(|s| crate::python_bindings::types::PyPaneState::from(&s)))
    }

    /// Clear pane state
    fn clear_pane_state(&mut self) -> PyResult<()> {
        self.inner.clear_pane_state();
        Ok(())
    }

    /// Create window layout (static method)
    #[staticmethod]
    fn create_window_layout(
        id: String,
        name: String,
        direction: &str,
        panes: Vec<String>,
        sizes: Vec<u8>,
        active_pane: usize,
    ) -> PyResult<crate::python_bindings::types::PyWindowLayout> {
        use crate::terminal::{LayoutDirection, Terminal};

        let dir = match direction.to_lowercase().as_str() {
            "horizontal" => LayoutDirection::Horizontal,
            "vertical" => LayoutDirection::Vertical,
            _ => {
                return Err(PyValueError::new_err(
                    "Invalid direction (use 'horizontal' or 'vertical')",
                ))
            }
        };

        let layout = Terminal::create_window_layout(id, name, dir, panes, sizes, active_pane);

        Ok(crate::python_bindings::types::PyWindowLayout::from(&layout))
    }

    /// Create session state (static method)
    #[staticmethod]
    fn create_session_state(
        id: String,
        name: String,
        panes: Vec<crate::python_bindings::types::PyPaneState>,
        layouts: Vec<crate::python_bindings::types::PyWindowLayout>,
        active_layout: usize,
    ) -> PyResult<crate::python_bindings::types::PySessionState> {
        use crate::terminal::{LayoutDirection, PaneState, Terminal, WindowLayout};

        // Convert Python panes to Rust panes
        let rust_panes: Vec<PaneState> = panes
            .iter()
            .map(|p| PaneState {
                id: p.id.clone(),
                title: p.title.clone(),
                size: p.size,
                position: p.position,
                cwd: p.cwd.clone(),
                env: std::collections::HashMap::new(),
                content: p.content.clone(),
                cursor: p.cursor,
                alt_screen: p.alt_screen,
                scroll_offset: p.scroll_offset,
                created_at: p.created_at,
                last_activity: p.last_activity,
            })
            .collect();

        // Convert Python layouts to Rust layouts
        let rust_layouts: Vec<WindowLayout> = layouts
            .iter()
            .map(|l| {
                let direction = match l.direction.as_str() {
                    "horizontal" => LayoutDirection::Horizontal,
                    _ => LayoutDirection::Vertical,
                };
                WindowLayout {
                    id: l.id.clone(),
                    name: l.name.clone(),
                    direction,
                    panes: l.panes.clone(),
                    sizes: l.sizes.clone(),
                    active_pane: l.active_pane,
                }
            })
            .collect();

        let session = Terminal::create_session_state(
            id,
            name,
            rust_panes,
            rust_layouts,
            active_layout,
            std::collections::HashMap::new(),
        );

        Ok(crate::python_bindings::types::PySessionState::from(
            &session,
        ))
    }

    /// Serialize session to JSON (static method)
    #[staticmethod]
    fn serialize_session(
        session: &crate::python_bindings::types::PySessionState,
    ) -> PyResult<String> {
        use crate::terminal::{LayoutDirection, PaneState, SessionState, Terminal, WindowLayout};

        // Convert to Rust types
        let rust_panes: Vec<PaneState> = session
            .panes
            .iter()
            .map(|p| PaneState {
                id: p.id.clone(),
                title: p.title.clone(),
                size: p.size,
                position: p.position,
                cwd: p.cwd.clone(),
                env: std::collections::HashMap::new(),
                content: p.content.clone(),
                cursor: p.cursor,
                alt_screen: p.alt_screen,
                scroll_offset: p.scroll_offset,
                created_at: p.created_at,
                last_activity: p.last_activity,
            })
            .collect();

        let rust_layouts: Vec<WindowLayout> = session
            .layouts
            .iter()
            .map(|l| {
                let direction = match l.direction.as_str() {
                    "horizontal" => LayoutDirection::Horizontal,
                    _ => LayoutDirection::Vertical,
                };
                WindowLayout {
                    id: l.id.clone(),
                    name: l.name.clone(),
                    direction,
                    panes: l.panes.clone(),
                    sizes: l.sizes.clone(),
                    active_pane: l.active_pane,
                }
            })
            .collect();

        let rust_session = SessionState {
            id: session.id.clone(),
            name: session.name.clone(),
            panes: rust_panes,
            layouts: rust_layouts,
            active_layout: session.active_layout,
            metadata: std::collections::HashMap::new(),
            created_at: session.created_at,
            last_saved: session.last_saved,
        };

        Terminal::serialize_session(&rust_session).map_err(PyValueError::new_err)
    }

    /// Deserialize session from JSON (static method)
    #[staticmethod]
    fn deserialize_session(json: &str) -> PyResult<crate::python_bindings::types::PySessionState> {
        use crate::terminal::Terminal;

        let session = Terminal::deserialize_session(json).map_err(PyValueError::new_err)?;

        Ok(crate::python_bindings::types::PySessionState::from(
            &session,
        ))
    }
}
