//! Semantic snapshot data types for terminal state capture
//!
//! Provides structured types for capturing terminal state as semantic snapshots,
//! including visible text, scrollback, shell integration zones, command history,
//! and working directory tracking. All types support serde serialization for
//! JSON/YAML interchange.

use serde::{Deserialize, Serialize};

/// Scope of a semantic snapshot capture
///
/// Controls how much terminal history is included in the snapshot.
/// This is a method parameter type and is not serialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotScope {
    /// Capture only the visible screen content
    Visible,
    /// Capture the last N commands and their output
    Recent(usize),
    /// Capture everything including full scrollback
    Full,
}

/// A semantic snapshot of terminal state
///
/// Contains structured information about the terminal including visible text,
/// scrollback content, shell integration zones, command history, and
/// working directory tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSnapshot {
    /// Unix timestamp (milliseconds) when the snapshot was captured
    pub timestamp: u64,
    /// Terminal width in columns
    pub cols: usize,
    /// Terminal height in rows
    pub rows: usize,
    /// Terminal title (from OSC 2)
    pub title: String,
    /// Current cursor column (0-indexed)
    pub cursor_col: usize,
    /// Current cursor row (0-indexed)
    pub cursor_row: usize,
    /// Whether the alternate screen buffer is active
    pub alt_screen_active: bool,
    /// Visible screen text content
    pub visible_text: String,
    /// Scrollback buffer text (if included by scope)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollback_text: Option<String>,
    /// Shell integration zones included in the snapshot
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<ZoneInfo>,
    /// Command history entries included in the snapshot
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandInfo>,
    /// Current working directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Current hostname
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Current username
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Working directory change history
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cwd_history: Vec<CwdChangeInfo>,
    /// Number of lines in the scrollback buffer
    pub scrollback_lines: usize,
    /// Total number of shell integration zones
    pub total_zones: usize,
    /// Total number of commands in history
    pub total_commands: usize,
}

/// Information about a shell integration zone
///
/// Zones represent semantic regions of terminal output identified by
/// shell integration sequences (prompt, input, output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneInfo {
    /// Zone identifier
    pub id: usize,
    /// Zone type (e.g., "prompt", "input", "output")
    pub zone_type: String,
    /// Absolute row where the zone starts (in scrollback coordinates)
    pub abs_row_start: usize,
    /// Absolute row where the zone ends (in scrollback coordinates)
    pub abs_row_end: usize,
    /// Text content of the zone
    pub text: String,
    /// Command associated with this zone (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Exit code of the command (if completed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Unix timestamp (milliseconds) when the zone was created
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
}

