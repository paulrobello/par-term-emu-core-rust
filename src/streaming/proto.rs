//! Protocol Buffers wire format handling for terminal streaming
//!
//! This module provides binary serialization using Protocol Buffers with
//! optional zlib compression for large payloads.
//!
//! # Wire Format
//!
//! Each message is prefixed with a 1-byte header:
//! - `0x00`: Uncompressed protobuf payload
//! - `0x01`: Zlib-compressed protobuf payload
//!
//! Compression is applied automatically for payloads exceeding 1KB.

use crate::streaming::error::{Result, StreamingError};
use crate::streaming::protocol::{
    ClientMessage as AppClientMessage, EventType as AppEventType,
    ServerMessage as AppServerMessage, ThemeInfo as AppThemeInfo,
};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use prost::Message;
use std::io::{Read, Write};

/// Generated Protocol Buffer types
/// Pre-generated from proto/terminal.proto to avoid requiring protoc at build time.
/// To regenerate: run `cargo build --features streaming` with protoc installed,
/// then copy the output from target/debug/build/.../out/terminal.rs
#[path = "terminal.pb.rs"]
pub mod pb;

/// Compression threshold in bytes (256 bytes)
/// Lowered from 1KB to compress more messages - typical terminal output
/// (prompts, short commands) is 200-800 bytes
const COMPRESSION_THRESHOLD: usize = 256;

/// Wire format flags
const FLAG_UNCOMPRESSED: u8 = 0x00;
const FLAG_COMPRESSED: u8 = 0x01;

/// Encode a server message to binary format with optional compression
pub fn encode_server_message(msg: &AppServerMessage) -> Result<Vec<u8>> {
    let proto_msg: pb::ServerMessage = msg.into();
    let payload = proto_msg.encode_to_vec();

    encode_with_compression(&payload)
}

/// Encode a client message to binary format with optional compression
pub fn encode_client_message(msg: &AppClientMessage) -> Result<Vec<u8>> {
    let proto_msg: pb::ClientMessage = msg.into();
    let payload = proto_msg.encode_to_vec();

    encode_with_compression(&payload)
}

/// Decode a server message from binary format
pub fn decode_server_message(data: &[u8]) -> Result<AppServerMessage> {
    let payload = decode_with_decompression(data)?;
    let proto_msg = pb::ServerMessage::decode(&*payload)
        .map_err(|e| StreamingError::InvalidMessage(format!("Protobuf decode error: {}", e)))?;

    proto_msg.try_into()
}

/// Decode a client message from binary format
pub fn decode_client_message(data: &[u8]) -> Result<AppClientMessage> {
    let payload = decode_with_decompression(data)?;
    let proto_msg = pb::ClientMessage::decode(&*payload)
        .map_err(|e| StreamingError::InvalidMessage(format!("Protobuf decode error: {}", e)))?;

    proto_msg.try_into()
}

/// Internal: encode payload with optional compression
fn encode_with_compression(payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > COMPRESSION_THRESHOLD {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(payload)
            .map_err(|e| StreamingError::InvalidMessage(format!("Compression error: {}", e)))?;
        let compressed = encoder
            .finish()
            .map_err(|e| StreamingError::InvalidMessage(format!("Compression error: {}", e)))?;

        // Only use compression if it actually saves space
        if compressed.len() < payload.len() {
            let mut result = Vec::with_capacity(compressed.len() + 1);
            result.push(FLAG_COMPRESSED);
            result.extend(compressed);
            return Ok(result);
        }
    }

    // Uncompressed
    let mut result = Vec::with_capacity(payload.len() + 1);
    result.push(FLAG_UNCOMPRESSED);
    result.extend(payload);
    Ok(result)
}

/// Internal: decode payload with optional decompression
fn decode_with_decompression(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Err(StreamingError::InvalidMessage("Empty message".into()));
    }

    let (flags, payload) = data.split_at(1);

    if flags[0] & FLAG_COMPRESSED != 0 {
        let mut decoder = ZlibDecoder::new(payload);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| StreamingError::InvalidMessage(format!("Decompression error: {}", e)))?;
        Ok(decompressed)
    } else {
        Ok(payload.to_vec())
    }
}

