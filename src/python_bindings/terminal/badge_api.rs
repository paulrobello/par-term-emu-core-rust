//! Badge API methods for `PyTerminal` (ARC-002: split out of the monolithic
//! `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API or
//! behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // ========== Badge Format Support (OSC 1337 SetBadgeFormat) ==========
    // badge_format, set_badge_format, clear_badge_format, evaluate_badge,
    // get_badge_session_variable, set_badge_session_variable, get_badge_session_variables:
    //   provided by impl_terminal_badge_session! (ARC-003/QA-001)

    /// Get a user variable value by name
    ///
    /// User variables are set via OSC 1337 SetUserVar sequences from
    /// shell integration scripts. They report session information like
    /// hostname and other custom key-value pairs.
    ///
    /// Args:
    ///     name: Variable name (e.g., "hostname", "currentDir")
    ///
    /// Returns:
    ///     Variable value as string, or None if not set
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     # After shell sends: OSC 1337 ; SetUserVar=hostname=<base64> ST
    ///     host = term.get_user_var("hostname")
    ///     ```
    fn get_user_var(&self, name: &str) -> PyResult<Option<String>> {
        Ok(self.inner.get_user_var(name).map(|s| s.to_string()))
    }

    /// Get all user variables as a dictionary
    ///
    /// Returns all user variables set via OSC 1337 SetUserVar sequences.
    ///
    /// Returns:
    ///     Dictionary mapping variable names to their string values
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     user_vars = term.get_user_vars()  # e.g., {"hostname": "server1"}
    ///     ```
    fn get_user_vars(&self) -> PyResult<HashMap<String, String>> {
        Ok(self.inner.get_user_vars().clone())
    }

    /// Get all semantic zones in the terminal buffer
    ///
    /// Returns a list of zone dictionaries, each containing:
    /// - zone_type: str - "prompt", "command", or "output"
    /// - abs_row_start: int - Absolute row where zone starts
    /// - abs_row_end: int - Absolute row where zone ends (inclusive)
    /// - command: str | None - Command text (for command/output zones)
    /// - exit_code: int | None - Exit code (for output zones after command finishes)
    /// - timestamp: int | None - Unix milliseconds when zone was created
    ///
    /// Returns:
    ///     List of zone dictionaries sorted by row position
    ///
    /// Example:
    ///     ```python
    ///     zones = term.get_zones()
    ///     for z in zones:
    ///         print(f"{z['zone_type']}: rows {z['abs_row_start']}-{z['abs_row_end']}")
    ///     ```
    fn get_zones(&self) -> PyResult<Vec<pyo3::Py<pyo3::types::PyDict>>> {
        use pyo3::types::PyDict;

        let zones = self.inner.get_zones();
        Python::attach(|py| {
            let mut result = Vec::with_capacity(zones.len());
            for zone in zones {
                let dict = PyDict::new(py);
                dict.set_item("zone_type", zone.zone_type.to_string())?;
                dict.set_item("abs_row_start", zone.abs_row_start)?;
                dict.set_item("abs_row_end", zone.abs_row_end)?;
                dict.set_item("command", zone.command.as_deref())?;
                dict.set_item("exit_code", zone.exit_code)?;
                dict.set_item("timestamp", zone.timestamp)?;
                result.push(dict.into());
            }
            Ok(result)
        })
    }

    /// Get the semantic zone containing the given absolute row
    ///
    /// Args:
    ///     abs_row: Absolute row number (scrollback_len + visible_row)
    ///
    /// Returns:
    ///     Zone dictionary or None if no zone contains this row
    ///
    /// Example:
    ///     ```python
    ///     zone = term.get_zone_at(term.scrollback_len() + 0)
    ///     if zone:
    ///         print(f"Row 0 is in a {zone['zone_type']} zone")
    ///     ```
    fn get_zone_at(&self, abs_row: usize) -> PyResult<Option<pyo3::Py<pyo3::types::PyDict>>> {
        use pyo3::types::PyDict;

        match self.inner.get_zone_at(abs_row) {
            Some(zone) => Python::attach(|py| {
                let dict = PyDict::new(py);
                dict.set_item("zone_type", zone.zone_type.to_string())?;
                dict.set_item("abs_row_start", zone.abs_row_start)?;
                dict.set_item("abs_row_end", zone.abs_row_end)?;
                dict.set_item("command", zone.command.as_deref())?;
                dict.set_item("exit_code", zone.exit_code)?;
                dict.set_item("timestamp", zone.timestamp)?;
                Ok(Some(dict.into()))
            }),
            None => Ok(None),
        }
    }

    /// Get the text content of the zone containing the given absolute row
    ///
    /// Extracts all text from the zone's rows, handling line wrapping and
    /// trimming trailing whitespace. Returns None if no zone contains this row.
    ///
    /// Args:
    ///     abs_row: Absolute row number (scrollback_len + visible_row)
    ///
    /// Returns:
    ///     Zone text content as a string, or None
    ///
    /// Example:
    ///     ```python
    ///     text = term.get_zone_text(some_row)
    ///     if text:
    ///         print(f"Zone content: {text}")
    ///     ```
    fn get_zone_text(&self, abs_row: usize) -> PyResult<Option<String>> {
        Ok(self.inner.get_zone_text(abs_row))
    }

    /// Get a semantic snapshot of the terminal state as a Python dict.
    ///
    /// Returns a structured representation of terminal state including
    /// content, zones, commands, and environment metadata, suitable
    /// for AI/LLM consumption.
    ///
    /// Args:
    ///     scope: Snapshot scope - "visible", "recent", or "full" (default: "visible")
    ///     max_commands: For "recent" scope, max number of commands to include (default: 10)
    ///
    /// Returns:
    ///     dict with keys: timestamp, cols, rows, title, cursor_col, cursor_row,
    ///     alt_screen_active, visible_text, scrollback_text, zones, commands,
    ///     cwd, hostname, username, cwd_history, scrollback_lines, total_zones,
    ///     total_commands
    ///
    /// Example:
    ///     >>> term = Terminal(80, 24)
    ///     >>> term.process(b"Hello")
    ///     >>> snap = term.get_semantic_snapshot(scope="visible")
    ///     >>> snap["cols"]
    ///     80
    #[pyo3(signature = (scope="visible", max_commands=10))]
    fn get_semantic_snapshot(
        &self,
        scope: &str,
        max_commands: usize,
    ) -> PyResult<pyo3::Py<pyo3::types::PyDict>> {
        use crate::terminal::semantic_snapshot::SnapshotScope;

        let snapshot_scope = match scope {
            "recent" => SnapshotScope::Recent(max_commands),
            "full" => SnapshotScope::Full,
            "visible" => SnapshotScope::Visible,
            _ => {
                return Err(PyValueError::new_err(
                    "scope must be 'visible', 'recent', or 'full'",
                ))
            }
        };

        let snapshot = self.inner.get_semantic_snapshot(snapshot_scope);
        let json = serde_json::to_string(&snapshot)
            .map_err(|e| PyRuntimeError::new_err(format!("Serialization failed: {}", e)))?;

        Python::attach(|py| {
            let json_module = py.import("json")?;
            let dict = json_module
                .call_method1("loads", (json,))?
                .cast_into::<pyo3::types::PyDict>()
                .map_err(|e| {
                    PyRuntimeError::new_err(format!("Expected dict from json.loads: {}", e))
                })?;
            Ok(dict.into())
        })
    }

    /// Get a semantic snapshot of the terminal state as a JSON string.
    ///
    /// This is more efficient than get_semantic_snapshot() when you need
    /// the data as a string (e.g., for sending to an LLM API).
    ///
    /// Args:
    ///     scope: Snapshot scope - "visible", "recent", or "full" (default: "visible")
    ///     max_commands: For "recent" scope, max number of commands to include (default: 10)
    ///
    /// Returns:
    ///     JSON string containing the semantic snapshot
    ///
    /// Example:
    ///     >>> term = Terminal(80, 24)
    ///     >>> json_str = term.get_semantic_snapshot_json(scope="full")
    #[pyo3(signature = (scope="visible", max_commands=10))]
    fn get_semantic_snapshot_json(&self, scope: &str, max_commands: usize) -> PyResult<String> {
        use crate::terminal::semantic_snapshot::SnapshotScope;

        let snapshot_scope = match scope {
            "recent" => SnapshotScope::Recent(max_commands),
            "full" => SnapshotScope::Full,
            "visible" => SnapshotScope::Visible,
            _ => {
                return Err(PyValueError::new_err(
                    "scope must be 'visible', 'recent', or 'full'",
                ))
            }
        };

        Ok(self.inner.get_semantic_snapshot_json(snapshot_scope))
    }

    /// Capture a cell-level snapshot of the terminal state for Instant Replay.
    ///
    /// Unlike `get_semantic_snapshot()` which captures text only, this captures
    /// raw Cell data including colors and attributes for pixel-perfect reconstruction.
    ///
    /// Returns:
    ///     dict: Snapshot metadata with keys:
    ///         - timestamp (int): Unix timestamp in milliseconds
    ///         - cols (int): Terminal width in columns
    ///         - rows (int): Terminal height in rows
    ///         - estimated_size_bytes (int): Approximate memory footprint in bytes
    ///
    /// Example:
    ///     >>> term = Terminal(80, 24)
    ///     >>> info = term.capture_replay_snapshot()
    ///     >>> print(f"Snapshot at {info['timestamp']}, size: {info['estimated_size_bytes']} bytes")
    fn capture_replay_snapshot(&mut self) -> PyResult<pyo3::Py<pyo3::types::PyDict>> {
        use pyo3::types::PyDict;

        let snap = self.inner.capture_snapshot();
        Python::attach(|py| {
            let dict = PyDict::new(py);
            dict.set_item("timestamp", snap.timestamp)?;
            dict.set_item("cols", snap.cols)?;
            dict.set_item("rows", snap.rows)?;
            dict.set_item("estimated_size_bytes", snap.estimated_size_bytes)?;
            Ok(dict.into())
        })
    }
}