/// Information about a command execution
///
/// Tracks command text, timing, exit status, and optionally the output text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    /// The command text
    pub command: String,
    /// Working directory when the command was executed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Unix timestamp (milliseconds) when the command started
    pub start_time: u64,
    /// Unix timestamp (milliseconds) when the command ended
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<u64>,
    /// Exit code of the command
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Whether the command succeeded (exit code 0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// Command output text (if included)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// Information about a working directory change
///
/// Tracks transitions between directories, optionally with user/host context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwdChangeInfo {
    /// Previous working directory (if known)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_cwd: Option<String>,
    /// New working directory
    pub new_cwd: String,
    /// Hostname where the change occurred
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Username at time of change
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Unix timestamp (milliseconds) of the change
    pub timestamp: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_scope_variants() {
        let visible = SnapshotScope::Visible;
        let recent = SnapshotScope::Recent(5);
        let full = SnapshotScope::Full;

        assert_eq!(visible, SnapshotScope::Visible);
        assert_eq!(recent, SnapshotScope::Recent(5));
        assert_ne!(recent, SnapshotScope::Recent(10));
        assert_eq!(full, SnapshotScope::Full);
        assert_ne!(visible, full);
    }

    #[test]
    fn test_snapshot_scope_clone() {
        let scope = SnapshotScope::Recent(3);
        let cloned = scope.clone();
        assert_eq!(scope, cloned);
    }

    #[test]
    fn test_snapshot_scope_debug() {
        let scope = SnapshotScope::Visible;
        let debug_str = format!("{:?}", scope);
        assert_eq!(debug_str, "Visible");

        let scope = SnapshotScope::Recent(10);
        let debug_str = format!("{:?}", scope);
        assert_eq!(debug_str, "Recent(10)");
    }

    #[test]
    fn test_semantic_snapshot_serialization_roundtrip() {
        let snapshot = SemanticSnapshot {
            timestamp: 1700000000000,
            cols: 80,
            rows: 24,
            title: "bash".to_string(),
            cursor_col: 5,
            cursor_row: 10,
            alt_screen_active: false,
            visible_text: "$ ls\nfile1.txt  file2.txt\n$".to_string(),
            scrollback_text: Some("previous output\n".to_string()),
            zones: vec![ZoneInfo {
                id: 0,
                zone_type: "output".to_string(),
                abs_row_start: 100,
                abs_row_end: 105,
                text: "file1.txt  file2.txt".to_string(),
                command: Some("ls".to_string()),
                exit_code: Some(0),
                timestamp: Some(1700000000000),
            }],
            commands: vec![CommandInfo {
                command: "ls".to_string(),
                cwd: Some("/home/user".to_string()),
                start_time: 1700000000000,
                end_time: Some(1700000000050),
                exit_code: Some(0),
                duration_ms: Some(50),
                success: Some(true),
                output: Some("file1.txt  file2.txt".to_string()),
            }],
            cwd: Some("/home/user".to_string()),
            hostname: Some("myhost".to_string()),
            username: Some("user".to_string()),
            cwd_history: vec![CwdChangeInfo {
                old_cwd: Some("/home".to_string()),
                new_cwd: "/home/user".to_string(),
                hostname: Some("myhost".to_string()),
                username: Some("user".to_string()),
                timestamp: 1699999999000,
            }],
            scrollback_lines: 500,
            total_zones: 10,
            total_commands: 5,
        };

        let json = serde_json::to_string(&snapshot).expect("serialization failed");
        let deserialized: SemanticSnapshot =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.timestamp, 1700000000000);
        assert_eq!(deserialized.cols, 80);
        assert_eq!(deserialized.rows, 24);
        assert_eq!(deserialized.title, "bash");
        assert_eq!(deserialized.cursor_col, 5);
        assert_eq!(deserialized.cursor_row, 10);
        assert!(!deserialized.alt_screen_active);
        assert_eq!(deserialized.visible_text, "$ ls\nfile1.txt  file2.txt\n$");
        assert_eq!(
            deserialized.scrollback_text,
            Some("previous output\n".to_string())
        );
        assert_eq!(deserialized.zones.len(), 1);
        assert_eq!(deserialized.commands.len(), 1);
        assert_eq!(deserialized.cwd, Some("/home/user".to_string()));
        assert_eq!(deserialized.hostname, Some("myhost".to_string()));
        assert_eq!(deserialized.username, Some("user".to_string()));
        assert_eq!(deserialized.cwd_history.len(), 1);
        assert_eq!(deserialized.scrollback_lines, 500);
        assert_eq!(deserialized.total_zones, 10);
        assert_eq!(deserialized.total_commands, 5);
    }

    #[test]
    fn test_semantic_snapshot_skip_serializing_none_fields() {
        let snapshot = SemanticSnapshot {
            timestamp: 1700000000000,
            cols: 80,
            rows: 24,
            title: String::new(),
            cursor_col: 0,
            cursor_row: 0,
            alt_screen_active: false,
            visible_text: String::new(),
            scrollback_text: None,
            zones: vec![],
            commands: vec![],
            cwd: None,
            hostname: None,
            username: None,
            cwd_history: vec![],
            scrollback_lines: 0,
            total_zones: 0,
            total_commands: 0,
        };

        let json = serde_json::to_string(&snapshot).expect("serialization failed");

        // None fields should be omitted
        assert!(!json.contains("scrollback_text"));
        assert!(!json.contains("cwd"));
        assert!(!json.contains("hostname"));
        assert!(!json.contains("username"));

        // Empty vecs should be omitted (use quoted key format to avoid matching
        // substrings like "total_zones" or "total_commands")
        assert!(!json.contains("\"zones\""));
        assert!(!json.contains("\"commands\""));
        assert!(!json.contains("\"cwd_history\""));

        // Required fields should always be present
        assert!(json.contains("timestamp"));
        assert!(json.contains("cols"));
        assert!(json.contains("rows"));
        assert!(json.contains("visible_text"));
        assert!(json.contains("scrollback_lines"));
        assert!(json.contains("total_zones"));
        assert!(json.contains("total_commands"));
    }

    #[test]
    fn test_zone_info_serialization() {
        let zone = ZoneInfo {
            id: 1,
            zone_type: "prompt".to_string(),
            abs_row_start: 50,
            abs_row_end: 51,
            text: "$ ".to_string(),
            command: None,
            exit_code: None,
            timestamp: None,
        };

        let json = serde_json::to_string(&zone).expect("serialization failed");

        // Optional None fields should be omitted
        assert!(!json.contains("command"));
        assert!(!json.contains("exit_code"));
        assert!(!json.contains("timestamp"));

        // Required fields should be present
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"zone_type\":\"prompt\""));
        assert!(json.contains("\"abs_row_start\":50"));
        assert!(json.contains("\"abs_row_end\":51"));
        assert!(json.contains("\"text\":\"$ \""));
    }

    #[test]
    fn test_zone_info_with_optional_fields() {
        let zone = ZoneInfo {
            id: 2,
            zone_type: "output".to_string(),
            abs_row_start: 52,
            abs_row_end: 55,
            text: "hello world".to_string(),
            command: Some("echo hello world".to_string()),
            exit_code: Some(0),
            timestamp: Some(1700000000000),
        };

        let json = serde_json::to_string(&zone).expect("serialization failed");
        let deserialized: ZoneInfo = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.command, Some("echo hello world".to_string()));
        assert_eq!(deserialized.exit_code, Some(0));
        assert_eq!(deserialized.timestamp, Some(1700000000000));
    }

    #[test]
    fn test_command_info_serialization() {
        let cmd = CommandInfo {
            command: "cargo build".to_string(),
            cwd: Some("/home/user/project".to_string()),
            start_time: 1700000000000,
            end_time: Some(1700000005000),
            exit_code: Some(0),
            duration_ms: Some(5000),
            success: Some(true),
            output: None,
        };

        let json = serde_json::to_string(&cmd).expect("serialization failed");
        let deserialized: CommandInfo =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.command, "cargo build");
        assert_eq!(deserialized.cwd, Some("/home/user/project".to_string()));
        assert_eq!(deserialized.start_time, 1700000000000);
        assert_eq!(deserialized.end_time, Some(1700000005000));
        assert_eq!(deserialized.exit_code, Some(0));
        assert_eq!(deserialized.duration_ms, Some(5000));
        assert_eq!(deserialized.success, Some(true));
        assert_eq!(deserialized.output, None);

        // output should be omitted when None
        assert!(!json.contains("output"));
    }

    #[test]
    fn test_command_info_minimal() {
        let cmd = CommandInfo {
            command: "pwd".to_string(),
            cwd: None,
            start_time: 1700000000000,
            end_time: None,
            exit_code: None,
            duration_ms: None,
            success: None,
            output: None,
        };

        let json = serde_json::to_string(&cmd).expect("serialization failed");

        // Only required fields should be present
        assert!(json.contains("\"command\":\"pwd\""));
        assert!(json.contains("\"start_time\":1700000000000"));

        // All optional fields should be omitted
        assert!(!json.contains("cwd"));
        assert!(!json.contains("end_time"));
        assert!(!json.contains("exit_code"));
        assert!(!json.contains("duration_ms"));
        assert!(!json.contains("success"));
        assert!(!json.contains("output"));
    }

    #[test]
    fn test_cwd_change_info_serialization() {
        let change = CwdChangeInfo {
            old_cwd: Some("/home".to_string()),
            new_cwd: "/home/user".to_string(),
            hostname: Some("myhost".to_string()),
            username: Some("user".to_string()),
            timestamp: 1700000000000,
        };

        let json = serde_json::to_string(&change).expect("serialization failed");
        let deserialized: CwdChangeInfo =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.old_cwd, Some("/home".to_string()));
        assert_eq!(deserialized.new_cwd, "/home/user");
        assert_eq!(deserialized.hostname, Some("myhost".to_string()));
        assert_eq!(deserialized.username, Some("user".to_string()));
        assert_eq!(deserialized.timestamp, 1700000000000);
    }

    #[test]
    fn test_cwd_change_info_minimal() {
        let change = CwdChangeInfo {
            old_cwd: None,
            new_cwd: "/tmp".to_string(),
            hostname: None,
            username: None,
            timestamp: 1700000000000,
        };

        let json = serde_json::to_string(&change).expect("serialization failed");

        assert!(!json.contains("old_cwd"));
        assert!(!json.contains("hostname"));
        assert!(!json.contains("username"));
        assert!(json.contains("\"new_cwd\":\"/tmp\""));
        assert!(json.contains("\"timestamp\":1700000000000"));
    }

    #[test]
    fn test_terminal_visible_snapshot() {
        use crate::terminal::Terminal;
        let mut term = Terminal::new(80, 24);
        term.process(b"Hello, World!\r\n");
        term.process(b"Second line");
        let snap = term.get_semantic_snapshot(SnapshotScope::Visible);
        assert_eq!(snap.cols, 80);
        assert_eq!(snap.rows, 24);
        assert!(!snap.alt_screen_active);
        assert!(snap.visible_text.contains("Hello, World!"));
        assert!(snap.visible_text.contains("Second line"));
        assert!(snap.scrollback_text.is_none());
        assert!(snap.commands.is_empty());
        assert_eq!(snap.total_commands, 0);
    }

    #[test]
    fn test_terminal_snapshot_json() {
        use crate::terminal::Terminal;
        let mut term = Terminal::new(80, 24);
        term.process(b"Test content");
        let json = term.get_semantic_snapshot_json(SnapshotScope::Visible);
        assert!(json.contains("\"cols\":80"));
        assert!(json.contains("Test content"));
        // Verify it's valid JSON
        let _parsed: SemanticSnapshot = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_terminal_full_snapshot() {
        use crate::terminal::Terminal;
        let mut term = Terminal::new(80, 24);
        term.process(b"Some content\r\n");
        let snap = term.get_semantic_snapshot(SnapshotScope::Full);
        assert_eq!(snap.cols, 80);
        assert!(snap.visible_text.contains("Some content"));
        // Full scope should have scrollback_text (or None if nothing in scrollback)
        assert_eq!(snap.total_commands, 0);
    }

    #[test]
    fn test_terminal_recent_snapshot() {
        use crate::terminal::Terminal;
        let term = Terminal::new(80, 24);
        // Recent(0) should have no commands
        let snap = term.get_semantic_snapshot(SnapshotScope::Recent(0));
        assert!(snap.commands.is_empty());
    }
}