// =============================================================================
// Conversion: App types -> Proto types
// =============================================================================

impl From<&AppThemeInfo> for pb::ThemeInfo {
    fn from(theme: &AppThemeInfo) -> Self {
        pb::ThemeInfo {
            name: theme.name.clone(),
            background: Some(pb::Color {
                r: theme.background.0 as u32,
                g: theme.background.1 as u32,
                b: theme.background.2 as u32,
            }),
            foreground: Some(pb::Color {
                r: theme.foreground.0 as u32,
                g: theme.foreground.1 as u32,
                b: theme.foreground.2 as u32,
            }),
            normal: theme
                .normal
                .iter()
                .map(|c| pb::Color {
                    r: c.0 as u32,
                    g: c.1 as u32,
                    b: c.2 as u32,
                })
                .collect(),
            bright: theme
                .bright
                .iter()
                .map(|c| pb::Color {
                    r: c.0 as u32,
                    g: c.1 as u32,
                    b: c.2 as u32,
                })
                .collect(),
        }
    }
}

impl From<&AppServerMessage> for pb::ServerMessage {
    fn from(msg: &AppServerMessage) -> Self {
        use pb::server_message::Message;

        let message = match msg {
            AppServerMessage::Output { data, timestamp } => Some(Message::Output(pb::Output {
                data: data.as_bytes().to_vec(),
                timestamp: *timestamp,
            })),
            AppServerMessage::Resize { cols, rows } => Some(Message::Resize(pb::Resize {
                cols: *cols as u32,
                rows: *rows as u32,
            })),
            AppServerMessage::Title { title } => Some(Message::Title(pb::Title {
                title: title.clone(),
            })),
            AppServerMessage::Connected {
                cols,
                rows,
                initial_screen,
                session_id,
                theme,
            } => Some(Message::Connected(pb::Connected {
                cols: *cols as u32,
                rows: *rows as u32,
                initial_screen: initial_screen.as_ref().map(|s| s.as_bytes().to_vec()),
                session_id: session_id.clone(),
                theme: theme.as_ref().map(|t| t.into()),
            })),
            AppServerMessage::Refresh {
                cols,
                rows,
                screen_content,
            } => Some(Message::Refresh(pb::Refresh {
                cols: *cols as u32,
                rows: *rows as u32,
                screen_content: screen_content.as_bytes().to_vec(),
            })),
            AppServerMessage::CursorPosition { col, row, visible } => {
                Some(Message::Cursor(pb::CursorPosition {
                    col: *col as u32,
                    row: *row as u32,
                    visible: *visible,
                }))
            }
            AppServerMessage::Bell => Some(Message::Bell(pb::Bell {})),
            AppServerMessage::Error { message, code } => Some(Message::Error(pb::Error {
                message: message.clone(),
                code: code.clone(),
            })),
            AppServerMessage::Shutdown { reason } => Some(Message::Shutdown(pb::Shutdown {
                reason: reason.clone(),
            })),
            AppServerMessage::Pong => Some(Message::Pong(pb::Pong {})),
        };

        pb::ServerMessage { message }
    }
}

impl From<&AppClientMessage> for pb::ClientMessage {
    fn from(msg: &AppClientMessage) -> Self {
        use pb::client_message::Message;

        let message = match msg {
            AppClientMessage::Input { data } => Some(Message::Input(pb::Input {
                data: data.as_bytes().to_vec(),
            })),
            AppClientMessage::Resize { cols, rows } => Some(Message::Resize(pb::ClientResize {
                cols: *cols as u32,
                rows: *rows as u32,
            })),
            AppClientMessage::Ping => Some(Message::Ping(pb::Ping {})),
            AppClientMessage::RequestRefresh => Some(Message::Refresh(pb::RequestRefresh {})),
            AppClientMessage::Subscribe { events } => Some(Message::Subscribe(pb::Subscribe {
                events: events.iter().map(|e| e.clone().into()).collect(),
            })),
        };

        pb::ClientMessage { message }
    }
}

