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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    #[test]
    fn test_add_and_get_clipboard_history() {
        let mut term = Terminal::new(80, 24);
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, "first entry".to_string(), None);
        let history = term.get_clipboard_history(ClipboardSlot::Clipboard);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "first entry");
    }

    #[test]
    fn test_get_latest_clipboard_returns_last() {
        let mut term = Terminal::new(80, 24);
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, "first".to_string(), None);
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, "second".to_string(), None);
        let latest = term.get_latest_clipboard(ClipboardSlot::Clipboard);
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().content, "second");
    }

    #[test]
    fn test_clear_clipboard_history() {
        let mut term = Terminal::new(80, 24);
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, "data".to_string(), None);
        assert!(!term
            .get_clipboard_history(ClipboardSlot::Clipboard)
            .is_empty());
        term.clear_clipboard_history(ClipboardSlot::Clipboard);
        assert!(term
            .get_clipboard_history(ClipboardSlot::Clipboard)
            .is_empty());
    }

    #[test]
    fn test_clear_all_clipboard_history() {
        let mut term = Terminal::new(80, 24);
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, "c".to_string(), None);
        term.add_to_clipboard_history(ClipboardSlot::Primary, "p".to_string(), None);
        term.clear_all_clipboard_history();
        assert!(term
            .get_clipboard_history(ClipboardSlot::Clipboard)
            .is_empty());
        assert!(term
            .get_clipboard_history(ClipboardSlot::Primary)
            .is_empty());
    }

    #[test]
    fn test_set_and_get_clipboard_with_slot() {
        let mut term = Terminal::new(80, 24);
        term.set_clipboard_with_slot("hello clipboard".to_string(), ClipboardSlot::Primary);
        let result = term.get_clipboard_from_slot(ClipboardSlot::Primary);
        assert_eq!(result, Some("hello clipboard".to_string()));
    }

    #[test]
    fn test_get_clipboard_from_slot_initially_none() {
        let term = Terminal::new(80, 24);
        assert!(term
            .get_clipboard_from_slot(ClipboardSlot::Clipboard)
            .is_none());
    }

    #[test]
    fn test_search_clipboard_history_finds_match() {
        let mut term = Terminal::new(80, 24);
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, "hello world".to_string(), None);
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, "foo bar".to_string(), None);
        let results = term.search_clipboard_history("hello", Some(ClipboardSlot::Clipboard));
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("hello"));
    }

    #[test]
    fn test_search_clipboard_history_no_match() {
        let mut term = Terminal::new(80, 24);
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, "hello world".to_string(), None);
        let results = term.search_clipboard_history("zzz", Some(ClipboardSlot::Clipboard));
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_clipboard_history_across_all_slots() {
        let mut term = Terminal::new(80, 24);
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, "clipboard data".to_string(), None);
        term.add_to_clipboard_history(ClipboardSlot::Primary, "primary data".to_string(), None);
        let results = term.search_clipboard_history("data", None);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_clipboard_history_truncates_large_content() {
        let mut term = Terminal::new(80, 24);
        let large = "x".repeat(11 * 1024 * 1024); // 11 MB (> MAX_CLIPBOARD_CONTENT_SIZE)
        term.add_to_clipboard_history(ClipboardSlot::Clipboard, large, None);
        let history = term.get_clipboard_history(ClipboardSlot::Clipboard);
        assert_eq!(history.len(), 1);
        assert!(
            history[0].content.len() <= 10 * 1024 * 1024 + 100,
            "content should be truncated to ~10MB, got {} bytes",
            history[0].content.len()
        );
    }

    #[test]
    fn test_set_and_get_clipboard_osc52() {
        let mut term = Terminal::new(80, 24);
        term.set_clipboard(Some("osc52 content".to_string()));
        assert_eq!(term.get_clipboard(), Some("osc52 content".to_string()));
    }

    #[test]
    fn test_get_clipboard_initial_none() {
        let term = Terminal::new(80, 24);
        assert!(term.get_clipboard().is_none());
    }

    #[test]
    fn test_set_clipboard_none_clears() {
        let mut term = Terminal::new(80, 24);
        term.set_clipboard(Some("data".to_string()));
        term.set_clipboard(None);
        assert!(term.get_clipboard().is_none());
    }

    #[test]
    fn test_record_clipboard_sync_and_get_events() {
        let mut term = Terminal::new(80, 24);
        term.record_clipboard_sync(
            ClipboardTarget::Clipboard,
            ClipboardOperation::Set,
            Some("synced data".to_string()),
            false,
        );
        let events = term.get_clipboard_sync_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content, "synced data");
    }

    #[test]
    fn test_clear_clipboard_sync_events() {
        let mut term = Terminal::new(80, 24);
        term.record_clipboard_sync(
            ClipboardTarget::Clipboard,
            ClipboardOperation::Set,
            Some("data".to_string()),
            false,
        );
        assert!(!term.get_clipboard_sync_events().is_empty());
        term.clear_clipboard_sync_events();
        assert!(term.get_clipboard_sync_events().is_empty());
    }

    #[test]
    fn test_clipboard_history_with_label() {
        let mut term = Terminal::new(80, 24);
        term.add_to_clipboard_history(
            ClipboardSlot::Clipboard,
            "labeled content".to_string(),
            Some("my label".to_string()),
        );
        let history = term.get_clipboard_history(ClipboardSlot::Clipboard);
        assert_eq!(history[0].label, Some("my label".to_string()));
    }

    #[test]
    fn test_max_clipboard_sync_events_setter() {
        let mut term = Terminal::new(80, 24);
        term.set_max_clipboard_sync_events(100);
        assert_eq!(term.max_clipboard_sync_events(), 100);
    }
}
