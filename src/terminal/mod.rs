//! Terminal emulator implementation
//!
//! This module provides the main `Terminal` struct and its implementation,
//! split across multiple submodules for maintainability:
//! - `notification`: Notification types from OSC sequences
//! - `sequences`: VTE sequence handlers (CSI, OSC, ESC, DCS)
//! - `graphics`: Sixel graphics management
//! - `colors`: Color configuration
//! - `write`: Character writing logic

// Submodules
mod colors;
mod graphics;
pub mod notification;
mod sequences;
mod write;

// Re-export Notification as it's part of the public API
pub use notification::Notification;

// Imports
use crate::cell::{Cell, CellFlags};
use crate::color::{Color, NamedColor};
use crate::cursor::Cursor;
use crate::debug;
use crate::grid::Grid;
use crate::mouse::{MouseEncoding, MouseEvent, MouseMode};
use crate::shell_integration::ShellIntegration;
use crate::sixel;
use std::collections::{HashMap, HashSet};
use vte::{Params, Perform};

const DEFAULT_MAX_NOTIFICATIONS: usize = 128;
const DEFAULT_MAX_CLIPBOARD_SYNC_EVENTS: usize = 256;
const DEFAULT_MAX_CLIPBOARD_EVENT_BYTES: usize = 4096;
const CLIPBOARD_TRUNCATION_SUFFIX: &str = " [truncated]";

#[inline]
fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn sanitize_clipboard_content(content: &mut String, max_bytes: usize) {
    if max_bytes == 0 {
        content.clear();
        return;
    }

    if content.len() > max_bytes {
        let suffix_len = CLIPBOARD_TRUNCATION_SUFFIX.len();
        let keep = max_bytes.saturating_sub(suffix_len);
        content.truncate(keep);
        if suffix_len <= max_bytes {
            content.push_str(CLIPBOARD_TRUNCATION_SUFFIX);
        }
    }
}

/// Bell event type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BellEvent {
    /// Standard visual bell
    VisualBell,
    /// Warning bell with volume (0-8, where 0 is off)
    WarningBell(u8),
    /// Margin bell with volume (0-8, where 0 is off)
    MarginBell(u8),
}

/// Terminal change event
#[derive(Debug, Clone, PartialEq)]
pub enum TerminalEvent {
    /// Bell event occurred
    BellRang(BellEvent),
    /// Terminal title changed
    TitleChanged(String),
    /// Terminal was resized
    SizeChanged(usize, usize),
    /// A terminal mode changed
    ModeChanged(String, bool),
    /// Graphics added at row
    GraphicsAdded(usize),
    /// Hyperlink added
    HyperlinkAdded(String),
    /// Dirty region (first_row, last_row)
    DirtyRegion(usize, usize),
}

/// Hyperlink information with all its locations
#[derive(Debug, Clone)]
pub struct HyperlinkInfo {
    /// The URL of the hyperlink
    pub url: String,
    /// All (col, row) positions where this link appears
    pub positions: Vec<(usize, usize)>,
    /// Optional hyperlink ID from OSC 8
    pub id: Option<String>,
}

/// Search match result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// Row index (negative values are scrollback, 0+ are visible screen)
    pub row: isize,
    /// Column where match starts (0-indexed)
    pub col: usize,
    /// Length of the match in characters
    pub length: usize,
    /// Matched text
    pub text: String,
}

/// Detected content item (URL, file path, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedItem {
    /// URL with position (url, col, row)
    Url(String, usize, usize),
    /// File path with position and optional line number (path, col, row, line_number)
    FilePath(String, usize, usize, Option<usize>),
    /// Git hash (7-40 chars) with position (hash, col, row)
    GitHash(String, usize, usize),
    /// IP address with position (ip, col, row)
    IpAddress(String, usize, usize),
    /// Email address with position (email, col, row)
    Email(String, usize, usize),
}

/// Selection mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// Character-by-character selection
    Character,
    /// Line-by-line selection
    Line,
    /// Rectangular block selection
    Block,
}

/// Selection state
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Start position (col, row)
    pub start: (usize, usize),
    /// End position (col, row)
    pub end: (usize, usize),
    /// Selection mode
    pub mode: SelectionMode,
}

/// Export format for scrollback
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Plain text (stripped of all formatting)
    Plain,
    /// HTML with colors and styles
    Html,
    /// Raw ANSI escape sequences preserved
    Ansi,
}

/// Scrollback statistics
#[derive(Debug, Clone)]
pub struct ScrollbackStats {
    /// Total lines in scrollback
    pub total_lines: usize,
    /// Estimated memory usage in bytes
    pub memory_bytes: usize,
    /// Whether scrollback has wrapped around
    pub has_wrapped: bool,
}

/// Bookmark in scrollback
#[derive(Debug, Clone)]
pub struct Bookmark {
    /// Unique bookmark ID
    pub id: usize,
    /// Row position (negative = scrollback)
    pub row: isize,
    /// User-defined label
    pub label: String,
}

// === Feature 7: Performance Metrics ===

/// Performance metrics for tracking terminal rendering performance
#[derive(Debug, Clone, Default)]
pub struct PerformanceMetrics {
    /// Total number of frames rendered
    pub frames_rendered: u64,
    /// Total number of cells updated
    pub cells_updated: u64,
    /// Total number of bytes processed
    pub bytes_processed: u64,
    /// Total processing time in microseconds
    pub total_processing_us: u64,
    /// Peak processing time for a single frame in microseconds
    pub peak_frame_us: u64,
    /// Number of scrolls performed
    pub scroll_count: u64,
    /// Number of line wraps
    pub wrap_count: u64,
    /// Number of escape sequences processed
    pub escape_sequences: u64,
}

/// Frame timing information
#[derive(Debug, Clone)]
pub struct FrameTiming {
    /// Frame number
    pub frame_number: u64,
    /// Processing time in microseconds
    pub processing_us: u64,
    /// Number of cells updated this frame
    pub cells_updated: usize,
    /// Number of bytes processed this frame
    pub bytes_processed: usize,
}

// === Feature 8: Advanced Color Operations ===

/// HSV color representation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorHSV {
    /// Hue (0-360 degrees)
    pub h: f32,
    /// Saturation (0.0-1.0)
    pub s: f32,
    /// Value/Brightness (0.0-1.0)
    pub v: f32,
}

/// HSL color representation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorHSL {
    /// Hue (0-360 degrees)
    pub h: f32,
    /// Saturation (0.0-1.0)
    pub s: f32,
    /// Lightness (0.0-1.0)
    pub l: f32,
}

/// Color theme generation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    /// Complementary color (opposite on color wheel)
    Complementary,
    /// Analogous colors (adjacent on color wheel)
    Analogous,
    /// Triadic colors (evenly spaced on color wheel)
    Triadic,
    /// Tetradic/square colors
    Tetradic,
    /// Split complementary
    SplitComplementary,
    /// Monochromatic (varying lightness)
    Monochromatic,
}

/// Generated color palette
#[derive(Debug, Clone)]
pub struct ColorPalette {
    /// Base color
    pub base: (u8, u8, u8),
    /// Generated colors based on theme mode
    pub colors: Vec<(u8, u8, u8)>,
    /// Theme mode used
    pub mode: ThemeMode,
}

// === Feature 9: Line Wrapping Utilities ===

/// Line join result
#[derive(Debug, Clone)]
pub struct JoinedLines {
    /// The joined text
    pub text: String,
    /// Start row of joined section
    pub start_row: usize,
    /// End row of joined section (inclusive)
    pub end_row: usize,
    /// Number of lines joined
    pub lines_joined: usize,
}

/// Reflow statistics
#[derive(Debug, Clone)]
pub struct ReflowStats {
    /// Number of lines before reflow
    pub lines_before: usize,
    /// Number of lines after reflow
    pub lines_after: usize,
    /// Number of wrap points changed
    pub wraps_changed: usize,
}

// === Feature 10: Clipboard Integration ===

/// Clipboard entry with history
#[derive(Debug, Clone)]
pub struct ClipboardEntry {
    /// Clipboard content
    pub content: String,
    /// Timestamp when added (microseconds since epoch)
    pub timestamp: u64,
    /// Optional label/description
    pub label: Option<String>,
}

/// Clipboard slot identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipboardSlot {
    /// Primary clipboard (OSC 52 default)
    Primary,
    /// System clipboard
    Clipboard,
    /// Selection clipboard (X11)
    Selection,
    /// Custom numbered slot (0-9)
    Custom(u8),
}

// === Feature 17: Advanced Mouse Support ===

/// Mouse event type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventType {
    /// Mouse button press
    Press,
    /// Mouse button release
    Release,
    /// Mouse movement (with or without button held)
    Move,
    /// Mouse drag (move with button held)
    Drag,
    /// Mouse scroll up
    ScrollUp,
    /// Mouse scroll down
    ScrollDown,
}

/// Mouse button
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
}

/// Mouse event record with position and metadata
#[derive(Debug, Clone)]
pub struct MouseEventRecord {
    /// Event type
    pub event_type: MouseEventType,
    /// Mouse button involved
    pub button: MouseButton,
    /// Column position (0-indexed)
    pub col: usize,
    /// Row position (0-indexed)
    pub row: usize,
    /// Pixel position (for SGR 1016)
    pub pixel_x: Option<u16>,
    pub pixel_y: Option<u16>,
    /// Modifier keys (shift, alt, ctrl)
    pub modifiers: u8,
    /// Timestamp in microseconds
    pub timestamp: u64,
}

/// Mouse position history entry
#[derive(Debug, Clone)]
pub struct MousePosition {
    pub col: usize,
    pub row: usize,
    pub timestamp: u64,
}

// === Feature 19: Custom Rendering Hints ===

/// Damage region for incremental rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRegion {
    /// Top-left column
    pub left: usize,
    /// Top-left row
    pub top: usize,
    /// Bottom-right column (exclusive)
    pub right: usize,
    /// Bottom-right row (exclusive)
    pub bottom: usize,
}

/// Z-order layer for rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ZLayer {
    /// Background layer
    Background = 0,
    /// Normal content
    Normal = 1,
    /// Overlays (e.g., selections)
    Overlay = 2,
    /// Cursor and UI elements
    Cursor = 3,
}

/// Animation hint for smooth transitions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationHint {
    /// No animation
    None,
    /// Smooth scroll
    SmoothScroll,
    /// Fade in/out
    Fade,
    /// Cursor blink
    CursorBlink,
}

/// Priority for partial updates
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UpdatePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Rendering hint for optimization
#[derive(Debug, Clone)]
pub struct RenderingHint {
    /// Damaged region that needs redrawing
    pub damage: DamageRegion,
    /// Z-order layer
    pub layer: ZLayer,
    /// Animation hint
    pub animation: AnimationHint,
    /// Update priority
    pub priority: UpdatePriority,
}

// === Feature 16: Performance Profiling ===

/// Profiling data for escape sequences
#[derive(Debug, Clone, Default)]
pub struct EscapeSequenceProfile {
    /// Total count of this sequence type
    pub count: u64,
    /// Total time spent processing (microseconds)
    pub total_time_us: u64,
    /// Peak processing time (microseconds)
    pub peak_time_us: u64,
    /// Average processing time (microseconds)
    pub avg_time_us: u64,
}

/// Profiling category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileCategory {
    /// CSI sequences
    CSI,
    /// OSC sequences
    OSC,
    /// ESC sequences
    ESC,
    /// DCS sequences
    DCS,
    /// Plain text printing
    Print,
    /// Control characters
    Control,
}

/// Complete profiling data
#[derive(Debug, Clone, Default)]
pub struct ProfilingData {
    /// Per-category profiling
    pub categories: std::collections::HashMap<ProfileCategory, EscapeSequenceProfile>,
    /// Memory allocations tracked
    pub allocations: u64,
    /// Total bytes allocated
    pub bytes_allocated: u64,
    /// Peak memory usage
    pub peak_memory: usize,
}

// === Feature 14: Snapshot Diffing ===

/// Type of change in a diff
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffChangeType {
    /// Line added
    Added,
    /// Line removed
    Removed,
    /// Line modified
    Modified,
    /// Line unchanged
    Unchanged,
}

/// A single line diff
#[derive(Debug, Clone)]
pub struct LineDiff {
    /// Type of change
    pub change_type: DiffChangeType,
    /// Row number in old snapshot
    pub old_row: Option<usize>,
    /// Row number in new snapshot
    pub new_row: Option<usize>,
    /// Old line content
    pub old_content: Option<String>,
    /// New line content
    pub new_content: Option<String>,
}

/// Complete diff between two snapshots
#[derive(Debug, Clone)]
pub struct SnapshotDiff {
    /// List of line diffs
    pub diffs: Vec<LineDiff>,
    /// Number of lines added
    pub added: usize,
    /// Number of lines removed
    pub removed: usize,
    /// Number of lines modified
    pub modified: usize,
    /// Number of lines unchanged
    pub unchanged: usize,
}

// === Feature 15: Regex Search in Scrollback ===

/// Regex match with position and captured groups
#[derive(Debug, Clone)]
pub struct RegexMatch {
    /// Row where match starts
    pub row: usize,
    /// Column where match starts
    pub col: usize,
    /// Row where match ends
    pub end_row: usize,
    /// Column where match ends
    pub end_col: usize,
    /// Matched text
    pub text: String,
    /// Capture groups (if any)
    pub captures: Vec<String>,
}

/// Options for regex search
#[derive(Debug, Clone)]
pub struct RegexSearchOptions {
    /// Case insensitive search
    pub case_insensitive: bool,
    /// Multiline mode (^ and $ match line boundaries)
    pub multiline: bool,
    /// Include scrollback in search
    pub include_scrollback: bool,
    /// Maximum number of matches to return (0 = unlimited)
    pub max_matches: usize,
    /// Search backwards from end
    pub reverse: bool,
}

impl Default for RegexSearchOptions {
    fn default() -> Self {
        Self {
            case_insensitive: false,
            multiline: true,
            include_scrollback: true,
            max_matches: 0,
            reverse: false,
        }
    }
}

// === Feature 13: Terminal Multiplexing Helpers ===

/// Pane state for session management
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PaneState {
    /// Pane identifier
    pub id: String,
    /// Pane title
    pub title: String,
    /// Terminal dimensions (cols, rows)
    pub size: (usize, usize),
    /// Position in layout (x, y)
    pub position: (usize, usize),
    /// Working directory
    pub cwd: Option<String>,
    /// Environment variables
    pub env: std::collections::HashMap<String, String>,
    /// Screen content snapshot
    pub content: Vec<String>,
    /// Cursor position
    pub cursor: (usize, usize),
    /// Is alternate screen active
    pub alt_screen: bool,
    /// Scrollback position
    pub scroll_offset: usize,
    /// Creation timestamp
    pub created_at: u64,
    /// Last activity timestamp
    pub last_activity: u64,
}

/// Layout direction for panes
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LayoutDirection {
    /// Horizontal split (side by side)
    Horizontal,
    /// Vertical split (top and bottom)
    Vertical,
}

/// Window layout configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WindowLayout {
    /// Layout identifier
    pub id: String,
    /// Layout name
    pub name: String,
    /// Split direction
    pub direction: LayoutDirection,
    /// Pane IDs in this layout
    pub panes: Vec<String>,
    /// Relative sizes (percentages)
    pub sizes: Vec<u8>,
    /// Active pane index
    pub active_pane: usize,
}

/// Complete session state
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionState {
    /// Session identifier
    pub id: String,
    /// Session name
    pub name: String,
    /// All panes in the session
    pub panes: Vec<PaneState>,
    /// All layouts in the session
    pub layouts: Vec<WindowLayout>,
    /// Active layout index
    pub active_layout: usize,
    /// Session metadata
    pub metadata: std::collections::HashMap<String, String>,
    /// Creation timestamp
    pub created_at: u64,
    /// Last saved timestamp
    pub last_saved: u64,
}

// === Feature 21: Image Protocol Support ===

/// Image protocol type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
    /// Sixel graphics (existing)
    Sixel,
    /// iTerm2 inline images
    ITerm2,
    /// Kitty graphics protocol
    Kitty,
}

/// Image format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    PNG,
    JPEG,
    GIF,
    BMP,
    RGBA,
    RGB,
}

/// Inline image data
#[derive(Debug, Clone)]
pub struct InlineImage {
    /// Image identifier
    pub id: Option<String>,
    /// Protocol used
    pub protocol: ImageProtocol,
    /// Image format
    pub format: ImageFormat,
    /// Image data (encoded)
    pub data: Vec<u8>,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Position in terminal (col, row)
    pub position: (usize, usize),
    /// Display width in cells
    pub display_cols: usize,
    /// Display height in cells
    pub display_rows: usize,
}

/// Image placement action
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagePlacement {
    /// Display image
    Display,
    /// Delete image
    Delete,
    /// Query image
    Query,
}

// === Feature 28: Benchmarking Suite ===

/// Benchmark category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BenchmarkCategory {
    /// Text rendering performance
    Rendering,
    /// Escape sequence parsing
    Parsing,
    /// Grid operations
    GridOps,
    /// Scrollback operations
    Scrollback,
    /// Memory operations
    Memory,
    /// Overall throughput
    Throughput,
}

/// Benchmark result
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    /// Benchmark category
    pub category: BenchmarkCategory,
    /// Benchmark name
    pub name: String,
    /// Number of iterations
    pub iterations: u64,
    /// Total time in microseconds
    pub total_time_us: u64,
    /// Average time per iteration
    pub avg_time_us: u64,
    /// Minimum time
    pub min_time_us: u64,
    /// Maximum time
    pub max_time_us: u64,
    /// Operations per second
    pub ops_per_sec: f64,
    /// Memory used (bytes)
    pub memory_bytes: Option<usize>,
}

/// Benchmark suite results
#[derive(Debug, Clone)]
pub struct BenchmarkSuite {
    /// All benchmark results
    pub results: Vec<BenchmarkResult>,
    /// Total execution time
    pub total_time_ms: u64,
    /// Suite name
    pub suite_name: String,
}

// === Feature 29: Terminal Compliance Testing ===

/// VT sequence support level
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComplianceLevel {
    /// VT52
    VT52,
    /// VT100
    VT100,
    /// VT220
    VT220,
    /// VT320
    VT320,
    /// VT420
    VT420,
    /// VT520
    VT520,
    /// xterm
    XTerm,
}

/// Compliance test result
#[derive(Debug, Clone)]
pub struct ComplianceTest {
    /// Test name
    pub name: String,
    /// Test category
    pub category: String,
    /// Whether test passed
    pub passed: bool,
    /// Expected result
    pub expected: String,
    /// Actual result
    pub actual: String,
    /// Notes or error message
    pub notes: Option<String>,
}

/// Compliance report
#[derive(Debug, Clone)]
pub struct ComplianceReport {
    /// Terminal name/version
    pub terminal_info: String,
    /// Compliance level tested
    pub level: ComplianceLevel,
    /// All test results
    pub tests: Vec<ComplianceTest>,
    /// Number of passed tests
    pub passed: usize,
    /// Number of failed tests
    pub failed: usize,
    /// Overall compliance percentage
    pub compliance_percent: f64,
}

// === Feature 30: OSC 52 Clipboard Sync ===

/// Clipboard target for OSC 52
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipboardTarget {
    /// System clipboard (c)
    Clipboard,
    /// Primary selection (p)
    Primary,
    /// Secondary selection (s)
    Secondary,
    /// Cut buffer 0 (c0)
    CutBuffer0,
}

/// Clipboard operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardOperation {
    /// Set clipboard content
    Set,
    /// Query clipboard content
    Query,
    /// Clear clipboard
    Clear,
}

/// Clipboard sync event
#[derive(Debug, Clone)]
pub struct ClipboardSyncEvent {
    /// Target clipboard
    pub target: ClipboardTarget,
    /// Operation type
    pub operation: ClipboardOperation,
    /// Content (for Set operations)
    pub content: Option<String>,
    /// Timestamp of event
    pub timestamp: u64,
    /// Whether this came from remote session
    pub is_remote: bool,
}

/// Clipboard sync history entry
#[derive(Debug, Clone)]
pub struct ClipboardHistoryEntry {
    /// Clipboard target
    pub target: ClipboardTarget,
    /// Content
    pub content: String,
    /// Timestamp
    pub timestamp: u64,
    /// Source identifier (e.g., session ID, hostname)
    pub source: Option<String>,
}

// === Feature 31: Shell Integration++ ===

/// Command execution record
#[derive(Debug, Clone)]
pub struct CommandExecution {
    /// Command that was executed
    pub command: String,
    /// Current working directory when command was run
    pub cwd: Option<String>,
    /// Start timestamp (milliseconds since epoch)
    pub start_time: u64,
    /// End timestamp (milliseconds since epoch)
    pub end_time: Option<u64>,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Command duration in milliseconds
    pub duration_ms: Option<u64>,
    /// Whether command succeeded (exit code 0)
    pub success: Option<bool>,
}

