//! Clipboard integration and sync
//!
//! Handles clipboard history, multiple clipboard slots, and OSC 52 sync events.

/// Maximum allowed clipboard content size in bytes (10 MB)
const MAX_CLIPBOARD_CONTENT_SIZE: usize = 10_485_760;

/// Clipboard entry with history
#[derive(Debug, Clone)]
pub struct ClipboardEntry {
    /// Clipboard content
    pub content: String,
    /// Timestamp when added (microseconds since epoch)
    pub timestamp: u64,
    /// Optional label/description
    pub label: Option<String>,
}

/// Clipboard slot identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipboardSlot {
    /// Primary clipboard (OSC 52 default)
    Primary,
    /// System clipboard
    Clipboard,
    /// Selection clipboard (X11)
    Selection,
    /// Custom numbered slot (0-9)
    Custom(u8),
}

/// Clipboard operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardOperation {
    /// Set clipboard content
    Set,
    /// Query clipboard content
    Query,
    /// Clear clipboard
    Clear,
}

/// Clipboard sync target
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipboardTarget {
    /// System clipboard (c)
    Clipboard,
    /// Primary selection (p)
    Primary,
    /// Secondary selection (s)
    Secondary,
    /// Cut buffer 0 (c0)
    CutBuffer0,
}

/// Clipboard sync event for diagnostics
#[derive(Debug, Clone)]
pub struct ClipboardSyncEvent {
    /// Target clipboard
    pub target: ClipboardTarget,
    /// Operation type
    pub operation: ClipboardOperation,
    /// Content (truncated if necessary)
    pub content: String,
    /// Whether it was a read or write
    pub is_write: bool,
    /// Whether it was a remote operation
    pub is_remote: bool,
    /// Timestamp (unix millis)
    pub timestamp: u64,
}

/// Clipboard history entry across targets
#[derive(Debug, Clone)]
pub struct ClipboardHistoryEntry {
    /// Target clipboard
    pub target: ClipboardTarget,
    /// Content
    pub content: String,
    /// Source identifier (e.g., session ID, hostname)
    pub source: Option<String>,
    /// Timestamp (unix millis)
    pub timestamp: u64,
}

use crate::terminal::Terminal;

impl Terminal {
    // === Feature 10: Clipboard Integration ===

    /// Add content to clipboard history
    ///
    /// Content exceeding `MAX_CLIPBOARD_CONTENT_SIZE` (10 MB) is truncated
    /// to prevent excessive memory usage from malicious clipboard payloads.
    pub fn add_to_clipboard_history(
        &mut self,
        slot: ClipboardSlot,
        content: String,
        label: Option<String>,
    ) {
        let content = if content.len() > MAX_CLIPBOARD_CONTENT_SIZE {
            content[..MAX_CLIPBOARD_CONTENT_SIZE].to_string()
        } else {
            content
        };

        let entry = ClipboardEntry {
            content,
            timestamp: crate::terminal::get_timestamp_us(),
            label,
        };

        let history = self.clipboard_history.entry(slot).or_default();
        history.push(entry);

        // Keep only last N entries
        if history.len() > self.max_clipboard_history {
            history.remove(0);
        }
    }

