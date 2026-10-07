//! Kitty graphics protocol support
//!
//! Parses Kitty APC graphics sequences:
//! `APC G <key>=<value>,<key>=<value>;<base64-data> ST`
//!
//! Reference: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use flate2::read::ZlibDecoder;

use crate::graphics::{
    next_graphic_id, AnimationControl, AnimationFrame, CompositionMode, GraphicProtocol,
    GraphicsError, GraphicsStore, ImageDimension, ImagePlacement, TerminalGraphic,
    MAX_IMAGE_DIMENSION, MAX_IMAGE_PIXELS,
};

/// cap: Decoded bytes one kitty transmission may accumulate across chunks
/// (SEC-116). The wire side carries base64 (+~33%); its cap lives in
/// `apc_filter.rs` as `MAX_KITTY_APC_BYTES`.
pub const MAX_KITTY_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

/// cap: Upper bound on one kitty zlib stream's decompressed output
/// (SEC-116) — `MAX_IMAGE_PIXELS * 4` (256 MiB), the same worst-case RGBA
/// allocation the decoder enforces, applied while inflating so a bomb
/// fails streaming instead of after allocating.
pub const MAX_KITTY_DECOMPRESSED_BYTES: usize = MAX_IMAGE_PIXELS * 4;

// `KittyParser` by stage (ARC-003): key/value + chunk parsing, command
// dispatch against the graphics store, and payload/medium decoding.
mod decode;
mod dispatch;
mod parse;

/// Kitty graphics transmission action
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyAction {
    /// Transmit image data (`t`).
    #[default]
    Transmit, // t - transmit image data
    /// Transmit and display (`T`).
    TransmitDisplay, // T - transmit and display
    /// Query terminal support (`q`).
    Query, // q - query terminal support
    /// Display a previously transmitted image (`p`).
    Put, // p - display previously transmitted image
    /// Delete images (`d`).
    Delete, // d - delete images
    /// Animation frame (`f`).
    Frame, // f - animation frame
    /// Animation control (`a`).
    AnimationControl, // a - animation control
}

impl KittyAction {
    /// Parse action character
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            't' => Some(KittyAction::Transmit),
            'T' => Some(KittyAction::TransmitDisplay),
            'q' => Some(KittyAction::Query),
            'p' => Some(KittyAction::Put),
            'd' => Some(KittyAction::Delete),
            'f' => Some(KittyAction::Frame),
            'a' => Some(KittyAction::AnimationControl),
            _ => None,
        }
    }
}

/// Kitty transmission format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyFormat {
    /// 32-bit RGBA (`f=32`).
    #[default]
    Rgba, // 32 - 32-bit RGBA
    /// 24-bit RGB (`f=24`).
    Rgb, // 24 - 24-bit RGB
    /// PNG-compressed (`f=100`).
    Png, // 100 - PNG compressed
}

impl KittyFormat {
    /// Parse format code
    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            24 => Some(KittyFormat::Rgb),
            32 => Some(KittyFormat::Rgba),
            100 => Some(KittyFormat::Png),
            _ => None,
        }
    }
}

/// Kitty transmission medium
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyMedium {
    /// Direct in-band data (`t=d`).
    #[default]
    Direct, // d - direct in-band data
    /// Read from a file (`t=f`).
    File, // f - read from file
    /// Read from a temp file and delete it (`t=t`).
    TempFile, // t - read from temp file and delete
    /// Read from shared memory (`t=s`).
    SharedMem, // s - read from shared memory
}

impl KittyMedium {
    /// Parse medium character
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'd' => Some(KittyMedium::Direct),
            'f' => Some(KittyMedium::File),
            't' => Some(KittyMedium::TempFile),
            's' => Some(KittyMedium::SharedMem),
            _ => None,
        }
    }
}

/// Whether Kitty graphics may load payloads from filesystem paths
/// (SEC-101/SEC-102). Terminal configuration, not per-escape state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileMediaMode {
    /// `t=f` and `t=t` both refused — no file is ever opened.
    Off,
    /// Only the gated `t=t` form: a `tty-graphics-protocol*` file inside an
    /// allowed temp root, deleted only after it decodes as an image.
    /// The default, matching kitty's own spec for the temp-file medium.
    #[default]
    TempOnly,
    /// Any path for both media (`t=f` reads any file the process can read;
    /// `t=t` still requires the temp-root gate before deleting).
    All,
}

impl FileMediaMode {
    /// Parse a mode name (case-insensitive, `-`/``_` tolerant) — the Python
    /// and streaming-config surface is stringly typed.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "off" => Some(Self::Off),
            "temp" | "temp_only" | "temponly" => Some(Self::TempOnly),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// Canonical lowercase name, the inverse of [`Self::from_name`].
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::TempOnly => "temp_only",
            Self::All => "all",
        }
    }
}

/// Kitty compression format (o= parameter)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyCompression {
    /// No compression (default).
    #[default]
    None, // No compression (default)
    /// zlib/deflate compression (`o=z`).
    Zlib, // zlib/deflate compression (o=z)
}

