//! Protocol definitions for terminal streaming
//!
//! This module defines the message formats used for WebSocket-based
//! terminal streaming between the server and web clients.

use serde::{Deserialize, Serialize};

/// Theme information for terminal color scheme
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeInfo {
    /// Theme name (e.g., "iterm2-dark", "monokai")
    pub name: String,
    /// Background color (RGB)
    pub background: (u8, u8, u8),
    /// Foreground color (RGB)
    pub foreground: (u8, u8, u8),
    /// Normal ANSI colors 0-7 (RGB)
    pub normal: [(u8, u8, u8); 8],
    /// Bright ANSI colors 8-15 (RGB)
    pub bright: [(u8, u8, u8); 8],
}

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMessage {
    /// Terminal output data (raw ANSI escape sequences)
    Output {
        /// Raw terminal output data including ANSI sequences
        data: String,
        /// Optional timestamp (Unix epoch in milliseconds)
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<u64>,
    },

    /// Terminal size changed
    Resize {
        /// Number of columns
        cols: u16,
        /// Number of rows
        rows: u16,
    },

    /// Terminal title changed
    Title {
        /// New terminal title
        title: String,
    },

    /// Connection established successfully
    Connected {
        /// Current terminal width in columns
        cols: u16,
        /// Current terminal height in rows
        rows: u16,
        /// Optional initial screen content
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_screen: Option<String>,
        /// Session ID for this connection
        session_id: String,
        /// Optional theme information
        #[serde(skip_serializing_if = "Option::is_none")]
        theme: Option<ThemeInfo>,
        /// Current badge text (from OSC 1337)
        #[serde(skip_serializing_if = "Option::is_none")]
        badge: Option<String>,
        /// Faint text alpha for SGR 2 dim text (0.0-1.0)
        #[serde(skip_serializing_if = "Option::is_none")]
        faint_text_alpha: Option<f32>,
        /// Current working directory
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        /// modifyOtherKeys mode (0=disabled, 1=special keys, 2=all keys)
        #[serde(skip_serializing_if = "Option::is_none")]
        modify_other_keys: Option<u32>,
        /// Unique client identifier for this connection
        #[serde(skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        /// Whether this connection is read-only
        #[serde(skip_serializing_if = "Option::is_none")]
        readonly: Option<bool>,
    },

    /// Screen refresh response (full screen content)
    Refresh {
        /// Current terminal width in columns
        cols: u16,
        /// Current terminal height in rows
        rows: u16,
        /// Full screen content with ANSI styling
        screen_content: String,
    },

    /// Cursor position changed (optional optimization)
    #[serde(rename = "cursor")]
    CursorPosition {
        /// Column position (0-indexed)
        col: u16,
        /// Row position (0-indexed)
        row: u16,
        /// Whether cursor is visible
        visible: bool,
    },

    /// Bell event occurred
    Bell,

    /// Current working directory changed (OSC 7)
    CwdChanged {
        /// Previous working directory
        #[serde(skip_serializing_if = "Option::is_none")]
        old_cwd: Option<String>,
        /// New working directory
        new_cwd: String,
        /// Hostname (if remote)
        #[serde(skip_serializing_if = "Option::is_none")]
        hostname: Option<String>,
        /// Username (if provided)
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        /// Timestamp of change (Unix epoch milliseconds)
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<u64>,
    },

    /// Trigger pattern matched terminal output
    TriggerMatched {
        /// ID of the trigger that matched
        trigger_id: u64,
        /// Row where the match occurred
        row: u16,
        /// Column where match starts
        col: u16,
        /// Column where match ends (exclusive)
        end_col: u16,
        /// Matched text
        text: String,
        /// Capture groups
        captures: Vec<String>,
        /// Timestamp when match occurred (Unix epoch milliseconds)
        timestamp: u64,
    },

    /// Trigger action result: display notification
    ActionNotify {
        /// ID of the trigger that produced this action
        trigger_id: u64,
        /// Notification title
        title: String,
        /// Notification message
        message: String,
    },

    /// Trigger action result: mark/bookmark a line
    ActionMarkLine {
        /// ID of the trigger that produced this action
        trigger_id: u64,
        /// Row to mark
        row: u16,
        /// Optional label for the mark
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// Optional RGB color for the mark
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<(u8, u8, u8)>,
    },

    /// Error occurred
    Error {
        /// Error message
        message: String,
        /// Optional error code
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },

    /// Server is shutting down
    Shutdown {
        /// Reason for shutdown
        reason: String,
    },

    /// Keepalive pong response
    Pong,

    /// Terminal mode changed (cursor visibility, mouse tracking, etc.)
    ModeChanged {
        /// Mode name (e.g., "cursor_visible", "mouse_tracking", "bracketed_paste")
        mode: String,
        /// Whether the mode is enabled
        enabled: bool,
    },

    /// Graphics/image added to terminal (Sixel, iTerm2, Kitty)
    GraphicsAdded {
        /// Row where graphics were added
        row: u16,
        /// Graphics format ("sixel", "iterm2", "kitty")
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<String>,
    },

    /// Hyperlink added (OSC 8)
    HyperlinkAdded {
        /// The URL of the hyperlink
        url: String,
        /// Row where the hyperlink appears
        row: u16,
        /// Column where hyperlink starts
        col: u16,
        /// Optional hyperlink ID from OSC 8
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

/// Messages sent from client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMessage {
    /// User input (keyboard)
    Input {
        /// Input data (can include escape sequences)
        data: String,
    },

    /// Terminal resize request
    Resize {
        /// Requested number of columns
        cols: u16,
        /// Requested number of rows
        rows: u16,
    },

    /// Ping for keepalive
    Ping,

    /// Request full screen refresh
    #[serde(rename = "refresh")]
    RequestRefresh,

    /// Subscribe to specific events
    Subscribe {
        /// Event types to subscribe to
        events: Vec<EventType>,
    },
}

/// Event types that clients can subscribe to
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    /// Terminal output
    Output,
    /// Cursor position changes
    Cursor,
    /// Bell events
    Bell,
    /// Title changes
    Title,
    /// Resize events
    Resize,
    /// CWD change events
    Cwd,
    /// Trigger match events
    Trigger,
    /// Trigger action result events (Notify, MarkLine)
    Action,
    /// Terminal mode change events
    Mode,
    /// Graphics/image events
    Graphics,
    /// Hyperlink events
    Hyperlink,
}

