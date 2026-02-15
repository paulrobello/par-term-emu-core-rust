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
