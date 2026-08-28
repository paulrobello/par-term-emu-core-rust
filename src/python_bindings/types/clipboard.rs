//! Clipboard entry, sync-event, and history types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Clipboard entry
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ClipboardEntry", from_py_object)]
#[derive(Clone)]
pub struct PyClipboardEntry {
    /// Clipboard text content
    pub content: String,
    /// Unix epoch milliseconds when captured
    pub timestamp: u64,
    /// Optional source label
    pub label: Option<String>,
}

#[pymethods]
impl PyClipboardEntry {
    fn __repr__(&self) -> String {
        format!(
            "ClipboardEntry(len={}, timestamp={})",
            self.content.len(),
            self.timestamp
        )
    }
}

/// Clipboard sync event
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ClipboardSyncEvent", from_py_object)]
#[derive(Clone)]
pub struct PyClipboardSyncEvent {
    /// Clipboard target (e.g. "system", "terminal")
    pub target: String,
    /// Sync operation kind (e.g. "set", "request")
    pub operation: String,
    /// Clipboard content (None for clears/requests)
    pub content: Option<String>,
    /// True for writes, False for reads
    pub is_write: bool,
    /// Unix epoch milliseconds when the event occurred
    pub timestamp: u64,
    /// True if the sync originated from a remote host
    pub is_remote: bool,
}

#[pymethods]
impl PyClipboardSyncEvent {
    fn __repr__(&self) -> String {
        format!(
            "ClipboardSyncEvent(target={}, operation={}, is_remote={})",
            self.target, self.operation, self.is_remote
        )
    }
}

impl From<&crate::terminal::ClipboardSyncEvent> for PyClipboardSyncEvent {
    fn from(event: &crate::terminal::ClipboardSyncEvent) -> Self {
        use crate::terminal::{ClipboardOperation, ClipboardTarget};

        let target = match event.target {
            ClipboardTarget::Clipboard => "clipboard",
            ClipboardTarget::Primary => "primary",
            ClipboardTarget::Secondary => "secondary",
            ClipboardTarget::CutBuffer0 => "cutbuffer0",
        }
        .to_string();

        let operation = match event.operation {
            ClipboardOperation::Set => "set",
            ClipboardOperation::Query => "query",
            ClipboardOperation::Clear => "clear",
        }
        .to_string();

        PyClipboardSyncEvent {
            target,
            operation,
            content: Some(event.content.clone()),
            is_write: event.is_write,
            is_remote: event.is_remote,
            timestamp: event.timestamp,
        }
    }
}

/// Clipboard history entry
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ClipboardHistoryEntry", from_py_object)]
#[derive(Clone)]
pub struct PyClipboardHistoryEntry {
    /// Clipboard target this entry came from
    pub target: String,
    /// Clipboard text content
    pub content: String,
    /// Unix epoch milliseconds when captured
    pub timestamp: u64,
    /// Optional origin description
    pub source: Option<String>,
}

#[pymethods]
impl PyClipboardHistoryEntry {
    fn __repr__(&self) -> String {
        format!(
            "ClipboardHistoryEntry(target={}, content_len={}, timestamp={})",
            self.target,
            self.content.len(),
            self.timestamp
        )
    }
}

impl From<&crate::terminal::ClipboardHistoryEntry> for PyClipboardHistoryEntry {
    fn from(entry: &crate::terminal::ClipboardHistoryEntry) -> Self {
        use crate::terminal::ClipboardTarget;

        let target = match entry.target {
            ClipboardTarget::Clipboard => "clipboard",
            ClipboardTarget::Primary => "primary",
            ClipboardTarget::Secondary => "secondary",
            ClipboardTarget::CutBuffer0 => "cutbuffer0",
        }
        .to_string();

        PyClipboardHistoryEntry {
            target,
            content: entry.content.clone(),
            timestamp: entry.timestamp,
            source: entry.source.clone(),
        }
    }
}
