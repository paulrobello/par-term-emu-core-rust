//! Scrollback API methods for `PyTerminal` (ARC-002: split out of the monolithic
//! `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API or
//! behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use std::collections::HashMap;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // ========== Buffer Statistics ==========

    /// Get terminal statistics
    ///
    /// Returns:
    ///     Dictionary with statistics: cols, rows, scrollback_lines, total_cells,
    ///     non_whitespace_lines, graphics_count, estimated_memory_bytes
    fn get_stats(&self) -> PyResult<HashMap<String, usize>> {
        let stats = self.inner.get_stats();
        let mut result = HashMap::new();
        result.insert("cols".to_string(), stats.cols);
        result.insert("rows".to_string(), stats.rows);
        result.insert("scrollback_lines".to_string(), stats.scrollback_lines);
        result.insert("total_cells".to_string(), stats.total_cells);
        result.insert(
            "non_whitespace_lines".to_string(),
            stats.non_whitespace_lines,
        );
        result.insert("graphics_count".to_string(), stats.graphics_count);
        result.insert(
            "estimated_memory_bytes".to_string(),
            stats.estimated_memory_bytes,
        );
        result.insert("hyperlink_count".to_string(), stats.hyperlink_count);
        result.insert(
            "hyperlink_memory_bytes".to_string(),
            stats.hyperlink_memory_bytes,
        );
        result.insert("color_stack_depth".to_string(), stats.color_stack_depth);
        result.insert("title_stack_depth".to_string(), stats.title_stack_depth);
        result.insert(
            "keyboard_stack_depth".to_string(),
            stats.keyboard_stack_depth,
        );
        result.insert(
            "response_buffer_size".to_string(),
            stats.response_buffer_size,
        );
        result.insert("dirty_row_count".to_string(), stats.dirty_row_count);
        result.insert("pending_bell_events".to_string(), stats.pending_bell_events);
        result.insert(
            "pending_terminal_events".to_string(),
            stats.pending_terminal_events,
        );
        Ok(result)
    }

    // count_non_whitespace_lines: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Get scrollback usage
    ///
    /// Returns:
    ///     Tuple of (used_lines, max_capacity)
    fn get_scrollback_usage(&self) -> PyResult<(usize, usize)> {
        Ok((
            self.inner.get_scrollback_usage(),
            self.inner.grid().max_scrollback(),
        ))
    }

    // === Scrollback Operations ===

    /// Export scrollback to various formats
    ///
    /// Args:
    ///     format: Export format: "plain", "html", or "ansi"
    ///     max_lines: Maximum number of scrollback lines to export (None = all)
    ///
    /// Returns:
    ///     Exported content as string
    #[pyo3(signature = (format="plain", max_lines=None))]
    fn export_scrollback(&self, format: &str, max_lines: Option<usize>) -> PyResult<String> {
        use crate::terminal::ExportFormat;
        let export_format = match format {
            "plain" => ExportFormat::Plain,
            "html" => ExportFormat::Html,
            "ansi" => ExportFormat::Ansi,
            _ => return Err(PyValueError::new_err("Invalid export format")),
        };
        Ok(self.inner.export_scrollback(export_format, max_lines))
    }

    /// Get scrollback statistics
    ///
    /// Returns:
    ///     ScrollbackStats object with total lines, memory usage, and wrap status
    fn scrollback_stats(&self) -> PyResult<crate::python_bindings::types::PyScrollbackStats> {
        let stats = self.inner.scrollback_stats();
        Ok(crate::python_bindings::types::PyScrollbackStats {
            total_lines: stats.total_lines,
            memory_bytes: stats.memory_bytes,
            has_wrapped: stats.has_wrapped,
        })
    }
}