impl ServerMessage {
    /// Create a new output message
    pub fn output(data: String) -> Self {
        Self::Output {
            data,
            timestamp: None,
        }
    }

    /// Create a new output message with timestamp
    pub fn output_with_timestamp(data: String, timestamp: u64) -> Self {
        Self::Output {
            data,
            timestamp: Some(timestamp),
        }
    }

    /// Create a new resize message
    pub fn resize(cols: u16, rows: u16) -> Self {
        Self::Resize { cols, rows }
    }

    /// Create a new title message
    pub fn title(title: String) -> Self {
        Self::Title { title }
    }

    /// Create a new connected message
    pub fn connected(cols: u16, rows: u16, session_id: String) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen: None,
            session_id,
            theme: None,
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        }
    }

    /// Create a new connected message with initial screen
    pub fn connected_with_screen(
        cols: u16,
        rows: u16,
        initial_screen: String,
        session_id: String,
    ) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen: Some(initial_screen),
            session_id,
            theme: None,
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        }
    }

    /// Create a new connected message with theme
    pub fn connected_with_theme(
        cols: u16,
        rows: u16,
        session_id: String,
        theme: ThemeInfo,
    ) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen: None,
            session_id,
            theme: Some(theme),
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        }
    }

    /// Create a new connected message with initial screen and theme
    pub fn connected_with_screen_and_theme(
        cols: u16,
        rows: u16,
        initial_screen: String,
        session_id: String,
        theme: ThemeInfo,
    ) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen: Some(initial_screen),
            session_id,
            theme: Some(theme),
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        }
    }

    /// Create a fully-specified connected message with all terminal state
    #[allow(clippy::too_many_arguments)]
    pub fn connected_full(
        cols: u16,
        rows: u16,
        initial_screen: Option<String>,
        session_id: String,
        theme: Option<ThemeInfo>,
        badge: Option<String>,
        faint_text_alpha: Option<f32>,
        cwd: Option<String>,
        modify_other_keys: Option<u32>,
        client_id: Option<String>,
        readonly: Option<bool>,
    ) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen,
            session_id,
            theme,
            badge,
            faint_text_alpha,
            cwd,
            modify_other_keys,
            client_id,
            readonly,
        }
    }

    /// Create a new refresh message with screen content
    pub fn refresh(cols: u16, rows: u16, screen_content: String) -> Self {
        Self::Refresh {
            cols,
            rows,
            screen_content,
        }
    }

    /// Create a new error message
    pub fn error(message: String) -> Self {
        Self::Error {
            message,
            code: None,
        }
    }

    /// Create a new error message with code
    pub fn error_with_code(message: String, code: String) -> Self {
        Self::Error {
            message,
            code: Some(code),
        }
    }

    /// Create a new cursor position message
    pub fn cursor(col: u16, row: u16, visible: bool) -> Self {
        Self::CursorPosition { col, row, visible }
    }

    /// Create a bell event message
    pub fn bell() -> Self {
        Self::Bell
    }

    /// Create a shutdown message
    pub fn shutdown(reason: String) -> Self {
        Self::Shutdown { reason }
    }

    /// Create a CWD changed message
    pub fn cwd_changed(new_cwd: String) -> Self {
        Self::CwdChanged {
            old_cwd: None,
            new_cwd,
            hostname: None,
            username: None,
            timestamp: None,
        }
    }

    /// Create a fully-specified CWD changed message
    pub fn cwd_changed_full(
        old_cwd: Option<String>,
        new_cwd: String,
        hostname: Option<String>,
        username: Option<String>,
        timestamp: u64,
    ) -> Self {
        Self::CwdChanged {
            old_cwd,
            new_cwd,
            hostname,
            username,
            timestamp: Some(timestamp),
        }
    }

    /// Create a trigger matched message
    pub fn trigger_matched(
        trigger_id: u64,
        row: u16,
        col: u16,
        end_col: u16,
        text: String,
        captures: Vec<String>,
        timestamp: u64,
    ) -> Self {
        Self::TriggerMatched {
            trigger_id,
            row,
            col,
            end_col,
            text,
            captures,
            timestamp,
        }
    }

    /// Create an action notify message
    pub fn action_notify(trigger_id: u64, title: String, message: String) -> Self {
        Self::ActionNotify {
            trigger_id,
            title,
            message,
        }
    }

    /// Create an action mark line message
    pub fn action_mark_line(
        trigger_id: u64,
        row: u16,
        label: Option<String>,
        color: Option<(u8, u8, u8)>,
    ) -> Self {
        Self::ActionMarkLine {
            trigger_id,
            row,
            label,
            color,
        }
    }

    /// Create a pong message (keepalive response)
    pub fn pong() -> Self {
        Self::Pong
    }

    /// Create a mode changed message
    pub fn mode_changed(mode: String, enabled: bool) -> Self {
        Self::ModeChanged { mode, enabled }
    }

    /// Create a graphics added message
    pub fn graphics_added(row: u16) -> Self {
        Self::GraphicsAdded { row, format: None }
    }

    /// Create a graphics added message with format
    pub fn graphics_added_with_format(row: u16, format: String) -> Self {
        Self::GraphicsAdded {
            row,
            format: Some(format),
        }
    }

    /// Create a hyperlink added message
    pub fn hyperlink_added(url: String, row: u16, col: u16) -> Self {
        Self::HyperlinkAdded {
            url,
            row,
            col,
            id: None,
        }
    }

    /// Create a hyperlink added message with ID
    pub fn hyperlink_added_with_id(url: String, row: u16, col: u16, id: String) -> Self {
        Self::HyperlinkAdded {
            url,
            row,
            col,
            id: Some(id),
        }
    }
}

