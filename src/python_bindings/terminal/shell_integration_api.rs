//! Shell integration API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API
//! or behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 31: Shell Integration++ ===

    /// Start tracking a command execution
    ///
    /// Args:
    ///     command: Command being executed
    fn start_command_execution(&mut self, command: String) -> PyResult<()> {
        self.inner.start_command_execution(command);
        Ok(())
    }

    /// End tracking the current command execution
    ///
    /// Args:
    ///     exit_code: Exit code of the command
    fn end_command_execution(&mut self, exit_code: i32) -> PyResult<()> {
        self.inner.end_command_execution(Some(exit_code));
        Ok(())
    }

    /// Get command execution history
    ///
    /// Returns:
    ///     List of PyCommandExecution
    fn get_command_history(
        &self,
    ) -> PyResult<Vec<crate::python_bindings::types::PyCommandExecution>> {
        Ok(self
            .inner
            .get_command_history()
            .iter()
            .map(crate::python_bindings::types::PyCommandExecution::from)
            .collect())
    }

    /// Get current executing command
    ///
    /// Returns:
    ///     Optional PyCommandExecution
    fn get_current_command(
        &self,
    ) -> PyResult<Option<crate::python_bindings::types::PyCommandExecution>> {
        Ok(self
            .inner
            .get_current_command()
            .map(crate::python_bindings::types::PyCommandExecution::from))
    }

    /// Get command output text by index (0 = most recent completed command).
    ///
    /// Args:
    ///     index: Command index (0 = most recent)
    ///
    /// Returns:
    ///     Output text if available, None if index out of bounds or output evicted
    ///
    /// Example:
    ///     ```python
    ///     output = term.get_command_output(0)
    ///     if output:
    ///         print(f"Last command output: {output}")
    ///     ```
    fn get_command_output(&self, index: usize) -> PyResult<Option<String>> {
        Ok(self.inner.get_command_output(index).map(|co| co.output))
    }

    /// Get all commands with extractable output text.
    /// Commands whose output has been evicted from scrollback are excluded.
    ///
    /// Returns:
    ///     List of dicts with keys: command, cwd, exit_code, output
    ///
    /// Example:
    ///     ```python
    ///     outputs = term.get_command_outputs()
    ///     for out in outputs:
    ///         print(f"{out['command']}: {out['output']}")
    ///     ```
    fn get_command_outputs(&self) -> PyResult<Vec<pyo3::Py<pyo3::types::PyDict>>> {
        use pyo3::types::PyDict;

        let outputs = self.inner.get_command_outputs();
        Python::attach(|py| {
            let mut result = Vec::with_capacity(outputs.len());
            for out in &outputs {
                let dict = PyDict::new(py);
                dict.set_item("command", &out.command)?;
                dict.set_item("cwd", out.cwd.as_deref())?;
                dict.set_item("exit_code", out.exit_code)?;
                dict.set_item("output", &out.output)?;
                result.push(dict.into());
            }
            Ok(result)
        })
    }

    /// Record a CWD change
    ///
    /// Args:
    ///     new_cwd: New working directory
    ///     hostname: Optional hostname (None for localhost)
    ///     username: Optional username (user@host form)
    #[pyo3(signature = (new_cwd, hostname=None, username=None))]
    fn record_cwd_change(
        &mut self,
        new_cwd: String,
        hostname: Option<String>,
        username: Option<String>,
    ) -> PyResult<()> {
        use crate::terminal::CwdChange;
        self.inner.record_cwd_change(CwdChange {
            old_cwd: None,
            new_cwd,
            hostname,
            username,
            timestamp: crate::terminal::unix_millis(),
        });
        Ok(())
    }

    /// Get CWD change history
    ///
    /// Returns:
    ///     List of PyCwdChange
    fn get_cwd_changes(&self) -> PyResult<Vec<crate::python_bindings::types::PyCwdChange>> {
        Ok(self
            .inner
            .get_cwd_changes()
            .iter()
            .map(crate::python_bindings::types::PyCwdChange::from)
            .collect())
    }

    /// Get shell integration statistics
    ///
    /// Returns:
    ///     PyShellIntegrationStats
    fn get_shell_integration_stats(
        &self,
    ) -> PyResult<crate::python_bindings::types::PyShellIntegrationStats> {
        let stats = self.inner.get_shell_integration_stats();
        Ok(crate::python_bindings::types::PyShellIntegrationStats::from(&stats))
    }

    /// Clear command execution history
    fn clear_command_history(&mut self) -> PyResult<()> {
        self.inner.clear_command_history();
        Ok(())
    }

    /// Clear CWD change history
    fn clear_cwd_history(&mut self) -> PyResult<()> {
        self.inner.clear_cwd_history();
        Ok(())
    }

    /// Set maximum command history size
    ///
    /// Args:
    ///     max: Maximum number of command entries
    fn set_max_command_history(&mut self, max: usize) -> PyResult<()> {
        self.inner.set_max_command_history(max);
        Ok(())
    }

    /// Set maximum CWD history size
    ///
    /// Args:
    ///     max: Maximum number of CWD change entries
    fn set_max_cwd_history(&mut self, max: usize) -> PyResult<()> {
        self.inner.set_max_cwd_history(max);
        Ok(())
    }
}