/// Export format for scrollback
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    /// Plain text (stripped of all formatting)
    Plain,
    /// HTML with colors and styles
    Html,
    /// Raw ANSI escape sequences preserved
    Ansi,
}

/// Scrollback statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollbackStats {
    /// Total lines in scrollback
    pub total_lines: usize,
    /// Estimated memory usage in bytes
    pub memory_bytes: usize,
    /// Whether scrollback has wrapped around
    pub has_wrapped: bool,
}

/// Bookmark in scrollback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    /// Unique bookmark ID
    pub id: usize,
    /// Row position (negative = scrollback)
    pub row: isize,
    /// User-defined label
    pub label: String,
}

/// Type of change in a diff
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffChangeType {
    /// Line added
    Added,
    /// Line removed
    Removed,
    /// Line modified
    Modified,
    /// Line unchanged
    Unchanged,
}

/// A single line diff
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineDiff {
    /// Type of change
    pub change_type: DiffChangeType,
    /// Row number in old snapshot
    pub old_row: Option<usize>,
    /// Row number in new snapshot
    pub new_row: Option<usize>,
    /// Old line content
    pub old_content: Option<String>,
    /// New line content
    pub new_content: Option<String>,
}

/// Complete diff between two snapshots
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    /// List of line diffs
    pub diffs: Vec<LineDiff>,
    /// Number of lines added
    pub added: usize,
    /// Number of lines removed
    pub removed: usize,
    /// Number of lines modified
    pub modified: usize,
    /// Number of lines unchanged
    pub unchanged: usize,
}