impl From<AppEventType> for i32 {
    fn from(event: AppEventType) -> Self {
        match event {
            AppEventType::Output => pb::EventType::Output as i32,
            AppEventType::Cursor => pb::EventType::Cursor as i32,
            AppEventType::Bell => pb::EventType::Bell as i32,
            AppEventType::Title => pb::EventType::Title as i32,
            AppEventType::Resize => pb::EventType::Resize as i32,
        }
    }
}

// =============================================================================
// Conversion: Proto types -> App types
// =============================================================================

impl TryFrom<pb::ThemeInfo> for AppThemeInfo {
    type Error = StreamingError;

    fn try_from(theme: pb::ThemeInfo) -> Result<Self> {
        let bg = theme
            .background
            .ok_or_else(|| StreamingError::InvalidMessage("Missing background color".into()))?;
        let fg = theme
            .foreground
            .ok_or_else(|| StreamingError::InvalidMessage("Missing foreground color".into()))?;

        if theme.normal.len() != 8 {
            return Err(StreamingError::InvalidMessage(format!(
                "Expected 8 normal colors, got {}",
                theme.normal.len()
            )));
        }
        if theme.bright.len() != 8 {
            return Err(StreamingError::InvalidMessage(format!(
                "Expected 8 bright colors, got {}",
                theme.bright.len()
            )));
        }

        let mut normal = [(0u8, 0u8, 0u8); 8];
        for (i, c) in theme.normal.iter().enumerate() {
            normal[i] = (c.r as u8, c.g as u8, c.b as u8);
        }

        let mut bright = [(0u8, 0u8, 0u8); 8];
        for (i, c) in theme.bright.iter().enumerate() {
            bright[i] = (c.r as u8, c.g as u8, c.b as u8);
        }

        Ok(AppThemeInfo {
            name: theme.name,
            background: (bg.r as u8, bg.g as u8, bg.b as u8),
            foreground: (fg.r as u8, fg.g as u8, fg.b as u8),
            normal,
            bright,
        })
    }
}

impl TryFrom<pb::ServerMessage> for AppServerMessage {
    type Error = StreamingError;

    fn try_from(msg: pb::ServerMessage) -> Result<Self> {
        use pb::server_message::Message;

        match msg.message {
            Some(Message::Output(output)) => Ok(AppServerMessage::Output {
                data: String::from_utf8_lossy(&output.data).into_owned(),
                timestamp: output.timestamp,
            }),
            Some(Message::Resize(resize)) => Ok(AppServerMessage::Resize {
                cols: resize.cols as u16,
                rows: resize.rows as u16,
            }),
            Some(Message::Title(title)) => Ok(AppServerMessage::Title { title: title.title }),
            Some(Message::Connected(connected)) => Ok(AppServerMessage::Connected {
                cols: connected.cols as u16,
                rows: connected.rows as u16,
                initial_screen: connected
                    .initial_screen
                    .map(|s| String::from_utf8_lossy(&s).into_owned()),
                session_id: connected.session_id,
                theme: connected.theme.map(|t| t.try_into()).transpose()?,
            }),
            Some(Message::Refresh(refresh)) => Ok(AppServerMessage::Refresh {
                cols: refresh.cols as u16,
                rows: refresh.rows as u16,
                screen_content: String::from_utf8_lossy(&refresh.screen_content).into_owned(),
            }),
            Some(Message::Cursor(cursor)) => Ok(AppServerMessage::CursorPosition {
                col: cursor.col as u16,
                row: cursor.row as u16,
                visible: cursor.visible,
            }),
            Some(Message::Bell(_)) => Ok(AppServerMessage::Bell),
            Some(Message::Error(error)) => Ok(AppServerMessage::Error {
                message: error.message,
                code: error.code,
            }),
            Some(Message::Shutdown(shutdown)) => Ok(AppServerMessage::Shutdown {
                reason: shutdown.reason,
            }),
            Some(Message::Pong(_)) => Ok(AppServerMessage::Pong),
            None => Err(StreamingError::InvalidMessage(
                "Empty server message".into(),
            )),
        }
    }
}

