//! File Transfer API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python
//! API or behavior change; these methods remain on the same `Terminal` Python
//! class.

use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // ========== File Transfer API ==========
    // get_active_transfers, get_completed_transfers, get_transfer,
    // take_completed_transfer, cancel_file_transfer, send_upload_data,
    // cancel_upload, set_max_transfer_size, get_max_transfer_size:
    //   provided by impl_terminal_file_transfer! (ARC-003/QA-001)
}