/// Compare two sets of screen lines and return the differences
pub fn diff_screen_lines(old_lines: &[String], new_lines: &[String]) -> SnapshotDiff {
    let mut diffs = Vec::new();
    let mut added = 0;
    let mut removed = 0;
    let mut modified = 0;
    let mut unchanged = 0;

    let max_rows = old_lines.len().max(new_lines.len());

    for row in 0..max_rows {
        let old_line = old_lines.get(row);
        let new_line = new_lines.get(row);

        match (old_line, new_line) {
            (Some(old_content), Some(new_content)) => {
                if old_content == new_content {
                    unchanged += 1;
                    diffs.push(LineDiff {
                        change_type: DiffChangeType::Unchanged,
                        old_row: Some(row),
                        new_row: Some(row),
                        old_content: Some(old_content.clone()),
                        new_content: Some(new_content.clone()),
                    });
                } else {
                    modified += 1;
                    diffs.push(LineDiff {
                        change_type: DiffChangeType::Modified,
                        old_row: Some(row),
                        new_row: Some(row),
                        old_content: Some(old_content.clone()),
                        new_content: Some(new_content.clone()),
                    });
                }
            }
            (None, Some(new_content)) => {
                added += 1;
                diffs.push(LineDiff {
                    change_type: DiffChangeType::Added,
                    old_row: None,
                    new_row: Some(row),
                    old_content: None,
                    new_content: Some(new_content.clone()),
                });
            }
            (Some(old_content), None) => {
                removed += 1;
                diffs.push(LineDiff {
                    change_type: DiffChangeType::Removed,
                    old_row: Some(row),
                    new_row: None,
                    old_content: Some(old_content.clone()),
                    new_content: None,
                });
            }
            (None, None) => {
                // This shouldn't happen
            }
        }
    }

    SnapshotDiff {
        diffs,
        added,
        removed,
        modified,
        unchanged,
    }
}

