//! Scrollback API methods for `PyTerminal` (ARC-002: split out of the monolithic
//! `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API or
//! behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // ========== Buffer Statistics ==========

    // get_stats: provided by impl_terminal_exports! (ARC-007)

    // count_non_whitespace_lines: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // get_scrollback_usage: provided by impl_terminal_exports! (ARC-007)

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
