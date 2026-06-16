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

    /// Get word boundaries at cursor position for smart selection
    ///
    /// Args:
    ///     col: Column position (0-indexed)
    ///     row: Row position (0-indexed)
    ///     word_chars: Optional custom word characters
    ///
    /// Returns:
    ///     ((start_col, start_row), (end_col, end_row)) or None if not on a word
    #[allow(clippy::type_complexity)]
    fn select_word(
        &mut self,
        col: usize,
        row: usize,
        word_chars: Option<&str>,
    ) -> PyResult<Option<((usize, usize), (usize, usize))>> {
        Ok(self.inner.select_word(col, row, word_chars))
    }

    // ========== Content Search ==========

    /// Find all occurrences of text in the visible screen
    ///
    /// Args:
    ///     pattern: Text to search for
    ///     case_sensitive: Whether search is case-sensitive (default: True)
    ///
    /// Returns:
    ///     List of (col, row) positions where pattern was found
    #[pyo3(signature = (pattern, case_sensitive = true))]
    fn find_text(&self, pattern: &str, case_sensitive: bool) -> PyResult<Vec<(usize, usize)>> {
        Ok(self
            .inner
            .find_text(pattern, case_sensitive)
            .into_iter()
            .map(|m| (m.col, m.row as usize))
            .collect())
    }

    /// Find next occurrence of text from given position
    ///
    /// Args:
    ///     pattern: Text to search for
    ///     from_col: Starting column position
    ///     from_row: Starting row position
    ///     case_sensitive: Whether search is case-sensitive (default: True)
    ///
    /// Returns:
    ///     (col, row) of next match, or None if not found
    #[pyo3(signature = (pattern, from_col, from_row, case_sensitive = true))]
    fn find_next(
        &self,
        pattern: &str,
        from_col: usize,
        from_row: usize,
        case_sensitive: bool,
    ) -> PyResult<Option<(usize, usize)>> {
        Ok(self
            .inner
            .find_next(pattern, from_col, from_row, case_sensitive)
            .map(|m| (m.col, m.row as usize)))
    }

    // ========== Advanced Text Selection ==========

    /// Find matching bracket/parenthesis at cursor position
    ///
    /// Supports: (), [], {}, <>
    ///
    /// Args:
    ///     col: Column position (0-indexed)
    ///     row: Row position (0-indexed)
    ///
    /// Returns:
    ///     (col, row) position of matching bracket, or None
    fn find_matching_bracket(&self, col: usize, row: usize) -> PyResult<Option<(usize, usize)>> {
        Ok(self.inner.find_matching_bracket(col, row))
    }

    /// Select text within semantic delimiters
    ///
    /// Extracts content between matching delimiters around cursor.
    /// Supports: (), [], {}, <>, "", '', ``
    ///
    /// Args:
    ///     col: Column position (0-indexed)
    ///     row: Row position (0-indexed)
    ///     delimiters: String of delimiters to check (e.g., "()[]{}\"'")
    ///
    /// Returns:
    ///     Content between delimiters, or None if not inside delimiters
    ///
    /// Example:
    ///     # Cursor inside "hello world"
    ///     text = term.select_semantic_region(10, 0, "\"")  # Returns "hello world"
    fn select_semantic_region(
        &mut self,
        col: usize,
        row: usize,
        delimiters: &str,
    ) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .select_semantic_region(col, row, Some(delimiters)))
    }

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
