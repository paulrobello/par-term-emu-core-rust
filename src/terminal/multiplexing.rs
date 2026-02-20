//! Terminal multiplexing helpers
//!
//! Provides types for session management, window layouts, and pane state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Pane state for session management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneState {
    /// Pane identifier
    pub id: String,
    /// Pane title
    pub title: String,
    /// Terminal dimensions (cols, rows)
    pub size: (usize, usize),
    /// Position in layout (x, y)
    pub position: (usize, usize),
    /// Working directory
    pub cwd: Option<String>,
    /// Environment variables
    pub env: HashMap<String, String>,
    /// Screen content snapshot
    pub content: Vec<String>,
    /// Cursor position
    pub cursor: (usize, usize),
    /// Is alternate screen active
    pub alt_screen: bool,
    /// Scrollback position
    pub scroll_offset: usize,
    /// Creation timestamp
    pub created_at: u64,
    /// Last activity timestamp
    pub last_activity: u64,
}

/// Layout direction for panes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutDirection {
    /// Horizontal split (side by side)
    Horizontal,
    /// Vertical split (top and bottom)
    Vertical,
}

/// Window layout configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowLayout {
    /// Layout identifier
    pub id: String,
    /// Layout name
    pub name: String,
    /// Split direction
    pub direction: LayoutDirection,
    /// Pane IDs in this layout
    pub panes: Vec<String>,
    /// Relative sizes (percentages)
    pub sizes: Vec<u8>,
    /// Active pane index
    pub active_pane: usize,
}

/// Complete session state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Session identifier
    pub id: String,
    /// Session name
    pub name: String,
    /// All panes in the session
    pub panes: Vec<PaneState>,
    /// All layouts in the session
    pub layouts: Vec<WindowLayout>,
    /// Active layout index
    pub active_layout: usize,
    /// Session metadata
    pub metadata: HashMap<String, String>,
    /// Creation timestamp
    pub created_at: u64,
    /// Last saved timestamp
    pub last_saved: u64,
}

use crate::terminal::Terminal;

impl Terminal {
    // === Feature 13: Terminal Multiplexing Helpers ===

    /// Capture current terminal state as PaneState
    ///
    /// If `cwd` is provided, it overrides the shell-integration detected cwd.
    pub fn capture_pane_state(&self, id: String, cwd: Option<String>) -> PaneState {
        let (cols, rows) = self.size();
        PaneState {
            id,
            title: self.title().to_string(),
            size: (cols, rows),
            position: (0, 0), // Layout position should be set by manager
            cwd: cwd.or_else(|| self.shell_integration.cwd().map(|s| s.to_string())),
            env: HashMap::new(), // Environment could be captured if available
            content: self.get_logical_lines(),
            cursor: (self.cursor.col, self.cursor.row),
            alt_screen: self.alt_screen_active,
            scroll_offset: 0,
            created_at: crate::terminal::unix_millis(),
            last_activity: crate::terminal::unix_millis(),
        }
    }

    /// Restore terminal state from PaneState
    pub fn restore_pane_state(&mut self, state: &PaneState) {
        self.resize(state.size.0, state.size.1);
        self.set_title(state.title.clone());
        self.cursor.col = state.cursor.0;
        self.cursor.row = state.cursor.1;
        self.pane_state = Some(state.clone());
        // In a real implementation, we would also restore grid content
    }

    /// Set current pane state
    pub fn set_pane_state(&mut self, state: PaneState) {
        self.pane_state = Some(state);
    }

    /// Get current pane state
    pub fn get_pane_state(&self) -> Option<PaneState> {
        self.pane_state.clone()
    }

    /// Clear current pane state
    pub fn clear_pane_state(&mut self) {
        self.pane_state = None;
    }

    /// Create a new window layout
    pub fn create_window_layout(
        id: String,
        name: String,
        direction: LayoutDirection,
        panes: Vec<String>,
        sizes: Vec<u8>,
        active_pane: usize,
    ) -> WindowLayout {
        WindowLayout {
            id,
            name,
            direction,
            panes,
            sizes,
            active_pane,
        }
    }

    /// Create a new session state
    pub fn create_session_state(
        id: String,
        name: String,
        panes: Vec<PaneState>,
        layouts: Vec<WindowLayout>,
        active_layout: usize,
        metadata: HashMap<String, String>,
    ) -> SessionState {
        SessionState {
            id,
            name,
            panes,
            layouts,
            active_layout,
            metadata,
            created_at: crate::terminal::unix_millis(),
            last_saved: crate::terminal::unix_millis(),
        }
    }

    /// Serialize session to JSON
    pub fn serialize_session(session: &SessionState) -> Result<String, String> {
        serde_json::to_string(session).map_err(|e| e.to_string())
    }

    /// Deserialize session from JSON
    pub fn deserialize_session(json: &str) -> Result<SessionState, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;
    use std::collections::HashMap;

