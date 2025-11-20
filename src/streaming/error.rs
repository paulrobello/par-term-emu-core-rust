//! Error types for terminal streaming

use std::fmt;

/// Errors that can occur during terminal streaming operations
#[derive(Debug)]
pub enum StreamingError {
    /// WebSocket error
    WebSocketError(String),

    /// IO error
    IoError(std::io::Error),

    /// Serialization/deserialization error
    SerializationError(serde_json::Error),

    /// Invalid message format
    InvalidMessage(String),

    /// Connection closed
    ConnectionClosed,

    /// Client disconnected
    ClientDisconnected(String),

    /// Server error
    ServerError(String),

    /// Terminal error
    TerminalError(String),

    /// Invalid input
    InvalidInput(String),

    /// Rate limit exceeded
    RateLimitExceeded,

    /// Maximum clients reached
    MaxClientsReached,

    /// Authentication failed
    AuthenticationFailed(String),

    /// Permission denied
    PermissionDenied(String),
}

impl fmt::Display for StreamingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamingError::WebSocketError(msg) => write!(f, "WebSocket error: {}", msg),
            StreamingError::IoError(err) => write!(f, "IO error: {}", err),
            StreamingError::SerializationError(err) => write!(f, "Serialization error: {}", err),
            StreamingError::InvalidMessage(msg) => write!(f, "Invalid message: {}", msg),
            StreamingError::ConnectionClosed => write!(f, "Connection closed"),
            StreamingError::ClientDisconnected(id) => {
                write!(f, "Client disconnected: {}", id)
            }
            StreamingError::ServerError(msg) => write!(f, "Server error: {}", msg),
            StreamingError::TerminalError(msg) => write!(f, "Terminal error: {}", msg),
            StreamingError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            StreamingError::RateLimitExceeded => write!(f, "Rate limit exceeded"),
            StreamingError::MaxClientsReached => write!(f, "Maximum number of clients reached"),
            StreamingError::AuthenticationFailed(msg) => {
                write!(f, "Authentication failed: {}", msg)
            }
            StreamingError::PermissionDenied(msg) => write!(f, "Permission denied: {}", msg),
        }
    }
}

impl std::error::Error for StreamingError {}

impl From<std::io::Error> for StreamingError {
    fn from(err: std::io::Error) -> Self {
        StreamingError::IoError(err)
    }
}

impl From<serde_json::Error> for StreamingError {
    fn from(err: serde_json::Error) -> Self {
        StreamingError::SerializationError(err)
    }
}

#[cfg(feature = "streaming")]
impl From<tokio_tungstenite::tungstenite::Error> for StreamingError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        StreamingError::WebSocketError(err.to_string())
    }
}

/// Result type for streaming operations
pub type Result<T> = std::result::Result<T, StreamingError>;