use crate::cell::Cell;
use crate::terminal::Terminal;

impl Terminal {
    // === Scrollback Operations ===

    /// Export scrollback to various formats
    ///
    /// # Arguments
    /// * `format` - Export format (Plain, Html, Ansi)
    /// * `max_lines` - Maximum number of scrollback lines to export (None = all)
    ///
    /// Returns the exported content as a string.
    pub fn export_scrollback(&self, format: ExportFormat, max_lines: Option<usize>) -> String {
        let scrollback_len = self.grid.scrollback_len();
        let lines_to_export = max_lines.unwrap_or(scrollback_len).min(scrollback_len);

        match format {
            ExportFormat::Plain => {
                let mut output = String::new();
                for i in (0..lines_to_export).rev() {
                    if let Some(line) = self.grid.scrollback_line(i) {
                        output.push_str(&crate::terminal::cells_to_text(line));
                        output.push('\n');
                    }
                }
                output
            }
            ExportFormat::Html => {
                let mut output = String::from("<pre>\n");
                for i in (0..lines_to_export).rev() {
                    if let Some(line) = self.grid.scrollback_line(i) {
                        let text = crate::terminal::cells_to_text(line);
                        output.push_str(&crate::terminal::html_escape(&text));
                        output.push('\n');
                    }
                }
                output.push_str("</pre>");
                output
            }
            ExportFormat::Ansi => {
                // For ANSI export, we'd need to preserve colors/attributes
                // For now, just export as plain text
                self.export_scrollback(ExportFormat::Plain, max_lines)
            }
        }
    }

    /// Get scrollback statistics
    pub fn scrollback_stats(&self) -> ScrollbackStats {
        let total_lines = self.grid.scrollback_len();
        let memory_bytes = total_lines * self.grid.cols() * std::mem::size_of::<Cell>();
        // Scrollback has wrapped if we've filled the buffer
        let has_wrapped = total_lines >= self.grid.max_scrollback();

        ScrollbackStats {
            total_lines,
            memory_bytes,
            has_wrapped,
        }
    }