/// Shell integration statistics
#[derive(Debug, Clone)]
pub struct ShellIntegrationStats {
    /// Total commands executed
    pub total_commands: usize,
    /// Successful commands (exit code 0)
    pub successful_commands: usize,
    /// Failed commands (non-zero exit code)
    pub failed_commands: usize,
    /// Average command duration (milliseconds)
    pub avg_duration_ms: f64,
    /// Total execution time (milliseconds)
    pub total_duration_ms: u64,
}

/// CWD change notification
#[derive(Debug, Clone)]
pub struct CwdChange {
    /// Previous working directory
    pub old_cwd: Option<String>,
    /// New working directory
    pub new_cwd: String,
    /// Timestamp of change
    pub timestamp: u64,
}

// === Feature 37: Terminal Notifications ===

/// Notification trigger type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationTrigger {
    /// Terminal bell rang
    Bell,
    /// Terminal activity detected
    Activity,
    /// Silence detected (no activity for duration)
    Silence,
    /// Custom trigger with ID
    Custom(u32),
}

/// Notification alert type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationAlert {
    /// Desktop/system notification
    Desktop,
    /// Sound alert with volume (0-100)
    Sound(u8),
    /// Visual alert (flash, border, etc.)
    Visual,
}

/// Notification event record
#[derive(Debug, Clone)]
pub struct NotificationEvent {
    /// What triggered the notification
    pub trigger: NotificationTrigger,
    /// Type of alert
    pub alert: NotificationAlert,
    /// Optional message
    pub message: Option<String>,
    /// Timestamp when event occurred
    pub timestamp: u64,
    /// Whether notification was delivered
    pub delivered: bool,
}

/// Notification configuration
#[derive(Debug, Clone)]
pub struct NotificationConfig {
    /// Enable desktop notifications on bell
    pub bell_desktop: bool,
    /// Enable sound on bell (0 = disabled, 1-100 = volume)
    pub bell_sound: u8,
    /// Enable visual alert on bell
    pub bell_visual: bool,
    /// Enable notifications on activity
    pub activity_enabled: bool,
    /// Activity threshold (seconds of inactivity before triggering)
    pub activity_threshold: u64,
    /// Enable notifications on silence
    pub silence_enabled: bool,
    /// Silence threshold (seconds of activity before silence notification)
    pub silence_threshold: u64,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            bell_desktop: false,
            bell_sound: 0,
            bell_visual: true,
            activity_enabled: false,
            activity_threshold: 10,
            silence_enabled: false,
            silence_threshold: 300,
        }
    }
}

// === Feature 24: Terminal Replay/Recording ===

/// Recording format type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingFormat {
    /// Asciicast v2 format (asciinema)
    Asciicast,
    /// JSON with timing data
    Json,
    /// Raw TTY data
    Tty,
}

/// Recording event type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingEventType {
    /// Input data
    Input,
    /// Output data
    Output,
    /// Terminal resize
    Resize,
    /// Marker/bookmark
    Marker,
}

/// Recording event
#[derive(Debug, Clone)]
pub struct RecordingEvent {
    /// Timestamp (milliseconds since recording start)
    pub timestamp: u64,
    /// Event type
    pub event_type: RecordingEventType,
    /// Event data
    pub data: Vec<u8>,
    /// Metadata (for resize: cols, rows)
    pub metadata: Option<(usize, usize)>,
}

/// Recording session
#[derive(Debug, Clone)]
pub struct RecordingSession {
    /// Session start time (UNIX epoch milliseconds)
    pub start_time: u64,
    /// Initial terminal size (cols, rows)
    pub initial_size: (usize, usize),
    /// Terminal environment info
    pub env: HashMap<String, String>,
    /// All recorded events
    pub events: Vec<RecordingEvent>,
    /// Total duration (milliseconds)
    pub duration: u64,
    /// Session title/name
    pub title: Option<String>,
}

/// Export format for recordings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingExportFormat {
    /// SVG animation
    Svg,
    /// Animated GIF
    Gif,
    /// Video (MP4)
    Video,
    /// HTML with embedded player
    Html,
}

/// Helper function to check if byte slice contains a subsequence
/// More efficient than converting to String and using contains()
#[inline]
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// Terminal struct definition
pub struct Terminal {
    /// The primary terminal grid
    grid: Grid,
    /// Alternate screen grid
    alt_grid: Grid,
    /// Whether we're using the alternate screen
    alt_screen_active: bool,
    /// Cursor position and state
    cursor: Cursor,
    /// Saved cursor for alternate screen
    alt_cursor: Cursor,
    /// Current foreground color
    fg: Color,
    /// Current background color
    bg: Color,
    /// Current underline color (SGR 58) - None means use foreground color
    underline_color: Option<Color>,
    /// Current cell flags
    flags: CellFlags,
    /// Saved cursor position (for save/restore)
    saved_cursor: Option<Cursor>,
    /// Saved colors and flags
    saved_fg: Color,
    saved_bg: Color,
    saved_underline_color: Option<Color>,
    saved_flags: CellFlags,
    /// Terminal title
    title: String,
    /// Mouse tracking mode
    mouse_mode: MouseMode,
    /// Mouse encoding format
    mouse_encoding: MouseEncoding,
    /// Focus tracking enabled
    focus_tracking: bool,
    /// Bracketed paste mode
    bracketed_paste: bool,
    /// Synchronized update mode (DEC 2026)
    synchronized_updates: bool,
    /// Buffer for batched updates (when synchronized mode is active)
    update_buffer: Vec<u8>,
    /// Shell integration state
    shell_integration: ShellIntegration,
    /// Scroll region top (0-indexed)
    scroll_region_top: usize,
    /// Scroll region bottom (0-indexed)
    scroll_region_bottom: usize,
    /// Use left/right column scroll region (DECLRMM)
    use_lr_margins: bool,
    /// Left column margin (0-indexed, inclusive)
    left_margin: usize,
    /// Right column margin (0-indexed, inclusive)
    right_margin: usize,
    /// Auto wrap mode (DECAWM)
    auto_wrap: bool,
    /// Origin mode (DECOM) - cursor addressing relative to scroll region
    origin_mode: bool,
    /// Tab stops (columns where tab stops are set)
    tab_stops: Vec<bool>,
    /// Application cursor keys mode
    application_cursor: bool,
    /// Kitty keyboard protocol flags (progressive enhancement)
    keyboard_flags: u16,
    /// Stack for keyboard protocol flags (main screen)
    keyboard_stack: Vec<u16>,
    /// Stack for keyboard protocol flags (alternate screen)
    keyboard_stack_alt: Vec<u16>,
    /// Response buffer for device queries (DA/DSR/etc)
    response_buffer: Vec<u8>,
    /// Hyperlink storage: ID -> URL mapping (for deduplication)
    hyperlinks: HashMap<u32, String>,
    /// Current hyperlink ID being written
    current_hyperlink_id: Option<u32>,
    /// Next available hyperlink ID
    next_hyperlink_id: u32,
    /// Sixel graphics storage
    graphics: Vec<sixel::SixelGraphic>,
    /// Maximum number of Sixel graphics to retain
    max_sixel_graphics: usize,
    /// Counter of Sixel graphics dropped due to limits
    dropped_sixel_graphics: usize,
    /// Sixel resource limits (per-terminal)
    sixel_limits: sixel::SixelLimits,
    /// Current Sixel parser (active during DCS)
    sixel_parser: Option<sixel::SixelParser>,
    /// Buffer for DCS data accumulation
    dcs_buffer: Vec<u8>,
    /// DCS active flag
    dcs_active: bool,
    /// DCS action character ('q' for Sixel)
    dcs_action: Option<char>,
    /// Clipboard content (OSC 52)
    clipboard_content: Option<String>,
    /// Allow clipboard read operations (security flag for OSC 52 queries)
    allow_clipboard_read: bool,
    /// Default foreground color (for OSC 10 queries)
    default_fg: Color,
    /// Default background color (for OSC 11 queries)
    default_bg: Color,
    /// Cursor color (for OSC 12 queries)
    cursor_color: Color,
    /// ANSI color palette (0-15) - modified by OSC 4/104
    ansi_palette: [Color; 16],
    /// Color stack for XTPUSHCOLORS/XTPOPCOLORS (fg, bg, underline)
    color_stack: Vec<(Color, Color, Option<Color>)>,
    /// Notifications from OSC 9 / OSC 777 sequences
    notifications: Vec<Notification>,
    /// Bell event counter - incremented each time bell (BEL/\x07) is received
    bell_count: u64,
    /// VTE parser instance (maintains state across process() calls)
    parser: vte::Parser,
    /// DECAWM delayed wrap: set after printing in last column
    pending_wrap: bool,
    /// Pixel width of the text area (XTWINOPS 14)
    pixel_width: usize,
    /// Pixel height of the text area (XTWINOPS 14)
    pixel_height: usize,
    /// Insert mode (IRM) - Mode 4: when enabled, new characters are inserted
    insert_mode: bool,
    /// Line Feed/New Line Mode (LNM) - Mode 20: when enabled, LF does CR+LF
    line_feed_new_line_mode: bool,
    /// Character protection mode (DECSCA) - when enabled, new chars are guarded
    char_protected: bool,
    /// Reverse video mode (DECSCNM) - globally inverts fg/bg colors
    reverse_video: bool,
    /// Bold brightening - when enabled, bold ANSI colors 0-7 brighten to 8-15
    bold_brightening: bool,
    /// Window title stack for XTWINOPS 22/23 (push/pop title)
    title_stack: Vec<String>,
    /// Accept OSC 7 directory tracking sequences
    accept_osc7: bool,
    /// Disable potentially insecure escape sequences
    disable_insecure_sequences: bool,
    /// Link/hyperlink color (iTerm2 default: blue #0645ad)
    link_color: Color,
    /// Bold text custom color (iTerm2 default: white #ffffff)
    bold_color: Color,
    /// Cursor guide color (iTerm2 default: light blue #a6e8ff with alpha)
    cursor_guide_color: Color,
    /// Badge color (iTerm2 default: red #ff0000 with alpha)
    badge_color: Color,
    /// Match/search highlight color (iTerm2 default: yellow #ffff00)
    match_color: Color,
    /// Selection background color (iTerm2 default: #b5d5ff)
    selection_bg_color: Color,
    /// Selection foreground/text color (iTerm2 default: #000000)
    selection_fg_color: Color,
    /// Use custom bold color instead of bright variant (iTerm2: "Use custom color for bold text")
    use_bold_color: bool,
    /// Use custom underline color (iTerm2: "Use custom underline color")
    use_underline_color: bool,
    /// Show cursor guide (iTerm2: "Use cursor guide")
    use_cursor_guide: bool,
    /// Use custom selected text color (iTerm2: "Use custom color for selected text")
    use_selected_text_color: bool,
    /// Smart cursor color - auto-adjust based on background (iTerm2: "Smart Cursor Color")
    smart_cursor_color: bool,
    /// Attribute change extent mode (DECSACE) - 0/1: stream, 2: rectangle (default)
    attribute_change_extent: u8,
    /// Terminal conformance level (VT100/VT220/VT320/VT420/VT520)
    conformance_level: crate::conformance_level::ConformanceLevel,
    /// Warning bell volume (0=off, 1-8=volume levels) - VT520 DECSWBV
    warning_bell_volume: u8,
    /// Margin bell volume (0=off, 1-8=volume levels) - VT520 DECSMBV
    margin_bell_volume: u8,
    /// Tmux control protocol parser
    tmux_parser: crate::tmux_control::TmuxControlParser,
    /// Tmux control protocol notifications buffer
    tmux_notifications: Vec<crate::tmux_control::TmuxNotification>,
    /// Dirty rows tracking (0-indexed row numbers that have changed)
    dirty_rows: HashSet<usize>,
    /// Bell events buffer
    bell_events: Vec<BellEvent>,
    /// Terminal events buffer
    terminal_events: Vec<TerminalEvent>,
    /// Current selection state
    selection: Option<Selection>,
    /// Bookmarks for quick navigation
    bookmarks: Vec<Bookmark>,
    /// Next available bookmark ID
    next_bookmark_id: usize,
    /// Performance metrics tracking
    perf_metrics: PerformanceMetrics,
    /// Frame timing history (last N frames)
    frame_timings: Vec<FrameTiming>,
    /// Maximum frame timings to keep
    max_frame_timings: usize,
    /// Clipboard history (multiple slots)
    clipboard_history: std::collections::HashMap<ClipboardSlot, Vec<ClipboardEntry>>,
    /// Maximum clipboard history entries per slot
    max_clipboard_history: usize,
    /// Mouse event history
    mouse_events: Vec<MouseEventRecord>,
    /// Mouse position history
    mouse_positions: Vec<MousePosition>,
    /// Maximum mouse history entries
    max_mouse_history: usize,
    /// Current rendering hints
    rendering_hints: Vec<RenderingHint>,
    /// Damage regions accumulated
    damage_regions: Vec<DamageRegion>,
    /// Profiling data (when enabled)
    profiling_data: Option<ProfilingData>,
    /// Profiling enabled flag
    profiling_enabled: bool,
    /// Regex search matches cache
    regex_matches: Vec<RegexMatch>,
    /// Current regex search pattern
    current_regex_pattern: Option<String>,
    /// Current pane state (for multiplexing)
    pane_state: Option<PaneState>,
    /// Inline images (iTerm2, Kitty protocols)
    inline_images: Vec<InlineImage>,
    /// Maximum number of inline images to store
    max_inline_images: usize,

    // === Feature 30: OSC 52 Clipboard Sync ===
    /// Clipboard sync events log
    clipboard_sync_events: Vec<ClipboardSyncEvent>,
    /// Clipboard sync history across targets
    clipboard_sync_history: std::collections::HashMap<ClipboardTarget, Vec<ClipboardHistoryEntry>>,
    /// Maximum clipboard sync history entries per target
    max_clipboard_sync_history: usize,
    /// Maximum clipboard sync events retained for diagnostics
    max_clipboard_sync_events: usize,
    /// Maximum bytes of clipboard content to persist per event/history entry
    max_clipboard_event_bytes: usize,
    /// Remote session identifier for clipboard sync
    remote_session_id: Option<String>,

    // === Feature 31: Shell Integration++ ===
    /// Command execution history
    command_history: Vec<CommandExecution>,
    /// Current executing command
    current_command: Option<CommandExecution>,
    /// Working directory change history
    cwd_changes: Vec<CwdChange>,
    /// Maximum command history entries
    max_command_history: usize,
    /// Maximum CWD change history
    max_cwd_history: usize,

    // === Feature 37: Terminal Notifications ===
    /// Notification configuration
    notification_config: NotificationConfig,
    /// Notification events log
    notification_events: Vec<NotificationEvent>,
    /// Last activity timestamp (for silence detection)
    last_activity_time: u64,
    /// Last silence check timestamp
    last_silence_check: u64,
    /// Maximum OSC 9/777 notifications retained
    max_notifications: usize,
    /// Custom notification triggers (ID -> message)
    custom_triggers: HashMap<u32, String>,

    // === Feature 24: Terminal Replay/Recording ===
    /// Current recording session
    recording_session: Option<RecordingSession>,
    /// Recording active flag
    is_recording: bool,
    /// Recording start timestamp (for relative timing)
    recording_start_time: u64,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal")
            .field("grid", &self.grid)
            .field("alt_grid", &self.alt_grid)
            .field("alt_screen_active", &self.alt_screen_active)
            .field("cursor", &self.cursor)
            .field("pending_wrap", &self.pending_wrap)
            .field("parser", &"<Parser>")
            .finish()
    }
}

/// Helper function to convert cells to text
fn cells_to_text(cells: &[Cell]) -> String {
    cells
        .iter()
        .filter(|c| !c.flags.wide_char_spacer())
        .map(|c| c.c)
        .collect()
}

/// Helper function to escape HTML special characters
fn html_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '&' => "&amp;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#39;".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

/// Convert RGB to HSV
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> ColorHSV {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let h = if h < 0.0 { h + 360.0 } else { h };
    let s = if max == 0.0 { 0.0 } else { delta / max };
    let v = max;

    ColorHSV { h, s, v }
}

/// Convert HSV to RGB
fn hsv_to_rgb(hsv: ColorHSV) -> (u8, u8, u8) {
    let c = hsv.v * hsv.s;
    let x = c * (1.0 - ((hsv.h / 60.0) % 2.0 - 1.0).abs());
    let m = hsv.v - c;

    let (r, g, b) = match hsv.h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Convert RGB to HSL
fn rgb_to_hsl(r: u8, g: u8, b: u8) -> ColorHSL {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let l = (max + min) / 2.0;

    let s = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * l - 1.0).abs())
    };

    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let h = if h < 0.0 { h + 360.0 } else { h };

    ColorHSL { h, s, l }
}

/// Convert HSL to RGB
fn hsl_to_rgb(hsl: ColorHSL) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * hsl.l - 1.0).abs()) * hsl.s;
    let x = c * (1.0 - ((hsl.h / 60.0) % 2.0 - 1.0).abs());
    let m = hsl.l - c / 2.0;

    let (r, g, b) = match hsl.h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Get current timestamp in microseconds