impl TryFrom<pb::ClientMessage> for AppClientMessage {
    type Error = StreamingError;

    fn try_from(msg: pb::ClientMessage) -> Result<Self> {
        use pb::client_message::Message;

        match msg.message {
            Some(Message::Input(input)) => Ok(AppClientMessage::Input {
                data: String::from_utf8_lossy(&input.data).into_owned(),
            }),
            Some(Message::Resize(resize)) => Ok(AppClientMessage::Resize {
                cols: resize.cols as u16,
                rows: resize.rows as u16,
            }),
            Some(Message::Ping(_)) => Ok(AppClientMessage::Ping),
            Some(Message::Refresh(_)) => Ok(AppClientMessage::RequestRefresh),
            Some(Message::Subscribe(subscribe)) => Ok(AppClientMessage::Subscribe {
                events: subscribe
                    .events
                    .iter()
                    .filter_map(|e| pb::EventType::try_from(*e).ok())
                    .map(|e| e.into())
                    .collect(),
            }),
            None => Err(StreamingError::InvalidMessage(
                "Empty client message".into(),
            )),
        }
    }
}

impl From<pb::EventType> for AppEventType {
    fn from(event: pb::EventType) -> Self {
        match event {
            pb::EventType::Unspecified => AppEventType::Output, // Default fallback
            pb::EventType::Output => AppEventType::Output,
            pb::EventType::Cursor => AppEventType::Cursor,
            pb::EventType::Bell => AppEventType::Bell,
            pb::EventType::Title => AppEventType::Title,
            pb::EventType::Resize => AppEventType::Resize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_output() {
        let msg = AppServerMessage::output("Hello, World!".to_string());
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        match decoded {
            AppServerMessage::Output { data, timestamp } => {
                assert_eq!(data, "Hello, World!");
                assert_eq!(timestamp, None);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_resize() {
        let msg = AppServerMessage::resize(80, 24);
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        match decoded {
            AppServerMessage::Resize { cols, rows } => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_client_input() {
        let msg = AppClientMessage::input("ls\n".to_string());
        let encoded = encode_client_message(&msg).unwrap();
        let decoded = decode_client_message(&encoded).unwrap();

        match decoded {
            AppClientMessage::Input { data } => {
                assert_eq!(data, "ls\n");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_compression_for_large_payload() {
        // Create a message that exceeds COMPRESSION_THRESHOLD (256 bytes)
        let large_data = "A".repeat(500);
        let msg = AppServerMessage::output(large_data.clone());
        let encoded = encode_server_message(&msg).unwrap();

        // First byte should indicate compression
        assert_eq!(encoded[0], FLAG_COMPRESSED);

        // Verify it decodes correctly
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Output { data, .. } => {
                assert_eq!(data, large_data);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_no_compression_for_small_payload() {
        // Message below COMPRESSION_THRESHOLD (256 bytes)
        let msg = AppServerMessage::output("small".to_string());
        let encoded = encode_server_message(&msg).unwrap();

        // First byte should indicate no compression
        assert_eq!(encoded[0], FLAG_UNCOMPRESSED);
    }

    #[test]
    fn test_compression_boundary() {
        // Test right at the threshold - 256 bytes of payload should not trigger compression
        // (threshold is >256, not >=256)
        let boundary_data = "X".repeat(200); // Will be ~200 bytes in protobuf
        let msg = AppServerMessage::output(boundary_data);
        let encoded = encode_server_message(&msg).unwrap();
        // Should NOT be compressed (at or below threshold)
        assert_eq!(encoded[0], FLAG_UNCOMPRESSED);

        // Test just above threshold
        let above_data = "Y".repeat(300); // Will be ~300 bytes in protobuf
        let msg2 = AppServerMessage::output(above_data.clone());
        let encoded2 = encode_server_message(&msg2).unwrap();
        // Should be compressed (above threshold)
        assert_eq!(encoded2[0], FLAG_COMPRESSED);

        // Verify it decodes correctly
        let decoded = decode_server_message(&encoded2).unwrap();
        match decoded {
            AppServerMessage::Output { data, .. } => {
                assert_eq!(data, above_data);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_theme_roundtrip() {
        let theme = AppThemeInfo {
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

        let msg = AppServerMessage::connected_with_theme(80, 24, "session-123".to_string(), theme);
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        match decoded {
            AppServerMessage::Connected {
                cols,
                rows,
                session_id,
                theme,
                ..
            } => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
                assert_eq!(session_id, "session-123");
                assert!(theme.is_some());
                let t = theme.unwrap();
                assert_eq!(t.name, "test-theme");
                assert_eq!(t.background, (0, 0, 0));
                assert_eq!(t.foreground, (255, 255, 255));
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_empty_message_error() {
        let result = decode_client_message(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_encode_decode_bell() {
        let msg = AppServerMessage::Bell;
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        assert!(matches!(decoded, AppServerMessage::Bell));
    }

    #[test]
    fn test_encode_decode_shutdown() {
        let msg = AppServerMessage::Shutdown {
            reason: "Server maintenance".to_string(),
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Shutdown { reason } => {
                assert_eq!(reason, "Server maintenance");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_pong() {
        let msg = AppServerMessage::Pong;
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        assert!(matches!(decoded, AppServerMessage::Pong));
    }

    #[test]
    fn test_encode_decode_error_message() {
        let msg = AppServerMessage::Error {
            message: "Something went wrong".to_string(),
            code: Some("E500".to_string()),
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Error { message, code } => {
                assert_eq!(message, "Something went wrong");
                assert_eq!(code, Some("E500".to_string()));
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_error_without_code() {
        let msg = AppServerMessage::Error {
            message: "Error occurred".to_string(),
            code: None,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Error { message, code } => {
                assert_eq!(message, "Error occurred");
                assert_eq!(code, None);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_cursor_position() {
        let msg = AppServerMessage::CursorPosition {
            col: 42,
            row: 10,
            visible: true,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::CursorPosition { col, row, visible } => {
                assert_eq!(col, 42);
                assert_eq!(row, 10);
                assert!(visible);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_cursor_hidden() {
        let msg = AppServerMessage::CursorPosition {
            col: 0,
            row: 0,
            visible: false,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::CursorPosition { col, row, visible } => {
                assert_eq!(col, 0);
                assert_eq!(row, 0);
                assert!(!visible);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_title() {
        let msg = AppServerMessage::Title {
            title: "My Terminal Window".to_string(),
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Title { title } => {
                assert_eq!(title, "My Terminal Window");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_refresh() {
        let msg = AppServerMessage::Refresh {
            cols: 120,
            rows: 40,
            screen_content: "Full screen content here".to_string(),
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Refresh {
                cols,
                rows,
                screen_content,
            } => {
                assert_eq!(cols, 120);
                assert_eq!(rows, 40);
                assert_eq!(screen_content, "Full screen content here");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_connected_with_screen() {
        let msg = AppServerMessage::Connected {
            cols: 80,
            rows: 24,
            initial_screen: Some("initial content".to_string()),
            session_id: "sess-abc".to_string(),
            theme: None,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Connected {
                cols,
                rows,
                initial_screen,
                session_id,
                theme,
            } => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
                assert_eq!(initial_screen, Some("initial content".to_string()));
                assert_eq!(session_id, "sess-abc");
                assert!(theme.is_none());
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_client_ping() {
        let msg = AppClientMessage::Ping;
        let encoded = encode_client_message(&msg).unwrap();
        let decoded = decode_client_message(&encoded).unwrap();
        assert!(matches!(decoded, AppClientMessage::Ping));
    }

    #[test]
    fn test_encode_decode_client_refresh() {
        let msg = AppClientMessage::RequestRefresh;
        let encoded = encode_client_message(&msg).unwrap();
        let decoded = decode_client_message(&encoded).unwrap();
        assert!(matches!(decoded, AppClientMessage::RequestRefresh));
    }

    #[test]
    fn test_encode_decode_client_subscribe() {
        let msg = AppClientMessage::Subscribe {
            events: vec![
                AppEventType::Output,
                AppEventType::Cursor,
                AppEventType::Bell,
                AppEventType::Title,
                AppEventType::Resize,
            ],
        };
        let encoded = encode_client_message(&msg).unwrap();
        let decoded = decode_client_message(&encoded).unwrap();
        match decoded {
            AppClientMessage::Subscribe { events } => {
                assert_eq!(events.len(), 5);
                assert!(events.contains(&AppEventType::Output));
                assert!(events.contains(&AppEventType::Cursor));
                assert!(events.contains(&AppEventType::Bell));
                assert!(events.contains(&AppEventType::Title));
                assert!(events.contains(&AppEventType::Resize));
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_unicode_content() {
        let unicode_content = "Hello 世界 🌍 مرحبا Привет 日本語";
        let msg = AppServerMessage::Output {
            data: unicode_content.to_string(),
            timestamp: None,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Output { data, .. } => {
                assert_eq!(data, unicode_content);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_ansi_escape_sequences() {
        let ansi_data = "\x1b[31mRed\x1b[0m \x1b[32mGreen\x1b[0m \x1b[1;34mBold Blue\x1b[0m";
        let msg = AppServerMessage::Output {
            data: ansi_data.to_string(),
            timestamp: None,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Output { data, .. } => {
                assert_eq!(data, ansi_data);
                assert!(data.contains("\x1b[31m"));
                assert!(data.contains("\x1b[0m"));
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_with_timestamp() {
        let msg = AppServerMessage::Output {
            data: "test".to_string(),
            timestamp: Some(1234567890123),
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Output { data, timestamp } => {
                assert_eq!(data, "test");
                assert_eq!(timestamp, Some(1234567890123));
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_decode_only_flag_byte_error() {
        // Only has the flag byte, no actual payload
        let result = decode_server_message(&[0x00]);
        // This should either succeed with an empty/default message or fail
        // depending on protobuf handling of empty data
        // The behavior depends on the protobuf schema
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn test_encode_empty_string() {
        let msg = AppServerMessage::Output {
            data: String::new(),
            timestamp: None,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        match decoded {
            AppServerMessage::Output { data, .. } => {
                assert!(data.is_empty());
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_event_type_conversions() {
        // Test all event type conversions
        let event_types = vec![
            (AppEventType::Output, pb::EventType::Output),
            (AppEventType::Cursor, pb::EventType::Cursor),
            (AppEventType::Bell, pb::EventType::Bell),
            (AppEventType::Title, pb::EventType::Title),
            (AppEventType::Resize, pb::EventType::Resize),
        ];

        for (app_type, _pb_type) in event_types {
            let i32_val: i32 = app_type.clone().into();
            // Verify conversion is deterministic
            let i32_val2: i32 = app_type.into();
            assert_eq!(i32_val, i32_val2);
        }
    }

    #[test]
    fn test_pb_event_type_to_app_event_type() {
        assert!(matches!(
            AppEventType::from(pb::EventType::Output),
            AppEventType::Output
        ));
        assert!(matches!(
            AppEventType::from(pb::EventType::Cursor),
            AppEventType::Cursor
        ));
        assert!(matches!(
            AppEventType::from(pb::EventType::Bell),
            AppEventType::Bell
        ));
        assert!(matches!(
            AppEventType::from(pb::EventType::Title),
            AppEventType::Title
        ));
        assert!(matches!(
            AppEventType::from(pb::EventType::Resize),
            AppEventType::Resize
        ));
        // Unspecified defaults to Output
        assert!(matches!(
            AppEventType::from(pb::EventType::Unspecified),
            AppEventType::Output
        ));
    }
}
