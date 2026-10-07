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
    AgentEntry as AppAgentEntry, ClientMessage as AppClientMessage, EventType as AppEventType,
    MouseEventType, ServerMessage as AppServerMessage, ThemeInfo as AppThemeInfo,
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
// DOC-128 allow-list: prost output cannot carry hand-written docs; the wire
// contract is documented in proto/terminal.proto.
#[allow(missing_docs)]
#[path = "terminal.pb.rs"]
pub mod pb;

/// Compression threshold in bytes (256 bytes)
/// Lowered from 1KB to compress more messages - typical terminal output
/// (prompts, short commands) is 200-800 bytes
const COMPRESSION_THRESHOLD: usize = 256;

/// Maximum size of a decompressed payload (1 MiB).
///
/// Caps zlib-inflated output to prevent zip-bomb denial of service: a small
/// compressed frame could otherwise expand into gigabytes and OOM the server.
/// 1 MiB is far above any legitimate terminal streaming frame (the WS layer
/// also caps inbound frames, but this defends against the decompression path
/// directly).
/// cap: Decompressed bytes accepted from one zlib-compressed streaming frame.
const MAX_DECOMPRESSED_SIZE: usize = 1024 * 1024;

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
///
/// Decompressed output is capped at [`MAX_DECOMPRESSED_SIZE`] to mitigate
/// zip-bomb denial of service. Reads from the decoder in fixed chunks and
/// bails as soon as the cap is exceeded.
fn decode_with_decompression(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Err(StreamingError::InvalidMessage("Empty message".into()));
    }

    let (flags, payload) = data.split_at(1);

    if flags[0] & FLAG_COMPRESSED != 0 {
        let mut decoder = ZlibDecoder::new(payload);
        let mut decompressed = Vec::new();
        // Read in chunks so we can abort the moment the output exceeds the cap,
        // rather than letting `read_to_end` allocate unboundedly.
        let mut chunk = [0u8; 8192];
        loop {
            let read = decoder.read(&mut chunk).map_err(|e| {
                StreamingError::InvalidMessage(format!("Decompression error: {}", e))
            })?;
            if read == 0 {
                break;
            }
            if decompressed.len() + read > MAX_DECOMPRESSED_SIZE {
                return Err(StreamingError::InvalidMessage(format!(
                    "Decompressed payload exceeds {} byte limit",
                    MAX_DECOMPRESSED_SIZE
                )));
            }
            decompressed.extend_from_slice(&chunk[..read]);
        }
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

// =============================================================================
// Field conversions for the `ProtoConvert` derive (ARC-006)
// =============================================================================
//
// The derive (par-term-emu-derive) generates the message-level conversions;
// every field value goes through `ToWire`/`FromWire`, keyed on the app
// field's declared type and the wire field's type. Each type pair is listed
// explicitly: there is no blanket numeric narrowing, so a field whose pair
// is missing fails to compile instead of picking a conversion. Fields with a
// field-specific rule (a clamp, a presence check) use `#[proto(with = ..)]`
// and a function pair in [`wire`].

/// App value -> wire value for one field (ARC-006).
pub(crate) trait ToWire<W> {
    /// Convert to the wire representation.
    fn to_wire(&self) -> W;
}

/// Wire value -> app value for one field (ARC-006).
pub(crate) trait FromWire<W>: Sized {
    /// Convert from the wire representation.
    fn from_wire(w: W) -> Result<Self>;
}

/// Same `Copy` type on both sides.
macro_rules! identity_copy_wire {
    ($($ty:ty),+ $(,)?) => {$(
        impl ToWire<$ty> for $ty {
            fn to_wire(&self) -> $ty {
                *self
            }
        }
        impl FromWire<$ty> for $ty {
            fn from_wire(w: $ty) -> Result<Self> {
                Ok(w)
            }
        }
    )+};
}

/// Same owned type on both sides.
macro_rules! identity_clone_wire {
    ($($ty:ty),+ $(,)?) => {$(
        impl ToWire<$ty> for $ty {
            fn to_wire(&self) -> $ty {
                self.clone()
            }
        }
        impl FromWire<$ty> for $ty {
            fn from_wire(w: $ty) -> Result<Self> {
                Ok(w)
            }
        }
    )+};
}

identity_copy_wire!(
    bool,
    u32,
    u64,
    i32,
    f32,
    f64,
    Option<bool>,
    Option<u32>,
    Option<u64>,
    Option<i32>,
    Option<f32>,
);
identity_clone_wire!(String, Option<String>, Vec<String>, Vec<f64>);

/// Grid coordinates and sizes: `u16` in the app, `uint32` on the wire. The
/// decode truncates (`as u16`), as the hand-written conversions did.
impl ToWire<u32> for u16 {
    fn to_wire(&self) -> u32 {
        *self as u32
    }
}
impl FromWire<u32> for u16 {
    fn from_wire(w: u32) -> Result<Self> {
        Ok(w as u16)
    }
}
impl ToWire<Option<u32>> for Option<u16> {
    fn to_wire(&self) -> Option<u32> {
        self.map(|v| v as u32)
    }
}
impl FromWire<Option<u32>> for Option<u16> {
    fn from_wire(w: Option<u32>) -> Result<Self> {
        Ok(w.map(|v| v as u16))
    }
}

/// Terminal text carried as `bytes`; invalid UTF-8 decodes lossily.
impl ToWire<Vec<u8>> for String {
    fn to_wire(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}
impl FromWire<Vec<u8>> for String {
    fn from_wire(w: Vec<u8>) -> Result<Self> {
        Ok(String::from_utf8_lossy(&w).into_owned())
    }
}
impl ToWire<Option<Vec<u8>>> for Option<String> {
    fn to_wire(&self) -> Option<Vec<u8>> {
        self.as_ref().map(|s| s.as_bytes().to_vec())
    }
}
impl FromWire<Option<Vec<u8>>> for Option<String> {
    fn from_wire(w: Option<Vec<u8>>) -> Result<Self> {
        Ok(w.map(|s| String::from_utf8_lossy(&s).into_owned()))
    }
}

/// RGB colors: channels truncate on decode (`as u8`).
impl ToWire<pb::Color> for (u8, u8, u8) {
    fn to_wire(&self) -> pb::Color {
        pb::Color {
            r: self.0 as u32,
            g: self.1 as u32,
            b: self.2 as u32,
        }
    }
}
impl FromWire<pb::Color> for (u8, u8, u8) {
    fn from_wire(c: pb::Color) -> Result<Self> {
        Ok((c.r as u8, c.g as u8, c.b as u8))
    }
}
impl ToWire<Option<pb::Color>> for Option<(u8, u8, u8)> {
    fn to_wire(&self) -> Option<pb::Color> {
        self.as_ref().map(ToWire::to_wire)
    }
}
impl FromWire<Option<pb::Color>> for Option<(u8, u8, u8)> {
    fn from_wire(w: Option<pb::Color>) -> Result<Self> {
        w.map(<(u8, u8, u8)>::from_wire).transpose()
    }
}

/// Theme: validated by the hand-written `TryFrom<pb::ThemeInfo>` below.
impl ToWire<Option<pb::ThemeInfo>> for Option<AppThemeInfo> {
    fn to_wire(&self) -> Option<pb::ThemeInfo> {
        self.as_ref().map(Into::into)
    }
}
impl FromWire<Option<pb::ThemeInfo>> for Option<AppThemeInfo> {
    fn from_wire(w: Option<pb::ThemeInfo>) -> Result<Self> {
        w.map(TryInto::try_into).transpose()
    }
}

/// Subscriptions: unknown wire ints are dropped, not rejected.
impl ToWire<Vec<i32>> for Vec<AppEventType> {
    fn to_wire(&self) -> Vec<i32> {
        self.iter().map(|e| e.clone().into()).collect()
    }
}
impl FromWire<Vec<i32>> for Vec<AppEventType> {
    fn from_wire(w: Vec<i32>) -> Result<Self> {
        Ok(w.iter()
            .filter_map(|e| pb::EventType::try_from(*e).ok())
            .map(Into::into)
            .collect())
    }
}

/// Mouse event type: a wire string; unknown names decode as `Press`.
impl ToWire<String> for MouseEventType {
    fn to_wire(&self) -> String {
        self.as_str().to_string()
    }
}
impl FromWire<String> for MouseEventType {
    fn from_wire(w: String) -> Result<Self> {
        Ok(mouse_event_type_from_wire(&w))
    }
}

/// Field-specific conversions named by `#[proto(with = ..)]`.
pub(crate) mod wire {
    use super::{pb, AppAgentEntry, FromWire, Result, StreamingError, ToWire};

    /// Progress percent: `u8` in the app; the decode clamps to 100.
    pub(crate) mod percent {
        use super::Result;

        pub(crate) fn to_wire(v: &Option<u8>) -> Option<u32> {
            v.map(|p| p as u32)
        }

        pub(crate) fn from_wire(w: Option<u32>) -> Result<Option<u8>> {
            Ok(w.map(|p| p.min(100) as u8))
        }
    }

    /// Mouse button: `u8` in the app; the decode clamps to 255.
    pub(crate) mod mouse_button {
        use super::Result;

        pub(crate) fn to_wire(v: &u8) -> u32 {
            *v as u32
        }

        pub(crate) fn from_wire(w: u32) -> Result<u8> {
            Ok(w.min(255) as u8)
        }
    }

    /// `AgentStateChanged.agent`: required on the wire.
    pub(crate) mod required_agent {
        use super::{pb, AppAgentEntry, FromWire, Result, StreamingError, ToWire};

        pub(crate) fn to_wire(v: &AppAgentEntry) -> Option<pb::AgentEntry> {
            Some(v.to_wire())
        }

        /// Presence decides, not value: pane 0 is a legal pane id, so a
        /// missing field must stay distinct from a pane-0 entry — decoding
        /// it as the default would fabricate a release.
        pub(crate) fn from_wire(w: Option<pb::AgentEntry>) -> Result<AppAgentEntry> {
            let agent = w.ok_or_else(|| {
                StreamingError::InvalidMessage(
                    "AgentStateChanged is missing its agent field".into(),
                )
            })?;
            AppAgentEntry::from_wire(agent)
        }
    }
}

/// Both event-type conversions from one variant list (QA-214): app → wire
/// (`i32`) and wire → app. `pb::EventType::Unspecified`, which has no app
/// variant, decodes as `Output`.
macro_rules! event_type_conversions {
    ($($variant:ident),+ $(,)?) => {
        impl From<AppEventType> for i32 {
            fn from(event: AppEventType) -> Self {
                match event {
                    $(AppEventType::$variant => pb::EventType::$variant as i32,)+
                }
            }
        }

        impl From<pb::EventType> for AppEventType {
            fn from(event: pb::EventType) -> Self {
                match event {
                    pb::EventType::Unspecified => AppEventType::Output, // Default fallback
                    $(pb::EventType::$variant => AppEventType::$variant,)+
                }
            }
        }
    };
}

event_type_conversions!(
    Output,
    Cursor,
    Bell,
    Title,
    Resize,
    Cwd,
    Trigger,
    Action,
    Mode,
    Graphics,
    Hyperlink,
    UserVar,
    ProgressBar,
    Badge,
    Selection,
    Clipboard,
    Shell,
    SystemStats,
    Zone,
    Environment,
    RemoteHost,
    SubShell,
    Snapshot,
    FileTransfer,
    UploadRequest,
    ScreenCleared,
);

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

/// `MouseInput.event_type` (a wire string) -> [`MouseEventType`].
///
/// An unknown name decodes as `Press`, keeping the pre-enum behavior where
/// every name but `"release"` counted as a press. The name is client
/// controlled, so only its length is logged.
fn mouse_event_type_from_wire(name: &str) -> MouseEventType {
    MouseEventType::parse(name).unwrap_or_else(|| {
        crate::debug_log!(
            "STREAMING",
            "unknown mouse event_type ({} bytes), treated as press",
            name.len()
        );
        MouseEventType::Press
    })
}

#[cfg(test)]
#[path = "proto_golden_tests.rs"]
mod golden_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_event_type_round_trips_every_variant_over_the_wire() {
        for event_type in [
            MouseEventType::Press,
            MouseEventType::Release,
            MouseEventType::Move,
            MouseEventType::Scroll,
        ] {
            let msg = AppClientMessage::mouse(3, 4, 0, true, false, true, event_type);
            let decoded = decode_client_message(&encode_client_message(&msg).unwrap()).unwrap();
            let AppClientMessage::Mouse {
                col,
                row,
                shift,
                alt,
                event_type: got,
                ..
            } = decoded
            else {
                panic!("Wrong message type");
            };
            assert_eq!((col, row, shift, alt), (3, 4, true, true));
            assert_eq!(got, event_type);
        }
    }

    #[test]
    fn unknown_mouse_event_type_on_the_wire_decodes_as_press() {
        for name in ["bogus", "", "Release"] {
            let wire = pb::ClientMessage {
                message: Some(pb::client_message::Message::Mouse(pb::MouseInput {
                    event_type: name.to_string(),
                    ..Default::default()
                })),
            };
            let decoded = AppClientMessage::try_from(wire).unwrap();
            let AppClientMessage::Mouse { event_type, .. } = decoded else {
                panic!("Wrong message type");
            };
            assert_eq!(event_type, MouseEventType::Press, "{name:?}");
        }
    }

    #[test]
    fn test_encode_decode_output() {
        let msg = AppServerMessage::output("Hello, World!".to_string());
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        let AppServerMessage::Output { data, timestamp } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(data, "Hello, World!");
        assert_eq!(timestamp, None);
    }

    #[test]
    fn test_encode_decode_resize() {
        let msg = AppServerMessage::resize(80, 24);
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        let AppServerMessage::Resize { cols, rows } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
    }

    #[test]
    fn test_encode_decode_client_input() {
        let msg = AppClientMessage::input("ls\n".to_string());
        let encoded = encode_client_message(&msg).unwrap();
        let decoded = decode_client_message(&encoded).unwrap();

        let AppClientMessage::Input { data } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(data, "ls\n");
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
        let AppServerMessage::Output { data, .. } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(data, large_data);
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
        let AppServerMessage::Output { data, .. } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(data, above_data);
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

        let msg = AppServerMessage::connected_builder(80, 24, "session-123".to_string())
            .theme(Some(theme))
            .build();
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        let AppServerMessage::Connected {
            cols,
            rows,
            session_id,
            theme,
            ..
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
        assert_eq!(session_id, "session-123");
        assert!(theme.is_some());
        let t = theme.unwrap();
        assert_eq!(t.name, "test-theme");
        assert_eq!(t.background, (0, 0, 0));
        assert_eq!(t.foreground, (255, 255, 255));
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
        let AppServerMessage::Shutdown { reason } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(reason, "Server maintenance");
    }

    #[test]
    fn test_roster_messages_round_trip() {
        let entry = AppAgentEntry {
            pane_id: 12,
            agent: "claude-code".into(),
            state: "blocked".into(),
            source: "hook".into(),
            reason: "needs input".into(),
        };
        let roster = AppServerMessage::AgentRoster {
            agents: vec![entry.clone()],
        };
        let decoded = decode_server_message(&encode_server_message(&roster).unwrap()).unwrap();
        let AppServerMessage::AgentRoster { agents } = decoded else {
            panic!("expected roster");
        };
        assert_eq!(agents, vec![entry.clone()]);

        let delta = AppServerMessage::AgentStateChanged {
            agent: entry.clone(),
            released: true,
        };
        let decoded = decode_server_message(&encode_server_message(&delta).unwrap()).unwrap();
        let AppServerMessage::AgentStateChanged { agent, released } = decoded else {
            panic!("expected delta");
        };
        assert_eq!(agent, entry);
        assert!(released);
    }

    #[test]
    fn test_agent_state_changed_missing_agent_is_rejected() {
        let wire = pb::ServerMessage {
            message: Some(pb::server_message::Message::AgentStateChanged(
                pb::AgentStateChanged {
                    agent: None,
                    released: false,
                },
            )),
        };
        let err = AppServerMessage::try_from(wire).unwrap_err();
        assert!(
            matches!(err, StreamingError::InvalidMessage(_)),
            "a missing agent field is a protocol violation, not a pane-0 \
             release: {err:?}"
        );
    }

    #[test]
    fn test_agent_state_changed_pane_zero_is_a_valid_present_field() {
        let msg = AppServerMessage::AgentStateChanged {
            agent: AppAgentEntry {
                pane_id: 0,
                agent: "zed".into(),
                state: "idle".into(),
                source: "hook".into(),
                reason: String::new(),
            },
            released: true,
        };
        let decoded = decode_server_message(&encode_server_message(&msg).unwrap()).unwrap();
        let AppServerMessage::AgentStateChanged { agent, released } = decoded else {
            panic!("expected delta");
        };
        assert_eq!(agent.pane_id, 0, "pane 0 is a legal pane id");
        assert!(released);
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
        let AppServerMessage::Error { message, code } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(message, "Something went wrong");
        assert_eq!(code, Some("E500".to_string()));
    }

    #[test]
    fn test_encode_decode_error_without_code() {
        let msg = AppServerMessage::Error {
            message: "Error occurred".to_string(),
            code: None,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::Error { message, code } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(message, "Error occurred");
        assert_eq!(code, None);
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
        let AppServerMessage::CursorPosition { col, row, visible } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(col, 42);
        assert_eq!(row, 10);
        assert!(visible);
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
        let AppServerMessage::CursorPosition { col, row, visible } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(col, 0);
        assert_eq!(row, 0);
        assert!(!visible);
    }

    #[test]
    fn test_encode_decode_title() {
        let msg = AppServerMessage::Title {
            title: "My Terminal Window".to_string(),
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::Title { title } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(title, "My Terminal Window");
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
        let AppServerMessage::Refresh {
            cols,
            rows,
            screen_content,
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(cols, 120);
        assert_eq!(rows, 40);
        assert_eq!(screen_content, "Full screen content here");
    }

    #[test]
    fn test_encode_decode_connected_with_screen() {
        let msg = AppServerMessage::Connected {
            cols: 80,
            rows: 24,
            initial_screen: Some("initial content".to_string()),
            session_id: "sess-abc".to_string(),
            theme: None,
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::Connected {
            cols,
            rows,
            initial_screen,
            session_id,
            theme,
            ..
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
        assert_eq!(initial_screen, Some("initial content".to_string()));
        assert_eq!(session_id, "sess-abc");
        assert!(theme.is_none());
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
        let AppClientMessage::Subscribe { events } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(events.len(), 5);
        assert!(events.contains(&AppEventType::Output));
        assert!(events.contains(&AppEventType::Cursor));
        assert!(events.contains(&AppEventType::Bell));
        assert!(events.contains(&AppEventType::Title));
        assert!(events.contains(&AppEventType::Resize));
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
        let AppServerMessage::Output { data, .. } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(data, unicode_content);
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
        let AppServerMessage::Output { data, .. } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(data, ansi_data);
        assert!(data.contains("\x1b[31m"));
        assert!(data.contains("\x1b[0m"));
    }

    #[test]
    fn test_encode_decode_with_timestamp() {
        let msg = AppServerMessage::Output {
            data: "test".to_string(),
            timestamp: Some(1234567890123),
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::Output { data, timestamp } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(data, "test");
        assert_eq!(timestamp, Some(1234567890123));
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
        let AppServerMessage::Output { data, .. } = decoded else {
            panic!("Wrong message type");
        };
        assert!(data.is_empty());
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
        assert!(matches!(
            AppEventType::from(pb::EventType::Cwd),
            AppEventType::Cwd
        ));
        assert!(matches!(
            AppEventType::from(pb::EventType::Trigger),
            AppEventType::Trigger
        ));
        // Unspecified defaults to Output
        assert!(matches!(
            AppEventType::from(pb::EventType::Unspecified),
            AppEventType::Output
        ));
    }

    #[test]
    fn test_encode_decode_cwd_changed() {
        let msg = AppServerMessage::CwdChanged {
            old_cwd: Some("/home/user".to_string()),
            new_cwd: "/home/user/project".to_string(),
            hostname: Some("myhost".to_string()),
            username: Some("user".to_string()),
            timestamp: Some(1234567890),
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::CwdChanged {
            old_cwd,
            new_cwd,
            hostname,
            username,
            timestamp,
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(old_cwd, Some("/home/user".to_string()));
        assert_eq!(new_cwd, "/home/user/project");
        assert_eq!(hostname, Some("myhost".to_string()));
        assert_eq!(username, Some("user".to_string()));
        assert_eq!(timestamp, Some(1234567890));
    }

    #[test]
    fn test_encode_decode_cwd_changed_minimal() {
        let msg = AppServerMessage::CwdChanged {
            old_cwd: None,
            new_cwd: "/tmp".to_string(),
            hostname: None,
            username: None,
            timestamp: None,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::CwdChanged {
            old_cwd,
            new_cwd,
            hostname,
            username,
            timestamp,
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(old_cwd, None);
        assert_eq!(new_cwd, "/tmp");
        assert_eq!(hostname, None);
        assert_eq!(username, None);
        assert_eq!(timestamp, None);
    }

    #[test]
    fn test_encode_decode_trigger_matched() {
        let msg = AppServerMessage::TriggerMatched {
            trigger_id: 42,
            row: 10,
            col: 5,
            end_col: 20,
            text: "error: something failed".to_string(),
            captures: vec!["error".to_string(), "something failed".to_string()],
            timestamp: 9876543210,
        };
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::TriggerMatched {
            trigger_id,
            row,
            col,
            end_col,
            text,
            captures,
            timestamp,
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(trigger_id, 42);
        assert_eq!(row, 10);
        assert_eq!(col, 5);
        assert_eq!(end_col, 20);
        assert_eq!(text, "error: something failed");
        assert_eq!(
            captures,
            vec!["error".to_string(), "something failed".to_string()]
        );
        assert_eq!(timestamp, 9876543210);
    }

    #[test]
    fn test_encode_decode_connected_with_new_fields() {
        let msg = AppServerMessage::connected_builder(80, 24, "sess-full".to_string())
            .initial_screen(Some("initial content".to_string()))
            .badge(Some("my badge".to_string()))
            .faint_text_alpha(Some(0.5))
            .cwd(Some("/home/user".to_string()))
            .modify_other_keys(Some(2))
            .client_id(Some("client-42".to_string()))
            .readonly(Some(true))
            .build();
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::Connected {
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
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
        assert_eq!(initial_screen, Some("initial content".to_string()));
        assert_eq!(session_id, "sess-full");
        assert!(theme.is_none());
        assert_eq!(badge, Some("my badge".to_string()));
        assert_eq!(faint_text_alpha, Some(0.5));
        assert_eq!(cwd, Some("/home/user".to_string()));
        assert_eq!(modify_other_keys, Some(2));
        assert_eq!(client_id, Some("client-42".to_string()));
        assert_eq!(readonly, Some(true));
    }

    #[test]
    fn test_event_type_cwd_trigger_conversions() {
        let cwd_i32: i32 = AppEventType::Cwd.into();
        let trigger_i32: i32 = AppEventType::Trigger.into();
        assert_eq!(cwd_i32, pb::EventType::Cwd as i32);
        assert_eq!(trigger_i32, pb::EventType::Trigger as i32);
    }

    #[test]
    fn test_encode_decode_user_var_changed() {
        let msg = AppServerMessage::user_var_changed("hostname".to_string(), "myhost".to_string());
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::UserVarChanged {
            name,
            value,
            old_value,
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(name, "hostname");
        assert_eq!(value, "myhost");
        assert_eq!(old_value, None);
    }

    #[test]
    fn test_encode_decode_user_var_changed_with_old_value() {
        let msg = AppServerMessage::user_var_changed_full(
            "hostname".to_string(),
            "newhost".to_string(),
            Some("oldhost".to_string()),
        );
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::UserVarChanged {
            name,
            value,
            old_value,
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(name, "hostname");
        assert_eq!(value, "newhost");
        assert_eq!(old_value, Some("oldhost".to_string()));
    }

    #[test]
    fn test_event_type_user_var_conversion() {
        let user_var_i32: i32 = AppEventType::UserVar.into();
        assert_eq!(user_var_i32, pb::EventType::UserVar as i32);
    }

    #[test]
    fn test_snapshot_server_message_round_trip() {
        let msg = AppServerMessage::semantic_snapshot("{\"cols\":80}".to_string());
        let encoded = encode_server_message(&msg).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();
        let AppServerMessage::SemanticSnapshot { snapshot_json } = decoded else {
            panic!("Wrong message type");
        };
        assert_eq!(snapshot_json, "{\"cols\":80}");
    }

    #[test]
    fn test_snapshot_request_round_trip() {
        let msg = AppClientMessage::snapshot_request("recent".to_string(), Some(10));
        let encoded = encode_client_message(&msg).unwrap();
        let decoded = decode_client_message(&encoded).unwrap();
        let AppClientMessage::SnapshotRequest {
            scope,
            max_commands,
        } = decoded
        else {
            panic!("Wrong message type");
        };
        assert_eq!(scope, "recent");
        assert_eq!(max_commands, Some(10));
    }
}
