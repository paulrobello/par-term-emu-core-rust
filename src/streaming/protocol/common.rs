//! Shared leaf types used by both message directions: theme, system stats,
//! payload size caps, damage ranges, and replayed inline-image frames.

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

/// CPU statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    all(
        any(feature = "python", feature = "python-test"),
        feature = "streaming"
    ),
    derive(par_term_emu_derive::PyDictConvert)
)]
#[cfg_attr(feature = "streaming", derive(par_term_emu_derive::ProtoConvert))]
#[cfg_attr(
    feature = "streaming",
    proto(wire = "crate::streaming::proto::pb::CpuStats")
)]
pub struct CpuStats {
    /// Overall CPU usage across all cores, in percent.
    pub overall_usage_percent: f64,
    /// Number of physical cores.
    pub physical_core_count: u32,
    /// Per-core usage in percent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub per_core_usage_percent: Vec<f64>,
    /// CPU brand string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Current frequency in MHz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_mhz: Option<u64>,
}

/// Memory statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    all(
        any(feature = "python", feature = "python-test"),
        feature = "streaming"
    ),
    derive(par_term_emu_derive::PyDictConvert)
)]
#[cfg_attr(feature = "streaming", derive(par_term_emu_derive::ProtoConvert))]
#[cfg_attr(
    feature = "streaming",
    proto(wire = "crate::streaming::proto::pb::MemoryStats")
)]
pub struct MemoryStats {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Used physical memory in bytes.
    pub used_bytes: u64,
    /// Available physical memory in bytes.
    pub available_bytes: u64,
    /// Total swap in bytes.
    pub swap_total_bytes: u64,
    /// Used swap in bytes.
    pub swap_used_bytes: u64,
}

/// Individual disk statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    all(
        any(feature = "python", feature = "python-test"),
        feature = "streaming"
    ),
    derive(par_term_emu_derive::PyDictConvert)
)]
#[cfg_attr(feature = "streaming", derive(par_term_emu_derive::ProtoConvert))]
#[cfg_attr(
    feature = "streaming",
    proto(wire = "crate::streaming::proto::pb::DiskStats")
)]
pub struct DiskStats {
    /// Disk name.
    pub name: String,
    /// Mount point path.
    pub mount_point: String,
    /// Total capacity in bytes.
    pub total_bytes: u64,
    /// Available space in bytes.
    pub available_bytes: u64,
    /// Disk kind (e.g. SSD, HDD).
    pub kind: String,
    /// File system name.
    pub file_system: String,
    /// Whether the disk is removable.
    pub is_removable: bool,
}

/// Network interface statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    all(
        any(feature = "python", feature = "python-test"),
        feature = "streaming"
    ),
    derive(par_term_emu_derive::PyDictConvert)
)]
#[cfg_attr(feature = "streaming", derive(par_term_emu_derive::ProtoConvert))]
#[cfg_attr(
    feature = "streaming",
    proto(wire = "crate::streaming::proto::pb::NetworkInterfaceStats")
)]
pub struct NetworkInterfaceStats {
    /// Interface name.
    pub name: String,
    /// Bytes received since the last sample.
    pub received_bytes: u64,
    /// Bytes transmitted since the last sample.
    pub transmitted_bytes: u64,
    /// Total bytes received.
    pub total_received_bytes: u64,
    /// Total bytes transmitted.
    pub total_transmitted_bytes: u64,
    /// Packets received.
    pub packets_received: u64,
    /// Packets transmitted.
    pub packets_transmitted: u64,
    /// Receive errors.
    pub errors_received: u64,
    /// Transmit errors.
    pub errors_transmitted: u64,
}

