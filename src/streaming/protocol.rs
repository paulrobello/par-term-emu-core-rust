//! Protocol definitions for terminal streaming
//!
//! This module defines the message formats used for WebSocket-based
//! terminal streaming between the server and web clients.

use serde::{Deserialize, Serialize};

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
}
