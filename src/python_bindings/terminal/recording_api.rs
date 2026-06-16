//! Recording API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API
//! or behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 24: Terminal Replay/Recording ===
    // start_recording, stop_recording, record_output, record_input, record_resize,
    // record_marker, get_recording_session, is_recording:
    //   provided by impl_terminal_recording! (ARC-003/QA-001)

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