/// System load averages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    all(
        any(feature = "python", feature = "python-test"),
        feature = "streaming"
    ),
    derive(par_term_emu_derive::PyDictConvert)
)]
#[cfg_attr(feature = "streaming", derive(par_term_emu_derive::ProtoConvert))]
#[cfg_attr(
    feature = "streaming",
    proto(wire = "crate::streaming::proto::pb::LoadAverage")
)]
pub struct LoadAverage {
    /// One-minute load average.
    pub one_minute: f64,
    /// Five-minute load average.
    pub five_minutes: f64,
    /// Fifteen-minute load average.
    pub fifteen_minutes: f64,
}

/// Size cap for [`ServerMessage::InlineImage`](super::ServerMessage::InlineImage) payloads: 8 MiB.
///
/// Enforced at both wire boundaries — `encode_server_message` rejects
/// oversize payloads server-side before encoding, and the wire decode
/// (`proto::wire::inline_image_data`) rejects them client-side — so an
/// image frame can never enter or leave the streaming wire unbounded.
///
/// cap: Image bytes accepted in one InlineImage streaming frame.
pub const MAX_INLINE_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// One `upload_chunk` frame's payload cap — one daemon-side staged chunk
/// (the daemon's own staging cap matches; see MUX.md "Client uploads").
/// Enforced at both wire boundaries by `proto::wire::upload_chunk_data`
/// and again by the server's handler, so an upload chunk can never enter
/// or leave the streaming wire unbounded.
///
/// cap: Bytes accepted in one UploadChunk message payload from a streaming client.
pub const MAX_UPLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// One inclusive run of viewport rows a terminal output chunk changed
/// (card 01a19ca4). Rows are 0-based, inclusive, viewport-relative; the
/// `damage` list on [`ServerMessage::Output`](super::ServerMessage::Output) is ascending and
/// non-overlapping, and when a chunk's change cannot be expressed compactly
/// (scroll, alternate-screen switch, RIS, resize, snapshot restore) it
/// covers the whole viewport — the complete-coverage form; there is no
/// sentinel variant. A client may always ignore the fields and full-scan.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(
    all(
        any(feature = "python", feature = "python-test"),
        feature = "streaming"
    ),
    derive(par_term_emu_derive::PyDictConvert)
)]
#[cfg_attr(feature = "streaming", derive(par_term_emu_derive::ProtoConvert))]
#[cfg_attr(
    feature = "streaming",
    proto(wire = "crate::streaming::proto::pb::DamageRange")
)]
pub struct DamageRange {
    /// First changed viewport row (0-based, inclusive).
    pub start_row: u32,
    /// Last changed viewport row (0-based, inclusive).
    pub end_row: u32,
}

/// One replayed inline-image frame: the payload of `Connected.graphics`, a
/// late joiner's seed of the frames already on screen. Same shape as the
/// [`ServerMessage::InlineImage`](super::ServerMessage::InlineImage) variant — it maps the same wire message —
/// so `Connected` carries a list of frames without nesting full server
/// messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    all(
        any(feature = "python", feature = "python-test"),
        feature = "streaming"
    ),
    derive(par_term_emu_derive::PyDictConvert)
)]
#[cfg_attr(feature = "streaming", derive(par_term_emu_derive::ProtoConvert))]
#[cfg_attr(
    feature = "streaming",
    proto(wire = "crate::streaming::proto::pb::InlineImage")
)]
pub struct InlineImageFrame {
    /// Terminal graphic id, for replace/delete tracking
    pub image_id: u64,
    /// par-mux pane the image belongs to; `None` = the streamed terminal itself
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<u32>,
    /// Payload format tag (`"rgba"`, `"png"`, `"jpeg"`, …)
    pub format: String,
    /// Pixel width
    pub width: u32,
    /// Pixel height
    pub height: u32,
    /// Encoded image bytes; at most [`MAX_INLINE_IMAGE_BYTES`]
    #[cfg_attr(
        feature = "streaming",
        proto(with = "crate::streaming::proto::wire::inline_image_data")
    )]
    pub data: Vec<u8>,
    /// Cell column of the image's top-left corner
    pub col: u32,
    /// Cell row of the image's top-left corner
    pub row: u32,
    /// Serving session id (`pane-12`); `None` = no pane attribution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}