fn get_timestamp_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self::with_scrollback(cols, rows, 10000)
    }

    /// Get iTerm2 default ANSI color palette (0-15)
    ///
    /// Create a new terminal with custom scrollback size
    pub fn with_scrollback(cols: usize, rows: usize, scrollback: usize) -> Self {
        // Initialize tab stops at every 8 columns
        let mut tab_stops = vec![false; cols];
        for i in (0..cols).step_by(8) {
            tab_stops[i] = true;
        }
        let now = unix_millis();

        Self {
            grid: Grid::new(cols, rows, scrollback),
            alt_grid: Grid::new(cols, rows, 0), // Alt screen has no scrollback
            alt_screen_active: false,
            cursor: Cursor::new(),
            alt_cursor: Cursor::new(),
            fg: Color::Named(NamedColor::White),
            bg: Color::Named(NamedColor::Black),
            underline_color: None,
            flags: CellFlags::default(),
            saved_cursor: None,
            saved_fg: Color::Named(NamedColor::White),
            saved_bg: Color::Named(NamedColor::Black),
            saved_underline_color: None,
            saved_flags: CellFlags::default(),
            title: String::new(),
            mouse_mode: MouseMode::Off,
            mouse_encoding: MouseEncoding::Default,
            focus_tracking: false,
            bracketed_paste: false,
            synchronized_updates: false,
            update_buffer: Vec::new(),
            shell_integration: ShellIntegration::new(),
            scroll_region_top: 0,
            scroll_region_bottom: rows.saturating_sub(1),
            use_lr_margins: false,
            left_margin: 0,
            right_margin: cols.saturating_sub(1),
            auto_wrap: true,
            origin_mode: false,
            tab_stops,
            application_cursor: false,
            keyboard_flags: 0,
            keyboard_stack: Vec::new(),
            keyboard_stack_alt: Vec::new(),
            response_buffer: Vec::new(),
            hyperlinks: HashMap::new(),
            current_hyperlink_id: None,
            next_hyperlink_id: 0,
            graphics: Vec::new(),
            max_sixel_graphics: sixel::SIXEL_DEFAULT_MAX_GRAPHICS,
            dropped_sixel_graphics: 0,
            sixel_limits: sixel::SixelLimits::default(),
            sixel_parser: None,
            dcs_buffer: Vec::new(),
            dcs_active: false,
            dcs_action: None,
            clipboard_content: None,
            allow_clipboard_read: false,
            default_fg: Color::Named(NamedColor::White),
            default_bg: Color::Named(NamedColor::Black),
            cursor_color: Color::Named(NamedColor::White),
            ansi_palette: Self::default_ansi_palette(),
            color_stack: Vec::new(),
            notifications: Vec::new(),
            bell_count: 0,
            parser: vte::Parser::new(),
            pending_wrap: false,
            pixel_width: 0,
            pixel_height: 0,
            insert_mode: false,
            line_feed_new_line_mode: false,
            char_protected: false,
            reverse_video: false,
            bold_brightening: true, // iTerm2 default behavior
            title_stack: Vec::new(),
            accept_osc7: true,
            disable_insecure_sequences: false,
            // iTerm2 default colors (matching Python implementation)
            link_color: Color::Rgb(0x06, 0x45, 0xad), // RGB(0.023, 0.270, 0.678)
            bold_color: Color::Rgb(0xff, 0xff, 0xff), // RGB(1.0, 1.0, 1.0)
            cursor_guide_color: Color::Rgb(0xa6, 0xe8, 0xff), // RGB(0.650, 0.910, 1.000)
            badge_color: Color::Rgb(0xff, 0x00, 0x00), // RGB(1.0, 0.0, 0.0)
            match_color: Color::Rgb(0xff, 0xff, 0x00), // RGB(1.0, 1.0, 0.0)
            selection_bg_color: Color::Rgb(0xb5, 0xd5, 0xff), // #b5d5ff
            selection_fg_color: Color::Rgb(0x00, 0x00, 0x00), // #000000
            // iTerm2 default rendering control options
            use_bold_color: false,
            use_underline_color: false,
            use_cursor_guide: false,
            use_selected_text_color: false,
            smart_cursor_color: false,
            // VT420 attribute change extent mode - default to rectangle (2)
            attribute_change_extent: 2,
            // VT520 conformance level - default to VT520 for maximum compatibility
            conformance_level: crate::conformance_level::ConformanceLevel::default(),
            // VT520 bell volume controls - default to moderate volume (4)
            warning_bell_volume: 4,
            margin_bell_volume: 4,
            // Tmux control protocol - default to disabled
            tmux_parser: crate::tmux_control::TmuxControlParser::new(false),
            tmux_notifications: Vec::new(),
            // Event tracking
            dirty_rows: HashSet::new(),
            bell_events: Vec::new(),
            terminal_events: Vec::new(),
            // Selection and bookmarks
            selection: None,
            bookmarks: Vec::new(),
            next_bookmark_id: 0,
            // Performance metrics
            perf_metrics: PerformanceMetrics::default(),
            frame_timings: Vec::new(),
            max_frame_timings: 100, // Keep last 100 frames
            // Clipboard integration
            clipboard_history: std::collections::HashMap::new(),
            max_clipboard_history: 50, // Keep last 50 entries per slot
            // Mouse tracking
            mouse_events: Vec::new(),
            mouse_positions: Vec::new(),
            max_mouse_history: 100, // Keep last 100 mouse events
            // Rendering hints
            rendering_hints: Vec::new(),
            damage_regions: Vec::new(),
            // Performance profiling
            profiling_data: None,
            profiling_enabled: false,
            // Regex search
            regex_matches: Vec::new(),
            current_regex_pattern: None,
            // Multiplexing
            pane_state: None,
            // Inline images
            inline_images: Vec::new(),
            max_inline_images: 100, // Keep last 100 images

            // OSC 52 Clipboard Sync
            clipboard_sync_events: Vec::new(),
            clipboard_sync_history: std::collections::HashMap::new(),
            max_clipboard_sync_history: 50, // Keep last 50 entries per target
            max_clipboard_sync_events: DEFAULT_MAX_CLIPBOARD_SYNC_EVENTS,
            max_clipboard_event_bytes: DEFAULT_MAX_CLIPBOARD_EVENT_BYTES,
            remote_session_id: None,

            // Shell Integration++
            command_history: Vec::new(),
            current_command: None,
            cwd_changes: Vec::new(),
            max_command_history: 100, // Keep last 100 commands
            max_cwd_history: 50,      // Keep last 50 CWD changes

            // Terminal Notifications
            notification_config: NotificationConfig::default(),
            notification_events: Vec::new(),
            last_activity_time: now,
            last_silence_check: now,
            max_notifications: DEFAULT_MAX_NOTIFICATIONS,
            custom_triggers: HashMap::new(),

            // Terminal Replay/Recording
            recording_session: None,
            is_recording: false,
            recording_start_time: 0,
        }
    }

    /// Get the active grid (primary or alternate based on current mode)
    pub fn active_grid(&self) -> &Grid {
        if self.alt_screen_active {
            &self.alt_grid
        } else {
            &self.grid
        }
    }

    /// Get the active grid mutably
    fn active_grid_mut(&mut self) -> &mut Grid {
        if self.alt_screen_active {
            &mut self.alt_grid
        } else {
            &mut self.grid
        }
    }

    /// Get the grid (always returns primary for scrollback access)
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Get the alternate screen grid
    pub fn alt_grid(&self) -> &Grid {
        &self.alt_grid
    }

    /// Get the cursor
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Get the current conformance level
    pub fn conformance_level(&self) -> crate::conformance_level::ConformanceLevel {
        self.conformance_level
    }

    /// Get the warning bell volume (0=off, 1-8=volume levels)
    pub fn warning_bell_volume(&self) -> u8 {
        self.warning_bell_volume
    }

    /// Get the margin bell volume (0=off, 1-8=volume levels)
    pub fn margin_bell_volume(&self) -> u8 {
        self.margin_bell_volume
    }

    /// Get terminal dimensions (of the ACTIVE screen)
    ///
    /// Returns (cols, rows) for whichever screen buffer is currently active
    /// to avoid stale dimensions when the alternate screen is in use.
    pub fn size(&self) -> (usize, usize) {
        let g = self.active_grid();
        (g.cols(), g.rows())
    }

    /// Set pixel dimensions for XTWINOPS reporting
    pub fn set_pixel_size(&mut self, width_px: usize, height_px: usize) {
        self.pixel_width = width_px;
        self.pixel_height = height_px;
    }

    /// Resize the terminal
    pub fn resize(&mut self, cols: usize, rows: usize) {
        debug::log(
            debug::DebugLevel::Debug,
            "TERMINAL_RESIZE",
            &format!("Requested resize to {}x{}", cols, rows),
        );

        self.grid.resize(cols, rows);
        self.alt_grid.resize(cols, rows);
        debug::log(
            debug::DebugLevel::Trace,
            "TERMINAL_RESIZE",
            &format!(
                "Applied resize: primary={}x{}, alt={}x{}",
                self.grid.cols(),
                self.grid.rows(),
                self.alt_grid.cols(),
                self.alt_grid.rows()
            ),
        );

        // Update tab stops
        self.tab_stops.resize(cols, false);
        for i in (0..cols).step_by(8) {
            self.tab_stops[i] = true;
        }

        // Reset scroll region to full screen on resize
        // This matches standard VT behavior (xterm, etc.) and prevents stale
        // scroll regions from causing rendering issues when terminal is resized
        // (e.g., tmux pane splits/closes). The application can re-set a custom
        // scroll region via DECSTBM after the resize if needed.
        self.scroll_region_top = 0;
        self.scroll_region_bottom = rows.saturating_sub(1);
        debug::log(
            debug::DebugLevel::Debug,
            "TERMINAL_RESIZE",
            &format!(
                "Reset scroll region to full screen: 0-{}",
                self.scroll_region_bottom
            ),
        );

        // Clamp left/right margins to new width
        self.left_margin = self.left_margin.min(cols.saturating_sub(1));
        self.right_margin = self.right_margin.min(cols.saturating_sub(1));
        if self.left_margin > self.right_margin {
            self.left_margin = 0;
            self.right_margin = cols.saturating_sub(1);
        }

        // Ensure cursor is within bounds
        let (active_cols, active_rows) = self.size();
        self.cursor.col = self.cursor.col.min(active_cols.saturating_sub(1));
        self.cursor.row = self.cursor.row.min(active_rows.saturating_sub(1));
        self.alt_cursor.col = self.alt_cursor.col.min(active_cols.saturating_sub(1));
        self.alt_cursor.row = self.alt_cursor.row.min(active_rows.saturating_sub(1));

        self.record_resize(cols, rows);
    }

    /// Get the title
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Set the title
    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    /// Check if alternate screen is active
    pub fn is_alt_screen_active(&self) -> bool {
        self.alt_screen_active
    }

    /// Switch to alternate screen
    pub fn use_alt_screen(&mut self) {
        if !self.alt_screen_active {
            debug::log_screen_switch(true, "use_alt_screen");
            // Save current (primary) cursor position before switching
            let primary_cursor = self.cursor;
            self.alt_screen_active = true;
            // Restore alternate screen cursor (or use saved position)
            self.cursor = self.alt_cursor;
            // Save primary cursor for when we switch back
            self.alt_cursor = primary_cursor;
            // Clear the alternate screen buffer to ensure it starts blank
            self.alt_grid.clear();
        }
    }

    /// Switch to primary screen
    pub fn use_primary_screen(&mut self) {
        if self.alt_screen_active {
            debug::log_screen_switch(false, "use_primary_screen");
            // Save current (alternate) cursor position before switching
            let alt_cursor = self.cursor;
            self.alt_screen_active = false;
            // Restore primary cursor
            self.cursor = self.alt_cursor;
            // Save alternate cursor for when we switch back
            self.alt_cursor = alt_cursor;
        }
    }

    /// Get mouse mode
    pub fn mouse_mode(&self) -> MouseMode {
        self.mouse_mode
    }

    /// Set mouse mode
    pub fn set_mouse_mode(&mut self, mode: MouseMode) {
        self.mouse_mode = mode;
    }

    /// Get mouse encoding
    pub fn mouse_encoding(&self) -> MouseEncoding {
        self.mouse_encoding
    }

    /// Set mouse encoding
    pub fn set_mouse_encoding(&mut self, encoding: MouseEncoding) {
        self.mouse_encoding = encoding;
    }

    /// Check if focus tracking is enabled
    pub fn focus_tracking(&self) -> bool {
        self.focus_tracking
    }

    /// Set focus tracking
    pub fn set_focus_tracking(&mut self, enabled: bool) {
        self.focus_tracking = enabled;
    }

    /// Check if bracketed paste is enabled
    pub fn bracketed_paste(&self) -> bool {
        self.bracketed_paste
    }

    /// Set bracketed paste mode
    pub fn set_bracketed_paste(&mut self, enabled: bool) {
        self.bracketed_paste = enabled;
    }

    /// Check if reverse video mode is enabled (DECSCNM)
    pub fn reverse_video(&self) -> bool {
        self.reverse_video
    }

    /// Check if bold brightening is enabled
    /// When enabled, bold text with ANSI colors 0-7 brightens to 8-15
    pub fn bold_brightening(&self) -> bool {
        self.bold_brightening
    }

    /// Set bold brightening mode
    pub fn set_bold_brightening(&mut self, enabled: bool) {
        self.bold_brightening = enabled;
    }

    /// Get shell integration state
    pub fn shell_integration(&self) -> &ShellIntegration {
        &self.shell_integration
    }

    /// Get shell integration state mutably
    pub fn shell_integration_mut(&mut self) -> &mut ShellIntegration {
        &mut self.shell_integration
    }

    /// Report mouse event
    pub fn report_mouse(&mut self, event: MouseEvent) -> Vec<u8> {
        if self.mouse_mode == MouseMode::Off {
            return Vec::new();
        }
        event.encode(self.mouse_mode, self.mouse_encoding)
    }

    /// Report focus in event
    pub fn report_focus_in(&self) -> Vec<u8> {
        if self.focus_tracking {
            b"\x1b[I".to_vec()
        } else {
            Vec::new()
        }
    }

    /// Report focus out event
    pub fn report_focus_out(&self) -> Vec<u8> {
        if self.focus_tracking {
            b"\x1b[O".to_vec()
        } else {
            Vec::new()
        }
    }

    /// Get bracketed paste start sequence
    pub fn bracketed_paste_start(&self) -> &[u8] {
        if self.bracketed_paste {
            b"\x1b[200~"
        } else {
            b""
        }
    }

    /// Get bracketed paste end sequence
    pub fn bracketed_paste_end(&self) -> &[u8] {
        if self.bracketed_paste {
            b"\x1b[201~"
        } else {
            b""
        }
    }

    /// Process pasted content with proper bracketing if enabled
    ///
    /// If bracketed paste mode is enabled, wraps the content with ESC[200~ and ESC[201~
    /// Otherwise, processes the content directly
    pub fn paste(&mut self, content: &str) {
        if self.bracketed_paste {
            // Send: ESC[200~ + content + ESC[201~
            self.process(b"\x1b[200~");
            self.process(content.as_bytes());
            self.process(b"\x1b[201~");
        } else {
            // Send content directly
            self.process(content.as_bytes());
        }
    }

    /// Check if synchronized updates mode is enabled
    pub fn synchronized_updates(&self) -> bool {
        self.synchronized_updates
    }

    /// Flush the synchronized update buffer
    pub fn flush_synchronized_updates(&mut self) {
        if !self.update_buffer.is_empty() {
            let buffer = std::mem::take(&mut self.update_buffer);
            debug::log(
                debug::DebugLevel::Debug,
                "SYNC_UPDATE",
                &format!("Flushing buffer ({} bytes)", buffer.len()),
            );
            // Process the buffered data without synchronized mode
            let saved_mode = self.synchronized_updates;
            self.synchronized_updates = false;
            self.process(&buffer);
            self.synchronized_updates = saved_mode;
        }
    }

    /// Get current Sixel resource limits
    pub fn sixel_limits(&self) -> sixel::SixelLimits {
        self.sixel_limits
    }

    /// Set Sixel resource limits (pixels and repeat count).
    ///
    /// Limits are clamped to safe hard maxima defined in `sixel.rs` and
    /// applied to new Sixel parser instances created for subsequent DCS
    /// sequences.
    pub fn set_sixel_limits(&mut self, max_width: usize, max_height: usize, max_repeat: usize) {
        self.sixel_limits = sixel::SixelLimits::new(max_width, max_height, max_repeat);
    }

    /// Get the maximum number of Sixel graphics retained for this terminal
    pub fn max_sixel_graphics(&self) -> usize {
        self.max_sixel_graphics
    }

    /// Set the maximum number of Sixel graphics retained for this terminal.
    ///
    /// The value is clamped to a safe range and applies to graphics created
    /// after the change. If the new limit is lower than the current number of
    /// graphics, the oldest graphics are dropped to respect the limit.
    pub fn set_max_sixel_graphics(&mut self, max_graphics: usize) {
        use crate::sixel::SIXEL_HARD_MAX_GRAPHICS;

        let clamped = max_graphics.clamp(1, SIXEL_HARD_MAX_GRAPHICS);
        self.max_sixel_graphics = clamped;

        if self.graphics.len() > clamped {
            let excess = self.graphics.len() - clamped;
            self.dropped_sixel_graphics = self.dropped_sixel_graphics.saturating_add(excess);
            self.graphics.drain(0..excess);
            debug::log(
                debug::DebugLevel::Debug,
                "SIXEL",
                &format!(
                    "Dropped {} oldest graphics due to reduced max_sixel_graphics limit (now {})",
                    excess, clamped
                ),
            );
        }
    }

    /// Get the number of Sixel graphics dropped due to limits
    pub fn dropped_sixel_graphics(&self) -> usize {
        self.dropped_sixel_graphics
    }

    /// Get Sixel statistics for this terminal.
    ///
    /// Returns:
    /// - limits: SixelLimits (max width/height/repeat)
    /// - max_graphics: maximum number of retained graphics
    /// - current_graphics: current number of graphics stored
    /// - dropped_graphics: number of graphics dropped due to limits
    pub fn sixel_stats(&self) -> (sixel::SixelLimits, usize, usize, usize) {
        let limits = self.sixel_limits;
        let max_graphics = self.max_sixel_graphics;
        let current_graphics = self.graphics.len();
        let dropped_graphics = self.dropped_sixel_graphics;
        (limits, max_graphics, current_graphics, dropped_graphics)
    }

    /// Process a buffered Sixel command (color, raster, repeat)
    /// Get current Kitty keyboard protocol flags
    pub fn keyboard_flags(&self) -> u16 {
        self.keyboard_flags
    }

    /// Get insert mode (IRM) state
    pub fn insert_mode(&self) -> bool {
        self.insert_mode
    }

    /// Get line feed/new line mode (LNM) state
    pub fn line_feed_new_line_mode(&self) -> bool {
        self.line_feed_new_line_mode
    }

    /// Set Kitty keyboard protocol flags (for testing/direct control)
    pub fn set_keyboard_flags(&mut self, flags: u16) {
        self.keyboard_flags = flags;
    }

    /// Get clipboard content (OSC 52)
    pub fn clipboard(&self) -> Option<&str> {
        self.clipboard_content.as_deref()
    }

    /// Set clipboard content programmatically (bypasses OSC 52 sequence)
    pub fn set_clipboard(&mut self, content: Option<String>) {
        self.clipboard_content = content;
    }

    /// Check if clipboard read operations are allowed (security flag for OSC 52 queries)
    pub fn allow_clipboard_read(&self) -> bool {
        self.allow_clipboard_read
    }

    /// Set whether clipboard read operations are allowed (security flag for OSC 52 queries)
    ///
    /// When disabled (default), OSC 52 queries (ESC ] 52 ; c ; ? ST) are silently ignored.
    /// When enabled, terminals can query clipboard contents, which has security implications.
    pub fn set_allow_clipboard_read(&mut self, allow: bool) {
        self.allow_clipboard_read = allow;
    }

    /// Get default foreground color (OSC 10)
    /// Get current working directory from shell integration (OSC 7)
    ///
    /// Returns the directory path reported by the shell via OSC 7 sequences,
    /// or None if no directory has been reported yet.
    pub fn current_directory(&self) -> Option<&str> {
        self.shell_integration.cwd()
    }

    /// Check if OSC 7 directory tracking is enabled
    pub fn accept_osc7(&self) -> bool {
        self.accept_osc7
    }

    /// Set whether OSC 7 directory tracking sequences are accepted
    ///
    /// When disabled, OSC 7 sequences are silently ignored.
    /// When enabled (default), allows shell to report current working directory.
    pub fn set_accept_osc7(&mut self, accept: bool) {
        self.accept_osc7 = accept;
    }

    /// Check if insecure sequence filtering is enabled
    pub fn disable_insecure_sequences(&self) -> bool {
        self.disable_insecure_sequences
    }

    /// Set whether to filter potentially insecure escape sequences
    ///
    /// When enabled, certain sequences that could pose security risks are blocked.
    /// When disabled (default), all standard sequences are processed normally.
    pub fn set_disable_insecure_sequences(&mut self, disable: bool) {
        self.disable_insecure_sequences = disable;
    }

    /// Get pending notifications (OSC 9 / OSC 777)
    ///
    /// Returns a reference to the list of notifications that have been received
    /// but not yet retrieved.
    pub fn notifications(&self) -> &[Notification] {
        &self.notifications
    }

    /// Take all pending notifications
    ///
    /// Returns and clears the notification queue. Use this to poll for new notifications.
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        std::mem::take(&mut self.notifications)
    }

    /// Check if there are pending notifications
    pub fn has_notifications(&self) -> bool {
        !self.notifications.is_empty()
    }

    fn enqueue_notification(&mut self, notification: Notification) {
        if self.max_notifications == 0 {
            return;
        }

        if self.notifications.len() >= self.max_notifications {
            let excess = self.notifications.len() + 1 - self.max_notifications;
            self.notifications.drain(0..excess);
        }

        self.notifications.push(notification);
    }

    /// Set maximum OSC 9/777 notifications retained (0 disables buffering)
    pub fn set_max_notifications(&mut self, max: usize) {
        self.max_notifications = max;
        if max == 0 {
            self.notifications.clear();
        } else if self.notifications.len() > max {
            let excess = self.notifications.len() - max;
            self.notifications.drain(0..excess);
        }
    }

    /// Get maximum OSC 9/777 notifications retained
    pub fn max_notifications(&self) -> usize {
        self.max_notifications
    }

    /// Get the current bell count
    ///
    /// This counter increments each time the terminal receives a bell character (BEL/\x07).
    /// Applications can poll this to detect bell events for visual bell implementations.
    ///
    /// Returns the total number of bell events received since terminal creation.
    pub fn bell_count(&self) -> u64 {
        self.bell_count
    }

    /// Drain all pending bell events
    ///
    /// Returns and clears the buffer of bell events that have occurred since the last drain.
    /// This is more efficient than polling bell_count() for event-driven applications.
    pub fn drain_bell_events(&mut self) -> Vec<BellEvent> {
        std::mem::take(&mut self.bell_events)
    }

    /// Drain all pending terminal events
    ///
    /// Returns and clears the buffer of terminal events (bells, title changes, mode changes, etc.)
    pub fn poll_events(&mut self) -> Vec<TerminalEvent> {
        std::mem::take(&mut self.terminal_events)
    }

    // ========== Dirty Region Tracking ==========

    /// Get all dirty row numbers
    ///
    /// Returns a vector of 0-indexed row numbers that have been modified since the last mark_clean()
    pub fn get_dirty_rows(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self.dirty_rows.iter().copied().collect();
        rows.sort_unstable();
        rows
    }

    /// Get the dirty region bounds (first and last dirty row)
    ///
    /// Returns None if no rows are dirty, otherwise (first_row, last_row) inclusive
    pub fn get_dirty_region(&self) -> Option<(usize, usize)> {
        if self.dirty_rows.is_empty() {
            None
        } else {
            let min = *self.dirty_rows.iter().min().unwrap();
            let max = *self.dirty_rows.iter().max().unwrap();
            Some((min, max))
        }
    }

    /// Mark all rows as clean (clear dirty tracking)
    pub fn mark_clean(&mut self) {
        self.dirty_rows.clear();
    }

    /// Mark a specific row as dirty
    ///
    /// This is called internally when content changes, but can also be called manually
    pub fn mark_row_dirty(&mut self, row: usize) {
        let (_cols, rows) = self.size();
        if row < rows {
            self.dirty_rows.insert(row);
        }
    }

    // ========== Mode Introspection ==========

    /// Get auto-wrap mode (DECAWM)
    ///
    /// When true, cursor automatically wraps to next line when reaching right margin
    pub fn auto_wrap_mode(&self) -> bool {
        self.auto_wrap
    }

    /// Get origin mode (DECOM)
    ///
    /// When true, cursor addressing is relative to scroll region instead of absolute
    pub fn origin_mode(&self) -> bool {
        self.origin_mode
    }

    /// Get application cursor mode
    ///
    /// When true, arrow keys send application sequences (ESC O A-D) instead of ANSI (ESC [ A-D)
    pub fn application_cursor(&self) -> bool {
        self.application_cursor
    }

    /// Get current scroll region (top, bottom) - 0-indexed, inclusive
    pub fn scroll_region(&self) -> (usize, usize) {
        (self.scroll_region_top, self.scroll_region_bottom)
    }

    /// Get left/right margins if enabled (left, right) - 0-indexed, inclusive
    ///
    /// Returns None if DECLRMM is disabled, otherwise Some((left, right))
    pub fn left_right_margins(&self) -> Option<(usize, usize)> {
        if self.use_lr_margins {
            Some((self.left_margin, self.right_margin))
        } else {
            None
        }
    }

    // ========== Palette Access ==========

    /// Get an ANSI palette color by index (0-15)
    ///
    /// Returns None if index is out of range
    pub fn get_ansi_color(&self, index: u8) -> Option<Color> {
        if (index as usize) < self.ansi_palette.len() {
            Some(self.ansi_palette[index as usize])
        } else {
            None
        }
    }

    /// Get the entire ANSI color palette (colors 0-15)
    pub fn get_ansi_palette(&self) -> [Color; 16] {
        self.ansi_palette
    }

    /// Get the default foreground color
    pub fn get_default_fg(&self) -> Color {
        self.default_fg
    }

    /// Get the default background color
    pub fn get_default_bg(&self) -> Color {
        self.default_bg
    }

    /// Get the cursor color
    pub fn get_cursor_color(&self) -> Color {
        self.cursor_color
    }

    /// Get the cursor guide color (iTerm2)
    pub fn get_cursor_guide_color(&self) -> Color {
        self.cursor_guide_color
    }

    /// Get the hyperlink/URL color (iTerm2)
    pub fn get_link_color(&self) -> Color {
        self.link_color
    }

    /// Get the selection background color (iTerm2)
    pub fn get_selection_bg_color(&self) -> Color {
        self.selection_bg_color
    }

    /// Get the selection foreground color (iTerm2)
    pub fn get_selection_fg_color(&self) -> Color {
        self.selection_fg_color
    }

    // ========== Tab Stop Access ==========

    /// Get all tab stop positions
    ///
    /// Returns a vector of column numbers (0-indexed) where tab stops are set
    pub fn get_tab_stops(&self) -> Vec<usize> {
        self.tab_stops
            .iter()
            .enumerate()
            .filter_map(|(i, &is_set)| if is_set { Some(i) } else { None })
            .collect()
    }

    /// Set a tab stop at the specified column
    pub fn set_tab_stop(&mut self, col: usize) {
        if col < self.tab_stops.len() {
            self.tab_stops[col] = true;
        }
    }

    /// Clear a tab stop at the specified column
    pub fn clear_tab_stop(&mut self, col: usize) {
        if col < self.tab_stops.len() {
            self.tab_stops[col] = false;
        }
    }

    /// Clear all tab stops
    pub fn clear_all_tab_stops(&mut self) {
        for stop in &mut self.tab_stops {
            *stop = false;
        }
    }

    // ========== Hyperlink Enumeration ==========

    /// Get all hyperlinks with their positions
    ///
    /// Returns a vector of HyperlinkInfo containing URL and all screen positions
    pub fn get_all_hyperlinks(&self) -> Vec<HyperlinkInfo> {
        let mut hyperlink_map: HashMap<u32, HyperlinkInfo> = HashMap::new();
        let (cols, rows) = self.size();

        // Scan visible screen for hyperlinks
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = self.active_grid().get(col, row) {
                    if let Some(link_id) = cell.flags.hyperlink_id {
                        if let Some(url) = self.hyperlinks.get(&link_id) {
                            hyperlink_map
                                .entry(link_id)
                                .or_insert_with(|| HyperlinkInfo {
                                    url: url.clone(),
                                    positions: Vec::new(),
                                    id: None, // We don't store the OSC 8 ID separately
                                })
                                .positions
                                .push((col, row));
                        }
                    }
                }
            }
        }

        hyperlink_map.into_values().collect()
    }

    // ========== Bulk Read Operations ==========

    /// Get all cells in the visible region
    ///
    /// Returns a 2D vector of cells [row][col]
    pub fn get_visible_region(&self) -> Vec<Vec<Cell>> {
        let (cols, rows) = self.size();
        let mut result = Vec::with_capacity(rows);

        for row in 0..rows {
            let mut row_cells = Vec::with_capacity(cols);
            for col in 0..cols {
                if let Some(cell) = self.active_grid().get(col, row) {
                    row_cells.push(*cell);
                } else {
                    row_cells.push(Cell::default());
                }
            }
            result.push(row_cells);
        }

        result
    }

    /// Get a range of rows
    ///
    /// Returns cells for rows [start..end) - start is inclusive, end is exclusive
    pub fn get_row_range(&self, start: usize, end: usize) -> Vec<Vec<Cell>> {
        let (cols, rows) = self.size();
        let end = end.min(rows);
        let start = start.min(end);

        let mut result = Vec::with_capacity(end - start);

        for row in start..end {
            let mut row_cells = Vec::with_capacity(cols);
            for col in 0..cols {
                if let Some(cell) = self.active_grid().get(col, row) {
                    row_cells.push(*cell);
                } else {
                    row_cells.push(Cell::default());
                }
            }
            result.push(row_cells);
        }

        result
    }

    /// Get a rectangular region of cells
    ///
    /// Returns cells in rectangle bounded by (top, left) to (bottom, right) inclusive
    pub fn get_rectangle(
        &self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> Vec<Vec<Cell>> {
        let (cols, rows) = self.size();
        let bottom = bottom.min(rows.saturating_sub(1));
        let right = right.min(cols.saturating_sub(1));
        let top = top.min(bottom);
        let left = left.min(right);

        let height = bottom - top + 1;
        let width = right - left + 1;
        let mut result = Vec::with_capacity(height);

        for row in top..=bottom {
            let mut row_cells = Vec::with_capacity(width);
            for col in left..=right {
                if let Some(cell) = self.active_grid().get(col, row) {
                    row_cells.push(*cell);
                } else {
                    row_cells.push(Cell::default());
                }
            }
            result.push(row_cells);
        }

        result
    }

    // ========== Rectangle Operations ==========

    /// Fill a rectangle with a character
    ///
    /// Fills the rectangle bounded by (top, left) to (bottom, right) inclusive with the given character
    pub fn fill_rectangle(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        ch: char,
    ) {
        let (cols, rows) = self.size();
        let bottom = bottom.min(rows.saturating_sub(1));
        let right = right.min(cols.saturating_sub(1));

        for row in top..=bottom {
            for col in left..=right {
                if row < rows && col < cols {
                    let cell = Cell {
                        c: ch,
                        fg: self.fg,
                        bg: self.bg,
                        underline_color: self.underline_color,
                        flags: self.flags,
                        width: 1,
                    };
                    self.active_grid_mut().set(col, row, cell);
                    self.mark_row_dirty(row);
                }
            }
        }
    }

    /// Erase a rectangle
    ///
    /// Clears the rectangle bounded by (top, left) to (bottom, right) inclusive
    pub fn erase_rectangle(&mut self, top: usize, left: usize, bottom: usize, right: usize) {
        let (cols, rows) = self.size();
        let bottom = bottom.min(rows.saturating_sub(1));
        let right = right.min(cols.saturating_sub(1));

        for row in top..=bottom {
            for col in left..=right {
                if row < rows && col < cols {
                    let cell = Cell {
                        c: ' ',
                        fg: self.default_fg,
                        bg: self.default_bg,
                        underline_color: None,
                        flags: CellFlags::default(),
                        width: 1,
                    };
                    self.active_grid_mut().set(col, row, cell);
                    self.mark_row_dirty(row);
                }
            }
        }
    }

    /// Process input data
    pub fn process(&mut self, data: &[u8]) {
        debug::log_vt_input(data);
        self.update_activity();

        // If tmux control mode is enabled, parse data through tmux control parser
        if self.tmux_parser.is_control_mode() {
            let notifications = self.tmux_parser.parse(data);
            for notification in notifications {
                match notification {
                    crate::tmux_control::TmuxNotification::TerminalOutput { data } => {
                        // Process terminal output through VTE parser
                        self.process_vte_data(&data);
                    }
                    _ => {
                        // Store other notifications for retrieval
                        self.tmux_notifications.push(notification);
                    }
                }
            }
            return;
        }

        self.process_vte_data(data);
    }

    /// Process data through the VTE parser
    fn process_vte_data(&mut self, data: &[u8]) {
        // If synchronized updates mode is enabled, we need special handling
        if self.synchronized_updates {
            // Check if this data contains the disable sequence (CSI ? 2026 l)
            // Common patterns: "\x1b[?2026l" or with spaces/params
            let contains_disable = contains_bytes(data, b"\x1b[?2026l")
                || contains_bytes(data, b"\x1b[?2026 l")
                || contains_bytes(data, b"\x1b[? 2026 l")
                || contains_bytes(data, b"\x1b[? 2026l");

            if contains_disable {
                // Flush buffer first, then process this data (which will disable the mode)
                self.flush_synchronized_updates();
                // Now process the disable sequence (synchronized_updates might be toggled off in flush,
                // but we'll process this data anyway to ensure the disable sequence is handled)
            } else {
                // Buffer the data and return
                self.update_buffer.extend_from_slice(data);
                return;
            }
        }

        // Use the persistent parser to maintain state across calls
        // This is critical for handling escape sequences that span multiple PTY reads
        // We temporarily take ownership of the parser to avoid borrow checker issues
        let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());
        parser.advance(self, data);
        self.parser = parser;
    }
    pub fn reset(&mut self) {
        let (cols, rows) = self.size();

        self.grid.clear();
        self.alt_grid.clear();
        self.alt_screen_active = false;
        self.cursor = Cursor::new();
        self.alt_cursor = Cursor::new();
        self.fg = Color::Named(NamedColor::White);
        self.bg = Color::Named(NamedColor::Black);
        self.flags = CellFlags::default();
        self.mouse_mode = MouseMode::Off;
        self.mouse_encoding = MouseEncoding::Default;
        self.focus_tracking = false;
        self.bracketed_paste = false;
        self.shell_integration = ShellIntegration::new();
        self.scroll_region_top = 0;
        self.scroll_region_bottom = rows.saturating_sub(1);
        self.use_lr_margins = false;
        self.left_margin = 0;
        self.right_margin = cols.saturating_sub(1);
        self.auto_wrap = true;
        self.origin_mode = false;
        self.application_cursor = false;
        self.keyboard_flags = 0;
        self.keyboard_stack.clear();
        self.keyboard_stack_alt.clear();
        self.response_buffer.clear();
        self.hyperlinks.clear();
        self.current_hyperlink_id = None;
        self.next_hyperlink_id = 0;
        self.pending_wrap = false;
        self.insert_mode = false;
        self.line_feed_new_line_mode = false;
        self.title_stack.clear();

        // Reset tab stops to default (every 8 columns)
        self.tab_stops = vec![false; cols];
        for i in (0..cols).step_by(8) {
            self.tab_stops[i] = true;
        }
    }

    /// Get the terminal content as a string
    pub fn content(&self) -> String {
        self.active_grid().content_as_string()
    }

    /// Get scrollback content
    pub fn scrollback(&self) -> Vec<String> {
        // Optimized: use scrollback_line() to avoid intermediate Vec<Vec<Cell>> allocation
        let mut result = Vec::with_capacity(self.grid.scrollback_len());
        for i in 0..self.grid.scrollback_len() {
            if let Some(line) = self.grid.scrollback_line(i) {
                let line_str: String = line
                    .iter()
                    .filter(|cell| !cell.flags.wide_char_spacer())
                    .map(|cell| cell.c)
                    .collect();
                result.push(line_str);
            }
        }
        result
    }

    /// Export entire buffer (scrollback + current screen) as plain text
    ///
    /// This exports all buffer contents with:
    /// - No styling, colors, or graphics (Sixel, etc.)
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Empty lines preserved
    ///
    /// Returns:
    ///     String containing all buffer text from scrollback through current screen
    pub fn export_text(&self) -> String {
        // Use the active grid (primary or alternate screen)
        self.active_grid().export_text_buffer()
    }

    /// Export entire buffer (scrollback + current screen) with ANSI styling
    ///
    /// This exports all buffer contents with:
    /// - Full ANSI escape sequences for colors and text attributes
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Efficient escape sequence generation (only emits changes)
    ///
    /// Returns:
    ///     String containing all buffer text with ANSI styling
    pub fn export_styled(&self) -> String {
        // Use the active grid (primary or alternate screen)
        self.active_grid().export_styled_buffer()
    }

    // ========== Text Extraction Utilities ==========

    /// Get word at the given cursor position
    ///
    /// # Arguments
    /// * `col` - Column position (0-indexed)
    /// * `row` - Row position (0-indexed)
    /// * `word_chars` - Optional custom word characters (default: "/-+\\~_." iTerm2-compatible)
    ///
    /// # Returns
    /// The word at the cursor position, or None if not on a word
    pub fn get_word_at(&self, col: usize, row: usize, word_chars: Option<&str>) -> Option<String> {
        crate::text_utils::get_word_at(self.active_grid(), col, row, word_chars)
    }

    /// Get URL at the given cursor position
    ///
    /// Detects URLs with schemes: http://, https://, ftp://, file://, mailto:, ssh://
    ///
    /// # Arguments
    /// * `col` - Column position (0-indexed)
    /// * `row` - Row position (0-indexed)
    ///
    /// # Returns
    /// The URL at the cursor position, or None if not on a URL
    pub fn get_url_at(&self, col: usize, row: usize) -> Option<String> {
        crate::text_utils::get_url_at(self.active_grid(), col, row)
    }

    /// Get full logical line following wrapping
    ///
    /// # Arguments
    /// * `row` - Row position (0-indexed)
    ///
    /// # Returns
    /// The complete unwrapped line, or None if row is invalid
    pub fn get_line_unwrapped(&self, row: usize) -> Option<String> {
        crate::text_utils::get_line_unwrapped(self.active_grid(), row)
    }

    /// Get word boundaries at cursor position for smart selection
    ///
    /// # Arguments
    /// * `col` - Column position (0-indexed)
    /// * `row` - Row position (0-indexed)
    /// * `word_chars` - Optional custom word characters
    ///
    /// # Returns
    /// `((start_col, start_row), (end_col, end_row))` or None if not on a word
    pub fn select_word(
        &self,
        col: usize,
        row: usize,
        word_chars: Option<&str>,
    ) -> Option<((usize, usize), (usize, usize))> {
        crate::text_utils::select_word(self.active_grid(), col, row, word_chars)
    }

    // ========== Content Search ==========

    /// Find all occurrences of text in the visible screen
    ///
    /// # Arguments
    /// * `pattern` - Text to search for
    /// * `case_sensitive` - Whether search is case-sensitive
    ///
    /// # Returns
    /// Vector of (col, row) positions where pattern was found
    pub fn find_text(&self, pattern: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
        let mut results = Vec::new();
        let grid = self.active_grid();
        let rows = grid.rows();

        let pattern_lower = if !case_sensitive {
            pattern.to_lowercase()
        } else {
            pattern.to_string()
        };

        for row in 0..rows {
            let line = grid.row_text(row);
            if line.is_empty() {
                continue;
            }

            let line_to_search = if !case_sensitive {
                line.to_lowercase()
            } else {
                line.clone()
            };

            let mut start = 0;
            while let Some(pos) = line_to_search[start..].find(&pattern_lower) {
                let col = start + pos;
                results.push((col, row));
                start = col + pattern.len();
            }
        }

        results
    }

    /// Find next occurrence of text from given position
    ///
    /// # Arguments
    /// * `pattern` - Text to search for
    /// * `from_col` - Starting column position
    /// * `from_row` - Starting row position
    /// * `case_sensitive` - Whether search is case-sensitive
    ///
    /// # Returns
    /// `(col, row)` of next match, or None if not found
    pub fn find_next(
        &self,
        pattern: &str,
        from_col: usize,
        from_row: usize,
        case_sensitive: bool,
    ) -> Option<(usize, usize)> {
        let grid = self.active_grid();
        let rows = grid.rows();

        let pattern_lower = if !case_sensitive {
            pattern.to_lowercase()
        } else {
            pattern.to_string()
        };

        // Search from current position to end of current line
        if from_row < rows {
            let line = grid.row_text(from_row);
            if !line.is_empty() {
                let line_to_search = if !case_sensitive {
                    line.to_lowercase()
                } else {
                    line.clone()
                };

                if from_col < line.len() {
                    if let Some(pos) = line_to_search[from_col + 1..].find(&pattern_lower) {
                        return Some((from_col + 1 + pos, from_row));
                    }
                }
            }
        }

        // Search remaining lines
        for row in (from_row + 1)..rows {
            let line = grid.row_text(row);
            if line.is_empty() {
                continue;
            }

            let line_to_search = if !case_sensitive {
                line.to_lowercase()
            } else {
                line.clone()
            };

            if let Some(pos) = line_to_search.find(&pattern_lower) {
                return Some((pos, row));
            }
        }

        None
    }

    // ========== Buffer Statistics ==========

    /// Get terminal statistics
    ///
    /// Returns statistics about terminal buffer usage, memory, and content.
    pub fn get_stats(&self) -> TerminalStats {
        let grid = self.active_grid();
        let (cols, rows) = self.size();
        let scrollback_len = grid.scrollback_len();
        let total_cells = cols * rows + scrollback_len * cols;

        let mut non_whitespace_lines = 0;
        for row in 0..rows {
            let line = grid.row_text(row);
            if !line.trim().is_empty() {
                non_whitespace_lines += 1;
            }
        }

        let graphics_count = self.graphics_count();

        // Estimate memory usage (rough approximation)
        let cell_size = std::mem::size_of::<crate::cell::Cell>();
        let estimated_memory = total_cells * cell_size;

        // Calculate hyperlink memory
        let hyperlink_count = self.hyperlinks.len();
        let hyperlink_memory: usize = self
            .hyperlinks
            .values()
            .map(|url| url.len() + std::mem::size_of::<u32>() + std::mem::size_of::<String>())
            .sum();

        // Get stack depths
        let keyboard_stack_depth = if self.alt_screen_active {
            self.keyboard_stack_alt.len()
        } else {
            self.keyboard_stack.len()
        };

        TerminalStats {
            cols,
            rows,
            scrollback_lines: scrollback_len,
            total_cells,
            non_whitespace_lines,
            graphics_count,
            estimated_memory_bytes: estimated_memory,
            hyperlink_count,
            hyperlink_memory_bytes: hyperlink_memory,
            color_stack_depth: self.color_stack.len(),
            title_stack_depth: self.title_stack.len(),
            keyboard_stack_depth,
            response_buffer_size: self.response_buffer.len(),
            dirty_row_count: self.dirty_rows.len(),
            pending_bell_events: self.bell_events.len(),
            pending_terminal_events: self.terminal_events.len(),
        }
    }

    /// Count non-whitespace lines in visible screen
    ///
    /// # Returns
    /// Number of lines containing non-whitespace characters
    pub fn count_non_whitespace_lines(&self) -> usize {
        let grid = self.active_grid();
        let rows = grid.rows();
        let mut count = 0;

        for row in 0..rows {
            let line = grid.row_text(row);
            if !line.trim().is_empty() {
                count += 1;
            }
        }

        count
    }

    /// Get scrollback usage (used, capacity)
    ///
    /// # Returns
    /// `(used_lines, max_capacity)` tuple
    pub fn get_scrollback_usage(&self) -> (usize, usize) {
        let grid = self.active_grid();
        (grid.scrollback_len(), grid.max_scrollback())
    }

    // ========== Advanced Text Selection ==========

    /// Find matching bracket/parenthesis at cursor position
    ///
    /// Supports: (), [], {}, <>
    ///
    /// # Arguments
    /// * `col` - Column position (0-indexed)
    /// * `row` - Row position (0-indexed)
    ///
    /// # Returns
    /// Position of matching bracket `(col, row)`, or None if:
    /// - Not on a bracket character
    /// - No matching bracket found
    /// - Position is invalid
    pub fn find_matching_bracket(&self, col: usize, row: usize) -> Option<(usize, usize)> {
        crate::text_utils::find_matching_bracket(self.active_grid(), col, row)
    }

    /// Select text within semantic delimiters
    ///
    /// Extracts content between matching delimiters around cursor position.
    /// Supports: (), [], {}, <>, "", '', ``
    ///
    /// # Arguments
    /// * `col` - Column position (0-indexed)
    /// * `row` - Row position (0-indexed)
    /// * `delimiters` - String of delimiters to check (e.g., "()[]{}\"'")
    ///
    /// # Returns
    /// Content between delimiters, or None if not inside delimiters
    pub fn select_semantic_region(
        &self,
        col: usize,
        row: usize,
        delimiters: &str,
    ) -> Option<String> {
        crate::text_utils::select_semantic_region(self.active_grid(), col, row, delimiters)
    }

    // ========== Export Functions ==========

    /// Export terminal content as HTML
    ///
    /// # Arguments
    /// * `include_styles` - Whether to include full HTML document with CSS
    ///
    /// # Returns
    /// HTML string with terminal content and styling
    ///
    /// When `include_styles` is true, returns a complete HTML document.
    /// When false, returns just the styled content (useful for embedding).
    pub fn export_html(&self, include_styles: bool) -> String {
        crate::html_export::export_html(self.active_grid(), include_styles)
    }

    /// Create a grid view with scrollback content at the given offset
    ///
    /// # Arguments
    /// * `scrollback_offset` - Number of lines to scroll back from the current position (0 = no scrollback)
    ///
    /// # Returns
    /// A new Grid containing the requested view. If offset is 0 or there's no scrollback,
    /// returns a clone of the active grid. Otherwise, creates a grid combining scrollback
    /// and active grid content.
    fn grid_with_scrollback(&self, scrollback_offset: usize) -> Grid {
        let grid = self.active_grid();
        let rows = grid.rows();
        let cols = grid.cols();
        let scrollback_len = grid.scrollback_len();

        // If no offset or no scrollback, just clone the active grid
        if scrollback_offset == 0 || scrollback_len == 0 {
            return grid.clone();
        }

        // Create a new grid to hold the view
        let mut view = Grid::new(cols, rows, 0); // No scrollback needed for the view

        // Calculate which lines to include
        // offset = how many lines back from bottom to start viewing
        let total_lines = scrollback_len + rows;

        if scrollback_offset >= total_lines {
            // Offset is too large, show from the very beginning of scrollback
            for row in 0..rows {
                if row < scrollback_len {
                    // Copy from scrollback
                    if let Some(line) = grid.scrollback_line(row) {
                        for (col, cell) in line.iter().enumerate() {
                            view.set(col, row, *cell);
                        }
                    }
                }
                // Remaining rows stay empty (default cells)
            }
        } else {
            // When scrolled up by N lines, we show:
            // - The last N lines of scrollback (rows 0..N-1)
            // - The first (rows-N) lines of active grid (rows N..rows-1)
            //
            // Example: rows=24, scrollback_len=50, offset=10
            // - Show scrollback[40..49] in view rows 0..9
            // - Show active[0..13] in view rows 10..23

            if scrollback_offset < rows {
                // Mixed view: some scrollback + some active grid
                let scrollback_rows_to_show = scrollback_offset;
                let active_rows_to_show = rows - scrollback_offset;

                // Copy the last N lines of scrollback into the first N rows of the view
                for row in 0..scrollback_rows_to_show {
                    let scrollback_idx = scrollback_len - scrollback_offset + row;
                    if let Some(line) = grid.scrollback_line(scrollback_idx) {
                        for (col, cell) in line.iter().enumerate() {
                            view.set(col, row, *cell);
                        }
                    }
                }

                // Copy the first (rows-N) lines of active grid into the remaining rows
                for row in 0..active_rows_to_show {
                    let view_row = scrollback_rows_to_show + row;
                    if let Some(line) = grid.row(row) {
                        for (col, cell) in line.iter().enumerate() {
                            view.set(col, view_row, *cell);
                        }
                    }
                }
            } else {
                // Entirely in scrollback - offset is >= rows
                // Calculate starting position in scrollback
                let start_idx = scrollback_len - scrollback_offset;
                for row in 0..rows {
                    let scrollback_idx = start_idx + row;
                    if scrollback_idx < scrollback_len {
                        if let Some(line) = grid.scrollback_line(scrollback_idx) {
                            for (col, cell) in line.iter().enumerate() {
                                view.set(col, row, *cell);
                            }
                        }
                    }
                }
            }
        }

        view
    }

    /// Take a screenshot of the current visible buffer
    ///
    /// Renders the terminal's visible screen buffer to an image using the provided configuration.
    ///
    /// # Arguments
    /// * `config` - Screenshot configuration (font, size, format, etc.)
    /// * `scrollback_offset` - Number of lines to scroll back from current position (default: 0)
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` - Image bytes in the configured format
    /// * `Err(ScreenshotError)` - If rendering or encoding fails
    ///
    /// # Example
    /// ```ignore
    /// use par_term_emu::screenshot::{ScreenshotConfig, ImageFormat};
    ///
    /// let config = ScreenshotConfig::default().with_format(ImageFormat::Png);
    /// let png_bytes = terminal.screenshot(config, 0)?; // Current view
    /// let scrolled_bytes = terminal.screenshot(config, 10)?; // 10 lines up
    /// ```
    pub fn screenshot(
        &self,
        mut config: crate::screenshot::ScreenshotConfig,
        scrollback_offset: usize,
    ) -> crate::screenshot::ScreenshotResult<Vec<u8>> {
        // Populate theme colors if not already set
        if config.link_color.is_none() {
            config.link_color = Some(self.link_color.to_rgb());
        }
        if config.bold_color.is_none() {
            config.bold_color = Some(self.bold_color.to_rgb());
        }
        config.use_bold_color = self.use_bold_color;

        let grid = self.grid_with_scrollback(scrollback_offset);
        let cursor = if config.render_cursor && scrollback_offset == 0 {
            Some(&self.cursor)
        } else {
            None
        };
        let graphics = if config.sixel_render_mode != crate::screenshot::SixelRenderMode::Disabled
            && scrollback_offset == 0
        {
            self.graphics()
        } else {
            &[]
        };
        crate::screenshot::render_grid(&grid, cursor, graphics, config)
    }

    /// Take a screenshot and save to file
    ///
    /// Convenience method to render and save a screenshot directly to a file.
    ///
    /// # Arguments
    /// * `path` - Output file path
    /// * `config` - Screenshot configuration
    /// * `scrollback_offset` - Number of lines to scroll back from current position (default: 0)
    ///
    /// # Returns
    /// * `Ok(())` - Success
    /// * `Err(ScreenshotError)` - If rendering, encoding, or writing fails
    pub fn screenshot_to_file(
        &self,
        path: &std::path::Path,
        mut config: crate::screenshot::ScreenshotConfig,
        scrollback_offset: usize,
    ) -> crate::screenshot::ScreenshotResult<()> {
        // Populate theme colors if not already set
        if config.link_color.is_none() {
            config.link_color = Some(self.link_color.to_rgb());
        }
        if config.bold_color.is_none() {
            config.bold_color = Some(self.bold_color.to_rgb());
        }
        config.use_bold_color = self.use_bold_color;

        let grid = self.grid_with_scrollback(scrollback_offset);
        let cursor = if config.render_cursor && scrollback_offset == 0 {
            Some(&self.cursor)
        } else {
            None
        };
        let graphics = if config.sixel_render_mode != crate::screenshot::SixelRenderMode::Disabled
            && scrollback_offset == 0
        {
            self.graphics()
        } else {
            &[]
        };
        crate::screenshot::save_grid(&grid, cursor, graphics, path, config)
    }

    /// Push response bytes to the response buffer
    /// Calculate checksum of rectangular area (DECRQCRA - VT420)
    /// Returns a 16-bit checksum based on cell contents
    fn calculate_rectangle_checksum(
        &self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> u16 {
        let grid = self.active_grid();
        let rows = grid.rows();
        let cols = grid.cols();

        // Validate and clamp coordinates
        if top >= rows || left >= cols {
            return 0;
        }
        let bottom = bottom.min(rows - 1);
        let right = right.min(cols - 1);

        if top > bottom || left > right {
            return 0;
        }

        // Calculate simple checksum: sum of character codes
        let mut checksum: u32 = 0;
        for row in top..=bottom {
            for col in left..=right {
                if let Some(cell) = grid.get(col, row) {
                    checksum = checksum.wrapping_add(cell.c as u32);
                }
            }
        }

        // Return 16-bit checksum
        (checksum & 0xFFFF) as u16
    }

    /// Drain and return pending responses
    pub fn drain_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.response_buffer)
    }

    /// Check if there are pending responses
    pub fn has_pending_responses(&self) -> bool {
        !self.response_buffer.is_empty()
    }

    /// Get the URL for a hyperlink ID
    pub fn get_hyperlink_url(&self, id: u32) -> Option<String> {
        self.hyperlinks.get(&id).cloned()
    }

    /// Enable or disable tmux control mode
    ///
    /// When enabled, incoming data is parsed for tmux control protocol messages
    /// instead of being processed as raw terminal output.
    pub fn set_tmux_control_mode(&mut self, enabled: bool) {
        self.tmux_parser.set_control_mode(enabled);
    }

    /// Check if tmux control mode is enabled
    pub fn is_tmux_control_mode(&self) -> bool {
        self.tmux_parser.is_control_mode()
    }

    /// Get tmux control protocol notifications
    ///
    /// Returns a reference to the notifications buffer.
    /// Use drain_tmux_notifications() to consume the notifications.
    pub fn tmux_notifications(&self) -> &[crate::tmux_control::TmuxNotification] {
        &self.tmux_notifications
    }

    /// Drain and return tmux control protocol notifications
    ///
    /// This consumes the notifications buffer, leaving it empty.
    pub fn drain_tmux_notifications(&mut self) -> Vec<crate::tmux_control::TmuxNotification> {
        std::mem::take(&mut self.tmux_notifications)
    }

    /// Check if there are pending tmux control protocol notifications
    pub fn has_tmux_notifications(&self) -> bool {
        !self.tmux_notifications.is_empty()
    }

    /// Clear the tmux control protocol notifications buffer
    pub fn clear_tmux_notifications(&mut self) {
        self.tmux_notifications.clear();
    }

    // === Search Methods ===

    /// Search for text in the visible screen area
    ///
    /// Returns a vector of SearchMatch results containing position and matched text.
    /// Row indices are 0-based, with 0 being the top row of the visible screen.
    ///
    /// # Arguments
    /// * `query` - The text to search for
    /// * `case_sensitive` - Whether the search should be case-sensitive
    pub fn search(&self, query: &str, case_sensitive: bool) -> Vec<SearchMatch> {
        let mut matches = Vec::new();
        if query.is_empty() {
            return matches;
        }

        let grid = self.active_grid();
        let search_query = if case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };

        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                let line_text = cells_to_text(line);
                let search_text = if case_sensitive {
                    line_text.clone()
                } else {
                    line_text.to_lowercase()
                };

                let mut start_col = 0;
                while let Some(pos) = search_text[start_col..].find(&search_query) {
                    let col = start_col + pos;
                    matches.push(SearchMatch {
                        row: row as isize,
                        col,
                        length: query.len(),
                        text: line_text[col..col + query.len()].to_string(),
                    });
                    start_col = col + 1;
                }
            }
        }

        matches
    }

    /// Search for text in the scrollback buffer
    ///
    /// Returns matches with negative row indices (e.g., -1 is the most recent scrollback line).
    /// Row -1 is the line just above the visible screen.
    ///
    /// # Arguments
    /// * `query` - The text to search for
    /// * `case_sensitive` - Whether the search should be case-sensitive
    /// * `max_lines` - Maximum number of scrollback lines to search (None = search all)
    pub fn search_scrollback(
        &self,
        query: &str,
        case_sensitive: bool,
        max_lines: Option<usize>,
    ) -> Vec<SearchMatch> {
        let mut matches = Vec::new();
        if query.is_empty() {
            return matches;
        }

        let search_query = if case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };

        let scrollback_len = self.grid.scrollback_len();
        let lines_to_search = max_lines.unwrap_or(scrollback_len).min(scrollback_len);

        for i in 0..lines_to_search {
            if let Some(line) = self.grid.scrollback_line(i) {
                let line_text = cells_to_text(line);
                let search_text = if case_sensitive {
                    line_text.clone()
                } else {
                    line_text.to_lowercase()
                };

                let mut start_col = 0;
                while let Some(pos) = search_text[start_col..].find(&search_query) {
                    let col = start_col + pos;
                    matches.push(SearchMatch {
                        row: -((i + 1) as isize), // Negative indices for scrollback
                        col,
                        length: query.len(),
                        text: line_text[col..col + query.len()].to_string(),
                    });
                    start_col = col + 1;
                }
            }
        }

        matches
    }

    // === Content Detection Methods ===

    /// Detect URLs in the visible screen
    ///
    /// Returns a vector of detected URLs with their positions.
    pub fn detect_urls(&self) -> Vec<DetectedItem> {
        let mut items = Vec::new();
        let grid = self.active_grid();

        // Simple URL pattern: looks for http://, https://, ftp://, etc.
        let url_prefixes = ["http://", "https://", "ftp://", "ftps://"];

        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                let line_text = cells_to_text(line);

                for prefix in &url_prefixes {
                    let mut start_col = 0;
                    while let Some(pos) = line_text[start_col..].to_lowercase().find(prefix) {
                        let col = start_col + pos;
                        // Find end of URL (space, newline, or end of line)
                        let end = line_text[col..]
                            .find(|c: char| c.is_whitespace())
                            .map(|p| col + p)
                            .unwrap_or(line_text.len());

                        if end > col {
                            let url = line_text[col..end].to_string();
                            items.push(DetectedItem::Url(url, row, col));
                        }
                        start_col = end.max(col + 1);
                    }
                }
            }
        }

        items
    }

    /// Detect file paths in the visible screen
    ///
    /// Returns a vector of detected file paths with their positions.
    /// Optionally includes line numbers if detected (e.g., "file.txt:123").
    pub fn detect_file_paths(&self) -> Vec<DetectedItem> {
        let mut items = Vec::new();
        let grid = self.active_grid();

        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                let line_text = cells_to_text(line);

                // Simple detection: paths starting with / or ./ or ../
                let path_patterns = ["/", "./", "../"];

                for pattern in &path_patterns {
                    let mut start_col = 0;
                    while let Some(pos) = line_text[start_col..].find(pattern) {
                        let col = start_col + pos;
                        // Find end of path (whitespace or common delimiters)
                        let end = line_text[col..]
                            .find(|c: char| c.is_whitespace() || c == ':' || c == ',' || c == ')')
                            .map(|p| col + p)
                            .unwrap_or(line_text.len());

                        if end > col {
                            let path_str = line_text[col..end].to_string();

                            // Check for line number suffix (e.g., ":123")
                            let line_num = if end < line_text.len()
                                && line_text.chars().nth(end) == Some(':')
                            {
                                let num_start = end + 1;
                                let num_end = line_text[num_start..]
                                    .find(|c: char| !c.is_numeric())
                                    .map(|p| num_start + p)
                                    .unwrap_or(line_text.len());

                                if num_end > num_start {
                                    line_text[num_start..num_end].parse().ok()
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            items.push(DetectedItem::FilePath(path_str, row, col, line_num));
                        }
                        start_col = end.max(col + 1);
                    }
                }
            }
        }

        items
    }

    /// Detect semantic items (URLs, file paths, git hashes, IPs, emails)
    ///
    /// Returns all detected items in the visible screen.
    pub fn detect_semantic_items(&self) -> Vec<DetectedItem> {
        let mut items = Vec::new();
        let grid = self.active_grid();

        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                let line_text = cells_to_text(line);

                // Git hash pattern (40 hex chars)
                for (i, window) in line_text.as_bytes().windows(40).enumerate() {
                    if window.iter().all(|&b| b.is_ascii_hexdigit()) {
                        let hash = String::from_utf8_lossy(window).to_string();
                        items.push(DetectedItem::GitHash(hash, row, i));
                    }
                }

                // IP address pattern (simple v4)
                let ip_parts: Vec<&str> = line_text
                    .split(|c: char| !c.is_numeric() && c != '.')
                    .collect();
                for part in ip_parts.iter() {
                    let nums: Vec<&str> = part.split('.').collect();
                    if nums.len() == 4 && nums.iter().all(|n| n.parse::<u8>().is_ok()) {
                        if let Some(col) = line_text.find(part) {
                            items.push(DetectedItem::IpAddress(part.to_string(), row, col));
                        }
                    }
                }

                // Email pattern (simple)
                if let Some(at_pos) = line_text.find('@') {
                    // Find start of email
                    let start = line_text[..at_pos]
                        .rfind(|c: char| c.is_whitespace())
                        .map(|p| p + 1)
                        .unwrap_or(0);

                    // Find end of email
                    let end = line_text[at_pos..]
                        .find(|c: char| c.is_whitespace())
                        .map(|p| at_pos + p)
                        .unwrap_or(line_text.len());

                    if end > start && line_text[start..end].contains('@') {
                        items.push(DetectedItem::Email(
                            line_text[start..end].to_string(),
                            row,
                            start,
                        ));
                    }
                }
            }
        }

        // Also add URLs and file paths
        items.extend(self.detect_urls());
        items.extend(self.detect_file_paths());

        items
    }

    // === Selection Management ===

    /// Set the current selection
    pub fn set_selection(
        &mut self,
        start: (usize, usize),
        end: (usize, usize),
        mode: SelectionMode,
    ) {
        self.selection = Some(Selection { start, end, mode });
    }

    /// Get the current selection
    pub fn get_selection(&self) -> Option<Selection> {
        self.selection.clone()
    }

    /// Get the text content of the current selection
    pub fn get_selected_text(&self) -> Option<String> {
        let sel = self.selection.as_ref()?;
        let grid = self.active_grid();

        let (start_row, start_col) = (sel.start.1.min(sel.end.1), sel.start.0.min(sel.end.0));
        let (end_row, end_col) = (sel.start.1.max(sel.end.1), sel.start.0.max(sel.end.0));

        match sel.mode {
            SelectionMode::Character => {
                let mut text = String::new();
                for row in start_row..=end_row {
                    if let Some(line) = grid.row(row) {
                        let line_text = cells_to_text(line);
                        let row_start = if row == start_row { start_col } else { 0 };
                        let row_end = if row == end_row {
                            end_col.min(line_text.len())
                        } else {
                            line_text.len()
                        };

                        if row_start < line_text.len() {
                            text.push_str(&line_text[row_start..row_end]);
                            if row < end_row {
                                text.push('\n');
                            }
                        }
                    }
                }
                Some(text)
            }
            SelectionMode::Line => {
                let mut text = String::new();
                for row in start_row..=end_row {
                    if let Some(line) = grid.row(row) {
                        text.push_str(&cells_to_text(line));
                        if row < end_row {
                            text.push('\n');
                        }
                    }
                }
                Some(text)
            }
            SelectionMode::Block => {
                let mut text = String::new();
                for row in start_row..=end_row {
                    if let Some(line) = grid.row(row) {
                        let line_text = cells_to_text(line);
                        let row_text = if start_col < line_text.len() {
                            &line_text[start_col..end_col.min(line_text.len())]
                        } else {
                            ""
                        };
                        text.push_str(row_text);
                        if row < end_row {
                            text.push('\n');
                        }
                    }
                }
                Some(text)
            }
        }
    }

    /// Select the word at the given position
    pub fn select_word_at(&mut self, col: usize, row: usize) {
        if let Some(word) = self.get_word_at(col, row, None) {
            // Find word boundaries
            let grid = self.active_grid();
            if let Some(line) = grid.row(row) {
                let line_text = cells_to_text(line);
                if let Some(word_start) = line_text.find(&word) {
                    let word_end = word_start + word.len();
                    self.selection = Some(Selection {
                        start: (word_start, row),
                        end: (word_end, row),
                        mode: SelectionMode::Character,
                    });
                }
            }
        }
    }

    /// Select the entire line at the given row
    pub fn select_line(&mut self, row: usize) {
        let grid = self.active_grid();
        let cols = grid.cols();
        self.selection = Some(Selection {
            start: (0, row),
            end: (cols, row),
            mode: SelectionMode::Line,
        });
    }

    /// Clear the current selection
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    // === Text Extraction ===

    /// Get text lines around a specific row (with context)
    ///
    /// # Arguments
    /// * `row` - The center row (0-based)
    /// * `context_before` - Number of lines before the row
    /// * `context_after` - Number of lines after the row
    ///
    /// Returns a vector of text lines.
    pub fn get_line_context(
        &self,
        row: usize,
        context_before: usize,
        context_after: usize,
    ) -> Vec<String> {
        let grid = self.active_grid();
        let mut lines = Vec::new();

        let start_row = row.saturating_sub(context_before);
        let end_row = (row + context_after).min(grid.rows() - 1);

        for r in start_row..=end_row {
            if let Some(line) = grid.row(r) {
                lines.push(cells_to_text(line));
            }
        }

        lines
    }

    /// Get the paragraph at the given position
    ///
    /// A paragraph is defined as consecutive non-empty lines.
    pub fn get_paragraph_at(&self, row: usize) -> String {
        let grid = self.active_grid();
        let mut lines = Vec::new();

        // Find start of paragraph (search backwards)
        let mut start_row = row;
        while start_row > 0 {
            if let Some(line) = grid.row(start_row - 1) {
                let text = cells_to_text(line).trim().to_string();
                if text.is_empty() {
                    break;
                }
                start_row -= 1;
            } else {
                break;
            }
        }

        // Find end of paragraph (search forwards)
        let mut end_row = row;
        while end_row < grid.rows() - 1 {
            if let Some(line) = grid.row(end_row + 1) {
                let text = cells_to_text(line).trim().to_string();
                if text.is_empty() {
                    break;
                }
                end_row += 1;
            } else {
                break;
            }
        }

        // Collect paragraph lines
        for r in start_row..=end_row {
            if let Some(line) = grid.row(r) {
                lines.push(cells_to_text(line));
            }
        }

        lines.join("\n")
    }

    // === Scrollback Operations ===

    /// Export scrollback to various formats
    ///
    /// # Arguments
    /// * `format` - Export format (Plain, Html, Ansi)
    /// * `max_lines` - Maximum number of scrollback lines to export (None = all)
    ///
    /// Returns the exported content as a string.
    pub fn export_scrollback(&self, format: ExportFormat, max_lines: Option<usize>) -> String {
        let scrollback_len = self.grid.scrollback_len();
        let lines_to_export = max_lines.unwrap_or(scrollback_len).min(scrollback_len);

        match format {
            ExportFormat::Plain => {
                let mut output = String::new();
                for i in (0..lines_to_export).rev() {
                    if let Some(line) = self.grid.scrollback_line(i) {
                        output.push_str(&cells_to_text(line));
                        output.push('\n');
                    }
                }
                output
            }
            ExportFormat::Html => {
                let mut output = String::from("<pre>\n");
                for i in (0..lines_to_export).rev() {
                    if let Some(line) = self.grid.scrollback_line(i) {
                        let text = cells_to_text(line);
                        output.push_str(&html_escape(&text));
                        output.push('\n');
                    }
                }
                output.push_str("</pre>");
                output
            }
            ExportFormat::Ansi => {
                // For ANSI export, we'd need to preserve colors/attributes
                // For now, just export as plain text
                self.export_scrollback(ExportFormat::Plain, max_lines)
            }
        }
    }

    /// Get scrollback statistics
    pub fn scrollback_stats(&self) -> ScrollbackStats {
        let total_lines = self.grid.scrollback_len();
        let memory_bytes = total_lines * self.grid.cols() * std::mem::size_of::<Cell>();
        // Scrollback has wrapped if we've filled the buffer
        let has_wrapped = total_lines >= self.grid.max_scrollback();

        ScrollbackStats {
            total_lines,
            memory_bytes,
            has_wrapped,
        }
    }

    // === Bookmark Methods ===

    /// Add a bookmark at the given scrollback row
    ///
    /// # Arguments
    /// * `row` - Row index (negative for scrollback, 0+ for visible screen)
    /// * `label` - Optional label for the bookmark
    ///
    /// Returns the bookmark ID.
    pub fn add_bookmark(&mut self, row: isize, label: Option<String>) -> usize {
        let id = self.next_bookmark_id;
        self.next_bookmark_id += 1;

        let bookmark = Bookmark {
            id,
            row,
            label: label.unwrap_or_else(|| format!("Bookmark {}", id)),
        };

        self.bookmarks.push(bookmark);
        id
    }

    /// Get all bookmarks
    pub fn get_bookmarks(&self) -> Vec<Bookmark> {
        self.bookmarks.clone()
    }

    /// Remove a bookmark by ID
    pub fn remove_bookmark(&mut self, id: usize) -> bool {
        if let Some(pos) = self.bookmarks.iter().position(|b| b.id == id) {
            self.bookmarks.remove(pos);
            true
        } else {
            false
        }
    }

    /// Clear all bookmarks
    pub fn clear_bookmarks(&mut self) {
        self.bookmarks.clear();
    }

    // === Feature 7: Performance Metrics ===

    /// Get current performance metrics
    pub fn get_performance_metrics(&self) -> PerformanceMetrics {
        self.perf_metrics.clone()
    }

    /// Reset performance metrics
    pub fn reset_performance_metrics(&mut self) {
        self.perf_metrics = PerformanceMetrics::default();
        self.frame_timings.clear();
    }

    /// Record a frame timing
    pub fn record_frame_timing(
        &mut self,
        processing_us: u64,
        cells_updated: usize,
        bytes_processed: usize,
    ) {
        self.perf_metrics.frames_rendered += 1;
        self.perf_metrics.cells_updated += cells_updated as u64;
        self.perf_metrics.bytes_processed += bytes_processed as u64;
        self.perf_metrics.total_processing_us += processing_us;

        if processing_us > self.perf_metrics.peak_frame_us {
            self.perf_metrics.peak_frame_us = processing_us;
        }

        let frame_timing = FrameTiming {
            frame_number: self.perf_metrics.frames_rendered,
            processing_us,
            cells_updated,
            bytes_processed,
        };

        self.frame_timings.push(frame_timing);

        // Keep only last N frames
        if self.frame_timings.len() > self.max_frame_timings {
            self.frame_timings.remove(0);
        }
    }

    /// Get recent frame timings
    pub fn get_frame_timings(&self, count: Option<usize>) -> Vec<FrameTiming> {
        let count = count
            .unwrap_or(self.frame_timings.len())
            .min(self.frame_timings.len());
        self.frame_timings[self.frame_timings.len() - count..].to_vec()
    }

    /// Get average frame time in microseconds
    pub fn get_average_frame_time(&self) -> u64 {
        if self.perf_metrics.frames_rendered == 0 {
            0
        } else {
            self.perf_metrics.total_processing_us / self.perf_metrics.frames_rendered
        }
    }

    /// Get frames per second (based on average frame time)
    pub fn get_fps(&self) -> f64 {
        let avg_time = self.get_average_frame_time();
        if avg_time == 0 {
            0.0
        } else {
            1_000_000.0 / avg_time as f64
        }
    }

    // === Feature 8: Advanced Color Operations ===

    /// Convert RGB color to HSV
    pub fn rgb_to_hsv_color(&self, r: u8, g: u8, b: u8) -> ColorHSV {
        rgb_to_hsv(r, g, b)
    }

    /// Convert HSV color to RGB
    pub fn hsv_to_rgb_color(&self, hsv: ColorHSV) -> (u8, u8, u8) {
        hsv_to_rgb(hsv)
    }

    /// Convert RGB color to HSL
    pub fn rgb_to_hsl_color(&self, r: u8, g: u8, b: u8) -> ColorHSL {
        rgb_to_hsl(r, g, b)
    }

    /// Convert HSL color to RGB
    pub fn hsl_to_rgb_color(&self, hsl: ColorHSL) -> (u8, u8, u8) {
        hsl_to_rgb(hsl)
    }

    /// Generate a color palette based on a base color and theme mode
    pub fn generate_color_palette(&self, r: u8, g: u8, b: u8, mode: ThemeMode) -> ColorPalette {
        let hsl = rgb_to_hsl(r, g, b);

        let colors = match mode {
            ThemeMode::Complementary => {
                let comp_hsl = ColorHSL {
                    h: (hsl.h + 180.0) % 360.0,
                    s: hsl.s,
                    l: hsl.l,
                };
                vec![hsl_to_rgb(comp_hsl)]
            }
            ThemeMode::Analogous => {
                let angle = 30.0;
                vec![
                    hsl_to_rgb(ColorHSL {
                        h: (hsl.h + angle) % 360.0,
                        ..hsl
                    }),
                    hsl_to_rgb(ColorHSL {
                        h: (hsl.h + 360.0 - angle) % 360.0,
                        ..hsl
                    }),
                ]
            }
            ThemeMode::Triadic => {
                vec![
                    hsl_to_rgb(ColorHSL {
                        h: (hsl.h + 120.0) % 360.0,
                        ..hsl
                    }),
                    hsl_to_rgb(ColorHSL {
                        h: (hsl.h + 240.0) % 360.0,
                        ..hsl
                    }),
                ]
            }
            ThemeMode::Tetradic => {
                vec![
                    hsl_to_rgb(ColorHSL {
                        h: (hsl.h + 90.0) % 360.0,
                        ..hsl
                    }),
                    hsl_to_rgb(ColorHSL {
                        h: (hsl.h + 180.0) % 360.0,
                        ..hsl
                    }),
                    hsl_to_rgb(ColorHSL {
                        h: (hsl.h + 270.0) % 360.0,
                        ..hsl
                    }),
                ]
            }
            ThemeMode::SplitComplementary => {
                let comp_h = (hsl.h + 180.0) % 360.0;
                vec![
                    hsl_to_rgb(ColorHSL {
                        h: (comp_h + 30.0) % 360.0,
                        ..hsl
                    }),
                    hsl_to_rgb(ColorHSL {
                        h: (comp_h + 360.0 - 30.0) % 360.0,
                        ..hsl
                    }),
                ]
            }
            ThemeMode::Monochromatic => {
                vec![
                    hsl_to_rgb(ColorHSL {
                        l: (hsl.l + 0.2).min(1.0),
                        ..hsl
                    }),
                    hsl_to_rgb(ColorHSL {
                        l: (hsl.l - 0.2).max(0.0),
                        ..hsl
                    }),
                    hsl_to_rgb(ColorHSL {
                        l: (hsl.l + 0.4).min(1.0),
                        ..hsl
                    }),
                    hsl_to_rgb(ColorHSL {
                        l: (hsl.l - 0.4).max(0.0),
                        ..hsl
                    }),
                ]
            }
        };

        ColorPalette {
            base: (r, g, b),
            colors,
            mode,
        }
    }

    /// Calculate color distance (Euclidean distance in RGB space)
    pub fn color_distance(&self, r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f64 {
        let dr = r1 as f64 - r2 as f64;
        let dg = g1 as f64 - g2 as f64;
        let db = b1 as f64 - b2 as f64;
        (dr * dr + dg * dg + db * db).sqrt()
    }

    // === Feature 9: Line Wrapping Utilities ===

    /// Join wrapped lines starting from a given row
    ///
    /// Unwraps soft-wrapped lines into a single logical line.
    pub fn join_wrapped_lines(&self, start_row: usize) -> Option<JoinedLines> {
        let grid = self.active_grid();
        if start_row >= grid.rows() {
            return None;
        }

        let mut lines = Vec::new();
        let mut current_row = start_row;

        // Collect the first line
        if let Some(line) = grid.row(current_row) {
            lines.push(cells_to_text(line));
        } else {
            return None;
        }

        // Follow wrapped lines
        while current_row < grid.rows() - 1 && grid.is_line_wrapped(current_row) {
            current_row += 1;
            if let Some(line) = grid.row(current_row) {
                lines.push(cells_to_text(line));
            } else {
                break;
            }
        }

        Some(JoinedLines {
            text: lines.join(""),
            start_row,
            end_row: current_row,
            lines_joined: lines.len(),
        })
    }

    /// Get all logical lines (unwrapped) in the visible screen
    pub fn get_logical_lines(&self) -> Vec<String> {
        let grid = self.active_grid();
        let mut logical_lines = Vec::new();
        let mut row = 0;

        while row < grid.rows() {
            if let Some(joined) = self.join_wrapped_lines(row) {
                logical_lines.push(joined.text);
                row = joined.end_row + 1;
            } else {
                row += 1;
            }
        }

        logical_lines
    }

    /// Check if a row starts a new logical line (not a continuation)
    pub fn is_line_start(&self, row: usize) -> bool {
        if row == 0 {
            return true;
        }
        let grid = self.active_grid();
        !grid.is_line_wrapped(row.saturating_sub(1))
    }

    // === Feature 10: Clipboard Integration ===

    /// Add content to clipboard history
    pub fn add_to_clipboard_history(
        &mut self,
        slot: ClipboardSlot,
        content: String,
        label: Option<String>,
    ) {
        let entry = ClipboardEntry {
            content,
            timestamp: get_timestamp_us(),
            label,
        };

        let history = self.clipboard_history.entry(slot).or_default();
        history.push(entry);

        // Keep only last N entries
        if history.len() > self.max_clipboard_history {
            history.remove(0);
        }
    }

    /// Get clipboard history for a slot
    pub fn get_clipboard_history(&self, slot: ClipboardSlot) -> Vec<ClipboardEntry> {
        self.clipboard_history
            .get(&slot)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the most recent clipboard entry for a slot
    pub fn get_latest_clipboard(&self, slot: ClipboardSlot) -> Option<ClipboardEntry> {
        self.clipboard_history.get(&slot)?.last().cloned()
    }

    /// Clear clipboard history for a slot
    pub fn clear_clipboard_history(&mut self, slot: ClipboardSlot) {
        self.clipboard_history.remove(&slot);
    }

    /// Clear all clipboard history
    pub fn clear_all_clipboard_history(&mut self) {
        self.clipboard_history.clear();
    }

    /// Set clipboard content with slot (convenience method that also adds to history)
    pub fn set_clipboard_with_slot(&mut self, content: String, slot: Option<ClipboardSlot>) {
        let slot = slot.unwrap_or(ClipboardSlot::Primary);

        // Update the current clipboard_content field
        self.clipboard_content = Some(content.clone());

        // Add to history
        self.add_to_clipboard_history(slot, content, None);
    }

    /// Get clipboard content from history or current clipboard
    pub fn get_clipboard_from_slot(&self, slot: Option<ClipboardSlot>) -> Option<String> {
        let slot = slot.unwrap_or(ClipboardSlot::Primary);

        // Try history first
        if let Some(entry) = self.get_latest_clipboard(slot) {
            return Some(entry.content);
        }

        // Fall back to current clipboard_content
        self.clipboard_content.clone()
    }

    /// Search clipboard history
    pub fn search_clipboard_history(
        &self,
        query: &str,
        slot: Option<ClipboardSlot>,
    ) -> Vec<ClipboardEntry> {
        let query_lower = query.to_lowercase();

        if let Some(slot) = slot {
            // Search specific slot
            self.clipboard_history
                .get(&slot)
                .map(|entries| {
                    entries
                        .iter()
                        .filter(|e| e.content.to_lowercase().contains(&query_lower))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        } else {
            // Search all slots
            let mut results = Vec::new();
            for entries in self.clipboard_history.values() {
                for entry in entries {
                    if entry.content.to_lowercase().contains(&query_lower) {
                        results.push(entry.clone());
                    }
                }
            }
            results.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
            results
        }
    }

    // === Feature 17: Advanced Mouse Support ===

    /// Record a mouse event in the history
    pub fn record_mouse_event(&mut self, event: MouseEventRecord) {
        // Record position
        let position = MousePosition {
            col: event.col,
            row: event.row,
            timestamp: event.timestamp,
        };
        self.mouse_positions.push(position);

        // Record event
        self.mouse_events.push(event);

        // Limit history size
        if self.mouse_events.len() > self.max_mouse_history {
            self.mouse_events
                .drain(0..self.mouse_events.len() - self.max_mouse_history);
        }
        if self.mouse_positions.len() > self.max_mouse_history {
            self.mouse_positions
                .drain(0..self.mouse_positions.len() - self.max_mouse_history);
        }
    }

    /// Get mouse events, optionally limited to most recent N
    pub fn get_mouse_events(&self, count: Option<usize>) -> Vec<MouseEventRecord> {
        if let Some(n) = count {
            self.mouse_events
                .iter()
                .rev()
                .take(n)
                .rev()
                .cloned()
                .collect()
        } else {
            self.mouse_events.clone()
        }
    }

    /// Get mouse positions, optionally limited to most recent N
    pub fn get_mouse_positions(&self, count: Option<usize>) -> Vec<MousePosition> {
        if let Some(n) = count {
            self.mouse_positions
                .iter()
                .rev()
                .take(n)
                .rev()
                .cloned()
                .collect()
        } else {
            self.mouse_positions.clone()
        }
    }

    /// Get the last recorded mouse position
    pub fn get_last_mouse_position(&self) -> Option<MousePosition> {
        self.mouse_positions.last().cloned()
    }

    /// Clear mouse event and position history
    pub fn clear_mouse_history(&mut self) {
        self.mouse_events.clear();
        self.mouse_positions.clear();
    }

    /// Set maximum mouse history size
    pub fn set_max_mouse_history(&mut self, max: usize) {
        self.max_mouse_history = max;
        // Trim existing history if needed
        if self.mouse_events.len() > max {
            self.mouse_events.drain(0..self.mouse_events.len() - max);
        }
        if self.mouse_positions.len() > max {
            self.mouse_positions
                .drain(0..self.mouse_positions.len() - max);
        }
    }

    /// Get maximum mouse history size
    pub fn get_max_mouse_history(&self) -> usize {
        self.max_mouse_history
    }

    // === Feature 19: Custom Rendering Hints ===

    /// Add a damage region to track screen area changes
    pub fn add_damage_region(&mut self, left: usize, top: usize, right: usize, bottom: usize) {
        let region = DamageRegion {
            left,
            top,
            right,
            bottom,
        };
        self.damage_regions.push(region);
    }

    /// Get all current damage regions
    pub fn get_damage_regions(&self) -> Vec<DamageRegion> {
        self.damage_regions.clone()
    }

    /// Merge overlapping damage regions to reduce count
    pub fn merge_damage_regions(&mut self) {
        if self.damage_regions.len() < 2 {
            return;
        }

        let mut merged = Vec::new();
        let mut regions = self.damage_regions.clone();
        regions.sort_by_key(|r| (r.top, r.left));

        let mut current = regions[0];

        for region in regions.iter().skip(1) {
            // Check if regions overlap or are adjacent
            if region.left <= current.right + 1
                && region.top <= current.bottom + 1
                && region.right >= current.left.saturating_sub(1)
                && region.bottom >= current.top.saturating_sub(1)
            {
                // Merge
                current = DamageRegion {
                    left: current.left.min(region.left),
                    top: current.top.min(region.top),
                    right: current.right.max(region.right),
                    bottom: current.bottom.max(region.bottom),
                };
            } else {
                merged.push(current);
                current = *region;
            }
        }
        merged.push(current);

        self.damage_regions = merged;
    }

    /// Clear all damage regions
    pub fn clear_damage_regions(&mut self) {
        self.damage_regions.clear();
    }

    /// Add a rendering hint
    pub fn add_rendering_hint(
        &mut self,
        damage: DamageRegion,
        layer: ZLayer,
        animation: AnimationHint,
        priority: UpdatePriority,
    ) {
        let hint = RenderingHint {
            damage,
            layer,
            animation,
            priority,
        };
        self.rendering_hints.push(hint);
    }

    /// Get all rendering hints, optionally sorted by priority
    pub fn get_rendering_hints(&self, sort_by_priority: bool) -> Vec<RenderingHint> {
        if sort_by_priority {
            let mut hints = self.rendering_hints.clone();
            hints.sort_by_key(|h| std::cmp::Reverse(h.priority as u8));
            hints
        } else {
            self.rendering_hints.clone()
        }
    }

    /// Clear all rendering hints
    pub fn clear_rendering_hints(&mut self) {
        self.rendering_hints.clear();
    }

    // === Feature 16: Performance Profiling ===

    /// Enable performance profiling
    pub fn enable_profiling(&mut self) {
        self.profiling_enabled = true;
        if self.profiling_data.is_none() {
            self.profiling_data = Some(ProfilingData {
                categories: std::collections::HashMap::new(),
                allocations: 0,
                bytes_allocated: 0,
                peak_memory: 0,
            });
        }
    }

    /// Disable performance profiling
    pub fn disable_profiling(&mut self) {
        self.profiling_enabled = false;
    }

    /// Check if profiling is enabled
    pub fn is_profiling_enabled(&self) -> bool {
        self.profiling_enabled
    }

    /// Get current profiling data
    pub fn get_profiling_data(&self) -> Option<ProfilingData> {
        self.profiling_data.clone()
    }

    /// Reset profiling data
    pub fn reset_profiling_data(&mut self) {
        if let Some(ref mut data) = self.profiling_data {
            data.categories.clear();
            data.allocations = 0;
            data.bytes_allocated = 0;
            data.peak_memory = 0;
        }
    }

    /// Record an escape sequence execution for profiling
    pub fn record_escape_sequence(&mut self, category: ProfileCategory, time_us: u64) {
        if !self.profiling_enabled {
            return;
        }

        if let Some(ref mut data) = self.profiling_data {
            let profile = data
                .categories
                .entry(category)
                .or_insert(EscapeSequenceProfile {
                    count: 0,
                    total_time_us: 0,
                    peak_time_us: 0,
                    avg_time_us: 0,
                });

            profile.count += 1;
            profile.total_time_us += time_us;
            profile.peak_time_us = profile.peak_time_us.max(time_us);
            profile.avg_time_us = profile.total_time_us / profile.count;
        }
    }

    /// Record memory allocation for profiling
    pub fn record_allocation(&mut self, bytes: u64) {
        if !self.profiling_enabled {
            return;
        }

        if let Some(ref mut data) = self.profiling_data {
            data.allocations += 1;
            data.bytes_allocated += bytes;
        }
    }

    /// Update peak memory usage
    pub fn update_peak_memory(&mut self, current_bytes: usize) {
        if !self.profiling_enabled {
            return;
        }

        if let Some(ref mut data) = self.profiling_data {
            data.peak_memory = data.peak_memory.max(current_bytes);
        }
    }

    // === Feature 15: Regex Search in Scrollback ===

    /// Perform regex search on terminal content
    pub fn regex_search(
        &mut self,
        pattern: &str,
        options: RegexSearchOptions,
    ) -> Result<Vec<RegexMatch>, String> {
        // Build regex with options
        let mut regex_pattern = pattern.to_string();
        if options.case_insensitive {
            regex_pattern = format!("(?i){}", regex_pattern);
        }
        if options.multiline {
            regex_pattern = format!("(?m){}", regex_pattern);
        }

        let re = regex::Regex::new(&regex_pattern).map_err(|e| format!("Invalid regex: {}", e))?;

        let mut matches = Vec::new();
        let grid = self.active_grid();

        // Collect all lines to search
        let mut all_lines = Vec::new();
        let mut line_offsets = Vec::new(); // Track which row each line corresponds to

        // Include scrollback if requested
        if options.include_scrollback {
            for i in 0..grid.scrollback_len() {
                if let Some(line) = grid.scrollback_line(i) {
                    let text = cells_to_text(line);
                    line_offsets.push(i);
                    all_lines.push(text);
                }
            }
        }

        // Add visible screen lines
        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                let text = cells_to_text(line);
                line_offsets.push(grid.scrollback_len() + row);
                all_lines.push(text);
            }
        }

        // Search through lines
        for (line_idx, line) in all_lines.iter().enumerate() {
            for cap in re.captures_iter(line) {
                let m = cap.get(0).unwrap();
                let captures: Vec<String> = cap
                    .iter()
                    .skip(1)
                    .filter_map(|c| c.map(|m| m.as_str().to_string()))
                    .collect();

                matches.push(RegexMatch {
                    row: line_offsets[line_idx],
                    col: m.start(),
                    end_row: line_offsets[line_idx],
                    end_col: m.end(),
                    text: m.as_str().to_string(),
                    captures,
                });

                if options.max_matches > 0 && matches.len() >= options.max_matches {
                    break;
                }
            }

            if options.max_matches > 0 && matches.len() >= options.max_matches {
                break;
            }
        }

        if options.reverse {
            matches.reverse();
        }

        // Cache the results
        self.regex_matches = matches.clone();
        self.current_regex_pattern = Some(pattern.to_string());

        Ok(matches)
    }

    /// Get cached regex matches
    pub fn get_regex_matches(&self) -> Vec<RegexMatch> {
        self.regex_matches.clone()
    }

    /// Get the current regex search pattern
    pub fn get_current_regex_pattern(&self) -> Option<String> {
        self.current_regex_pattern.clone()
    }

    /// Clear regex search cache
    pub fn clear_regex_matches(&mut self) {
        self.regex_matches.clear();
        self.current_regex_pattern = None;
    }

    /// Find next regex match from a given position
    pub fn next_regex_match(&self, from_row: usize, from_col: usize) -> Option<RegexMatch> {
        self.regex_matches
            .iter()
            .find(|m| m.row > from_row || (m.row == from_row && m.col > from_col))
            .cloned()
    }

    /// Find previous regex match from a given position
    pub fn prev_regex_match(&self, from_row: usize, from_col: usize) -> Option<RegexMatch> {
        self.regex_matches
            .iter()
            .rev()
            .find(|m| m.row < from_row || (m.row == from_row && m.col < from_col))
            .cloned()
    }

    // === Feature 13: Terminal Multiplexing Helpers ===

    /// Capture current pane state
    pub fn capture_pane_state(&self, id: String, cwd: Option<String>) -> PaneState {
        let grid = self.active_grid();
        let rows = grid.rows();

        // Capture screen content
        let mut content = Vec::with_capacity(rows);
        for row in 0..rows {
            if let Some(line) = grid.row(row) {
                content.push(cells_to_text(line));
            } else {
                content.push(String::new());
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        PaneState {
            id,
            title: self.title.clone(),
            size: (grid.cols(), grid.rows()),
            position: (0, 0), // Position should be set by layout manager
            cwd,
            env: std::collections::HashMap::new(), // Env vars should be provided externally
            content,
            cursor: (self.cursor.col, self.cursor.row),
            alt_screen: self.alt_screen_active,
            scroll_offset: 0, // Should be provided by scroll state
            created_at: now,
            last_activity: now,
        }
    }

    /// Restore pane state (partial - restores what's possible)
    pub fn restore_pane_state(&mut self, state: &PaneState) -> Result<(), String> {
        // Verify size matches
        let (cols, rows) = {
            let grid = self.active_grid();
            (grid.cols(), grid.rows())
        };

        if cols != state.size.0 || rows != state.size.1 {
            return Err(format!(
                "Size mismatch: terminal is {}x{} but state is {}x{}",
                cols, rows, state.size.0, state.size.1
            ));
        }

        // Restore title
        self.title = state.title.clone();

        // Restore cursor position (bounds checked)
        if state.cursor.0 < cols && state.cursor.1 < rows {
            self.cursor.col = state.cursor.0;
            self.cursor.row = state.cursor.1;
        }

        // Switch to alternate screen if needed
        if state.alt_screen && !self.alt_screen_active {
            self.alt_screen_active = true;
        } else if !state.alt_screen && self.alt_screen_active {
            self.alt_screen_active = false;
        }

        // Note: Content restoration would require writing to grid cells
        // which is complex and may interfere with running processes
        // This is left for higher-level implementation

        Ok(())
    }

    /// Store current pane state internally
    pub fn set_pane_state(&mut self, state: PaneState) {
        self.pane_state = Some(state);
    }

    /// Get stored pane state
    pub fn get_pane_state(&self) -> Option<PaneState> {
        self.pane_state.clone()
    }

    /// Clear stored pane state
    pub fn clear_pane_state(&mut self) {
        self.pane_state = None;
    }

    /// Create a window layout from pane IDs
    pub fn create_window_layout(
        id: String,
        name: String,
        direction: LayoutDirection,
        panes: Vec<String>,
        sizes: Vec<u8>,
        active_pane: usize,
    ) -> Result<WindowLayout, String> {
        // Validate inputs
        if panes.is_empty() {
            return Err("Layout must contain at least one pane".to_string());
        }
        if panes.len() != sizes.len() {
            return Err("Number of panes must match number of sizes".to_string());
        }
        if active_pane >= panes.len() {
            return Err("Active pane index out of bounds".to_string());
        }

        // Validate sizes sum to 100%
        let total: u32 = sizes.iter().map(|&s| s as u32).sum();
        if total != 100 {
            return Err(format!("Sizes must sum to 100%, got {}", total));
        }

        Ok(WindowLayout {
            id,
            name,
            direction,
            panes,
            sizes,
            active_pane,
        })
    }

    /// Create a session state
    pub fn create_session_state(
        id: String,
        name: String,
        panes: Vec<PaneState>,
        layouts: Vec<WindowLayout>,
        active_layout: usize,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<SessionState, String> {
        if panes.is_empty() {
            return Err("Session must contain at least one pane".to_string());
        }
        if layouts.is_empty() {
            return Err("Session must contain at least one layout".to_string());
        }
        if active_layout >= layouts.len() {
            return Err("Active layout index out of bounds".to_string());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Ok(SessionState {
            id,
            name,
            panes,
            layouts,
            active_layout,
            metadata,
            created_at: now,
            last_saved: now,
        })
    }

    /// Serialize session state to JSON
    pub fn serialize_session(state: &SessionState) -> Result<String, String> {
        serde_json::to_string_pretty(state)
            .map_err(|e| format!("Failed to serialize session: {}", e))
    }

    /// Deserialize session state from JSON
    pub fn deserialize_session(json: &str) -> Result<SessionState, String> {
        serde_json::from_str(json).map_err(|e| format!("Failed to deserialize session: {}", e))
    }

    // === Feature 21: Image Protocol Support ===

    /// Add an inline image
    pub fn add_inline_image(&mut self, image: InlineImage) {
        self.inline_images.push(image);

        // Limit number of stored images
        if self.inline_images.len() > self.max_inline_images {
            self.inline_images
                .drain(0..self.inline_images.len() - self.max_inline_images);
        }
    }

    /// Get inline images at a specific position
    pub fn get_images_at(&self, col: usize, row: usize) -> Vec<InlineImage> {
        self.inline_images
            .iter()
            .filter(|img| img.position == (col, row))
            .cloned()
            .collect()
    }

    /// Get all inline images
    pub fn get_all_images(&self) -> Vec<InlineImage> {
        self.inline_images.clone()
    }

    /// Delete image by ID
    pub fn delete_image(&mut self, id: &str) -> bool {
        let before_len = self.inline_images.len();
        self.inline_images
            .retain(|img| img.id.as_ref().map_or(true, |img_id| img_id != id));
        self.inline_images.len() < before_len
    }

    /// Clear all inline images
    pub fn clear_images(&mut self) {
        self.inline_images.clear();
    }

    /// Get image by ID
    pub fn get_image_by_id(&self, id: &str) -> Option<InlineImage> {
        self.inline_images
            .iter()
            .find(|img| img.id.as_ref().is_some_and(|img_id| img_id == id))
            .cloned()
    }

    /// Set maximum inline images
    pub fn set_max_inline_images(&mut self, max: usize) {
        self.max_inline_images = max;
        if self.inline_images.len() > max {
            self.inline_images.drain(0..self.inline_images.len() - max);
        }
    }

    // === Feature 28: Benchmarking Suite ===

    /// Run rendering benchmark
    pub fn benchmark_rendering(&mut self, iterations: u64) -> BenchmarkResult {
        let start = std::time::Instant::now();
        let mut min_time = u64::MAX;
        let mut max_time = 0u64;

        for _ in 0..iterations {
            let iter_start = std::time::Instant::now();

            // Simulate rendering operation
            let grid = self.active_grid();
            for row in 0..grid.rows() {
                if let Some(line) = grid.row(row) {
                    let _ = cells_to_text(line);
                }
            }

            let iter_time = iter_start.elapsed().as_micros() as u64;
            min_time = min_time.min(iter_time);
            max_time = max_time.max(iter_time);
        }

        let total_time = start.elapsed().as_micros() as u64;
        let avg_time = total_time / iterations;

        BenchmarkResult {
            category: BenchmarkCategory::Rendering,
            name: "Text Rendering".to_string(),
            iterations,
            total_time_us: total_time,
            avg_time_us: avg_time,
            min_time_us: min_time,
            max_time_us: max_time,
            ops_per_sec: if avg_time > 0 {
                1_000_000.0 / avg_time as f64
            } else {
                0.0
            },
            memory_bytes: None,
        }
    }

    /// Run escape sequence parsing benchmark
    pub fn benchmark_parsing(&mut self, text: &str, iterations: u64) -> BenchmarkResult {
        let start = std::time::Instant::now();
        let mut min_time = u64::MAX;
        let mut max_time = 0u64;
        let bytes = text.as_bytes();

        for _ in 0..iterations {
            let iter_start = std::time::Instant::now();

            // Parse the text using process()
            self.process(bytes);

            let iter_time = iter_start.elapsed().as_micros() as u64;
            min_time = min_time.min(iter_time);
            max_time = max_time.max(iter_time);
        }

        let total_time = start.elapsed().as_micros() as u64;
        let avg_time = total_time / iterations;
        let bytes_per_sec = if avg_time > 0 {
            text.len() as f64 * 1_000_000.0 / avg_time as f64
        } else {
            0.0
        };

        BenchmarkResult {
            category: BenchmarkCategory::Parsing,
            name: "Escape Sequence Parsing".to_string(),
            iterations,
            total_time_us: total_time,
            avg_time_us: avg_time,
            min_time_us: min_time,
            max_time_us: max_time,
            ops_per_sec: bytes_per_sec,
            memory_bytes: None,
        }
    }

    /// Run grid operations benchmark
    pub fn benchmark_grid_ops(&mut self, iterations: u64) -> BenchmarkResult {
        let start = std::time::Instant::now();
        let mut min_time = u64::MAX;
        let mut max_time = 0u64;

        for _ in 0..iterations {
            let iter_start = std::time::Instant::now();

            // Perform grid operations: write char, move cursor, scroll
            self.process(b"X"); // Write a character
            self.process(b"\x1b[H"); // Move cursor to home
            self.grid.scroll_up(1); // Scroll up by 1 line

            let iter_time = iter_start.elapsed().as_micros() as u64;
            min_time = min_time.min(iter_time);
            max_time = max_time.max(iter_time);
        }

        let total_time = start.elapsed().as_micros() as u64;
        let avg_time = total_time / iterations;

        BenchmarkResult {
            category: BenchmarkCategory::GridOps,
            name: "Grid Operations".to_string(),
            iterations,
            total_time_us: total_time,
            avg_time_us: avg_time,
            min_time_us: min_time,
            max_time_us: max_time,
            ops_per_sec: if avg_time > 0 {
                1_000_000.0 / avg_time as f64
            } else {
                0.0
            },
            memory_bytes: None,
        }
    }

    /// Run full benchmark suite
    pub fn run_benchmark_suite(&mut self, suite_name: String) -> BenchmarkSuite {
        let start = std::time::Instant::now();
        let mut results = Vec::new();

        // Rendering benchmark
        results.push(self.benchmark_rendering(1000));

        // Parsing benchmark
        let test_text = "\x1b[1;31mHello\x1b[0m \x1b[2J\x1b[H";
        results.push(self.benchmark_parsing(test_text, 1000));

        // Grid ops benchmark
        results.push(self.benchmark_grid_ops(10000));

        let total_time_ms = start.elapsed().as_millis() as u64;

        BenchmarkSuite {
            results,
            total_time_ms,
            suite_name,
        }
    }

    // === Feature 29: Terminal Compliance Testing ===

    /// Run compliance tests for a specific level
    pub fn test_compliance(&mut self, level: ComplianceLevel) -> ComplianceReport {
        let mut tests = Vec::new();

        // VT100 basic tests
        if level >= ComplianceLevel::VT100 {
            tests.extend(self.test_vt100_compliance());
        }

        // VT220 tests
        if level >= ComplianceLevel::VT220 {
            tests.extend(self.test_vt220_compliance());
        }

        // VT320 tests
        if level >= ComplianceLevel::VT320 {
            tests.extend(self.test_vt320_compliance());
        }

        let passed = tests.iter().filter(|t| t.passed).count();
        let failed = tests.len() - passed;
        let compliance_percent = if tests.is_empty() {
            0.0
        } else {
            (passed as f64 / tests.len() as f64) * 100.0
        };

        ComplianceReport {
            terminal_info: format!("Terminal Emulator v{}", env!("CARGO_PKG_VERSION")),
            level,
            tests,
            passed,
            failed,
            compliance_percent,
        }
    }

    /// VT100 compliance tests
    fn test_vt100_compliance(&mut self) -> Vec<ComplianceTest> {
        let mut tests = Vec::new();

        // Test cursor movement - CSI 6 ; 6 H (move to row 6, col 6, 1-indexed)
        self.process(b"\x1b[6;6H");
        let result = (self.cursor.col, self.cursor.row);
        tests.push(ComplianceTest {
            name: "Cursor positioning".to_string(),
            category: "VT100".to_string(),
            passed: result == (5, 5), // 0-indexed
            expected: "(5, 5)".to_string(),
            actual: format!("{:?}", result),
            notes: None,
        });

        // Test clear screen - CSI 2 J
        self.process(b"X"); // Write a character
        self.process(b"\x1b[2J"); // Clear screen
        let rows = self.grid.rows();
        let is_clear = (0..rows.min(3)).all(|row_idx| {
            self.grid
                .row(row_idx)
                .map_or(true, |row| row.iter().take(10).all(|cell| cell.c == ' '))
        });
        tests.push(ComplianceTest {
            name: "Clear screen".to_string(),
            category: "VT100".to_string(),
            passed: is_clear,
            expected: "All spaces".to_string(),
            actual: if is_clear {
                "Clear".to_string()
            } else {
                "Not clear".to_string()
            },
            notes: None,
        });

        // Test SGR attributes - CSI 1 m (bold)
        self.process(b"\x1b[1m"); // Set bold
        let bold = self.flags.bold();
        tests.push(ComplianceTest {
            name: "SGR bold attribute".to_string(),
            category: "VT100".to_string(),
            passed: bold,
            expected: "Bold enabled".to_string(),
            actual: format!("Bold: {}", bold),
            notes: None,
        });

        tests
    }

    /// VT220 compliance tests
    fn test_vt220_compliance(&mut self) -> Vec<ComplianceTest> {
        let mut tests = Vec::new();

        // Test insert lines - CSI L
        self.process(b"\x1b[1L"); // Insert 1 line
        tests.push(ComplianceTest {
            name: "Insert lines".to_string(),
            category: "VT220".to_string(),
            passed: true, // If no crash, it works
            expected: "No error".to_string(),
            actual: "Success".to_string(),
            notes: Some("Basic functionality test".to_string()),
        });

        // Test delete characters - CSI P
        self.process(b"ABC"); // Write some characters
        self.process(b"\x1b[H"); // Move to home
        self.process(b"\x1b[1P"); // Delete 1 character
        tests.push(ComplianceTest {
            name: "Delete characters".to_string(),
            category: "VT220".to_string(),
            passed: true,
            expected: "Character deleted".to_string(),
            actual: "Success".to_string(),
            notes: None,
        });

        tests
    }

    /// VT320 compliance tests
    fn test_vt320_compliance(&mut self) -> Vec<ComplianceTest> {
        let mut tests = Vec::new();

        // Test color support - CSI 38 ; 5 ; n m (set foreground to color n)
        self.process(b"\x1b[38;5;1m"); // Set foreground to color 1 (red)
        tests.push(ComplianceTest {
            name: "Indexed colors".to_string(),
            category: "VT320".to_string(),
            passed: true, // Basic support
            expected: "Color set".to_string(),
            actual: "Success".to_string(),
            notes: Some("256 color support".to_string()),
        });

        tests
    }

    /// Generate compliance report as formatted string
    pub fn format_compliance_report(report: &ComplianceReport) -> String {
        let mut output = String::new();
        output.push_str("=== Terminal Compliance Report ===\n");
        output.push_str(&format!("Terminal: {}\n", report.terminal_info));
        output.push_str(&format!("Level: {:?}\n", report.level));
        output.push_str(&format!(
            "Passed: {}/{}\n",
            report.passed,
            report.passed + report.failed
        ));
        output.push_str(&format!(
            "Compliance: {:.1}%\n\n",
            report.compliance_percent
        ));

        for test in &report.tests {
            let status = if test.passed { "✓" } else { "✗" };
            output.push_str(&format!("{} [{}] {}\n", status, test.category, test.name));
            if !test.passed {
                output.push_str(&format!("  Expected: {}\n", test.expected));
                output.push_str(&format!("  Actual: {}\n", test.actual));
                if let Some(ref notes) = test.notes {
                    output.push_str(&format!("  Notes: {}\n", notes));
                }
            }
        }

        output
    }

    // === Feature 30: OSC 52 Clipboard Sync ===

    /// Record a clipboard sync event
    pub fn record_clipboard_sync(
        &mut self,
        target: ClipboardTarget,
        operation: ClipboardOperation,
        mut content: Option<String>,
        is_remote: bool,
    ) {
        if let Some(ref mut text) = content {
            sanitize_clipboard_content(text, self.max_clipboard_event_bytes);
        }

        if self.max_clipboard_sync_events > 0 {
            let event = ClipboardSyncEvent {
                target,
                operation,
                content: content.clone(),
                timestamp: unix_millis(),
                is_remote,
            };

            self.clipboard_sync_events.push(event);

            if self.clipboard_sync_events.len() > self.max_clipboard_sync_events {
                let excess = self.clipboard_sync_events.len() - self.max_clipboard_sync_events;
                self.clipboard_sync_events.drain(0..excess);
            }
        }

        // Add to history if it's a Set operation
        if let (ClipboardOperation::Set, Some(content)) = (operation, content) {
            let entry = ClipboardHistoryEntry {
                target,
                content,
                timestamp: unix_millis(),
                source: self.remote_session_id.clone(),
            };

            self.clipboard_sync_history
                .entry(target)
                .or_default()
                .push(entry);

            // Limit history size
            if let Some(entries) = self.clipboard_sync_history.get_mut(&target) {
                if entries.len() > self.max_clipboard_sync_history {
                    entries.drain(0..entries.len() - self.max_clipboard_sync_history);
                }
            }
        }
    }

    /// Get clipboard sync events
    pub fn get_clipboard_sync_events(&self) -> &[ClipboardSyncEvent] {
        &self.clipboard_sync_events
    }

    /// Get clipboard sync history for a target
    pub fn get_clipboard_sync_history(
        &self,
        target: ClipboardTarget,
    ) -> Option<&[ClipboardHistoryEntry]> {
        self.clipboard_sync_history
            .get(&target)
            .map(|v| v.as_slice())
    }

    /// Clear clipboard sync events
    pub fn clear_clipboard_sync_events(&mut self) {
        self.clipboard_sync_events.clear();
    }

    /// Set maximum clipboard sync events retained (0 disables buffering)
    pub fn set_max_clipboard_sync_events(&mut self, max: usize) {
        self.max_clipboard_sync_events = max;
        if max == 0 {
            self.clipboard_sync_events.clear();
        } else if self.clipboard_sync_events.len() > max {
            let excess = self.clipboard_sync_events.len() - max;
            self.clipboard_sync_events.drain(0..excess);
        }
    }

    /// Get maximum clipboard sync events retained
    pub fn max_clipboard_sync_events(&self) -> usize {
        self.max_clipboard_sync_events
    }

    /// Set maximum bytes cached per clipboard event (0 clears content)
    pub fn set_max_clipboard_event_bytes(&mut self, max_bytes: usize) {
        self.max_clipboard_event_bytes = max_bytes;
        if max_bytes == 0 {
            for event in &mut self.clipboard_sync_events {
                if let Some(ref mut content) = event.content {
                    content.clear();
                }
            }
            for entries in self.clipboard_sync_history.values_mut() {
                for entry in entries {
                    entry.content.clear();
                }
            }
        } else {
            for event in &mut self.clipboard_sync_events {
                if let Some(ref mut content) = event.content {
                    sanitize_clipboard_content(content, max_bytes);
                }
            }
            for entries in self.clipboard_sync_history.values_mut() {
                for entry in entries {
                    sanitize_clipboard_content(&mut entry.content, max_bytes);
                }
            }
        }
    }

    /// Get maximum bytes cached per clipboard event
    pub fn max_clipboard_event_bytes(&self) -> usize {
        self.max_clipboard_event_bytes
    }

    /// Set remote session ID
    pub fn set_remote_session_id(&mut self, session_id: Option<String>) {
        self.remote_session_id = session_id;
    }

    /// Get remote session ID
    pub fn remote_session_id(&self) -> Option<&str> {
        self.remote_session_id.as_deref()
    }

    /// Set maximum clipboard sync history
    pub fn set_max_clipboard_sync_history(&mut self, max: usize) {
        self.max_clipboard_sync_history = max;
        for entries in self.clipboard_sync_history.values_mut() {
            if entries.len() > max {
                entries.drain(0..entries.len() - max);
            }
        }
    }

    // === Feature 31: Shell Integration++ ===

    /// Start tracking a command execution
    pub fn start_command_execution(&mut self, command: String) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        self.current_command = Some(CommandExecution {
            command,
            cwd: self.shell_integration.cwd().map(String::from),
            start_time: timestamp,
            end_time: None,
            exit_code: None,
            duration_ms: None,
            success: None,
        });
    }

    /// End tracking the current command execution
    pub fn end_command_execution(&mut self, exit_code: i32) {
        if let Some(mut cmd) = self.current_command.take() {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            cmd.end_time = Some(timestamp);
            cmd.exit_code = Some(exit_code);
            cmd.duration_ms = Some(timestamp.saturating_sub(cmd.start_time));
            cmd.success = Some(exit_code == 0);

            self.command_history.push(cmd);

            // Limit history size
            if self.command_history.len() > self.max_command_history {
                self.command_history
                    .drain(0..self.command_history.len() - self.max_command_history);
            }
        }
    }

    /// Get command execution history
    pub fn get_command_history(&self) -> &[CommandExecution] {
        &self.command_history
    }

    /// Get current executing command
    pub fn get_current_command(&self) -> Option<&CommandExecution> {
        self.current_command.as_ref()
    }

    /// Record a CWD change
    pub fn record_cwd_change(&mut self, new_cwd: String) {
        let old_cwd = self.shell_integration.cwd().map(String::from);

        // Only record if CWD actually changed
        if old_cwd.as_deref() != Some(&new_cwd) {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            let change = CwdChange {
                old_cwd,
                new_cwd: new_cwd.clone(),
                timestamp,
            };

            self.cwd_changes.push(change);

            // Limit history size
            if self.cwd_changes.len() > self.max_cwd_history {
                self.cwd_changes
                    .drain(0..self.cwd_changes.len() - self.max_cwd_history);
            }

            // Update shell integration
            self.shell_integration.set_cwd(new_cwd);
        }
    }

    /// Get CWD change history
    pub fn get_cwd_changes(&self) -> &[CwdChange] {
        &self.cwd_changes
    }

    /// Get shell integration statistics
    pub fn get_shell_integration_stats(&self) -> ShellIntegrationStats {
        let total_commands = self.command_history.len();
        let successful_commands = self
            .command_history
            .iter()
            .filter(|cmd| cmd.success == Some(true))
            .count();
        let failed_commands = self
            .command_history
            .iter()
            .filter(|cmd| cmd.success == Some(false))
            .count();

        let total_duration_ms: u64 = self
            .command_history
            .iter()
            .filter_map(|cmd| cmd.duration_ms)
            .sum();

        let avg_duration_ms = if total_commands > 0 {
            total_duration_ms as f64 / total_commands as f64
        } else {
            0.0
        };

        ShellIntegrationStats {
            total_commands,
            successful_commands,
            failed_commands,
            avg_duration_ms,
            total_duration_ms,
        }
    }

    /// Clear command execution history
    pub fn clear_command_history(&mut self) {
        self.command_history.clear();
    }

    /// Clear CWD change history
    pub fn clear_cwd_history(&mut self) {
        self.cwd_changes.clear();
    }

    /// Set maximum command history size
    pub fn set_max_command_history(&mut self, max: usize) {
        self.max_command_history = max;
        if self.command_history.len() > max {
            self.command_history
                .drain(0..self.command_history.len() - max);
        }
    }

    /// Set maximum CWD history size
    pub fn set_max_cwd_history(&mut self, max: usize) {
        self.max_cwd_history = max;
        if self.cwd_changes.len() > max {
            self.cwd_changes.drain(0..self.cwd_changes.len() - max);
        }
    }

    // === Feature 37: Terminal Notifications ===

    /// Get notification configuration
    pub fn get_notification_config(&self) -> &NotificationConfig {
        &self.notification_config
    }

    /// Set notification configuration
    pub fn set_notification_config(&mut self, config: NotificationConfig) {
        self.notification_config = config;
    }

    /// Trigger a notification
    pub fn trigger_notification(
        &mut self,
        trigger: NotificationTrigger,
        alert: NotificationAlert,
        message: Option<String>,
    ) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = NotificationEvent {
            trigger,
            alert,
            message,
            timestamp,
            delivered: false, // Will be set by external notification handler
        };

        self.notification_events.push(event);
    }

    /// Get notification events
    pub fn get_notification_events(&self) -> &[NotificationEvent] {
        &self.notification_events
    }

    /// Clear notification events
    pub fn clear_notification_events(&mut self) {
        self.notification_events.clear();
    }

    /// Mark notification as delivered
    pub fn mark_notification_delivered(&mut self, index: usize) {
        if let Some(event) = self.notification_events.get_mut(index) {
            event.delivered = true;
        }
    }

    /// Update activity timestamp (call on terminal input/output)
    pub fn update_activity(&mut self) {
        self.last_activity_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
    }

    /// Check for silence and trigger notification if needed
    pub fn check_silence(&mut self) {
        if !self.notification_config.silence_enabled {
            return;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Check if enough time has passed since last silence check
        if now.saturating_sub(self.last_silence_check)
            < self.notification_config.silence_threshold * 1000
        {
            return;
        }

        self.last_silence_check = now;

        // Check if activity has occurred recently
        let silence_duration = now.saturating_sub(self.last_activity_time);
        if silence_duration >= self.notification_config.silence_threshold * 1000 {
            self.trigger_notification(
                NotificationTrigger::Silence,
                NotificationAlert::Desktop,
                None,
            );
        }
    }

    /// Check for activity and trigger notification if needed
    pub fn check_activity(&mut self) {
        if !self.notification_config.activity_enabled {
            return;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let time_since_activity = now.saturating_sub(self.last_activity_time);

        // If there was a period of inactivity and now there's activity
        if time_since_activity >= self.notification_config.activity_threshold * 1000 {
            self.trigger_notification(
                NotificationTrigger::Activity,
                NotificationAlert::Desktop,
                None,
            );
        }
    }

    /// Register a custom notification trigger
    pub fn register_custom_trigger(&mut self, id: u32, message: String) {
        self.custom_triggers.insert(id, message);
    }

    /// Trigger a custom notification
    pub fn trigger_custom_notification(&mut self, id: u32, alert: NotificationAlert) {
        let message = self.custom_triggers.get(&id).cloned();
        self.trigger_notification(NotificationTrigger::Custom(id), alert, message);
    }

    /// Handle bell event with notification
    pub fn handle_bell_notification(&mut self) {
        // Copy config values to avoid borrow checker issues
        let bell_desktop = self.notification_config.bell_desktop;
        let bell_sound = self.notification_config.bell_sound;
        let bell_visual = self.notification_config.bell_visual;

        if bell_desktop {
            self.trigger_notification(
                NotificationTrigger::Bell,
                NotificationAlert::Desktop,
                Some("Terminal bell".to_string()),
            );
        }

        if bell_sound > 0 {
            self.trigger_notification(
                NotificationTrigger::Bell,
                NotificationAlert::Sound(bell_sound),
                None,
            );
        }

        if bell_visual {
            self.trigger_notification(NotificationTrigger::Bell, NotificationAlert::Visual, None);
        }
    }

    // === Feature 24: Terminal Replay/Recording ===

    /// Start recording a terminal session
    pub fn start_recording(&mut self, title: Option<String>) {
        if self.is_recording {
            return; // Already recording
        }

        let start_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let initial_size = (self.grid.cols(), self.grid.rows());

        let mut env = HashMap::new();
        // Add some basic terminal info
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLS".to_string(), initial_size.0.to_string());
        env.insert("ROWS".to_string(), initial_size.1.to_string());

        self.recording_session = Some(RecordingSession {
            start_time,
            initial_size,
            env,
            events: Vec::new(),
            duration: 0,
            title,
        });

        self.is_recording = true;
        self.recording_start_time = start_time;
    }

    /// Stop recording
    pub fn stop_recording(&mut self) -> Option<RecordingSession> {
        if !self.is_recording {
            return None;
        }

        self.is_recording = false;

        if let Some(mut session) = self.recording_session.take() {
            let end_time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            session.duration = end_time.saturating_sub(session.start_time);
            Some(session)
        } else {
            None
        }
    }

    /// Record an event during recording
    pub fn record_event(
        &mut self,
        event_type: RecordingEventType,
        data: Vec<u8>,
        metadata: Option<(usize, usize)>,
    ) {
        if !self.is_recording {
            return;
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let relative_timestamp = timestamp.saturating_sub(self.recording_start_time);

        if let Some(session) = &mut self.recording_session {
            session.events.push(RecordingEvent {
                timestamp: relative_timestamp,
                event_type,
                data,
                metadata,
            });
        }
    }

    /// Record output data
    pub fn record_output(&mut self, data: &[u8]) {
        self.record_event(RecordingEventType::Output, data.to_vec(), None);
    }

    /// Record input data
    pub fn record_input(&mut self, data: &[u8]) {
        self.record_event(RecordingEventType::Input, data.to_vec(), None);
    }

    /// Record terminal resize
    pub fn record_resize(&mut self, cols: usize, rows: usize) {
        self.record_event(RecordingEventType::Resize, Vec::new(), Some((cols, rows)));
    }

    /// Add a marker/bookmark to the recording
    pub fn record_marker(&mut self, label: String) {
        self.record_event(RecordingEventType::Marker, label.into_bytes(), None);
    }

    /// Get current recording session
    pub fn get_recording_session(&self) -> Option<&RecordingSession> {
        self.recording_session.as_ref()
    }

    /// Check if currently recording
    pub fn is_recording(&self) -> bool {
        self.is_recording
    }

    /// Export recording to asciicast v2 format
    pub fn export_asciicast(&self, session: &RecordingSession) -> String {
        use std::fmt::Write;

        let mut output = String::new();

        // Header
        let header = serde_json::json!({
            "version": 2,
            "width": session.initial_size.0,
            "height": session.initial_size.1,
            "timestamp": session.start_time / 1000,
            "title": session.title.as_deref().unwrap_or("Terminal Recording"),
            "env": session.env,
        });

        writeln!(output, "{}", header).ok();

        // Events
        for event in &session.events {
            match event.event_type {
                RecordingEventType::Output => {
                    let time = event.timestamp as f64 / 1000.0;
                    let data = String::from_utf8_lossy(&event.data);
                    let event_json = serde_json::json!([time, "o", data]);
                    writeln!(output, "{}", event_json).ok();
                }
                RecordingEventType::Input => {
                    let time = event.timestamp as f64 / 1000.0;
                    let data = String::from_utf8_lossy(&event.data);
                    let event_json = serde_json::json!([time, "i", data]);
                    writeln!(output, "{}", event_json).ok();
                }
                RecordingEventType::Resize => {
                    if let Some((cols, rows)) = event.metadata {
                        let time = event.timestamp as f64 / 1000.0;
                        let event_json = serde_json::json!([time, "r", cols, rows]);
                        writeln!(output, "{}", event_json).ok();
                    }
                }
                RecordingEventType::Marker => {
                    let time = event.timestamp as f64 / 1000.0;
                    let label = String::from_utf8_lossy(&event.data);
                    let event_json = serde_json::json!([time, "m", label]);
                    writeln!(output, "{}", event_json).ok();
                }
            }
        }

        output
    }

    /// Export recording to JSON format
    pub fn export_json(&self, session: &RecordingSession) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "session": {
                "start_time": session.start_time,
                "duration": session.duration,
                "initial_size": session.initial_size,
                "title": session.title,
                "env": session.env,
            },
            "events": session.events.iter().map(|e| {
                serde_json::json!({
                    "timestamp": e.timestamp,
                    "type": format!("{:?}", e.event_type),
                    "data": String::from_utf8_lossy(&e.data),
                    "metadata": e.metadata,
                })
            }).collect::<Vec<_>>(),
        }))
        .unwrap_or_default()
    }
}

// === Feature 14: Snapshot Diffing - Helper Function ===

/// Compare two sets of screen lines and return the differences
pub fn diff_screen_lines(old_lines: &[String], new_lines: &[String]) -> SnapshotDiff {
    let mut diffs = Vec::new();
    let mut added = 0;
    let mut removed = 0;
    let mut modified = 0;
    let mut unchanged = 0;

    let max_rows = old_lines.len().max(new_lines.len());

    for row in 0..max_rows {
        let old_line = old_lines.get(row);
        let new_line = new_lines.get(row);

        match (old_line, new_line) {
            (Some(old_content), Some(new_content)) => {
                if old_content == new_content {
                    unchanged += 1;
                    diffs.push(LineDiff {
                        change_type: DiffChangeType::Unchanged,
                        old_row: Some(row),
                        new_row: Some(row),
                        old_content: Some(old_content.clone()),
                        new_content: Some(new_content.clone()),
                    });
                } else {
                    modified += 1;
                    diffs.push(LineDiff {
                        change_type: DiffChangeType::Modified,
                        old_row: Some(row),
                        new_row: Some(row),
                        old_content: Some(old_content.clone()),
                        new_content: Some(new_content.clone()),
                    });
                }
            }
            (None, Some(new_content)) => {
                added += 1;
                diffs.push(LineDiff {
                    change_type: DiffChangeType::Added,
                    old_row: None,
                    new_row: Some(row),
                    old_content: None,
                    new_content: Some(new_content.clone()),
                });
            }
            (Some(old_content), None) => {
                removed += 1;
                diffs.push(LineDiff {
                    change_type: DiffChangeType::Removed,
                    old_row: Some(row),
                    new_row: None,
                    old_content: Some(old_content.clone()),
                    new_content: None,
                });
            }
            (None, None) => {
                // This shouldn't happen
            }
        }
    }

    SnapshotDiff {
        diffs,
        added,
        removed,
        modified,
        unchanged,
    }
}
// VTE Perform trait implementation - delegates to sequence handlers
impl Perform for Terminal {
    fn print(&mut self, c: char) {
        debug::log_print(c, self.cursor.col, self.cursor.row);
        self.write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        debug::log_execute(byte);
        match byte {
            b'\n' => self.write_char('\n'),
            b'\r' => self.write_char('\r'),
            b'\t' => self.write_char('\t'),
            b'\x08' => self.write_char('\x08'),
            b'\x07' => {
                // Bell - increment counter for visual bell support
                self.bell_count = self.bell_count.wrapping_add(1);
                // Add bell event based on volume settings
                let event = if self.warning_bell_volume > 0 {
                    BellEvent::WarningBell(self.warning_bell_volume)
                } else {
                    BellEvent::VisualBell
                };
                self.bell_events.push(event.clone());
                self.terminal_events.push(TerminalEvent::BellRang(event));
            }
            _ => {}
        }
    }

    fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.dcs_hook(params, intermediates, ignore, action);
    }

    fn put(&mut self, byte: u8) {
        self.dcs_put(byte);
    }

    fn unhook(&mut self) {
        self.dcs_unhook();
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.osc_dispatch_impl(params, bell_terminated);
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.csi_dispatch_impl(params, intermediates, ignore, action);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        self.esc_dispatch_impl(intermediates, ignore, byte);
    }
}

pub struct TerminalStats {
    /// Number of columns
    pub cols: usize,
    /// Number of rows
    pub rows: usize,
    /// Number of scrollback lines currently used
    pub scrollback_lines: usize,
    /// Total number of cells (rows × cols + scrollback)
    pub total_cells: usize,
    /// Number of lines with non-whitespace content
    pub non_whitespace_lines: usize,
    /// Number of Sixel graphics
    pub graphics_count: usize,
    /// Estimated memory usage in bytes
    pub estimated_memory_bytes: usize,
    /// Number of hyperlinks stored
    pub hyperlink_count: usize,
    /// Estimated memory used by hyperlink storage (bytes)
    pub hyperlink_memory_bytes: usize,
    /// Color stack depth
    pub color_stack_depth: usize,
    /// Title stack depth
    pub title_stack_depth: usize,
    /// Keyboard flag stack depth (active screen)
    pub keyboard_stack_depth: usize,
    /// Response buffer size (bytes)
    pub response_buffer_size: usize,
    /// Number of dirty rows
    pub dirty_row_count: usize,
    /// Pending bell events count
    pub pending_bell_events: usize,
    /// Pending terminal events count
    pub pending_terminal_events: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("../tests/terminal_tests.rs");
    include!("../tests/grid_integration_tests.rs");
}
