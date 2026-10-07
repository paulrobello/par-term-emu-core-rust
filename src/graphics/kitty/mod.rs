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

impl KittyParser {
    /// Create a new parser
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset parser state for new transmission
    pub fn reset(&mut self) {
        let retain_temp_files = self.retain_temp_files;
        let allow_file_media = self.allow_file_media;
        *self = Self::default();
        self.retain_temp_files = retain_temp_files;
        self.allow_file_media = allow_file_media;
    }

    /// Parse a Kitty graphics payload
    ///
    /// Format: `key=value,key=value,...;base64data`
    pub fn parse_chunk(&mut self, payload: &str) -> Result<bool, GraphicsError> {
        // Split into params and data
        let (params_str, data_str) = payload.split_once(';').unwrap_or((payload, ""));

        // Parse key=value pairs
        for pair in params_str.split(',') {
            if let Some((key, value)) = pair.split_once('=') {
                self.params.insert(key.to_string(), value.to_string());

                match key {
                    "a" => {
                        if let Some(c) = value.chars().next() {
                            self.action = KittyAction::from_char(c).unwrap_or_default();
                        }
                    }
                    "f" => {
                        if let Ok(code) = value.parse::<u32>() {
                            self.format = KittyFormat::from_code(code).unwrap_or_default();
                        }
                    }
                    "t" => {
                        if let Some(c) = value.chars().next() {
                            self.medium = KittyMedium::from_char(c).unwrap_or_default();
                        }
                    }
                    "i" => {
                        self.image_id = value.parse().ok();
                    }
                    "p" => {
                        self.placement_id = value.parse().ok();
                    }
                    "s" => {
                        // Animation control state (for AnimationControl action) takes priority
                        if self.action == KittyAction::AnimationControl {
                            self.animation_control = AnimationControl::from_value(value);
                            debug_log!(
                                "KITTY",
                                "Parsed animation control: s={} -> {:?}",
                                value,
                                self.animation_control
                            );
                        } else {
                            // Otherwise it's width
                            self.width = value.parse().ok();
                        }
                    }
                    "v" => {
                        // v= is overloaded: height for images, num_plays for animation control
                        if self.action == KittyAction::AnimationControl {
                            // Number of times to play animation (v= for animation control)
                            // Per Kitty spec: v=0 ignored, v=1 infinite, v=N means play N times total
                            self.num_plays = value.parse().ok();
                        } else {
                            // Height for image transmission/display
                            self.height = value.parse().ok();
                        }
                    }
                    "c" => {
                        // Frame composition mode (for Frame action) takes priority
                        if self.action == KittyAction::Frame {
                            if let Some(first_char) = value.chars().next() {
                                self.frame_composition = CompositionMode::from_char(first_char);
                            }
                        } else {
                            // Otherwise it's columns
                            self.columns = value.parse().ok();
                        }
                    }
                    "r" => {
                        // Frame number (for Frame action) takes priority
                        if self.action == KittyAction::Frame {
                            self.frame_number = value.parse().ok();
                        } else {
                            // Otherwise it's rows
                            self.rows = value.parse().ok();
                        }
                    }
                    "x" => {
                        self.source_x = value.parse().ok();
                    }
                    "y" => {
                        self.source_y = value.parse().ok();
                    }
                    "w" => {
                        self.source_width = value.parse().ok();
                    }
                    "h" => {
                        self.source_height = value.parse().ok();
                    }
                    "X" => {
                        self.x_offset = value.parse().ok();
                    }
                    "Y" => {
                        self.y_offset = value.parse().ok();
                    }
                    "m" => {
                        self.more_chunks = value == "1";
                    }
                    "d" => {
                        // Delete specification — recorded in `params` above
                        // and resolved after the full key=value list is
                        // parsed, so `d=` may precede its identifying
                        // params (see the post-loop resolution below).
                    }
                    "U" => {
                        // Virtual placement
                        self.is_virtual = value == "1";
                    }
                    "P" => {
                        // Parent image ID for relative positioning
                        self.parent_image_id = value.parse().ok();
                    }
                    "Q" => {
                        // Parent placement ID for relative positioning
                        self.parent_placement_id = value.parse().ok();
                    }
                    "H" => {
                        // Relative X offset in pixels
                        self.relative_x_offset = value.parse().ok();
                    }
                    "V" => {
                        // Relative Y offset in pixels (note: different from v=height).
                        // Parsed unconditionally to match H=; the offset is only *applied*
                        // when a parent (P=) is set (see build_graphic), so storing it
                        // early is harmless and removes the H=/V= asymmetry where V=
                        // was silently dropped unless P= had already been parsed.
                        self.relative_y_offset = value.parse().ok();
                    }
                    "o" => {
                        // Compression format
                        if let Some(c) = value.chars().next() {
                            if let Some(comp) = KittyCompression::from_char(c) {
                                self.compression = comp;
                            }
                        }
                    }
                    "z" => {
                        // z= is overloaded: frame delay for animations, z-index for placements
                        if self.action == KittyAction::Frame {
                            self.frame_delay_ms = value.parse().ok();
                        } else {
                            self.z_index = value.parse().ok();
                        }
                    }
                    "q" => {
                        // Quietness level (0 = reply, 1 = suppress OK, 2 = suppress all)
                        if let Ok(level) = value.parse::<u8>() {
                            self.quietness = level;
                        }
                    }
                    "C" => {
                        // C=1: do not move the cursor after display; C=0 or
                        // omitted keeps the default (cursor advances below
                        // the image) — terminal layer applies the move.
                        self.suppress_cursor_move = value == "1";
                    }
                    _ => {}
                }
            }
        }

        // Resolve delete criteria after the whole key=value list has been
        // parsed: emitters disagree on key order (Herdr sends `d=` before
        // `i=`/`p=`, e.g. `a=d,d=I,i=<id>`), so resolving while scanning
        // would see `image_id`/`placement_id` still unset and silently
        // no-op the delete. `params` accumulates across chunks, so a `d=`
        // seen in an earlier chunk re-resolves once later params arrive.
        if let Some(spec) = self.params.get("d").cloned() {
            self.parse_delete_target(&spec);
        }

        // Decode and accumulate base64 data
        if !data_str.is_empty() {
            // Try STANDARD first (with padding), then NO_PAD if that fails
            // This handles both padded and unpadded base64 (Kitty allows both)
            let decoded =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data_str)
                    .or_else(|_| {
                        base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD_NO_PAD,
                            data_str,
                        )
                    })
                    .map_err(|e| GraphicsError::Base64Error(e.to_string()))?;
            if self.data_bytes + decoded.len() > MAX_KITTY_PAYLOAD_BYTES {
                // Over the cap: drop the accumulated transmission entirely
                // so a later one starts from zero, not from the overflow.
                self.reset();
                return Err(GraphicsError::KittyError(format!(
                    "payload exceeds {} bytes",
                    MAX_KITTY_PAYLOAD_BYTES
                )));
            }
            self.data_bytes += decoded.len();
            self.data_chunks.push(decoded);
        }

        // Return true if more chunks expected
        Ok(self.more_chunks)
    }

    /// Resolve delete target specification after all parameters are parsed.
    fn parse_delete_target(&mut self, value: &str) {
        // Identifying parameters may follow `d=` (as in Herdr's
        // `a=d,d=I,i=<id>` / `a=d,d=i,i=<id>,p=<pid>` ordering).
        if let Some(c) = value.chars().next() {
            self.delete_target = match c {
                'a' | 'A' => Some(KittyDeleteTarget::All),
                'c' | 'C' => Some(KittyDeleteTarget::AtCursor),
                'z' | 'Z' => Some(KittyDeleteTarget::OnScreen),
                'i' | 'I' => self.image_id.map(|iid| match self.placement_id {
                    Some(pid) => KittyDeleteTarget::ByPlacement(iid, Some(pid)),
                    None => KittyDeleteTarget::ById(iid),
                }),
                'p' | 'P' => self
                    .image_id
                    .map(|iid| KittyDeleteTarget::ByPlacement(iid, self.placement_id)),
                'x' | 'X' => self.source_x.map(KittyDeleteTarget::ByColumn),
                'y' | 'Y' => self.source_y.map(KittyDeleteTarget::ByRow),
                _ => None,
            };
        }
    }

    /// Get accumulated data, decompressing if necessary.
    ///
    /// Decompression failures propagate (SEC-116): the old raw-bytes
    /// fallback fed compressed bytes to the pixel decoder on a limit
    /// error — garbage at best, an attack surface at worst.
    pub fn get_data(&self) -> Result<Vec<u8>, GraphicsError> {
        let raw = self.data_chunks.concat();
        if self.compression == KittyCompression::Zlib {
            Self::decompress_zlib(&raw, self.decompression_limit())
        } else {
            Ok(raw)
        }
    }

    /// The zlib output bound for this transmission: with the pixel
    /// geometry known (`f=24`/`f=32` carrying `s=` and `v=`), exactly
    /// width × height × bytes-per-pixel; otherwise the global RGBA worst
    /// case. Either way clamped by [`MAX_KITTY_DECOMPRESSED_BYTES`], so a
    /// lying `s=`/`v=` pair cannot raise the ceiling.
    fn decompression_limit(&self) -> usize {
        let bytes_per_pixel = match self.format {
            KittyFormat::Rgb => 3,
            KittyFormat::Rgba => 4,
            KittyFormat::Png => return MAX_KITTY_DECOMPRESSED_BYTES,
        };
        match (self.width, self.height) {
            (Some(width), Some(height)) => (width as usize)
                .saturating_mul(height as usize)
                .saturating_mul(bytes_per_pixel)
                .min(MAX_KITTY_DECOMPRESSED_BYTES),
            _ => MAX_KITTY_DECOMPRESSED_BYTES,
        }
    }

    /// Decompress zlib-compressed data, bounding the output at `limit`
    /// while inflating (SEC-116): `take(limit + 1)` stops the decoder
    /// after one byte past the bound, so an over-limit stream is detected
    /// without ever allocating its full size.
    fn decompress_zlib(data: &[u8], limit: usize) -> Result<Vec<u8>, GraphicsError> {
        // Empty input is not a valid zlib stream, but this API treats it as
        // empty output; flate2 versions differ here and callers rely on
        // zero-length payloads not being an error.
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let mut decompressed = Vec::new();
        ZlibDecoder::new(data)
            .take(limit as u64 + 1)
            .read_to_end(&mut decompressed)
            .map_err(|e| GraphicsError::KittyError(format!("Zlib decompression failed: {}", e)))?;
        if decompressed.len() > limit {
            return Err(GraphicsError::KittyError(format!(
                "Decompressed size {} exceeds limit {}",
                decompressed.len(),
                limit
            )));
        }
        Ok(decompressed)
    }

    /// Check if data was compressed
    pub fn is_compressed(&self) -> bool {
        self.compression != KittyCompression::None
    }

    /// Build an ImagePlacement from the parsed Kitty parameters
    pub fn build_placement(&self) -> ImagePlacement {
        let mut placement = ImagePlacement::inline();

        if let Some(cols) = self.columns {
            placement.columns = Some(cols);
            placement.requested_width = ImageDimension::cells(cols as f64);
        }

        if let Some(rows) = self.rows {
            placement.rows = Some(rows);
            placement.requested_height = ImageDimension::cells(rows as f64);
        }

        if let Some(z) = self.z_index {
            placement.z_index = z;
        }

        if let Some(x) = self.source_x {
            placement.source_x = x;
        }
        if let Some(y) = self.source_y {
            placement.source_y = y;
        }
        if let Some(width) = self.source_width {
            placement.source_width = width;
        }
        if let Some(height) = self.source_height {
            placement.source_height = height;
        }
        if let Some(x) = self.x_offset {
            placement.x_offset = x;
        }
        if let Some(y) = self.y_offset {
            placement.y_offset = y;
        }

        placement
    }

    /// Build a TerminalGraphic from parsed data
    pub fn build_graphic(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        match self.action {
            KittyAction::Delete => self.apply_delete_command(position, store),

            KittyAction::Query => {
                // Query doesn't create a graphic
                Ok(KittyGraphicResult::None)
            }

            KittyAction::Put => self.apply_placement(position, store),

            KittyAction::Transmit | KittyAction::TransmitDisplay => {
                self.apply_transmit_command(position, store)
            }

            KittyAction::Frame => self.apply_frame_command(position, store),

            KittyAction::AnimationControl => self.apply_animation_control(store),
        }
    }

    /// Handle `a=d` (delete): resolve the delete target against the
    /// graphics store and remove matching placements/images.
    fn apply_delete_command(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        if let Some(target) = &self.delete_target {
            match target {
                KittyDeleteTarget::All => store.clear(),
                KittyDeleteTarget::ById(id) => {
                    store.delete_kitty_graphics(Some(*id), None);
                }
                KittyDeleteTarget::ByPlacement(iid, pid) => {
                    store.delete_kitty_graphics(Some(*iid), *pid);
                }
                KittyDeleteTarget::AtCursor => {
                    let (cursor_col, cursor_row) = position;
                    store
                        .placements
                        .retain(|g| g.position != (cursor_col, cursor_row));
                }
                KittyDeleteTarget::InCell => {
                    // Same as AtCursor in our context since we use the cursor position
                    let (cursor_col, cursor_row) = position;
                    store
                        .placements
                        .retain(|g| g.position != (cursor_col, cursor_row));
                }
                KittyDeleteTarget::OnScreen => {
                    // Remove all visible placements but preserve shared images
                    store.placements.clear();
                }
                KittyDeleteTarget::ByColumn(col) => {
                    let target_col = *col as usize;
                    store.placements.retain(|g| {
                        let start_col = g.position.0;
                        let cell_width = g.cell_dimensions.map(|(w, _)| w as usize).unwrap_or(1);
                        let end_col = start_col + g.width.div_ceil(cell_width);
                        target_col < start_col || target_col >= end_col
                    });
                }
                KittyDeleteTarget::ByRow(row) => {
                    let target_row = *row as usize;
                    store.placements.retain(|g| {
                        let start_row = g.position.1;
                        let cell_height = g.cell_dimensions.map(|(_, h)| h as usize).unwrap_or(2);
                        let end_row = start_row + g.height.div_ceil(cell_height);
                        target_row < start_row || target_row >= end_row
                    });
                }
            }
        }
        Ok(KittyGraphicResult::None)
    }

    /// Handle `a=p` (put): display a previously transmitted image, or
    /// create a virtual (Unicode-placeholder) placement when `U=1`.
    fn apply_placement(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        // Display previously transmitted image or create virtual placement
        let image_id = self.image_id.unwrap_or(0);

        // If U=1, create a virtual placement
        if self.is_virtual {
            let cols = self.columns.unwrap_or(1) as usize;
            let rows = self.rows.unwrap_or(1) as usize;
            let placement_id = self.placement_id.unwrap_or(0);

            // Create virtual placement without image data
            let mut graphic = TerminalGraphic::new(
                next_graphic_id(),
                GraphicProtocol::Kitty,
                position,
                cols,
                rows,
                vec![], // Virtual placements don't need pixel data
            );
            graphic.kitty_image_id = Some(image_id);
            graphic.kitty_placement_id = Some(placement_id);
            graphic.is_virtual = true;
            store.add_virtual_placement(graphic);

            // Return virtual placement info for placeholder insertion
            return Ok(KittyGraphicResult::VirtualPlacement {
                image_id,
                placement_id,
                position,
                cols,
                rows,
            });
        }

        // Regular placement
        if let Some((width, height, pixels)) = store.get_kitty_image(image_id) {
            let mut graphic = TerminalGraphic::with_shared_pixels(
                next_graphic_id(),
                GraphicProtocol::Kitty,
                position,
                width,
                height,
                pixels,
            );
            graphic.kitty_image_id = Some(image_id);
            graphic.kitty_placement_id = self.placement_id;
            graphic.placement = self.build_placement();

            // Handle relative positioning
            if let Some(parent_img_id) = self.parent_image_id {
                graphic.parent_image_id = Some(parent_img_id);
                graphic.parent_placement_id = self.parent_placement_id;
                graphic.relative_x_offset = self.relative_x_offset.unwrap_or(0);
                graphic.relative_y_offset = self.relative_y_offset.unwrap_or(0);
            }

            return Ok(KittyGraphicResult::Graphic(graphic));
        }
        Err(GraphicsError::KittyError("Image not found".to_string()))
    }

    /// Handle `a=t`/`a=T` (transmit / transmit+display): decode the
    /// payload per the transmission medium/format, cache it if an image id
    /// was given, and build a graphic when the action is `TransmitDisplay`.
    fn apply_transmit_command(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        let raw_data = self.get_data()?;
        if raw_data.is_empty() {
            return Err(GraphicsError::KittyError("No image data".to_string()));
        }

        let compressed = self.is_compressed();

        let (width, height, pixels) =
            self.decode_payload(raw_data, "Shared memory transmission not supported")?;

        // Store for reuse if image_id is specified
        if let Some(image_id) = self.image_id {
            store.store_kitty_image(image_id, width, height, pixels.clone());
        }

        // Create graphic if TransmitDisplay, or virtual placement if U=1
        if self.action == KittyAction::TransmitDisplay {
            if self.is_virtual {
                let cols = self.columns.unwrap_or(1) as usize;
                let rows = self.rows.unwrap_or(1) as usize;
                let image_id = self.image_id.unwrap_or(0);
                let placement_id = self.placement_id.unwrap_or(0);

                // Create virtual placement
                let mut graphic = TerminalGraphic::new(
                    next_graphic_id(),
                    GraphicProtocol::Kitty,
                    position,
                    cols,
                    rows,
                    vec![], // Virtual placements don't need pixel data
                );
                graphic.kitty_image_id = Some(image_id);
                graphic.kitty_placement_id = Some(placement_id);
                graphic.is_virtual = true;
                graphic.was_compressed = compressed;
                store.add_virtual_placement(graphic);

                // Return virtual placement info for placeholder insertion
                Ok(KittyGraphicResult::VirtualPlacement {
                    image_id,
                    placement_id,
                    position,
                    cols,
                    rows,
                })
            } else {
                let mut graphic = TerminalGraphic::new(
                    next_graphic_id(),
                    GraphicProtocol::Kitty,
                    position,
                    width,
                    height,
                    pixels,
                );
                graphic.kitty_image_id = self.image_id;
                graphic.kitty_placement_id = self.placement_id;
                graphic.was_compressed = compressed;
                graphic.placement = self.build_placement();

                // Handle relative positioning
                if let Some(parent_img_id) = self.parent_image_id {
                    graphic.parent_image_id = Some(parent_img_id);
                    graphic.parent_placement_id = self.parent_placement_id;
                    graphic.relative_x_offset = self.relative_x_offset.unwrap_or(0);
                    graphic.relative_y_offset = self.relative_y_offset.unwrap_or(0);
                }

                Ok(KittyGraphicResult::Graphic(graphic))
            }
        } else {
            // Transmit only, no display
            Ok(KittyGraphicResult::None)
        }
    }

    /// Handle `a=f` (animation frame): decode the frame's payload, append
    /// it to the image's animation, and (for frame 1) also create the
    /// placement that displays the animation.
    fn apply_frame_command(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        // Add animation frame
        let raw_data = self.get_data()?;
        if raw_data.is_empty() {
            return Err(GraphicsError::KittyError("No frame data".to_string()));
        }

        let compressed = self.is_compressed();

        let image_id = self
            .image_id
            .ok_or_else(|| GraphicsError::KittyError("Frame requires image ID".to_string()))?;

        // Decode frame data
        let (width, height, pixels) =
            self.decode_payload(raw_data, "Shared memory not supported for frames")?;

        // Create frame
        let frame_num = self.frame_number.unwrap_or(1);
        let mut frame = AnimationFrame::new(frame_num, pixels.clone(), width, height);

        if let Some(delay) = self.frame_delay_ms {
            frame = frame.with_delay(delay);
        }

        if let Some(x) = self.source_x {
            if let Some(y) = self.source_y {
                frame = frame.with_offset(x, y);
            }
        }

        if let Some(comp) = self.frame_composition {
            frame = frame.with_composition(comp);
        }

        // Add frame to animation
        store.add_animation_frame(image_id, frame);

        // Frame 1 creates both animation entry AND a placement for display
        if frame_num == 1 {
            // Store as shared image so it can be referenced by Put commands
            store.store_kitty_image(image_id, width, height, pixels.clone());

            // Create placement to display the animation
            let mut graphic = TerminalGraphic::new(
                next_graphic_id(),
                GraphicProtocol::Kitty,
                position,
                width,
                height,
                pixels,
            );
            graphic.kitty_image_id = Some(image_id);
            graphic.kitty_placement_id = self.placement_id;
            graphic.was_compressed = compressed;
            graphic.placement = self.build_placement();

            // Handle relative positioning
            if let Some(parent_img_id) = self.parent_image_id {
                graphic.parent_image_id = Some(parent_img_id);
                graphic.parent_placement_id = self.parent_placement_id;
                graphic.relative_x_offset = self.relative_x_offset.unwrap_or(0);
                graphic.relative_y_offset = self.relative_y_offset.unwrap_or(0);
            }

            return Ok(KittyGraphicResult::Graphic(graphic));
        }

        // Subsequent frames only add to animation, don't create new placements
        Ok(KittyGraphicResult::None)
    }

    /// Handle `a=a` (animation control): update loop count and/or playback
    /// state for an existing animated image.
    fn apply_animation_control(
        &self,
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        // Control animation playback
        let image_id = self.image_id.ok_or_else(|| {
            GraphicsError::KittyError("Animation control requires image ID".to_string())
        })?;

        // Handle num_plays (v= parameter) for setting loop count
        // Per Kitty spec: v=0 ignored, v=1 infinite, v=N means play N times total
        // We store loop_count as (N-1) so animation stops after (N-1) additional loops
        if let Some(num_plays) = self.num_plays {
            if num_plays > 0 {
                let loop_count = if num_plays == 1 {
                    0 // v=1 means infinite looping
                } else {
                    num_plays - 1 // Store N-1 to get N total plays
                };
                debug_info!(
                    "KITTY",
                    "Setting loop count for image_id={}: num_plays={}, loop_count={}",
                    image_id,
                    num_plays,
                    loop_count
                );
                store.set_animation_loops(image_id, loop_count);
            }
        }

        // Handle state control (s= parameter)
        if let Some(control) = self.animation_control {
            debug_info!(
                "KITTY",
                "Applying animation control: image_id={}, control={:?}",
                image_id,
                control
            );
            store.control_animation(image_id, control);
        } else {
            debug_log!(
                "KITTY",
                "Animation control command received but no control parsed (image_id={})",
                image_id
            );
        }

        Ok(KittyGraphicResult::None)
    }

    /// Load image data per transmission medium and decode it to raw RGBA
    /// pixels — the payload-resolution step shared by the transmit and
    /// frame handlers. `raw_data` is the already-concatenated (and, if
    /// applicable, decompressed) payload from [`Self::get_data`];
    /// `shared_mem_error` is the caller's error message for the
    /// (unsupported) `SharedMem` medium, since Transmit and Frame each use
    /// distinct wording there.
    fn decode_payload(
        &self,
        raw_data: Vec<u8>,
        shared_mem_error: &str,
    ) -> Result<(usize, usize, Vec<u8>), GraphicsError> {
        // Load image data based on transmission medium. For file media the
        // load also returns the `t=t` cleanup path: deletion is deferred
        // until the payload decodes, so a non-image temp file is never
        // destroyed (SEC-101).
        let (image_data, delete_after_decode) = match self.medium {
            KittyMedium::File | KittyMedium::TempFile => {
                // For file transmission, raw_data is a file path (not base64-encoded)
                self.load_file_data(&raw_data)?
            }
            KittyMedium::Direct => {
                // For direct transmission, use data as-is
                (raw_data, None)
            }
            KittyMedium::SharedMem => {
                return Err(GraphicsError::KittyError(shared_mem_error.to_string()));
            }
        };

        let decoded = self.decode_pixels(&image_data);

        // Delete the t=t temp file only once the payload proved decodable.
        // Skipped when `retain_temp_files` is set (mux daemon terminals):
        // client mirrors re-read the same file from forwarded bytes — the
        // rendering client deletes it.
        if decoded.is_ok() {
            if let Some(pending) = delete_after_decode {
                delete_if_same_file(&pending);
            }
        }

        decoded
    }

    /// Load image data from a file medium (`t=f`/`t=t`) with security
    /// gating (SEC-101/SEC-102/SEC-103).
    ///
    /// Returns the file bytes plus, for a non-retained `t=t` read, the
    /// file the caller must delete **after** the payload decodes.
    fn load_file_data(
        &self,
        path_data: &[u8],
    ) -> Result<(Vec<u8>, Option<PendingDelete>), GraphicsError> {
        // Decode path from UTF-8 bytes (NOT base64-encoded for file transmission)
        let path_str = String::from_utf8(path_data.to_vec())
            .map_err(|e| GraphicsError::KittyError(format!("Invalid UTF-8 in file path: {}", e)))?;

        let path = Path::new(&path_str);

        // Security validations

        // 1. Reject paths containing a real parent-directory component.
        //    Component-wise, not substring: "my..notes.png" has no `..`
        //    component and must stay readable, while "a/../b" must not.
        if path.components().any(|c| c == Component::ParentDir) {
            return Err(GraphicsError::KittyError(
                "Directory traversal not allowed".to_string(),
            ));
        }

        // 2. Mode gate (SEC-101/SEC-102): file media is opt-in per medium,
        //    and is checked before touching the filesystem so a probe
        //    leaks no existence information.
        match self.medium {
            KittyMedium::File => {
                if self.allow_file_media != FileMediaMode::All {
                    return Err(GraphicsError::KittyError(
                        "File medium (t=f) requires allow_file_media=\"all\"".to_string(),
                    ));
                }
            }
            KittyMedium::TempFile if self.allow_file_media == FileMediaMode::Off => {
                return Err(GraphicsError::KittyError(
                    "Temp-file medium (t=t) is disabled (allow_file_media=\"off\")".to_string(),
                ));
            }
            _ => {}
        }

        // 3. t=t path gate (SEC-101): the canonicalized path must sit under
        //    an allowed temp root AND carry the spec's marker in its
        //    filename — kitty's own rule for the temp-file medium. Refuse
        //    without touching the file otherwise.
        let canonical = if self.medium == KittyMedium::TempFile {
            let canonical = path.canonicalize().map_err(|e| {
                GraphicsError::KittyError(format!("Cannot resolve temp file path: {}", e))
            })?;
            check_temp_file_gate(&canonical)?;
            Some(canonical)
        } else {
            None
        };

        // 4. Open without following a final symlink (SEC-103) and validate
        //    the handle itself, so the file we read is the file we checked.
        //    `t=t` opens the canonical path that passed the gate (SEC-130);
        //    `t=f` has no containment check, so it opens the path as given.
        let open_path = canonical.as_deref().unwrap_or(path);
        let mut file = open_no_follow(open_path).map_err(|e| {
            // The directory check comes first: Windows fails opening a
            // directory with ERROR_PATH_NOT_FOUND (io kind NotFound), so a
            // kind-ordered check would report an existing directory as
            // "File not found".
            if path.is_dir() {
                GraphicsError::KittyError(format!("Path is not a file: {}", path_str))
            } else if e.kind() == std::io::ErrorKind::NotFound {
                GraphicsError::KittyError(format!("File not found: {}", path_str))
            } else {
                GraphicsError::KittyError(format!("Cannot open file: {}", e))
            }
        })?;

        let metadata = file
            .metadata()
            .map_err(|e| GraphicsError::KittyError(format!("Cannot read file metadata: {}", e)))?;

        if !metadata.is_file() {
            return Err(GraphicsError::KittyError(format!(
                "Path is not a file: {}",
                path_str
            )));
        }

        // 4b. t=t: re-run the gate on the path the kernel reports for the
        //     open handle (SEC-130). A parent directory swapped for a
        //     symlink between canonicalize and open escapes O_NOFOLLOW,
        //     which guards only the final component; this catches it.
        if canonical.is_some() {
            if let Some(resolved) = opened_path(&file) {
                let resolved = resolved.map_err(|e| {
                    GraphicsError::KittyError(format!("Cannot resolve opened temp file: {}", e))
                })?;
                check_temp_file_gate(&resolved)?;
            }
        }

        // 5. Check file size (limit to 100MB for safety)
        /// cap: Bytes read from one kitty file medium named by an escape payload.
        const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024; // 100MB
        if metadata.len() > MAX_FILE_SIZE {
            return Err(GraphicsError::KittyError(format!(
                "File too large: {} bytes (max {})",
                metadata.len(),
                MAX_FILE_SIZE
            )));
        }

        // 6. Read from the validated handle
        let mut file_data = Vec::with_capacity(metadata.len() as usize);
        file.read_to_end(&mut file_data)
            .map_err(|e| GraphicsError::KittyError(format!("Cannot read file: {}", e)))?;

        // Pending delete: only a non-retained t=t read deletes, and only
        // after decode succeeds (the caller's job). It is pinned to the
        // inode read here, so a file swapped in afterwards survives.
        let pending_delete = match canonical {
            Some(canonical) if !self.retain_temp_files => {
                #[cfg(unix)]
                let dev_ino = {
                    use std::os::unix::fs::MetadataExt;
                    (metadata.dev(), metadata.ino())
                };
                Some(PendingDelete {
                    path: canonical,
                    #[cfg(unix)]
                    dev_ino,
                })
            }
            _ => None,
        };

        Ok((file_data, pending_delete))
    }

    /// Decode pixels based on format
    fn decode_pixels(&self, data: &[u8]) -> Result<(usize, usize, Vec<u8>), GraphicsError> {
        match self.format {
            KittyFormat::Png => {
                // Decode PNG via a size-limited reader so the decoder refuses
                // to allocate before hitting MAX_IMAGE_DIMENSION, guarding
                // against decompression-bomb PNGs (small input, huge output).
                let mut reader = image::ImageReader::new(std::io::Cursor::new(data))
                    .with_guessed_format()
                    .map_err(|e| GraphicsError::ImageError(e.to_string()))?;
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(MAX_IMAGE_DIMENSION as u32);
                limits.max_image_height = Some(MAX_IMAGE_DIMENSION as u32);
                reader.limits(limits);
                let img = reader
                    .decode()
                    .map_err(|e| GraphicsError::ImageError(e.to_string()))?;
                let rgba = img.to_rgba8();
                let width = rgba.width() as usize;
                let height = rgba.height() as usize;
                // Also cap the total pixel product: two dimensions can each
                // pass MAX_IMAGE_DIMENSION individually yet still multiply
                // out to a huge RGBA buffer.
                let pixels = width.checked_mul(height).ok_or_else(|| {
                    GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                })?;
                if pixels > MAX_IMAGE_PIXELS {
                    return Err(GraphicsError::ImageTooLarge(pixels, MAX_IMAGE_PIXELS));
                }
                Ok((width, height, rgba.into_raw()))
            }

            KittyFormat::Rgba => {
                // Raw RGBA data
                let width = self.width.ok_or_else(|| {
                    GraphicsError::KittyError("Width required for raw format".to_string())
                })? as usize;
                let height = self.height.ok_or_else(|| {
                    GraphicsError::KittyError("Height required for raw format".to_string())
                })? as usize;
                // checked_mul: width/height are attacker-controlled u32 values; without
                // this, `width * height * 4` can wrap usize and bypass the size check,
                // yielding a graphic with huge dims over a tiny buffer (OOB/panic DoS).
                let expected = width
                    .checked_mul(height)
                    .and_then(|px| px.checked_mul(4))
                    .ok_or_else(|| {
                        GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                    })?;
                if data.len() != expected {
                    return Err(GraphicsError::KittyError(format!(
                        "Data size mismatch: got {}, expected {}",
                        data.len(),
                        expected
                    )));
                }
                Ok((width, height, data.to_vec()))
            }

            KittyFormat::Rgb => {
                // Raw RGB data - convert to RGBA
                let width = self.width.ok_or_else(|| {
                    GraphicsError::KittyError("Width required for raw format".to_string())
                })? as usize;
                let height = self.height.ok_or_else(|| {
                    GraphicsError::KittyError("Height required for raw format".to_string())
                })? as usize;
                // checked_mul throughout (see Rgba): attacker-controlled dimensions.
                let pixels = width.checked_mul(height).ok_or_else(|| {
                    GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                })?;
                let expected = pixels.checked_mul(3).ok_or_else(|| {
                    GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                })?;
                if data.len() != expected {
                    return Err(GraphicsError::KittyError(format!(
                        "Data size mismatch: got {}, expected {}",
                        data.len(),
                        expected
                    )));
                }

                // Convert RGB to RGBA (capacity validated non-overflowing).
                let mut rgba = Vec::with_capacity(pixels.checked_mul(4).ok_or_else(|| {
                    GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                })?);
                for chunk in data.chunks(3) {
                    rgba.push(chunk[0]);
                    rgba.push(chunk[1]);
                    rgba.push(chunk[2]);
                    rgba.push(255); // Alpha
                }
                Ok((width, height, rgba))
            }
        }
    }
}

#[cfg(test)]
mod tests;