impl ClientMessage {
    /// Create a new input message
    pub fn input(data: String) -> Self {
        Self::Input { data }
    }

    /// Create a new resize message
    pub fn resize(cols: u16, rows: u16) -> Self {
        Self::Resize { cols, rows }
    }

    /// Create a ping message
    pub fn ping() -> Self {
        Self::Ping
    }

    /// Create a refresh request message
    pub fn request_refresh() -> Self {
        Self::RequestRefresh
    }

    /// Create a subscribe message
    pub fn subscribe(events: Vec<EventType>) -> Self {
        Self::Subscribe { events }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_message_output_serialization() {
        let msg = ServerMessage::output("Hello, World!".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"output"#));
        assert!(json.contains(r#""data":"Hello, World!"#));

        // Deserialize back
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        match deserialized {
            ServerMessage::Output { data, timestamp } => {
                assert_eq!(data, "Hello, World!");
                assert_eq!(timestamp, None);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_server_message_resize_serialization() {
        let msg = ServerMessage::resize(80, 24);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"resize"#));
        assert!(json.contains(r#""cols":80"#));
        assert!(json.contains(r#""rows":24"#));
    }

    #[test]
    fn test_client_message_input_serialization() {
        let msg = ClientMessage::input("ls\n".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"input"#));
        assert!(json.contains(r#""data":"ls\n"#));

        // Deserialize back
        let deserialized: ClientMessage = serde_json::from_str(&json).unwrap();
        match deserialized {
            ClientMessage::Input { data } => {
                assert_eq!(data, "ls\n");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_client_message_resize_serialization() {
        let msg = ClientMessage::resize(100, 30);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"resize"#));
        assert!(json.contains(r#""cols":100"#));
        assert!(json.contains(r#""rows":30"#));
    }

    #[test]
    fn test_server_message_error_serialization() {
        let msg = ServerMessage::error("Something went wrong".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"error"#));
        assert!(json.contains(r#""message":"Something went wrong"#));
    }

    #[test]
    fn test_server_message_connected_serialization() {
        let msg = ServerMessage::connected(80, 24, "session-123".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"connected"#));
        assert!(json.contains(r#""session_id":"session-123"#));
        assert!(!json.contains(r#""initial_screen"#)); // Should be omitted when None
    }

    #[test]
    fn test_event_type_serialization() {
        let events = vec![EventType::Output, EventType::Bell];
        let json = serde_json::to_string(&events).unwrap();
        assert!(json.contains(r#""output"#));
        assert!(json.contains(r#""bell"#));
    }

    #[test]
    fn test_theme_info_serialization() {
        let theme = ThemeInfo {
            name: "test-theme".to_string(),
            background: (0, 0, 0),
            foreground: (255, 255, 255),
            normal: [
                (0, 0, 0),
                (255, 0, 0),
                (0, 255, 0),
                (255, 255, 0),
                (0, 0, 255),
                (255, 0, 255),
                (0, 255, 255),
                (255, 255, 255),
            ],
            bright: [
                (128, 128, 128),
                (255, 128, 128),
                (128, 255, 128),
                (255, 255, 128),
                (128, 128, 255),
                (255, 128, 255),
                (128, 255, 255),
                (255, 255, 255),
            ],
        };

        let json = serde_json::to_string(&theme).unwrap();
        assert!(json.contains(r#""name":"test-theme"#));
        assert!(json.contains(r#""background":[0,0,0]"#));
        assert!(json.contains(r#""foreground":[255,255,255]"#));

        // Deserialize back
        let deserialized: ThemeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test-theme");
        assert_eq!(deserialized.background, (0, 0, 0));
        assert_eq!(deserialized.foreground, (255, 255, 255));
    }

    #[test]
    fn test_connected_message_with_theme() {
        let theme = ThemeInfo {
            name: "test-theme".to_string(),
            background: (0, 0, 0),
            foreground: (255, 255, 255),
            normal: [
                (0, 0, 0),
                (255, 0, 0),
                (0, 255, 0),
                (255, 255, 0),
                (0, 0, 255),
                (255, 0, 255),
                (0, 255, 255),
                (255, 255, 255),
            ],
            bright: [
                (128, 128, 128),
                (255, 128, 128),
                (128, 255, 128),
                (255, 255, 128),
                (128, 128, 255),
                (255, 128, 255),
                (128, 255, 255),
                (255, 255, 255),
            ],
        };

        let msg = ServerMessage::connected_with_theme(80, 24, "session-123".to_string(), theme);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"connected"#));
        assert!(json.contains(r#""session_id":"session-123"#));
        assert!(json.contains(r#""theme":{"#));
        assert!(json.contains(r#""name":"test-theme"#));
    }

    #[test]
    fn test_connected_full_serialization() {
        let msg = ServerMessage::connected_full(
            120,
            40,
            None,
            "session-full".to_string(),
            None,
            Some("mybadge".to_string()),
            Some(0.5),
            Some("/home/user".to_string()),
            Some(2),
            None,
            None,
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""badge":"mybadge"#));
        assert!(json.contains(r#""faint_text_alpha":0.5"#));
        assert!(json.contains(r#""cwd":"/home/user"#));
        assert!(json.contains(r#""modify_other_keys":2"#));
    }

    #[test]
    fn test_cwd_changed_serialization() {
        let msg = ServerMessage::cwd_changed_full(
            Some("/old/dir".to_string()),
            "/new/dir".to_string(),
            Some("myhost".to_string()),
            Some("user".to_string()),
            1234567890,
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"cwdchanged"#));
        assert!(json.contains(r#""new_cwd":"/new/dir"#));
        assert!(json.contains(r#""old_cwd":"/old/dir"#));
    }

    #[test]
    fn test_trigger_matched_serialization() {
        let msg = ServerMessage::trigger_matched(
            42,
            10,
            5,
            15,
            "matched text".to_string(),
            vec!["group1".to_string()],
            9999999,
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"triggermatched"#));
        assert!(json.contains(r#""trigger_id":42"#));
        assert!(json.contains(r#""text":"matched text"#));
    }

    #[test]
    fn test_event_type_cwd_trigger_serialization() {
        let events = vec![EventType::Cwd, EventType::Trigger];
        let json = serde_json::to_string(&events).unwrap();
        assert!(json.contains(r#""cwd"#));
        assert!(json.contains(r#""trigger"#));
    }
}
