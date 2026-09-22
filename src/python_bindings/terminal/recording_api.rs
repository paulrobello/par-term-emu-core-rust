//! Recording API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API
//! or behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 24: Terminal Replay/Recording ===
    // start_recording, stop_recording, record_output, record_input, record_resize,
    // record_marker, get_recording_session, is_recording:
    //   provided by impl_terminal_recording! (ARC-003/QA-001)

    // export_asciicast: provided by impl_terminal_exports! (ARC-007)

    // export_asciicast_v3: provided by impl_terminal_exports! (ARC-007)

    // export_json: provided by impl_terminal_exports! (ARC-007)
}
