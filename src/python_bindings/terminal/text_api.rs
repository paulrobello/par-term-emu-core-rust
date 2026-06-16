//! Text API methods for `PyTerminal` (ARC-002: split out of the monolithic
//! `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API or
//! behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // ========== Text Extraction Utilities ==========
    // get_word_at, get_url_at, get_line_unwrapped:
    //   provided by impl_terminal_cell_line_queries! (ARC-003/QA-001)

    // select_word, find_text, find_next, find_matching_bracket, select_semantic_region:
    //   provided by impl_terminal_search_select! (ARC-003/QA-001)

    /// Export terminal content as HTML
    ///
    /// Args:
    ///     include_styles: Whether to include full HTML document with CSS (default: True)
    ///
    /// Returns:
    ///     HTML string with terminal content and styling
    ///
    /// When include_styles is True, returns a complete HTML document.
    /// When False, returns just the styled content (useful for embedding).
    #[pyo3(signature = (include_styles = true))]
    fn export_html(&self, include_styles: bool) -> PyResult<String> {
        Ok(self.inner.export_html(include_styles))
    }

    // === Text Extraction ===

    /// Get text lines around a specific row (with context)
    ///
    /// Args:
    ///     row: Center row (0-based)
    ///     context_before: Number of lines before the row
    ///     context_after: Number of lines after the row
    ///
    /// Returns:
    ///     List of text lines
    fn get_line_context(
        &self,
        row: usize,
        context_before: usize,
        context_after: usize,
    ) -> PyResult<Vec<String>> {
        Ok(self
            .inner
            .get_line_context(row, context_before, context_after))
    }

    /// Get the paragraph at the given position
    ///
    /// A paragraph is defined as consecutive non-empty lines.
    ///
    /// Args:
    ///     row: Row index
    ///
    /// Returns:
    ///     Paragraph text as string
    fn get_paragraph_at(&self, row: usize) -> PyResult<String> {
        Ok(self.inner.get_paragraph_at(row))
    }

    // === Feature 9: Line Wrapping Utilities ===

    /// Join wrapped lines starting from a given row
    fn join_wrapped_lines(
        &self,
        start_row: usize,
    ) -> PyResult<Option<crate::python_bindings::types::PyJoinedLines>> {
        if let Some(joined) = self.inner.join_wrapped_lines(start_row) {
            Ok(Some(crate::python_bindings::types::PyJoinedLines {
                text: joined.text,
                start_row: joined.start_row,
                end_row: joined.end_row,
                lines_joined: joined.lines_joined,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get all logical lines (unwrapped)
    fn get_logical_lines(&self) -> PyResult<Vec<String>> {
        Ok(self.inner.get_logical_lines())
    }

    /// Check if a row starts a new logical line
    fn is_line_start(&self, row: usize) -> PyResult<bool> {
        Ok(self.inner.is_line_start(row))
    }
}
