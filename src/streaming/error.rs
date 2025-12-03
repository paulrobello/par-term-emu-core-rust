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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_error_display_websocket() {
        let err = StreamingError::WebSocketError("connection failed".to_string());
        assert_eq!(err.to_string(), "WebSocket error: connection failed");
    }

    #[test]
    fn test_streaming_error_display_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = StreamingError::IoError(io_err);
        assert!(err.to_string().contains("IO error"));
    }

    #[test]
    fn test_streaming_error_display_serialization() {
        // Create a JSON error by parsing invalid JSON
        let json_err = serde_json::from_str::<serde_json::Value>("invalid").unwrap_err();
        let err = StreamingError::SerializationError(json_err);
        assert!(err.to_string().contains("Serialization error"));
    }

    #[test]
    fn test_streaming_error_display_invalid_message() {
        let err = StreamingError::InvalidMessage("bad format".to_string());
        assert_eq!(err.to_string(), "Invalid message: bad format");
    }

    #[test]
    fn test_streaming_error_display_connection_closed() {
        let err = StreamingError::ConnectionClosed;
        assert_eq!(err.to_string(), "Connection closed");
    }

    #[test]
    fn test_streaming_error_display_client_disconnected() {
        let err = StreamingError::ClientDisconnected("client-123".to_string());
        assert_eq!(err.to_string(), "Client disconnected: client-123");
    }

    #[test]
    fn test_streaming_error_display_server_error() {
        let err = StreamingError::ServerError("internal error".to_string());
        assert_eq!(err.to_string(), "Server error: internal error");
    }

    #[test]
    fn test_streaming_error_display_terminal_error() {
        let err = StreamingError::TerminalError("terminal locked".to_string());
        assert_eq!(err.to_string(), "Terminal error: terminal locked");
    }

    #[test]
    fn test_streaming_error_display_invalid_input() {
        let err = StreamingError::InvalidInput("invalid chars".to_string());
        assert_eq!(err.to_string(), "Invalid input: invalid chars");
    }

    #[test]
    fn test_streaming_error_display_rate_limit() {
        let err = StreamingError::RateLimitExceeded;
        assert_eq!(err.to_string(), "Rate limit exceeded");
    }

    #[test]
    fn test_streaming_error_display_max_clients() {
        let err = StreamingError::MaxClientsReached;
        assert_eq!(err.to_string(), "Maximum number of clients reached");
    }

    #[test]
    fn test_streaming_error_display_auth_failed() {
        let err = StreamingError::AuthenticationFailed("invalid token".to_string());
        assert_eq!(err.to_string(), "Authentication failed: invalid token");
    }

    #[test]
    fn test_streaming_error_display_permission_denied() {
        let err = StreamingError::PermissionDenied("read only".to_string());
        assert_eq!(err.to_string(), "Permission denied: read only");
    }

    #[test]
    fn test_streaming_error_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe broken");
        let err: StreamingError = io_err.into();
        match err {
            StreamingError::IoError(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe);
            }
            _ => panic!("Expected IoError variant"),
        }
    }

    #[test]
    fn test_streaming_error_from_serde_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("{invalid}").unwrap_err();
        let err: StreamingError = json_err.into();
        match err {
            StreamingError::SerializationError(_) => {}
            _ => panic!("Expected SerializationError variant"),
        }
    }

    #[test]
    fn test_streaming_error_is_std_error() {
        let err = StreamingError::ConnectionClosed;
        let std_err: &dyn std::error::Error = &err;
        assert!(!std_err.to_string().is_empty());
    }

    #[test]
    fn test_streaming_error_debug_impl() {
        let err = StreamingError::WebSocketError("test".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("WebSocketError"));
        assert!(debug_str.contains("test"));
    }
}
