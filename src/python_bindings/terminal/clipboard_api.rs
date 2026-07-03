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

    /// Add content to the clipboard history for a slot
    ///
    /// Content larger than 10 MB is truncated to prevent excessive memory
    /// usage. History for each slot is capped; the oldest entry is dropped
    /// once the cap is exceeded.
    ///
    /// Args:
    ///     slot: Clipboard slot name — one of "primary", "clipboard",
    ///         "selection", or "custom0".."custom9" (case-insensitive)
    ///     content: Text content to store
    ///     label: Optional description for this entry (default: None)
    ///
    /// Raises:
    ///     ValueError: If `slot` is not a recognized slot name
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.add_to_clipboard_history("clipboard", "hello", label="greeting")
    ///     ```
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
    ///
    /// Args:
    ///     slot: Clipboard slot name — one of "primary", "clipboard",
    ///         "selection", or "custom0".."custom9" (case-insensitive)
    ///
    /// Returns:
    ///     list[ClipboardEntry]: Entries oldest-first, each with `content`,
    ///     `timestamp` (microseconds), and `label`. Empty list if the slot
    ///     has never had content added.
    ///
    /// Raises:
    ///     ValueError: If `slot` is not a recognized slot name
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.add_to_clipboard_history("clipboard", "hello")
    ///     history = term.get_clipboard_history("clipboard")
    ///     ```
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

    /// Get the most recent clipboard entry for a slot
    ///
    /// Args:
    ///     slot: Clipboard slot name — one of "primary", "clipboard",
    ///         "selection", or "custom0".."custom9" (case-insensitive)
    ///
    /// Returns:
    ///     ClipboardEntry | None: The newest entry for the slot, or None if
    ///     the slot has no history
    ///
    /// Raises:
    ///     ValueError: If `slot` is not a recognized slot name
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.add_to_clipboard_history("clipboard", "hello")
    ///     entry = term.get_latest_clipboard("clipboard")
    ///     ```
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
    ///
    /// Args:
    ///     slot: Clipboard slot name — one of "primary", "clipboard",
    ///         "selection", or "custom0".."custom9" (case-insensitive)
    ///
    /// Raises:
    ///     ValueError: If `slot` is not a recognized slot name
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

    /// Set clipboard content for a slot, recording it in that slot's history
    ///
    /// Args:
    ///     content: Text content to store
    ///     slot: Clipboard slot name — one of "primary", "clipboard",
    ///         "selection", or "custom0".."custom9" (case-insensitive);
    ///         defaults to "clipboard" if not given (default: None)
    ///
    /// Raises:
    ///     ValueError: If `slot` is given but not a recognized slot name
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.set_clipboard_with_slot("hello", slot="primary")
    ///     ```
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

    /// Get the most recent clipboard content for a slot
    ///
    /// Args:
    ///     slot: Clipboard slot name — one of "primary", "clipboard",
    ///         "selection", or "custom0".."custom9" (case-insensitive);
    ///         defaults to "clipboard" if not given (default: None)
    ///
    /// Returns:
    ///     str | None: The latest content stored in the slot, or None if the
    ///     slot has no history
    ///
    /// Raises:
    ///     ValueError: If `slot` is given but not a recognized slot name
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.set_clipboard_with_slot("hello", slot="primary")
    ///     content = term.get_clipboard_from_slot(slot="primary")
    ///     ```
    #[pyo3(signature = (slot=None))]
    fn get_clipboard_from_slot(&self, slot: Option<String>) -> PyResult<Option<String>> {
        let clipboard_slot = slot
            .as_ref()
            .map(|s| super::parse_clipboard_slot(s))
            .transpose()?
            .unwrap_or(crate::terminal::ClipboardSlot::Clipboard);
        Ok(self.inner.get_clipboard_from_slot(clipboard_slot))
    }

    /// Search clipboard history for entries containing a substring
    ///
    /// Args:
    ///     query: Substring to search for (case-sensitive, plain substring match)
    ///     slot: Clipboard slot name to restrict the search to — one of
    ///         "primary", "clipboard", "selection", or "custom0".."custom9"
    ///         (case-insensitive); if None, searches all slots (default: None)
    ///
    /// Returns:
    ///     list[ClipboardEntry]: Matching entries across the searched slot(s),
    ///     each with `content`, `timestamp` (microseconds), and `label`
    ///
    /// Raises:
    ///     ValueError: If `slot` is given but not a recognized slot name
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.add_to_clipboard_history("clipboard", "hello world")
    ///     matches = term.search_clipboard_history("world")
    ///     ```
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

    /// Record an OSC 52 clipboard sync event for diagnostics/history
    ///
    /// The event is content-sanitized and truncated to
    /// `get_max_clipboard_event_bytes()` before storage, appended to the
    /// event log (`get_clipboard_sync_events()`), and — for "set" operations
    /// with content — also appended to that target's history
    /// (`get_clipboard_sync_history()`).
    ///
    /// Args:
    ///     target: Clipboard target — one of "clipboard", "primary",
    ///         "secondary", "cutbuffer0" (case-insensitive)
    ///     operation: Operation type — one of "set", "query", "clear"
    ///         (case-insensitive)
    ///     content: Content associated with the event (typically only present
    ///         for "set" operations)
    ///     is_remote: Whether this event originated from a remote session
    ///         (e.g. over SSH); when true, `content` is attributed to the
    ///         session ID set via `set_remote_session_id()`
    ///
    /// Raises:
    ///     ValueError: If `target` or `operation` is not one of the supported
    ///         values above
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.record_clipboard_sync("clipboard", "set", "hello", False)
    ///     ```
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

    /// Get all recorded OSC 52 clipboard sync events
    ///
    /// Returns:
    ///     list[ClipboardSyncEvent]: Events oldest-first (capped at
    ///     `get_max_clipboard_sync_events()`), each with `target`,
    ///     `operation`, `content`, `is_write`, `timestamp` (milliseconds),
    ///     and `is_remote`
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.record_clipboard_sync("clipboard", "set", "hello", False)
    ///     events = term.get_clipboard_sync_events()
    ///     ```
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

    /// Get clipboard sync history (content set via OSC 52) for a target
    ///
    /// Args:
    ///     target: Clipboard target — one of "clipboard", "primary",
    ///         "secondary", "cutbuffer0" (case-insensitive)
    ///
    /// Returns:
    ///     list[ClipboardHistoryEntry]: Entries oldest-first (capped at
    ///     `set_max_clipboard_sync_history()`), each with `target`, `content`,
    ///     `timestamp` (milliseconds), and `source` (remote session ID, if
    ///     any). Always wrapped in an `Optional` for API compatibility, but
    ///     currently never returns None — an empty list is returned if the
    ///     target has no history.
    ///
    /// Raises:
    ///     ValueError: If `target` is not one of the supported values above
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.record_clipboard_sync("clipboard", "set", "hello", False)
    ///     history = term.get_clipboard_sync_history("clipboard")
    ///     ```
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
