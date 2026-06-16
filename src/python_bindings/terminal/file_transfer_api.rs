//! File Transfer API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python
//! API or behavior change; these methods remain on the same `Terminal` Python
//! class.

use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // ========== File Transfer API ==========

    /// Get all active (in-progress) file transfers
    ///
    /// Returns a list of dictionaries, each describing an active transfer with keys:
    /// - id (int): Unique transfer identifier
    /// - direction (str): "download" or "upload"
    /// - filename (str | None): File name, or None if empty
    /// - status (str): "pending", "in_progress", "completed", "failed", or "cancelled"
    /// - bytes_transferred (int): Number of bytes transferred so far
    /// - total_bytes (int | None): Total expected bytes, or None if unknown
    /// - started_at (int): Unix milliseconds when the transfer started
    /// - completed_at (int | None): Unix milliseconds when the transfer completed, or None
    ///
    /// Returns:
    ///     List of transfer dictionaries
    ///
    /// Example:
    ///     ```python
    ///     transfers = term.get_active_transfers()
    ///     for t in transfers:
    ///         print(f"Transfer {t['id']}: {t['filename']} ({t['status']})")
    ///     ```
    #[pyo3(text_signature = "($self)")]
    fn get_active_transfers(&self) -> PyResult<Vec<pyo3::Py<pyo3::types::PyDict>>> {
        let transfers = self.inner.get_active_transfers();
        Python::attach(|py| {
            let mut result = Vec::with_capacity(transfers.len());
            for transfer in &transfers {
                result.push(super::transfer_to_py_dict(py, transfer, false)?);
            }
            Ok(result)
        })
    }

    /// Get all completed file transfers (includes failed and cancelled)
    ///
    /// Returns a list of dictionaries describing completed transfers.
    /// See `get_active_transfers` for dictionary key descriptions.
    ///
    /// Returns:
    ///     List of transfer dictionaries
    ///
    /// Example:
    ///     ```python
    ///     completed = term.get_completed_transfers()
    ///     for t in completed:
    ///         print(f"Transfer {t['id']}: {t['status']}")
    ///     ```
    #[pyo3(text_signature = "($self)")]
    fn get_completed_transfers(&self) -> PyResult<Vec<pyo3::Py<pyo3::types::PyDict>>> {
        let transfers = self.inner.get_completed_transfers();
        Python::attach(|py| {
            let mut result = Vec::with_capacity(transfers.len());
            for transfer in &transfers {
                result.push(super::transfer_to_py_dict(py, transfer, false)?);
            }
            Ok(result)
        })
    }

    /// Get a specific active transfer by ID
    ///
    /// Args:
    ///     transfer_id: The unique transfer identifier
    ///
    /// Returns:
    ///     Transfer dictionary if found, None otherwise
    ///
    /// Example:
    ///     ```python
    ///     transfer = term.get_transfer(1)
    ///     if transfer:
    ///         print(f"Transfer {transfer['id']}: {transfer['status']}")
    ///     ```
    #[pyo3(text_signature = "($self, transfer_id)")]
    fn get_transfer(&self, transfer_id: u64) -> PyResult<Option<pyo3::Py<pyo3::types::PyDict>>> {
        match self.inner.get_transfer(transfer_id) {
            Some(transfer) => {
                Python::attach(|py| Ok(Some(super::transfer_to_py_dict(py, &transfer, false)?)))
            }
            None => Ok(None),
        }
    }

    /// Take a completed transfer by ID, removing it from the completed buffer
    ///
    /// Unlike `get_transfer`, this removes the transfer from the completed buffer
    /// and includes the file data in the returned dictionary under the "data" key.
    ///
    /// Args:
    ///     transfer_id: The unique transfer identifier
    ///
    /// Returns:
    ///     Transfer dictionary with "data" key (bytes) if found, None otherwise
    ///
    /// Example:
    ///     ```python
    ///     transfer = term.take_completed_transfer(1)
    ///     if transfer:
    ///         data = transfer['data']
    ///         print(f"Got {len(data)} bytes for {transfer['filename']}")
    ///     ```
    #[pyo3(text_signature = "($self, transfer_id)")]
    fn take_completed_transfer(
        &mut self,
        transfer_id: u64,
    ) -> PyResult<Option<pyo3::Py<pyo3::types::PyDict>>> {
        match self.inner.take_completed_transfer(transfer_id) {
            Some(transfer) => {
                Python::attach(|py| Ok(Some(super::transfer_to_py_dict(py, &transfer, true)?)))
            }
            None => Ok(None),
        }
    }

    /// Cancel an active file transfer
    ///
    /// Args:
    ///     transfer_id: The unique transfer identifier
    ///
    /// Returns:
    ///     True if the transfer was found and cancelled, False otherwise
    ///
    /// Example:
    ///     ```python
    ///     if term.cancel_file_transfer(1):
    ///         print("Transfer cancelled")
    ///     ```
    #[pyo3(text_signature = "($self, transfer_id)")]
    fn cancel_file_transfer(&mut self, transfer_id: u64) -> PyResult<bool> {
        Ok(self.inner.cancel_file_transfer(transfer_id))
    }

    /// Send upload data in response to an UploadRequested event
    ///
    /// Writes the iTerm2 upload response protocol to the response buffer.
    /// Call this after the user selects a file in response to an
    /// "upload_requested" terminal event.
    ///
    /// Args:
    ///     data: Raw file data bytes to upload
    ///
    /// Example:
    ///     ```python
    ///     with open("file.txt", "rb") as f:
    ///         term.send_upload_data(f.read())
    ///     ```
    #[pyo3(text_signature = "($self, data)")]
    fn send_upload_data(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner.send_upload_data(data);
        Ok(())
    }

    /// Cancel an upload request
    ///
    /// Writes a Ctrl-C to the response buffer to signal cancellation of
    /// the upload request to the remote application.
    ///
    /// Example:
    ///     ```python
    ///     term.cancel_upload()
    ///     ```
    #[pyo3(text_signature = "($self)")]
    fn cancel_upload(&mut self) -> PyResult<()> {
        self.inner.cancel_upload();
        Ok(())
    }

    /// Set the maximum allowed file transfer size in bytes
    ///
    /// Transfers exceeding this limit will be automatically failed.
    ///
    /// Args:
    ///     max_bytes: Maximum transfer size in bytes
    ///
    /// Example:
    ///     ```python
    ///     term.set_max_transfer_size(100 * 1024 * 1024)  # 100 MB
    ///     ```
    #[pyo3(text_signature = "($self, max_bytes)")]
    fn set_max_transfer_size(&mut self, max_bytes: usize) -> PyResult<()> {
        self.inner.set_max_transfer_size(max_bytes);
        Ok(())
    }

    /// Get the current maximum allowed file transfer size in bytes
    ///
    /// Returns:
    ///     Maximum transfer size in bytes (default: 50 MB)
    ///
    /// Example:
    ///     ```python
    ///     max_size = term.get_max_transfer_size()
    ///     print(f"Max transfer size: {max_size} bytes")
    ///     ```
    #[pyo3(text_signature = "($self)")]
    fn get_max_transfer_size(&self) -> PyResult<usize> {
        Ok(self.inner.get_max_transfer_size())
    }
}
