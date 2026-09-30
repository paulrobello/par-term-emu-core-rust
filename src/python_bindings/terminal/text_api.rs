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
    // export_html: provided by impl_terminal_exports! (ARC-003/QA-001)

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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("one\r\ntwo\r\nthree\r\nfour\r\n")
    ///     [line.rstrip() for line in term.get_line_context(2, 1, 1)]
    ///     # ['two', 'three', 'four']
    ///     ```
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("first line\r\nsecond line\r\n\r\nother\r\n")
    ///     [line.rstrip() for line in term.get_paragraph_at(0).split("\n")]
    ///     # ['first line', 'second line']
    ///     ```
    fn get_paragraph_at(&self, row: usize) -> PyResult<String> {
        Ok(self.inner.get_paragraph_at(row))
    }

    // === Feature 9: Line Wrapping Utilities ===

    /// Join wrapped lines starting from a given row
    ///
    /// Args:
    ///     start_row: Row (0-indexed) whose logical line to rejoin
    ///
    /// Returns:
    ///     JoinedLines | None: The rejoined line, or None if the row is out of range
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(10, 5)
    ///     term.process_str("abcdefghijklmno\r\n")   # wraps onto row 1
    ///     joined = term.join_wrapped_lines(0)
    ///     (joined.start_row, joined.end_row, joined.lines_joined)   # (0, 1, 2)
    ///     joined.text.rstrip()           # 'abcdefghijklmno'
    ///     term.join_wrapped_lines(99)    # None
    ///     ```
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
    ///
    /// Returns:
    ///     list[str]: Logical lines with wrapped segments joined
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(10, 5)
    ///     term.process_str("abcdefghijklmno\r\n")   # wraps onto row 1
    ///     term.get_logical_lines()[0]   # 'abcdefghijklmno'
    ///     ```
    fn get_logical_lines(&self) -> PyResult<Vec<String>> {
        Ok(self.inner.get_logical_lines())
    }

    /// Check if a row starts a new logical line
    ///
    /// Args:
    ///     row: 0-indexed row number
    ///
    /// Returns:
    ///     bool: True if the row begins a new logical line
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(10, 5)
    ///     term.process_str("abcdefghijklmno\r\n")   # wraps onto row 1
    ///     term.is_line_start(0)   # True
    ///     term.is_line_start(1)   # False (continuation of row 0)
    ///     ```
    fn is_line_start(&self, row: usize) -> PyResult<bool> {
        Ok(self.inner.is_line_start(row))
    }
}