impl KittyCompression {
    /// Parse compression character
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'z' => Some(KittyCompression::Zlib),
            _ => None,
        }
    }
}

/// Kitty delete target
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyDeleteTarget {
    /// All images (`a`).
    All, // a - all images
    /// By image id (`i`).
    ById(u32), // i - by image id
    /// By image id and optional placement id.
    ByPlacement(u32, Option<u32>), // (image_id, placement_id)
    /// Images at the cursor position (`c`).
    AtCursor, // c - at cursor position
    /// Images at a specific cell (`p`).
    InCell, // p - at specific cell
    /// Images visible on screen (`z`).
    OnScreen, // z - visible on screen
    /// Images in a column (`x`).
    ByColumn(u32), // x - in column
    /// Images in a row (`y`).
    ByRow(u32), // y - in row
}

/// Result of building a Kitty graphic
#[derive(Debug, Clone)]
pub enum KittyGraphicResult {
    /// A regular graphic that should be displayed
    Graphic(TerminalGraphic),
    /// A virtual placement - insert Unicode placeholders into grid
    VirtualPlacement {
        /// Kitty image id.
        image_id: u32,
        /// Kitty placement id.
        placement_id: u32,
        /// Placement position as (col, row).
        position: (usize, usize),
        /// Placement width in columns.
        cols: usize,
        /// Placement height in rows.
        rows: usize,
    },
    /// Command processed but no output (delete, query, transmit-only, etc.)
    None,
}

/// Kitty graphics parser
#[derive(Debug, Default)]
pub struct KittyParser {
    /// Current action
    pub action: KittyAction,
    /// Image ID for reuse
    pub image_id: Option<u32>,
    /// Placement ID
    pub placement_id: Option<u32>,
    /// Transmission format
    pub format: KittyFormat,
    /// Transmission medium
    pub medium: KittyMedium,
    /// Image width
    pub width: Option<u32>,
    /// Image height
    pub height: Option<u32>,
    /// Columns to display (for scaling)
    pub columns: Option<u32>,
    /// Rows to display (for scaling)
    pub rows: Option<u32>,
    /// X offset within cell (uppercase X= key)
    pub x_offset: Option<u32>,
    /// Y offset within cell (uppercase Y= key)
    pub y_offset: Option<u32>,
    /// Source crop origin X in pixels (lowercase x= key)
    pub source_x: Option<u32>,
    /// Source crop origin Y in pixels (lowercase y= key)
    pub source_y: Option<u32>,
    /// Source crop width in pixels (lowercase w= key)
    pub source_width: Option<u32>,
    /// Source crop height in pixels (lowercase h= key)
    pub source_height: Option<u32>,
    /// Compression format (o= parameter)
    pub compression: KittyCompression,
    /// More chunks expected
    pub more_chunks: bool,
    /// Accumulated data chunks
    data_chunks: Vec<Vec<u8>>,
    /// Decoded bytes accumulated so far (SEC-116) — tracks the total
    /// against [`MAX_KITTY_PAYLOAD_BYTES`]. Lives beside the chunks so
    /// every `reset()` path (which rebuilds the parser) zeroes it too.
    data_bytes: usize,
    /// Delete target
    pub delete_target: Option<KittyDeleteTarget>,
    /// Virtual placement (U=1)
    pub is_virtual: bool,
    /// Parent image ID for relative positioning (P= key)
    pub parent_image_id: Option<u32>,
    /// Parent placement ID for relative positioning (Q= key)
    pub parent_placement_id: Option<u32>,
    /// Relative X offset (H= key) in pixels
    pub relative_x_offset: Option<i32>,
    /// Relative Y offset (V= key) in pixels
    pub relative_y_offset: Option<i32>,
    /// Frame number for animation
    pub frame_number: Option<u32>,
    /// Frame delay in milliseconds
    pub frame_delay_ms: Option<u32>,
    /// Frame composition mode
    pub frame_composition: Option<CompositionMode>,
    /// Animation control
    pub animation_control: Option<AnimationControl>,
    /// Number of times to play animation (v= parameter)
    /// Per Kitty spec: v=0 ignored, v=1 infinite, v=N means play N times total
    pub num_plays: Option<u32>,
    /// Z-index for layering (z= for placement commands)
    pub z_index: Option<i32>,
    /// Quietness level (q= parameter)
    /// 0 = default (reply with OK and errors)
    /// 1 = suppress OK reply only
    /// 2 = suppress all replies
    pub quietness: u8,
    /// C=1: do not move the cursor after displaying the image (Kitty TGP).
    /// C=0 or omitted uses the default (cursor moves).
    pub suppress_cursor_move: bool,
    /// Configuration (not per-escape state): keep `t=t` temp files on disk
    /// after reading. A multiplexer's daemon-side terminal processes PTY
    /// bytes first, but client mirrors re-read the same file from the raw
    /// bytes forwarded to them — the daemon's read must not delete the
    /// file out from under them. The client that actually renders the
    /// graphic deletes it. Preserved by [`Self::reset`].
    pub retain_temp_files: bool,
    /// Configuration (not per-escape state): whether file mediums (`t=f`
    /// and `t=t`) may load payloads from disk at all, and which forms —
    /// see [`FileMediaMode`]. Preserved by [`Self::reset`].
    pub allow_file_media: FileMediaMode,
    /// Raw parameters for debugging
    params: HashMap<String, String>,
}