    fn make_pane_state(id: &str) -> PaneState {
        PaneState {
            id: id.to_string(),
            title: String::new(),
            size: (80, 24),
            position: (0, 0),
            cwd: None,
            env: HashMap::new(),
            content: vec![],
            cursor: (0, 0),
            alt_screen: false,
            scroll_offset: 0,
            created_at: 0,
            last_activity: 0,
        }
    }

    #[test]
    fn test_capture_pane_state_dimensions() {
        let term = Terminal::new(80, 24);
        let state = term.capture_pane_state("pane-1".to_string(), None);
        assert_eq!(state.id, "pane-1");
        assert_eq!(state.size, (80, 24));
    }

    #[test]
    fn test_capture_pane_state_with_cwd() {
        let term = Terminal::new(80, 24);
        let state = term.capture_pane_state("pane-1".to_string(), Some("/home/user".to_string()));
        assert_eq!(state.cwd, Some("/home/user".to_string()));
    }

    #[test]
    fn test_capture_pane_state_cursor_position() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[5;10H"); // row 5, col 10 (1-indexed)
        let state = term.capture_pane_state("pane-1".to_string(), None);
        assert_eq!(
            state.cursor,
            (9, 4),
            "cursor should be (col=9, row=4) 0-indexed"
        );
    }

    #[test]
    fn test_capture_pane_state_title() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]2;My Terminal\x07");
        let state = term.capture_pane_state("pane-1".to_string(), None);
        assert_eq!(state.title, "My Terminal");
    }

    #[test]
    fn test_restore_pane_state_updates_size() {
        let mut term = Terminal::new(80, 24);
        let mut state = term.capture_pane_state("pane-1".to_string(), None);
        state.size = (120, 40);
        term.restore_pane_state(&state);
        assert_eq!(term.size(), (120, 40));
    }

    #[test]
    fn test_set_and_get_pane_state() {
        let mut term = Terminal::new(80, 24);
        term.set_pane_state(make_pane_state("test-pane"));
        let retrieved = term.get_pane_state();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "test-pane");
    }

    #[test]
    fn test_get_pane_state_initially_none() {
        let term = Terminal::new(80, 24);
        assert!(term.get_pane_state().is_none());
    }

    #[test]
    fn test_clear_pane_state() {
        let mut term = Terminal::new(80, 24);
        term.set_pane_state(make_pane_state("p"));
        term.clear_pane_state();
        assert!(term.get_pane_state().is_none());
    }

    #[test]
    fn test_create_window_layout() {
        let layout = Terminal::create_window_layout(
            "layout-1".to_string(),
            "Main".to_string(),
            LayoutDirection::Horizontal,
            vec!["pane-a".to_string(), "pane-b".to_string()],
            vec![50, 50],
            0,
        );
        assert_eq!(layout.id, "layout-1");
        assert_eq!(layout.name, "Main");
        assert_eq!(layout.panes.len(), 2);
        assert_eq!(layout.active_pane, 0);
    }

    #[test]
    fn test_create_session_state() {
        let session = Terminal::create_session_state(
            "session-1".to_string(),
            "My Session".to_string(),
            vec![],
            vec![],
            0,
            HashMap::new(),
        );
        assert_eq!(session.id, "session-1");
        assert_eq!(session.name, "My Session");
        assert!(session.panes.is_empty());
    }

    #[test]
    fn test_serialize_deserialize_session_roundtrip() {
        let session = Terminal::create_session_state(
            "session-1".to_string(),
            "Test Session".to_string(),
            vec![],
            vec![],
            0,
            HashMap::new(),
        );
        let json = Terminal::serialize_session(&session).expect("serialization should succeed");
        assert!(!json.is_empty());
        let deserialized =
            Terminal::deserialize_session(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, "session-1");
        assert_eq!(deserialized.name, "Test Session");
    }

    #[test]
    fn test_serialize_session_produces_valid_json() {
        let session = Terminal::create_session_state(
            "s1".to_string(),
            "S".to_string(),
            vec![],
            vec![],
            0,
            HashMap::new(),
        );
        let json = Terminal::serialize_session(&session).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("serialized session should be valid JSON");
        assert_eq!(parsed["id"], "s1");
    }

    #[test]
    fn test_deserialize_session_invalid_json_returns_error() {
        let result = Terminal::deserialize_session("not valid json {{");
        assert!(result.is_err(), "invalid JSON should return Err");
    }

    #[test]
    fn test_layout_direction_variants() {
        let _horiz = Terminal::create_window_layout(
            "l1".to_string(),
            "L1".to_string(),
            LayoutDirection::Horizontal,
            vec![],
            vec![],
            0,
        );
        let _vert = Terminal::create_window_layout(
            "l2".to_string(),
            "L2".to_string(),
            LayoutDirection::Vertical,
            vec![],
            vec![],
            0,
        );
    }
}