    /// Get current scrollback usage (lines used)
    pub fn get_scrollback_usage(&self) -> usize {
        self.grid.scrollback_len()
    }

    /// Capture a semantic snapshot of the terminal state
    pub fn get_semantic_snapshot(&self, scope: SnapshotScope) -> SemanticSnapshot {
        let (cols, rows) = self.size();
        let scrollback_len = self.grid.scrollback_len();

        let mut snapshot = SemanticSnapshot {
            timestamp: crate::terminal::unix_millis(),
            cols,
            rows,
            title: self.title().to_string(),
            cursor_col: self.cursor.col,
            cursor_row: self.cursor.row,
            alt_screen_active: self.alt_screen_active,
            visible_text: self.content(),
            scrollback_text: None,
            zones: Vec::new(),
            commands: Vec::new(),
            cwd: self.shell_integration.cwd().map(|s| s.to_string()),
            hostname: None, // Could be extracted from shell_integration
            username: None, // Could be extracted from shell_integration
            cwd_history: Vec::new(),
            scrollback_lines: scrollback_len,
            total_zones: self.grid.zones().len(),
            total_commands: self.command_history.len(),
        };

        if scope == SnapshotScope::Full {
            snapshot.scrollback_text = Some(self.export_scrollback(ExportFormat::Plain, None));
        }

        snapshot
    }

    /// Capture a semantic snapshot and return it as a JSON string
    pub fn get_semantic_snapshot_json(&self, scope: SnapshotScope) -> String {
        serde_json::to_string(&self.get_semantic_snapshot(scope)).unwrap_or_default()
    }

    // === Bookmark Methods ===

    /// Add a bookmark at the given scrollback row
    ///
    /// # Arguments
    /// * `row` - Row index (negative for scrollback, 0+ for visible screen)
    /// * `label` - Optional label for the bookmark
    ///
    /// Returns the bookmark ID.
    pub fn add_bookmark(&mut self, row: isize, label: Option<String>) -> usize {
        let id = self.next_bookmark_id;
        self.next_bookmark_id += 1;

        let bookmark = Bookmark {
            id,
            row,
            label: label.unwrap_or_else(|| format!("Bookmark {}", id)),
        };

        self.bookmarks.push(bookmark);
        id
    }

    /// Get all bookmarks
    pub fn get_bookmarks(&self) -> Vec<Bookmark> {
        self.bookmarks.clone()
    }

    /// Remove a bookmark by ID
    pub fn remove_bookmark(&mut self, id: usize) -> bool {
        if let Some(pos) = self.bookmarks.iter().position(|b| b.id == id) {
            self.bookmarks.remove(pos);
            true
        } else {
            false
        }
    }

    /// Clear all bookmarks
    pub fn clear_bookmarks(&mut self) {
        self.bookmarks.clear();
    }

    /// Get the terminal content as a string (visible area)
    pub fn content(&self) -> String {
        self.get_logical_lines().join("\n")
    }

    /// Export the visible screen content with ANSI styling
    pub fn export_visible_screen_styled(&self) -> String {
        self.active_grid().export_visible_screen_styled()
    }

    /// Export entire buffer (scrollback + current screen) as plain text
    pub fn export_text(&self) -> String {
        let mut output = self.export_scrollback(ExportFormat::Plain, None);
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&self.content());
        output
    }

    /// Export entire buffer (scrollback + current screen) with ANSI styling
    pub fn export_styled(&self) -> String {
        // For now, just return plain text as a placeholder
        // In a full implementation, this would include ANSI sequences
        self.export_text()
    }

    /// Export the current screen as HTML
    pub fn export_html(&self, _include_styles: bool) -> String {
        let (_cols, rows) = self.size();
        let mut output = String::from("<pre style=\"font-family: monospace\">\n");

        for row in 0..rows {
            if let Some(line) = self.active_grid().row(row) {
                let text = crate::terminal::cells_to_text(line);
                let escaped = crate::terminal::html_escape(&text);
                output.push_str(&escaped);
                output.push('\n');
            }
        }

        output.push_str("</pre>");
        output
    }
}