/// Temp roots a `t=t` payload may name (SEC-101): kitty's spec restricts
/// the temp-file medium to files inside a temp directory. Both sides are
/// canonicalized, so a symlinked root (macOS `/tmp` → `/private/tmp`)
/// compares equal to the canonical path it yields.
fn is_under_allowed_temp_root(canonical: &Path) -> bool {
    let roots = [
        std::env::temp_dir(),
        std::path::PathBuf::from("/tmp"),
        std::path::PathBuf::from("/dev/shm"),
    ];
    roots.iter().any(|root| match root.canonicalize() {
        Ok(canon_root) => canonical.starts_with(canon_root),
        Err(_) => false, // root absent on this platform (e.g. /dev/shm on macOS)
    })
}

/// The `t=t` path gate (SEC-101): a resolved path must sit under an allowed
/// temp root and carry the spec's `tty-graphics-protocol` filename marker.
fn check_temp_file_gate(resolved: &Path) -> Result<(), GraphicsError> {
    if !is_under_allowed_temp_root(resolved) {
        return Err(GraphicsError::KittyError(
            "Temp file is outside the allowed temp directories".to_string(),
        ));
    }
    let name_has_marker = resolved
        .file_name()
        .map(|n| n.to_string_lossy().contains("tty-graphics-protocol"))
        .unwrap_or(false);
    if !name_has_marker {
        return Err(GraphicsError::KittyError(
            "Temp file name must contain \"tty-graphics-protocol\"".to_string(),
        ));
    }
    Ok(())
}

/// The path the kernel reports for an open handle (SEC-130). `None` means
/// the platform has no lookup, so the caller relies on the dev/ino delete
/// check alone.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn opened_path(file: &fs::File) -> Option<std::io::Result<PathBuf>> {
    use std::os::unix::io::AsRawFd;
    Some(fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())))
}

/// The path the kernel reports for an open handle (SEC-130), via
/// `fcntl(F_GETPATH)`.
#[cfg(target_vendor = "apple")]
fn opened_path(file: &fs::File) -> Option<std::io::Result<PathBuf>> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::AsRawFd;
    let mut buf = [0u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes a NUL-terminated path of at most PATH_MAX
    // bytes into its third argument, and `buf` is exactly PATH_MAX bytes.
    // The fd is borrowed from `file`, which stays open for the whole call.
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) };
    if rc == -1 {
        return Some(Err(std::io::Error::last_os_error()));
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Some(Ok(PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..len]))))
}

/// Other Unix and Windows expose no portable fd-to-path lookup; the dev/ino
/// delete check (Unix) still pins the delete to the file that was read.
#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn opened_path(_file: &fs::File) -> Option<std::io::Result<PathBuf>> {
    None
}

/// A `t=t` file to unlink after decode, pinned to the inode that was read
/// (SEC-130).
#[derive(Debug)]
struct PendingDelete {
    /// The canonical path that passed the temp gate and was opened.
    path: PathBuf,
    /// `(st_dev, st_ino)` of the handle that was read.
    #[cfg(unix)]
    dev_ino: (u64, u64),
}

/// Unlink a `t=t` file only if its path still names the file that was read
/// (SEC-130), so a file swapped in after the read survives. Returns whether
/// the file was removed.
fn delete_if_same_file(pending: &PendingDelete) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match fs::symlink_metadata(&pending.path) {
            Ok(m) if m.file_type().is_file() && (m.dev(), m.ino()) == pending.dev_ino => {
                fs::remove_file(&pending.path).is_ok()
            }
            Ok(_) => {
                crate::debug_error!(
                    "KITTY",
                    "t=t file {:?} changed after it was read; not deleting",
                    pending.path
                );
                false
            }
            Err(e) => {
                crate::debug_error!(
                    "KITTY",
                    "t=t file {:?} vanished before delete: {}",
                    pending.path,
                    e
                );
                false
            }
        }
    }
    #[cfg(not(unix))]
    {
        // Windows std exposes no stable file identity to compare here, so
        // the delete trusts the canonical path that passed the gate.
        fs::remove_file(&pending.path).is_ok()
    }
}

/// Open for reading without following a final symlink component (SEC-103),
/// so a swapped link cannot redirect the read after validation.
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        // Windows std has no O_NOFOLLOW equivalent; the canonicalize gate
        // above already resolved the path, so the symlink-swap window
        // between check and open is accepted here.
        std::fs::File::open(path)
    }
}

#[cfg(test)]
mod tests;
