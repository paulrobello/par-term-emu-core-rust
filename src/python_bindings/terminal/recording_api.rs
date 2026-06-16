//! Recording API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API
//! or behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 24: Terminal Replay/Recording ===

    /// Start recording a terminal session
    ///
    /// Args:
    ///     title: Optional session title
    fn start_recording(&mut self, title: Option<String>) -> PyResult<()> {
        self.inner.start_recording(title);
        Ok(())
    }

    /// Stop recording and return the session
    ///
    /// Returns:
    ///     RecordingSession object if recording was active, None otherwise
    fn stop_recording(
        &mut self,
    ) -> PyResult<Option<crate::python_bindings::types::PyRecordingSession>> {
        Ok(self
            .inner
            .stop_recording()
            .map(crate::python_bindings::types::PyRecordingSession::from))
    }

    /// Record output data
    ///
    /// Args:
    ///     data: Output data bytes
    fn record_output(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner.record_output(data);
        Ok(())
    }

    /// Record input data
    ///
    /// Args:
    ///     data: Input data bytes
    fn record_input(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner.record_input(data);
        Ok(())
    }

    /// Record terminal resize
    ///
    /// Args:
    ///     cols: Number of columns
    ///     rows: Number of rows
    fn record_resize(&mut self, cols: usize, rows: usize) -> PyResult<()> {
        self.inner.record_resize(cols, rows);
        Ok(())
    }

    /// Add a marker/bookmark to the recording
    ///
    /// Args:
    ///     label: Marker label
    fn record_marker(&mut self, label: String) -> PyResult<()> {
        self.inner.record_marker(label);
        Ok(())
    }

    /// Get current recording session
    ///
    /// Returns:
    ///     RecordingSession object if recording is active, None otherwise
    fn get_recording_session(
        &self,
    ) -> PyResult<Option<crate::python_bindings::types::PyRecordingSession>> {
        Ok(self
            .inner
            .get_recording_session()
            .map(crate::python_bindings::types::PyRecordingSession::from))
    }

    /// Check if currently recording
    ///
    /// Returns:
    ///     True if recording is active
    fn is_recording(&self) -> PyResult<bool> {
        Ok(self.inner.is_recording())
    }

    /// Export recording to asciicast v2 format
    ///
    /// Args:
    ///     session: RecordingSession from stop_recording()
    ///
    /// Returns:
    ///     Asciicast format string
    #[pyo3(signature = (session=None))]
    fn export_asciicast(
        &self,
        session: Option<&crate::python_bindings::types::PyRecordingSession>,
        _py: Python,
    ) -> PyResult<String> {
        if let Some(session) = session {
            Ok(self.inner.export_asciicast(&session.inner))
        } else if let Some(active) = self.inner.get_recording_session() {
            Ok(self.inner.export_asciicast(active))
        } else {
            Err(PyValueError::new_err(
                "No active recording session (pass session=stop_recording())",
            ))
        }
    }

    /// Export recording to JSON format
    ///
    /// Returns:
    ///     JSON format string
    #[pyo3(signature = (session=None))]
    fn export_json(
        &self,
        session: Option<&crate::python_bindings::types::PyRecordingSession>,
        _py: Python,
    ) -> PyResult<String> {
        if let Some(session) = session {
            Ok(self.inner.export_json(&session.inner))
        } else if let Some(active) = self.inner.get_recording_session() {
            Ok(self.inner.export_json(active))
        } else {
            Err(PyValueError::new_err(
                "No active recording session (pass session=stop_recording())",
            ))
        }
    }
}