    /// Get clipboard history for a slot
    pub fn get_clipboard_history(&self, slot: ClipboardSlot) -> Vec<ClipboardEntry> {
        self.clipboard_history
            .get(&slot)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the most recent clipboard entry for a slot
    pub fn get_latest_clipboard(&self, slot: ClipboardSlot) -> Option<ClipboardEntry> {
        self.clipboard_history
            .get(&slot)
            .and_then(|history| history.last().cloned())
    }

    /// Clear clipboard history for a slot
    pub fn clear_clipboard_history(&mut self, slot: ClipboardSlot) {
        self.clipboard_history.remove(&slot);
    }

    /// Clear all clipboard history
    pub fn clear_all_clipboard_history(&mut self) {
        self.clipboard_history.clear();
    }

    /// Set clipboard content with slot
    pub fn set_clipboard_with_slot(&mut self, content: String, slot: ClipboardSlot) {
        self.add_to_clipboard_history(slot, content, None);
    }

    /// Get clipboard content from slot
    pub fn get_clipboard_from_slot(&self, slot: ClipboardSlot) -> Option<String> {
        self.get_latest_clipboard(slot).map(|e| e.content)
    }

    /// Search clipboard history
    pub fn search_clipboard_history(
        &self,
        query: &str,
        slot: Option<ClipboardSlot>,
    ) -> Vec<ClipboardEntry> {
        let mut results = Vec::new();
        if let Some(s) = slot {
            if let Some(history) = self.clipboard_history.get(&s) {
                results.extend(
                    history
                        .iter()
                        .filter(|e| e.content.contains(query))
                        .cloned(),
                );
            }
        } else {
            for history in self.clipboard_history.values() {
                results.extend(
                    history
                        .iter()
                        .filter(|e| e.content.contains(query))
                        .cloned(),
                );
            }
        }
        results
    }

    /// Set clipboard content (OSC 52)
    pub fn set_clipboard(&mut self, content: Option<String>) {
        if let Some(c) = content {
            self.clipboard_content = Some(c.clone());
            self.add_to_clipboard_history(ClipboardSlot::Clipboard, c, Some("OSC 52".into()));
        } else {
            self.clipboard_content = None;
        }
    }

    /// Get clipboard content (OSC 52)
    pub fn get_clipboard(&self) -> Option<String> {
        self.clipboard_content.clone()
    }

    // === Feature 30: OSC 52 Clipboard Sync ===

    /// Record a clipboard sync event
    pub fn record_clipboard_sync(
        &mut self,
        target: ClipboardTarget,
        operation: ClipboardOperation,
        content: Option<String>,
        is_remote: bool,
    ) {
        let is_write = operation == ClipboardOperation::Set;
        let mut content_str = content.clone().unwrap_or_default();
        crate::terminal::sanitize_clipboard_content(
            &mut content_str,
            self.max_clipboard_event_bytes,
        );

        let event = ClipboardSyncEvent {
            target,
            operation,
            content: content_str,
            is_write,
            is_remote,
            timestamp: crate::terminal::unix_millis(),
        };

        self.clipboard_sync_events.push(event);
        if self.clipboard_sync_events.len() > self.max_clipboard_sync_events {
            self.clipboard_sync_events.remove(0);
        }

        if is_write {
            if let Some(c) = content {
                let history_entry = ClipboardHistoryEntry {
                    target,
                    content: c,
                    source: if is_remote {
                        self.remote_session_id.clone()
                    } else {
                        None
                    },
                    timestamp: crate::terminal::unix_millis(),
                };
                let history = self.clipboard_sync_history.entry(target).or_default();
                history.push(history_entry);
                if history.len() > self.max_clipboard_sync_history {
                    history.remove(0);
                }
            }
        }
    }

    /// Get clipboard sync events
    pub fn get_clipboard_sync_events(&self) -> &[ClipboardSyncEvent] {
        &self.clipboard_sync_events
    }

    /// Get clipboard sync history for a target
    pub fn get_clipboard_sync_history(
        &self,
        target: ClipboardTarget,
    ) -> Vec<ClipboardHistoryEntry> {
        self.clipboard_sync_history
            .get(&target)
            .cloned()
            .unwrap_or_default()
    }

    /// Clear clipboard sync events
    pub fn clear_clipboard_sync_events(&mut self) {
        self.clipboard_sync_events.clear();
    }

    /// Set maximum clipboard sync events
    pub fn set_max_clipboard_sync_events(&mut self, max: usize) {
        self.max_clipboard_sync_events = max;
        if self.clipboard_sync_events.len() > max {
            self.clipboard_sync_events
                .drain(0..self.clipboard_sync_events.len() - max);
        }
    }

    /// Set maximum clipboard event bytes
    pub fn set_max_clipboard_event_bytes(&mut self, max: usize) {
        self.max_clipboard_event_bytes = max;
    }

    /// Set remote session ID for clipboard sync
    pub fn set_remote_session_id(&mut self, id: Option<String>) {
        self.remote_session_id = id;
    }

    /// Set maximum clipboard sync history
    pub fn set_max_clipboard_sync_history(&mut self, max: usize) {
        self.max_clipboard_sync_history = max;
        for history in self.clipboard_sync_history.values_mut() {
            if history.len() > max {
                history.drain(0..history.len() - max);
            }
        }
    }

    /// Get maximum clipboard sync events
    pub fn max_clipboard_sync_events(&self) -> usize {
        self.max_clipboard_sync_events
    }

    /// Get maximum clipboard event bytes
    pub fn max_clipboard_event_bytes(&self) -> usize {
        self.max_clipboard_event_bytes
    }

    /// Get remote session ID
    pub fn remote_session_id(&self) -> Option<&str> {
        self.remote_session_id.as_deref()
    }
}
