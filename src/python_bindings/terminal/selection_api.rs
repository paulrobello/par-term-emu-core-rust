//! Selection API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python
//! API or behavior change; these methods remain on the same `Terminal` Python
//! class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Selection Management ===

    /// Set the current selection
    ///
    /// Args:
    ///     start: Start position (col, row) tuple
    ///     end: End position (col, row) tuple
    ///     mode: Selection mode: "character", "line", or "block"
    fn set_selection(
        &mut self,
        start: (usize, usize),
        end: (usize, usize),
        mode: &str,
    ) -> PyResult<()> {
        use crate::terminal::SelectionMode;
        let sel_mode = match mode {
            "character" => SelectionMode::Character,
            "line" => SelectionMode::Line,
            "block" => SelectionMode::Block,
            _ => return Err(PyValueError::new_err("Invalid selection mode")),
        };
        self.inner.set_selection(start, end, sel_mode);
        Ok(())
    }

    /// Get the current selection
    ///
    /// Returns:
    ///     Selection object or None if no selection
    fn get_selection(&self) -> PyResult<Option<crate::python_bindings::types::PySelection>> {
        if let Some(sel) = self.inner.get_selection() {
            let mode_str = match sel.mode {
                crate::terminal::SelectionMode::Character => "character",
                crate::terminal::SelectionMode::Line => "line",
                crate::terminal::SelectionMode::Block => "block",
            };
            Ok(Some(crate::python_bindings::types::PySelection {
                start: sel.start,
                end: sel.end,
                mode: mode_str.to_string(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Get the text content of the current selection
    ///
    /// Returns:
    ///     Selected text as string, or None if no selection
    fn get_selected_text(&self) -> PyResult<Option<String>> {
        Ok(self.inner.get_selected_text())
    }

    /// Select the word at the given position
    ///
    /// Args:
    ///     col: Column index
    ///     row: Row index
    fn select_word_at(&mut self, col: usize, row: usize) -> PyResult<()> {
        self.inner.select_word_at(col, row);
        Ok(())
    }

    /// Select the entire line at the given row
    ///
    /// Args:
    ///     row: Row index
    fn select_line(&mut self, row: usize) -> PyResult<()> {
        self.inner.select_line(row);
        Ok(())
    }

    /// Clear the current selection
    fn clear_selection(&mut self) -> PyResult<()> {
        self.inner.clear_selection();
        Ok(())
    }
}
