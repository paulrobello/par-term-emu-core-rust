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

    /// Create a pong message (keepalive response)
    pub fn pong() -> Self {
        Self::Pong
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
}
