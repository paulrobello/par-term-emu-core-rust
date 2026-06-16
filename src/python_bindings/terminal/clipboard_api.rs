//! Clipboard integration and OSC 52 sync API methods for `PyTerminal`
//! (ARC-002: split out of the monolithic `#[pymethods]` block in `mod.rs`). Pure
//! relocation — no Python API or behavior change; these methods remain on the same
//! `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 10: Clipboard Integration ===

    /// Add content to clipboard history
    #[pyo3(signature = (slot, content, label=None))]
    fn add_to_clipboard_history(
        &mut self,
        slot: &str,
        content: String,
        label: Option<String>,
    ) -> PyResult<()> {
        let clipboard_slot = super::parse_clipboard_slot(slot)?;
        self.inner
            .add_to_clipboard_history(clipboard_slot, content, label);
        Ok(())
    }

    /// Get clipboard history for a slot
    fn get_clipboard_history(
        &self,
        slot: &str,
    ) -> PyResult<Vec<crate::python_bindings::types::PyClipboardEntry>> {
        let clipboard_slot = super::parse_clipboard_slot(slot)?;
        let history = self.inner.get_clipboard_history(clipboard_slot);
        Ok(history
            .iter()
            .map(|e| crate::python_bindings::types::PyClipboardEntry {
                content: e.content.clone(),
                timestamp: e.timestamp,
                label: e.label.clone(),
            })
            .collect())
    }

    /// Get the most recent clipboard entry
    fn get_latest_clipboard(
        &self,
        slot: &str,
    ) -> PyResult<Option<crate::python_bindings::types::PyClipboardEntry>> {
        let clipboard_slot = super::parse_clipboard_slot(slot)?;
        if let Some(entry) = self.inner.get_latest_clipboard(clipboard_slot) {
            Ok(Some(crate::python_bindings::types::PyClipboardEntry {
                content: entry.content,
                timestamp: entry.timestamp,
                label: entry.label,
            }))
        } else {
            Ok(None)
        }
    }

    /// Clear clipboard history for a slot
    fn clear_clipboard_history(&mut self, slot: &str) -> PyResult<()> {
        let clipboard_slot = super::parse_clipboard_slot(slot)?;
        self.inner.clear_clipboard_history(clipboard_slot);
        Ok(())
    }

    /// Clear all clipboard history
    fn clear_all_clipboard_history(&mut self) -> PyResult<()> {
        self.inner.clear_all_clipboard_history();
        Ok(())
    }

    /// Set clipboard content with slot
    #[pyo3(signature = (content, slot=None))]
    fn set_clipboard_with_slot(&mut self, content: String, slot: Option<String>) -> PyResult<()> {
        let clipboard_slot = slot
            .as_ref()
            .map(|s| super::parse_clipboard_slot(s))
            .transpose()?
            .unwrap_or(crate::terminal::ClipboardSlot::Clipboard);
        self.inner.set_clipboard_with_slot(content, clipboard_slot);
        Ok(())
    }

    /// Get clipboard content from slot
    #[pyo3(signature = (slot=None))]
    fn get_clipboard_from_slot(&self, slot: Option<String>) -> PyResult<Option<String>> {
        let clipboard_slot = slot
            .as_ref()
            .map(|s| super::parse_clipboard_slot(s))
            .transpose()?
            .unwrap_or(crate::terminal::ClipboardSlot::Clipboard);
        Ok(self.inner.get_clipboard_from_slot(clipboard_slot))
    }

    /// Search clipboard history
    #[pyo3(signature = (query, slot=None))]
    fn search_clipboard_history(
        &self,
        query: &str,
        slot: Option<String>,
    ) -> PyResult<Vec<crate::python_bindings::types::PyClipboardEntry>> {
        let clipboard_slot = slot
            .as_ref()
            .map(|s| super::parse_clipboard_slot(s))
            .transpose()?;
        let results = self.inner.search_clipboard_history(query, clipboard_slot);
        Ok(results
            .iter()
            .map(|e| crate::python_bindings::types::PyClipboardEntry {
                content: e.content.clone(),
                timestamp: e.timestamp,
                label: e.label.clone(),
            })
            .collect())
    }

    // === Feature 30: OSC 52 Clipboard Sync ===

    /// Record a clipboard sync event
    ///
    /// Args:
    ///     target: Clipboard target ("clipboard", "primary", "secondary", "cutbuffer0")
    ///     operation: Operation type ("set", "query", "clear")
    ///     content: Optional content (for set operations)
    ///     is_remote: Whether this is from a remote session
    fn record_clipboard_sync(
        &mut self,
        target: &str,
        operation: &str,
        content: Option<String>,
        is_remote: bool,
    ) -> PyResult<()> {
        use crate::terminal::{ClipboardOperation, ClipboardTarget};

        let target = match target.to_lowercase().as_str() {
            "clipboard" => ClipboardTarget::Clipboard,
            "primary" => ClipboardTarget::Primary,
            "secondary" => ClipboardTarget::Secondary,
            "cutbuffer0" => ClipboardTarget::CutBuffer0,
            _ => return Err(PyValueError::new_err("Invalid clipboard target")),
        };

        let operation = match operation.to_lowercase().as_str() {
            "set" => ClipboardOperation::Set,
            "query" => ClipboardOperation::Query,
            "clear" => ClipboardOperation::Clear,
            _ => return Err(PyValueError::new_err("Invalid clipboard operation")),
        };

        self.inner
            .record_clipboard_sync(target, operation, content, is_remote);
        Ok(())
    }

    /// Get clipboard sync events
    ///
    /// Returns:
    ///     List of PyClipboardSyncEvent
    fn get_clipboard_sync_events(
        &self,
    ) -> PyResult<Vec<crate::python_bindings::types::PyClipboardSyncEvent>> {
        Ok(self
            .inner
            .get_clipboard_sync_events()
            .iter()
            .map(crate::python_bindings::types::PyClipboardSyncEvent::from)
            .collect())
    }

    /// Get clipboard sync history for a target
    ///
    /// Args:
    ///     target: Clipboard target ("clipboard", "primary", "secondary", "cutbuffer0")
    ///
    /// Returns:
    ///     List of PyClipboardHistoryEntry or None
    fn get_clipboard_sync_history(
        &self,
        target: &str,
    ) -> PyResult<Option<Vec<crate::python_bindings::types::PyClipboardHistoryEntry>>> {
        use crate::terminal::ClipboardTarget;

        let target = match target.to_lowercase().as_str() {
            "clipboard" => ClipboardTarget::Clipboard,
            "primary" => ClipboardTarget::Primary,
            "secondary" => ClipboardTarget::Secondary,
            "cutbuffer0" => ClipboardTarget::CutBuffer0,
            _ => return Err(PyValueError::new_err("Invalid clipboard target")),
        };

        let entries = self.inner.get_clipboard_sync_history(target);
        Ok(Some(
            entries
                .iter()
                .map(crate::python_bindings::types::PyClipboardHistoryEntry::from)
                .collect(),
        ))
    }

    /// Clear clipboard sync events
    fn clear_clipboard_sync_events(&mut self) -> PyResult<()> {
        self.inner.clear_clipboard_sync_events();
        Ok(())
    }

    /// Set maximum clipboard sync events retained (0 disables buffering)
    fn set_max_clipboard_sync_events(&mut self, max: usize) -> PyResult<()> {
        self.inner.set_max_clipboard_sync_events(max);
        Ok(())
    }

    /// Get maximum clipboard sync events retained
    fn get_max_clipboard_sync_events(&self) -> PyResult<usize> {
        Ok(self.inner.max_clipboard_sync_events())
    }

    /// Set maximum bytes cached per clipboard sync event (0 clears content)
    fn set_max_clipboard_event_bytes(&mut self, max_bytes: usize) -> PyResult<()> {
        self.inner.set_max_clipboard_event_bytes(max_bytes);
        Ok(())
    }

    /// Get maximum bytes cached per clipboard sync event
    fn get_max_clipboard_event_bytes(&self) -> PyResult<usize> {
        Ok(self.inner.max_clipboard_event_bytes())
    }

    /// Set remote session ID
    ///
    /// Args:
    ///     session_id: Optional session identifier
    fn set_remote_session_id(&mut self, session_id: Option<String>) -> PyResult<()> {
        self.inner.set_remote_session_id(session_id);
        Ok(())
    }

    /// Get remote session ID
    ///
    /// Returns:
    ///     Optional session identifier
    fn remote_session_id(&self) -> PyResult<Option<String>> {
        Ok(self.inner.remote_session_id().map(String::from))
    }

    /// Set maximum clipboard sync history
    ///
    /// Args:
    ///     max: Maximum number of entries per target
    fn set_max_clipboard_sync_history(&mut self, max: usize) -> PyResult<()> {
        self.inner.set_max_clipboard_sync_history(max);
        Ok(())
    }
}
