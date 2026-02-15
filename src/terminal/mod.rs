//! Terminal emulator implementation
//!
//! This module provides the main `Terminal` struct and its implementation,
//! split across multiple submodules for maintainability.

// Submodules
pub mod clipboard;
mod colors;
pub mod compliance;
pub mod event;
pub mod file_transfer;
mod graphics;
pub mod image;
pub mod macros;
pub mod metrics;
pub mod multiplexing;
pub mod notification;
pub mod progress;
pub mod recording;
pub mod replay;
pub mod screen;
pub mod search;
mod sequences;
pub mod shell_integration;
pub mod snapshot;
pub mod snapshot_manager;
pub mod terminal_snapshot;
pub mod trigger;
mod write;

// Re-export types as they're part of the public API
pub use clipboard::{
    ClipboardEntry, ClipboardHistoryEntry, ClipboardOperation, ClipboardSlot, ClipboardSyncEvent,
    ClipboardTarget,
};
pub use compliance::{ComplianceLevel, ComplianceReport, ComplianceTest};
pub use event::{BellEvent, CwdChange, ShellEvent, TerminalEvent, TerminalEventKind};
pub use file_transfer::{
    FileTransfer, FileTransferManager, TransferDirection, TransferId, TransferStatus,
};
pub(crate) use image::ITermMultipartState;
pub use image::{ImageFormat, ImagePlacement, ImageProtocol, InlineImage};
pub use metrics::{
    BenchmarkCategory, BenchmarkResult, BenchmarkSuite, EscapeSequenceProfile, FrameTiming,
    PerformanceMetrics, ProfileCategory, ProfilingData, TerminalStats,
};
pub use multiplexing::{LayoutDirection, PaneState, SessionState, WindowLayout};
pub use notification::{
    Notification, NotificationAlert, NotificationConfig, NotificationEvent, NotificationTrigger,
};
pub use progress::{
    NamedProgressBar, ProgressBar, ProgressBarAction, ProgressBarCommand, ProgressState,
};
pub use recording::{
    RecordingEvent, RecordingEventType, RecordingExportFormat, RecordingFormat, RecordingSession,
};
pub use screen::{
    hsl_to_rgb, hsv_to_rgb, rgb_to_hsl, rgb_to_hsv, AnimationHint, ColorHSL, ColorHSV,
    ColorPalette, DamageRegion, JoinedLines, ReflowStats, RenderingHint, Selection, SelectionMode,
    ThemeMode, UpdatePriority, ZLayer,
};
pub use search::{DetectedItem, HyperlinkInfo, RegexMatch, RegexSearchOptions, SearchMatch};
pub use shell_integration::{CommandExecution, CommandOutput, ShellIntegrationStats};
pub use snapshot::{
    diff_screen_lines, Bookmark, CommandInfo, CwdChangeInfo, DiffChangeType, ExportFormat,
    LineDiff, ScrollbackStats, SemanticSnapshot, SnapshotDiff, SnapshotScope, ZoneInfo,
};
pub use trigger::{
    ActionResult, Trigger, TriggerAction, TriggerHighlight, TriggerId, TriggerMatch,
    TriggerRegistry,
};

// Imports
use crate::cell::{Cell, CellFlags};
use crate::color::{Color, NamedColor};
use crate::cursor::{Cursor, CursorStyle};
use crate::debug;
use crate::graphics::{GraphicsLimits, GraphicsStore};
use crate::grid::Grid;
use crate::mouse::{MouseEncoding, MouseEvent, MouseEventRecord, MouseMode, MousePosition};
use crate::shell_integration::ShellIntegration;
use crate::sixel;
use std::collections::{HashMap, HashSet};

const DEFAULT_MAX_NOTIFICATIONS: usize = 128;
const DEFAULT_MAX_CLIPBOARD_SYNC_EVENTS: usize = 256;
const DEFAULT_MAX_CLIPBOARD_EVENT_BYTES: usize = 4096;
const CLIPBOARD_TRUNCATION_SUFFIX: &str = " [truncated]";
/// Hard upper limit for clipboard content (10 MB), regardless of configured max_bytes
const MAX_CLIPBOARD_CONTENT_SIZE: usize = 10_485_760;

#[inline]
pub fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn sanitize_clipboard_content(content: &mut String, max_bytes: usize) {
    if max_bytes == 0 {
        content.clear();
        return;
    }

    // Enforce hard upper limit regardless of configured max_bytes
    let effective_max = max_bytes.min(MAX_CLIPBOARD_CONTENT_SIZE);

    if content.len() > effective_max {
        let suffix_len = CLIPBOARD_TRUNCATION_SUFFIX.len();
        let keep = effective_max.saturating_sub(suffix_len);
        content.truncate(keep);
        if suffix_len <= effective_max {
            content.push_str(CLIPBOARD_TRUNCATION_SUFFIX);
        }
    }
}

/// Helper function to convert cells to text
pub fn cells_to_text(cells: &[Cell]) -> String {
    cells
        .iter()
        .map(|c| {
            if c.flags.wide_char_spacer() {
                " ".to_string()
            } else {
                c.get_grapheme()
            }
        })
        .collect::<Vec<String>>()
        .join("")
}

/// Helper function to escape HTML special characters
pub fn html_escape(s: &str) -> String {
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

/// Get current timestamp in microseconds
pub fn get_timestamp_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Helper function to check if byte slice contains a subsequence
/// More efficient than converting to String and using contains()
#[inline]
pub(crate) fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
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
    pub(crate) grid: Grid,
    /// Alternate screen grid
    pub(crate) alt_grid: Grid,
    /// Whether we're using the alternate screen
    pub(crate) alt_screen_active: bool,
    /// Cursor position and state
    pub(crate) cursor: Cursor,
    /// Saved cursor for alternate screen
    pub(crate) alt_cursor: Cursor,
    /// Current foreground color
    pub(crate) fg: Color,
    /// Current background color
    pub(crate) bg: Color,
    /// Current underline color (SGR 58) - None means use foreground color
    pub(crate) underline_color: Option<Color>,
    /// Current cell flags
    pub(crate) flags: CellFlags,
    /// Saved cursor position (for save/restore)
    pub(crate) saved_cursor: Option<Cursor>,
    /// Saved colors and flags
    pub(crate) saved_fg: Color,
    pub(crate) saved_bg: Color,
    pub(crate) saved_underline_color: Option<Color>,
    pub(crate) saved_flags: CellFlags,
    /// Terminal title
    pub(crate) title: String,
    /// Mouse tracking mode
    pub(crate) mouse_mode: MouseMode,
    /// Mouse encoding format
    pub(crate) mouse_encoding: MouseEncoding,
    /// Focus tracking enabled
    pub(crate) focus_tracking: bool,
    /// Bracketed paste mode
    pub(crate) bracketed_paste: bool,
    /// Synchronized update mode (DEC 2026)
    pub(crate) synchronized_updates: bool,
    /// Buffer for batched updates (when synchronized mode is active)
    pub(crate) update_buffer: Vec<u8>,
    /// Flag to track if synchronized updates were explicitly disabled during a flush
    pub(crate) sync_update_explicitly_disabled: bool,
    /// Shell integration state
    pub(crate) shell_integration: ShellIntegration,
    /// Scroll region top (0-indexed)
    pub(crate) scroll_region_top: usize,
    /// Scroll region bottom (0-indexed)
    pub(crate) scroll_region_bottom: usize,
    /// Use left/right column scroll region (DECLRMM)
    pub(crate) use_lr_margins: bool,
    /// Left column margin (0-indexed, inclusive)
    pub(crate) left_margin: usize,
    /// Right column margin (0-indexed, inclusive)
    pub(crate) right_margin: usize,
    /// Auto wrap mode (DECAWM)
    pub(crate) auto_wrap: bool,
    /// Origin mode (DECOM) - cursor addressing relative to scroll region
    pub(crate) origin_mode: bool,
    /// Tab stops (columns where tab stops are set)
    pub(crate) tab_stops: Vec<bool>,
    /// Application cursor keys mode
    pub(crate) application_cursor: bool,
    /// Kitty keyboard protocol flags (progressive enhancement)
    pub(crate) keyboard_flags: u16,
    /// Stack for keyboard protocol flags (main screen)
    pub(crate) keyboard_stack: Vec<u16>,
    /// Stack for keyboard protocol flags (alternate screen)
    pub(crate) keyboard_stack_alt: Vec<u16>,
    /// modifyOtherKeys mode (XTerm extension for enhanced keyboard input)
    /// 0 = disabled, 1 = report modifiers for special keys, 2 = report modifiers for all keys
    pub(crate) modify_other_keys_mode: u8,
    /// Response buffer for device queries (DA/DSR/etc)
    pub(crate) response_buffer: Vec<u8>,
    /// Hyperlink storage: ID -> URL mapping (for deduplication)
    pub(crate) hyperlinks: HashMap<u32, String>,
    /// Current hyperlink ID being written
    pub(crate) current_hyperlink_id: Option<u32>,
    /// Next available hyperlink ID
    pub(crate) next_hyperlink_id: u32,
    /// Unified graphics storage (Sixel, iTerm2, Kitty)
    pub(crate) graphics_store: GraphicsStore,
    /// Sixel resource limits (per-terminal, for decoding)
    pub(crate) sixel_limits: sixel::SixelLimits,
    /// Cell dimensions in pixels (width, height) for sixel graphics
    /// Default (1, 2) is for text-mode TUI with half-block rendering
    /// Pixel renderers should set actual cell dimensions
    pub(crate) cell_dimensions: (u32, u32),
    /// Current Sixel parser (active during DCS)
    pub(crate) sixel_parser: Option<sixel::SixelParser>,
    /// Buffer for DCS data accumulation
    pub(crate) dcs_buffer: Vec<u8>,
    /// DCS active flag
    pub(crate) dcs_active: bool,
    /// DCS action character ('q' for Sixel)
    pub(crate) dcs_action: Option<char>,
    /// iTerm2 multi-part image transfer state (MultipartFile/FilePart protocol)
    pub(crate) iterm_multipart_buffer: Option<ITermMultipartState>,
    /// File transfer manager for tracking file downloads and uploads
    pub(crate) file_transfer_manager: FileTransferManager,
    /// Clipboard content (OSC 52)
    pub(crate) clipboard_content: Option<String>,
    /// Allow clipboard read operations (security flag for OSC 52 queries)
    pub(crate) allow_clipboard_read: bool,
    /// Default foreground color (for OSC 10 queries)
    pub(crate) default_fg: Color,
    /// Default background color (for OSC 11 queries)
    pub(crate) default_bg: Color,
    /// Cursor color (for OSC 12 queries)
    pub(crate) cursor_color: Color,
    /// ANSI color palette (0-15) - modified by OSC 4/104
    pub(crate) ansi_palette: [Color; 16],
    /// Color stack for XTPUSHCOLORS/XTPOPCOLORS (fg, bg, underline)
    pub(crate) color_stack: Vec<(Color, Color, Option<Color>)>,
    /// Notifications from OSC 9 / OSC 777 sequences
    pub(crate) notifications: Vec<Notification>,
    /// Progress bar state from OSC 9;4 sequences (ConEmu/Windows Terminal style)
    pub(crate) progress_bar: ProgressBar,
    /// Named progress bars from OSC 934 sequences (keyed by ID)
    pub(crate) named_progress_bars: HashMap<String, NamedProgressBar>,
    /// Bell event counter - incremented each time bell (BEL/\x07) is received
    pub(crate) bell_count: u64,
    /// VTE parser instance (maintains state across process() calls)
    pub(crate) parser: vte::Parser,
    /// DECAWM delayed wrap: set after printing in last column
    pub(crate) pending_wrap: bool,
    /// Pixel width of the text area (XTWINOPS 14)
    pub(crate) pixel_width: usize,
    /// Pixel height of the text area (XTWINOPS 14)
    pub(crate) pixel_height: usize,
    /// Insert mode (IRM) - Mode 4: when enabled, new characters are inserted
    pub(crate) insert_mode: bool,
    /// Line Feed/New Line Mode (LNM) - Mode 20: when enabled, LF does CR+LF
    pub(crate) line_feed_new_line_mode: bool,
    /// Character protection mode (DECSCA) - when enabled, new chars are guarded
    pub(crate) char_protected: bool,
    /// Reverse video mode (DECSCNM) - globally inverts fg/bg colors
    pub(crate) reverse_video: bool,
    /// Bold brightening - when enabled, bold ANSI colors 0-7 brighten to 8-15
    pub(crate) bold_brightening: bool,
    /// Window title stack for XTWINOPS 22/23 (push/pop title)
    pub(crate) title_stack: Vec<String>,
    /// Accept OSC 7 directory tracking sequences
    pub(crate) accept_osc7: bool,
    /// Disable potentially insecure escape sequences
    pub(crate) disable_insecure_sequences: bool,
    /// Link/hyperlink color (iTerm2 default: blue #0645ad)
    pub(crate) link_color: Color,
    /// Bold text custom color (iTerm2 default: white #ffffff)
    pub(crate) bold_color: Color,
    /// Cursor guide color (iTerm2 default: light blue #a6e8ff with alpha)
    pub(crate) cursor_guide_color: Color,
    /// Badge color (iTerm2 default: red #ff0000 with alpha)
    pub(crate) badge_color: Color,
    /// Match/search highlight color (iTerm2 default: yellow #ffff00)
    pub(crate) match_color: Color,
    /// Selection background color (iTerm2 default: #b5d5ff)
    pub(crate) selection_bg_color: Color,
    /// Selection foreground/text color (iTerm2 default: #000000)
    pub(crate) selection_fg_color: Color,
    /// Use custom bold color instead of bright variant (iTerm2: "Use custom color for bold text")
    pub(crate) use_bold_color: bool,
    /// Use custom underline color (iTerm2: "Use custom underline color")
    pub(crate) use_underline_color: bool,
    /// Show cursor guide (iTerm2: "Use cursor guide")
    pub(crate) use_cursor_guide: bool,
    /// Use custom selected text color (iTerm2: "Use custom color for selected text")
    pub(crate) use_selected_text_color: bool,
    /// Smart cursor color - auto-adjust based on background (iTerm2: "Smart Cursor Color")
    pub(crate) smart_cursor_color: bool,
    /// Faint/dim text alpha multiplier (0.0-1.0, default 0.5)
    /// Applied to SGR 2 (dim) text during rendering
    pub(crate) faint_text_alpha: f32,
    /// Terminal conformance level (VT100/VT220/VT320/VT420/VT520)
    pub(crate) conformance_level: crate::conformance_level::ConformanceLevel,
    /// Warning bell volume (0=off, 1-8=volume levels) - VT520 DECSWBV
    pub(crate) warning_bell_volume: u8,
    /// Margin bell volume (0=off, 1-8=volume levels) - VT520 DECSMBV
    pub(crate) margin_bell_volume: u8,
    /// Tmux control protocol parser
    pub(crate) tmux_parser: crate::tmux_control::TmuxControlParser,
    /// Tmux control protocol notifications buffer
    pub(crate) tmux_notifications: Vec<crate::tmux_control::TmuxNotification>,
    /// Dirty rows tracking (0-indexed row numbers that have changed)
    pub(crate) dirty_rows: HashSet<usize>,
    /// Bell events buffer
    pub(crate) bell_events: Vec<BellEvent>,
    /// Terminal events buffer
    pub(crate) terminal_events: Vec<TerminalEvent>,
    /// Index of the next event to dispatch to observers (prevents duplicate dispatch)
    pub(crate) events_dispatched_up_to: usize,
    /// Registered observers for push-based event delivery
    pub(crate) observers: Vec<crate::observer::ObserverEntry>,
    /// Next observer ID to assign (monotonically increasing)
    pub(crate) next_observer_id: crate::observer::ObserverId,
    /// Next zone ID to assign (monotonically increasing)
    pub(crate) next_zone_id: usize,
    /// Last known hostname (for detecting remote host transitions)
    pub(crate) last_hostname: Option<String>,
    /// Last known username (for detecting remote host transitions)
    pub(crate) last_username: Option<String>,
    /// Current shell nesting depth (for sub-shell detection)
    pub(crate) shell_depth: usize,
    /// Whether we are currently inside command output (between OSC 133 C and D)
    pub(crate) in_command_output: bool,
    /// Current selection state
    pub(crate) selection: Option<Selection>,
    /// Bookmarks for quick navigation
    pub(crate) bookmarks: Vec<Bookmark>,
    /// Next available bookmark ID
    pub(crate) next_bookmark_id: usize,
    /// Performance metrics tracking
    pub(crate) perf_metrics: PerformanceMetrics,
    /// Frame timing history (last N frames)
    pub(crate) frame_timings: Vec<FrameTiming>,
    /// Maximum frame timings to keep
    pub(crate) max_frame_timings: usize,
    /// Clipboard history (multiple slots)
    pub(crate) clipboard_history: HashMap<ClipboardSlot, Vec<ClipboardEntry>>,
    /// Maximum clipboard history entries per slot
    pub(crate) max_clipboard_history: usize,
    /// Mouse event history
    pub(crate) mouse_events: Vec<MouseEventRecord>,
    /// Mouse position history
    pub(crate) mouse_positions: Vec<MousePosition>,
    /// Maximum mouse history entries
    pub(crate) max_mouse_history: usize,
    /// Current rendering hints
    pub(crate) rendering_hints: Vec<RenderingHint>,
    /// Damage regions accumulated
    pub(crate) damage_regions: Vec<DamageRegion>,
    /// Profiling data (when enabled)
    pub(crate) profiling_data: Option<ProfilingData>,
    /// Profiling enabled flag
    pub(crate) profiling_enabled: bool,
    /// Regex search matches cache
    pub(crate) regex_matches: Vec<RegexMatch>,
    /// Current regex search pattern
    pub(crate) current_regex_pattern: Option<String>,
    /// Current pane state (for multiplexing)
    pub(crate) pane_state: Option<PaneState>,
    /// Inline images (iTerm2, Kitty protocols)
    pub(crate) inline_images: Vec<InlineImage>,
    /// Maximum number of inline images to store
    pub(crate) max_inline_images: usize,

    // === Feature 30: OSC 52 Clipboard Sync ===
    /// Clipboard sync events log
    pub(crate) clipboard_sync_events: Vec<ClipboardSyncEvent>,
    /// Clipboard sync history across targets
    pub(crate) clipboard_sync_history: HashMap<ClipboardTarget, Vec<ClipboardHistoryEntry>>,
    /// Maximum clipboard sync history entries per target
    pub(crate) max_clipboard_sync_history: usize,
    /// Maximum clipboard sync events retained for diagnostics
    pub(crate) max_clipboard_sync_events: usize,
    /// Maximum bytes of clipboard content to persist per event/history entry
    pub(crate) max_clipboard_event_bytes: usize,
    /// Remote session identifier for clipboard sync
    pub(crate) remote_session_id: Option<String>,

    // === Feature 31: Shell Integration++ ===
    /// Command execution history
    pub(crate) command_history: Vec<CommandExecution>,
    /// Current executing command
    pub(crate) current_command: Option<CommandExecution>,
    /// Working directory change history
    pub(crate) cwd_changes: Vec<CwdChange>,
    /// Maximum command history entries
    pub(crate) max_command_history: usize,
    /// Maximum CWD change history
    pub(crate) max_cwd_history: usize,

    // === Feature 37: Terminal Notifications ===
    /// Notification configuration
    pub(crate) notification_config: NotificationConfig,
    /// Notification events log
    pub(crate) notification_events: Vec<NotificationEvent>,
    /// Last activity timestamp (for silence detection)
    pub(crate) last_activity_time: u64,
    /// Last silence check timestamp
    pub(crate) last_silence_check: u64,
    /// Maximum OSC 9/777 notifications retained
    pub(crate) max_notifications: usize,
    /// Custom notification triggers (ID -> message)
    pub(crate) custom_triggers: HashMap<u32, String>,

    // === Feature 24: Terminal Replay/Recording ===
    /// Current recording session
    pub(crate) recording_session: Option<RecordingSession>,
    /// Recording active flag
    pub(crate) is_recording: bool,
    /// Recording start timestamp (for relative timing)
    pub(crate) recording_start_time: u64,

    // === Feature 38: Macro Recording and Playback ===
    /// Macro library (name -> macro)
    pub(crate) macro_library: HashMap<String, crate::macros::Macro>,
    /// Current macro playback state
    pub(crate) macro_playback: Option<crate::macros::MacroPlayback>,
    /// Screenshot triggers from macro playback
    pub(crate) macro_screenshot_triggers: Vec<String>,

    // === Answerback String (ENQ response) ===
    /// Answerback string sent in response to ENQ (0x05) control character
    /// Default: empty (no response) for security
    /// Common values: "par-term", "vt100", or custom identification
    pub(crate) answerback_string: Option<String>,

    /// Unicode width configuration for character width calculations
    pub(crate) width_config: crate::unicode_width_config::WidthConfig,

    /// Unicode normalization form for text stored in cells
    pub(crate) normalization_form: crate::unicode_normalization_config::NormalizationForm,

    // === Badge Support (OSC 1337 SetBadgeFormat) ===
    /// Badge format string (from OSC 1337 SetBadgeFormat)
    /// Contains template with \(variable) placeholders
    pub(crate) badge_format: Option<String>,
    /// Session variables for badge format evaluation
    pub(crate) session_variables: crate::badge::SessionVariables,
    /// Optional event subscription filter
    pub(crate) event_subscription: Option<HashSet<TerminalEventKind>>,

    // === Feature 18: Triggers & Automation ===
    /// Trigger registry for pattern matching on terminal output
    pub(crate) trigger_registry: trigger::TriggerRegistry,
    /// Active trigger highlight overlays
    pub(crate) trigger_highlights: Vec<trigger::TriggerHighlight>,
    /// Pending action results for frontend consumption
    pub(crate) trigger_action_results: Vec<trigger::ActionResult>,
    /// Maximum action results to retain
    pub(crate) max_action_results: usize,
    /// Rows pending trigger scan (populated from dirty_rows when triggers exist)
    pub(crate) pending_trigger_rows: HashSet<usize>,
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
            sync_update_explicitly_disabled: false,
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
            modify_other_keys_mode: 0,
            response_buffer: Vec::new(),
            hyperlinks: HashMap::new(),
            current_hyperlink_id: None,
            next_hyperlink_id: 0,
            graphics_store: GraphicsStore::with_limits(GraphicsLimits::default()),
            sixel_limits: sixel::SixelLimits::default(),
            cell_dimensions: (1, 2), // Default for TUI half-block rendering
            sixel_parser: None,
            dcs_buffer: Vec::new(),
            dcs_active: false,
            dcs_action: None,
            iterm_multipart_buffer: None,
            file_transfer_manager: FileTransferManager::default(),
            clipboard_content: None,
            allow_clipboard_read: false,
            default_fg: Color::Named(NamedColor::White),
            default_bg: Color::Named(NamedColor::Black),
            cursor_color: Color::Named(NamedColor::White),
            ansi_palette: Self::default_ansi_palette(),
            color_stack: Vec::new(),
            notifications: Vec::new(),
            progress_bar: ProgressBar::default(),
            named_progress_bars: HashMap::new(),
            bell_count: 0,
            parser: vte::Parser::new(),
            pending_wrap: false,
            // Initialize pixel dimensions with reasonable defaults (10x20 per cell)
            // This ensures CSI 14 t queries return valid pixel dimensions after resize
            pixel_width: cols * 10,
            pixel_height: rows * 20,
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
            faint_text_alpha: 0.5, // 50% dimming for SGR 2 (faint/dim) text
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
            events_dispatched_up_to: 0,
            observers: Vec::new(),
            next_observer_id: 1,
            next_zone_id: 0,
            last_hostname: None,
            last_username: None,
            shell_depth: 0,
            in_command_output: false,
            // Selection and bookmarks
            selection: None,
            bookmarks: Vec::new(),
            next_bookmark_id: 0,
            // Performance metrics
            perf_metrics: PerformanceMetrics::default(),
            frame_timings: Vec::new(),
            max_frame_timings: 100, // Keep last 100 frames
            // Clipboard integration
            clipboard_history: HashMap::new(),
            max_clipboard_history: 10,
            // Mouse tracking
            mouse_events: Vec::new(),
            mouse_positions: Vec::new(),
            max_mouse_history: 100,
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
            max_inline_images: 100,
            // OSC 52 Clipboard Sync
            clipboard_sync_events: Vec::new(),
            clipboard_sync_history: HashMap::new(),
            max_clipboard_sync_history: 50,
            max_clipboard_sync_events: DEFAULT_MAX_CLIPBOARD_SYNC_EVENTS,
            max_clipboard_event_bytes: DEFAULT_MAX_CLIPBOARD_EVENT_BYTES,
            remote_session_id: None,
            // Shell Integration++
            command_history: Vec::new(),
            current_command: None,
            cwd_changes: Vec::new(),
            max_command_history: 100,
            max_cwd_history: 50,
            // Notifications
            notification_config: NotificationConfig::default(),
            notification_events: Vec::new(),
            last_activity_time: now,
            last_silence_check: now,
            max_notifications: DEFAULT_MAX_NOTIFICATIONS,
            custom_triggers: HashMap::new(),
            // Replay/Recording
            recording_session: None,
            is_recording: false,
            recording_start_time: 0,
            // Macros
            macro_library: HashMap::new(),
            macro_playback: None,
            macro_screenshot_triggers: Vec::new(),
            // Answerback
            answerback_string: None,
            // Unicode
            width_config: crate::unicode_width_config::WidthConfig::default(),
            normalization_form: crate::unicode_normalization_config::NormalizationForm::default(),
            // Badge
            badge_format: None,
            session_variables: crate::badge::SessionVariables::with_dimensions(
                cols as u16,
                rows as u16,
            ),
            event_subscription: None,
            // Triggers
            trigger_registry: TriggerRegistry::default(),
            trigger_highlights: Vec::new(),
            trigger_action_results: Vec::new(),
            max_action_results: 100,
            pending_trigger_rows: HashSet::new(),
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

    /// Get the scrollback buffer content as text
    pub fn scrollback(&self) -> Vec<String> {
        let scrollback_len = self.grid.scrollback_len();
        let mut lines = Vec::with_capacity(scrollback_len);
        for i in 0..scrollback_len {
            if let Some(line) = self.grid.scrollback_line(i) {
                lines.push(cells_to_text(line));
            }
        }
        lines
    }

    /// Get the cursor
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Set cursor style programmatically (bypasses DECSCUSR parsing)
    /// Use this when the terminal emulator's UI settings change the cursor style,
    /// rather than sending DECSCUSR escape sequences to the PTY.
    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor.set_style(style);
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

        let old_cols = self.grid.cols().max(1);
        let old_rows = self.grid.rows().max(1);

        self.grid.resize(cols, rows);
        self.alt_grid.resize(cols, rows);

        // Update pixel dimensions proportionally (10x20 per cell if not explicitly set)
        // This ensures CSI 14 t queries return valid pixel dimensions after resize
        if self.pixel_width == 0 || self.pixel_height == 0 {
            self.pixel_width = cols * 10;
            self.pixel_height = rows * 20;
        } else {
            // Maintain aspect ratio if pixel dimensions were explicitly set
            self.pixel_width = (self.pixel_width * cols) / old_cols;
            self.pixel_height = (self.pixel_height * rows) / old_rows;
        }

        // Update session variables for badges
        self.session_variables
            .set_dimensions(cols as u16, rows as u16);

        debug::log(
            debug::DebugLevel::Trace,
            "TERMINAL_RESIZE",
            &format!(
                "Applied resize: primary={}x{}, alt={}x{}, pixels={}x{}",
                self.grid.cols(),
                self.grid.rows(),
                self.alt_grid.cols(),
                self.alt_grid.rows(),
                self.pixel_width,
                self.pixel_height
            ),
        );

        // Update tab stops (guard against zero-width terminal)
        if cols > 0 {
            self.tab_stops.resize(cols, false);
            for i in (0..cols).step_by(8) {
                self.tab_stops[i] = true;
            }
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

        // Update session variables for badge evaluation
        self.session_variables
            .set_dimensions(cols as u16, rows as u16);

        self.record_resize(cols, rows);
    }

    /// Get the title
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Set the title
    pub fn set_title(&mut self, title: String) {
        // Also update session variables for badge evaluation
        self.session_variables.title = Some(title.clone());
        self.title = title;
    }

    // === Badge Format Support ===

    /// Get the current badge format template
    ///
    /// Returns the badge format string if one has been set via OSC 1337 SetBadgeFormat.
    /// The format may contain `\(variable)` placeholders for session variables.
    pub fn badge_format(&self) -> Option<&str> {
        self.badge_format.as_deref()
    }

    /// Set the badge format template
    ///
    /// This method is typically called when processing OSC 1337 SetBadgeFormat sequences.
    /// The format string should contain `\(variable)` placeholders.
    pub fn set_badge_format(&mut self, format: Option<String>) {
        self.badge_format = format;
    }

    /// Clear the badge format
    pub fn clear_badge_format(&mut self) {
        self.badge_format = None;
    }

    /// Get a reference to the session variables
    ///
    /// Session variables are used for badge format evaluation.
    pub fn session_variables(&self) -> &crate::badge::SessionVariables {
        &self.session_variables
    }

    /// Get a mutable reference to the session variables
    ///
    /// Use this to update session variables that will be used in badge evaluation.
    pub fn session_variables_mut(&mut self) -> &mut crate::badge::SessionVariables {
        &mut self.session_variables
    }

    /// Evaluate the current badge format with session variables
    ///
    /// Returns the evaluated badge string with all variables substituted,
    /// or None if no badge format is set.
    ///
    /// # Example
    /// ```ignore
    /// terminal.set_badge_format(Some(r"\(username)@\(hostname)".to_string()));
    /// terminal.session_variables_mut().set_username("alice");
    /// terminal.session_variables_mut().set_hostname("server1");
    /// assert_eq!(terminal.evaluate_badge(), Some("alice@server1".to_string()));
    /// ```
    pub fn evaluate_badge(&self) -> Option<String> {
        self.badge_format
            .as_ref()
            .map(|format| crate::badge::evaluate_badge_format(format, &self.session_variables))
    }

    /// Get a user variable by name
    ///
    /// Returns the value of a user variable set via OSC 1337 SetUserVar,
    /// or None if the variable is not set.
    pub fn get_user_var(&self, name: &str) -> Option<&str> {
        self.session_variables.custom.get(name).map(|s| s.as_str())
    }

    /// Get all user variables as a reference to the HashMap
    pub fn get_user_vars(&self) -> &HashMap<String, String> {
        &self.session_variables.custom
    }

    /// Set a user variable, emitting a UserVarChanged event if the value changed
    pub fn set_user_var(&mut self, name: String, value: String) {
        let old_value = self.session_variables.custom.get(&name).cloned();
        let changed = old_value.as_deref() != Some(&value);
        self.session_variables
            .custom
            .insert(name.clone(), value.clone());
        if changed {
            self.terminal_events.push(TerminalEvent::UserVarChanged {
                name,
                value,
                old_value,
            });
        }
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
            // Notify about alt screen entry
            self.terminal_events
                .push(crate::terminal::TerminalEvent::ModeChanged(
                    "alternate_screen".to_string(),
                    true,
                ));
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
            // Reset keyboard protocol flags when exiting alternate screen
            // TUI apps may enable Kitty keyboard protocol and fail to disable it on exit
            if self.keyboard_flags != 0 {
                self.keyboard_flags = 0;
                self.terminal_events
                    .push(crate::terminal::TerminalEvent::ModeChanged(
                        "keyboard_protocol".to_string(),
                        false,
                    ));
            }
            self.keyboard_stack_alt.clear();
            // Also reset modifyOtherKeys mode
            if self.modify_other_keys_mode != 0 {
                self.modify_other_keys_mode = 0;
                self.terminal_events
                    .push(crate::terminal::TerminalEvent::ModeChanged(
                        "modify_other_keys".to_string(),
                        false,
                    ));
            }
            // And focus tracking
            if self.focus_tracking {
                self.focus_tracking = false;
                self.terminal_events
                    .push(crate::terminal::TerminalEvent::ModeChanged(
                        "focus_tracking".to_string(),
                        false,
                    ));
            }
            // Notify about alt screen exit
            self.terminal_events
                .push(crate::terminal::TerminalEvent::ModeChanged(
                    "alternate_screen".to_string(),
                    false,
                ));
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

    /// Save current cursor state
    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some(self.cursor);
        self.saved_fg = self.fg;
        self.saved_bg = self.bg;
        self.saved_underline_color = self.underline_color;
        self.saved_flags = self.flags;
    }

    /// Restore previously saved cursor state
    pub fn restore_cursor(&mut self) {
        if let Some(saved) = self.saved_cursor {
            self.cursor = saved;
            self.fg = self.saved_fg;
            self.bg = self.saved_bg;
            self.underline_color = self.saved_underline_color;
            self.flags = self.saved_flags;
        }
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

    /// Get auto-wrap mode state
    pub fn auto_wrap_mode(&self) -> bool {
        self.auto_wrap
    }

    /// Get origin mode state
    pub fn origin_mode(&self) -> bool {
        self.origin_mode
    }

    /// Get application cursor mode state
    pub fn application_cursor(&self) -> bool {
        self.application_cursor
    }

    /// Get current scroll region (top, bottom)
    pub fn scroll_region(&self) -> (usize, usize) {
        (self.scroll_region_top, self.scroll_region_bottom)
    }

    /// Get left and right margins
    pub fn left_right_margins(&self) -> (usize, usize) {
        (self.left_margin, self.right_margin)
    }

    /// Get ANSI color by index
    pub fn get_ansi_color(&self, index: usize) -> Option<Color> {
        self.ansi_palette.get(index).cloned()
    }

    /// Get shell integration state
    pub fn shell_integration(&self) -> &ShellIntegration {
        &self.shell_integration
    }

    /// Get shell integration state mutably
    pub fn shell_integration_mut(&mut self) -> &mut ShellIntegration {
        &mut self.shell_integration
    }

    // ========== Semantic Zone Methods ==========

    /// Get all semantic zones from the primary grid
    pub fn get_zones(&self) -> &[crate::zone::Zone] {
        self.grid.zones()
    }

    /// Get the zone containing the given absolute row
    pub fn get_zone_at(&self, abs_row: usize) -> Option<&crate::zone::Zone> {
        self.grid.zone_at(abs_row)
    }

    /// Extract the text content of the zone containing the given absolute row.
    /// Returns None if no zone contains this row.
    pub fn get_zone_text(&self, abs_row: usize) -> Option<String> {
        let zone = self.grid.zone_at(abs_row)?;
        self.extract_text_from_row_range(zone.abs_row_start, zone.abs_row_end)
    }

    /// Extract text from an absolute row range (inclusive).
    /// Returns None if no rows could be found (e.g., evicted from scrollback).
    /// Handles wrapped lines by omitting newlines between them.
    fn extract_text_from_row_range(&self, abs_start: usize, abs_end: usize) -> Option<String> {
        let scrollback_len = self.grid.scrollback_len();

        // Check if the range is entirely evicted from the scrollback buffer
        let total_scrolled = self.grid.total_lines_scrolled();
        let max_sb = self.grid.max_scrollback();
        if total_scrolled > max_sb {
            let floor = total_scrolled - max_sb;
            if abs_end < floor {
                return None;
            }
        }

        let mut text = String::new();
        let mut found_any = false;

        for row in abs_start..=abs_end {
            if row < scrollback_len {
                // Row is in scrollback
                if let Some(line) = self.grid.scrollback_line(row) {
                    found_any = true;
                    let line_text: String = line
                        .iter()
                        .filter(|c| !c.flags.wide_char_spacer())
                        .map(|c| {
                            let mut s = String::new();
                            s.push(c.c);
                            for &combining in &c.combining {
                                s.push(combining);
                            }
                            s
                        })
                        .collect();
                    let trimmed = line_text.trim_end();
                    if !text.is_empty() {
                        // Check if previous line was wrapped
                        if row > abs_start && self.grid.is_scrollback_wrapped(row - 1) {
                            // Wrapped line - no newline
                        } else {
                            text.push('\n');
                        }
                    }
                    text.push_str(trimmed);
                }
            } else {
                // Row is in main grid
                let grid_row = row - scrollback_len;
                if let Some(line) = self.grid.row(grid_row) {
                    found_any = true;
                    let line_text: String = line
                        .iter()
                        .filter(|c| !c.flags.wide_char_spacer())
                        .map(|c| {
                            let mut s = String::new();
                            s.push(c.c);
                            for &combining in &c.combining {
                                s.push(combining);
                            }
                            s
                        })
                        .collect();
                    let trimmed = line_text.trim_end();
                    if !text.is_empty() && row > abs_start {
                        let prev_row = row - 1;
                        if prev_row < scrollback_len {
                            if !self.grid.is_scrollback_wrapped(prev_row) {
                                text.push('\n');
                            }
                        } else {
                            let prev_grid_row = prev_row - scrollback_len;
                            if !self.grid.is_line_wrapped(prev_grid_row) {
                                text.push('\n');
                            }
                        }
                    }
                    text.push_str(trimmed);
                }
            }
        }

        if found_any {
            Some(text)
        } else {
            None
        }
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
            self.sync_update_explicitly_disabled = false;
            self.synchronized_updates = false;
            self.process(&buffer);

            // Restore only if it was originally enabled and not explicitly disabled
            if saved_mode && !self.sync_update_explicitly_disabled && !self.synchronized_updates {
                self.synchronized_updates = true;
            }
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

    /// Get cell dimensions in pixels (width, height)
    ///
    /// Used for sixel graphics scroll calculations.
    /// Default is (1, 2) for TUI half-block rendering.
    pub fn cell_dimensions(&self) -> (u32, u32) {
        self.cell_dimensions
    }

    /// Set cell dimensions in pixels (width, height)
    ///
    /// Pixel-based renderers should call this with actual cell dimensions
    /// so sixel graphics scroll correctly. TUI renderers using half-blocks
    /// should use the default (1, 2).
    pub fn set_cell_dimensions(&mut self, width: u32, height: u32) {
        self.cell_dimensions = (width.max(1), height.max(1));
    }

    /// Get the maximum number of graphics retained for this terminal
    pub fn max_sixel_graphics(&self) -> usize {
        self.graphics_store.limits().max_graphics_count
    }

    /// Set the maximum number of graphics retained for this terminal.
    ///
    /// The value is clamped to a safe range and applies to graphics created
    /// after the change. If the new limit is lower than the current number of
    /// graphics, the oldest graphics are dropped to respect the limit.
    pub fn set_max_sixel_graphics(&mut self, max_graphics: usize) {
        use crate::sixel::SIXEL_HARD_MAX_GRAPHICS;

        let clamped = max_graphics.clamp(1, SIXEL_HARD_MAX_GRAPHICS);
        self.graphics_store.set_max_graphics(clamped);
    }

    /// Get the number of graphics dropped due to limits
    pub fn dropped_sixel_graphics(&self) -> usize {
        self.graphics_store.dropped_count()
    }

    /// Update all Kitty graphics animations and return list of image IDs that changed frames
    ///
    /// This method should be called regularly (e.g., 60Hz) to advance animation frames.
    /// It returns a list of image IDs whose frames changed, allowing frontends to
    /// selectively refresh only graphics that were updated.
    ///
    /// Returns:
    ///     List of image IDs that changed frames
    pub fn update_animations(&mut self) -> Vec<u32> {
        self.graphics_store.update_animations()
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
        let max_graphics = self.graphics_store.limits().max_graphics_count;
        let current_graphics = self.graphics_store.graphics_count();
        let dropped_graphics = self.graphics_store.dropped_count();
        (limits, max_graphics, current_graphics, dropped_graphics)
    }

    /// Process a buffered Sixel command (color, raster, repeat)
    /// Get current Kitty keyboard protocol flags
    pub fn keyboard_flags(&self) -> u16 {
        self.keyboard_flags
    }

    /// Push keyboard flags to stack
    pub fn push_keyboard_flags(&mut self, flags: u16) {
        self.keyboard_stack.push(self.keyboard_flags);
        self.keyboard_flags = flags;
    }

    /// Pop keyboard flags from stack
    pub fn pop_keyboard_flags(&mut self, count: usize) {
        for _ in 0..count {
            if let Some(flags) = self.keyboard_stack.pop() {
                self.keyboard_flags = flags;
            }
        }
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

    /// Get modifyOtherKeys mode (XTerm extension)
    /// 0 = disabled, 1 = report modifiers for special keys, 2 = report modifiers for all keys
    pub fn modify_other_keys_mode(&self) -> u8 {
        self.modify_other_keys_mode
    }

    /// Set modifyOtherKeys mode (for testing/direct control)
    pub fn set_modify_other_keys_mode(&mut self, mode: u8) {
        // Clamp to valid range (0-2)
        self.modify_other_keys_mode = mode.min(2);
    }

    /// Get clipboard content (OSC 52)
    pub fn clipboard(&self) -> Option<&str> {
        self.clipboard_content.as_deref()
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

    /// Get the answerback string sent in response to ENQ (0x05)
    pub fn answerback_string(&self) -> Option<&str> {
        self.answerback_string.as_deref()
    }

    /// Set the answerback string sent in response to ENQ (0x05) control character
    ///
    /// The answerback string is sent back to the PTY when the terminal receives
    /// an ENQ (enquiry, ASCII 0x05) character. This was historically used for
    /// terminal identification in multi-terminal environments.
    ///
    /// # Security Note
    /// Default is None (disabled) for security. Setting this may expose
    /// terminal identification information to applications.
    ///
    /// # Arguments
    /// * `answerback` - The string to send, or None to disable
    pub fn set_answerback_string(&mut self, answerback: Option<String>) {
        self.answerback_string = answerback;
    }

    /// Get the current Unicode width configuration
    ///
    /// Returns the configuration used for character width calculations,
    /// including Unicode version and ambiguous width handling.
    pub fn width_config(&self) -> &crate::unicode_width_config::WidthConfig {
        &self.width_config
    }

    /// Set the Unicode width configuration
    ///
    /// This affects how character widths are calculated for terminal display,
    /// particularly for:
    /// - East Asian Ambiguous width characters (narrow vs wide)
    /// - Emoji and other Unicode characters
    ///
    /// # Arguments
    /// * `config` - The new width configuration to use
    pub fn set_width_config(&mut self, config: crate::unicode_width_config::WidthConfig) {
        self.width_config = config;
    }

    /// Set the ambiguous width setting
    ///
    /// Convenience method to change only the ambiguous width treatment
    /// without modifying other width configuration settings.
    ///
    /// # Arguments
    /// * `width` - The ambiguous width setting (Narrow or Wide)
    pub fn set_ambiguous_width(&mut self, width: crate::unicode_width_config::AmbiguousWidth) {
        self.width_config.ambiguous_width = width;
    }

    /// Set the Unicode version for width calculations
    ///
    /// Convenience method to change only the Unicode version
    /// without modifying other width configuration settings.
    ///
    /// # Arguments
    /// * `version` - The Unicode version to use for width tables
    pub fn set_unicode_version(&mut self, version: crate::unicode_width_config::UnicodeVersion) {
        self.width_config.unicode_version = version;
    }

    /// Calculate the display width of a character using current config
    ///
    /// This uses the terminal's width configuration to determine
    /// how many cells a character occupies.
    ///
    /// # Arguments
    /// * `c` - The character to measure
    ///
    /// # Returns
    /// The display width in cells (0, 1, or 2)
    #[inline]
    pub fn char_width(&self, c: char) -> usize {
        crate::unicode_width_config::char_width(c, &self.width_config)
    }

    // === Tab Stop Management ===

    /// Get all tab stop positions
    pub fn get_tab_stops(&self) -> Vec<usize> {
        self.tab_stops
            .iter()
            .enumerate()
            .filter(|(_, &set)| set)
            .map(|(i, _)| i)
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
        for set in self.tab_stops.iter_mut() {
            *set = false;
        }
    }

    /// Get the current Unicode normalization form
    ///
    /// Returns the normalization form used for text stored in terminal cells.
    pub fn normalization_form(&self) -> crate::unicode_normalization_config::NormalizationForm {
        self.normalization_form
    }

    /// Set the Unicode normalization form
    ///
    /// Controls how Unicode text is normalized before being stored in cells.
    /// Default is NFC (Canonical Decomposition, followed by Canonical Composition).
    ///
    /// # Arguments
    /// * `form` - The normalization form to use
    pub fn set_normalization_form(
        &mut self,
        form: crate::unicode_normalization_config::NormalizationForm,
    ) {
        self.normalization_form = form;
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

    // === Progress Bar Methods (OSC 9;4) ===

    /// Get the current progress bar state
    ///
    /// Returns the progress bar state set via OSC 9;4 sequences.
    /// The progress bar has a state (hidden, normal, indeterminate, warning, error)
    /// and a percentage (0-100) for states that support it.
    pub fn progress_bar(&self) -> &ProgressBar {
        &self.progress_bar
    }

    /// Check if the progress bar is currently active (visible)
    ///
    /// Returns true if the progress bar is in any state other than Hidden.
    pub fn has_progress(&self) -> bool {
        self.progress_bar.is_active()
    }

    /// Get the current progress percentage (0-100)
    ///
    /// Returns the progress percentage. Only meaningful when the progress bar
    /// state is Normal, Warning, or Error.
    pub fn progress_value(&self) -> u8 {
        self.progress_bar.progress
    }

    /// Get the current progress bar state
    ///
    /// Returns the state (Hidden, Normal, Indeterminate, Warning, Error).
    pub fn progress_state(&self) -> ProgressState {
        self.progress_bar.state
    }

    /// Manually set the progress bar state
    ///
    /// This can be used to programmatically control the progress bar
    /// without receiving OSC 9;4 sequences.
    pub fn set_progress(&mut self, state: ProgressState, progress: u8) {
        self.progress_bar = ProgressBar::new(state, progress);
    }

    /// Clear/hide the progress bar
    ///
    /// Equivalent to receiving OSC 9;4;0.
    pub fn clear_progress(&mut self) {
        self.progress_bar = ProgressBar::hidden();
    }

    // Named progress bar methods (OSC 934)

    /// Get all named progress bars
    ///
    /// Returns the map of active named progress bars set via OSC 934 sequences.
    pub fn named_progress_bars(&self) -> &HashMap<String, NamedProgressBar> {
        &self.named_progress_bars
    }

    /// Get a specific named progress bar by ID
    pub fn get_named_progress_bar(&self, id: &str) -> Option<&NamedProgressBar> {
        self.named_progress_bars.get(id)
    }

    /// Set or update a named progress bar and emit an event
    pub fn set_named_progress_bar(&mut self, bar: NamedProgressBar) {
        let id = bar.id.clone();
        let state = bar.state;
        let percent = bar.percent;
        let label = bar.label.clone();
        self.named_progress_bars.insert(id.clone(), bar);
        self.terminal_events
            .push(TerminalEvent::ProgressBarChanged {
                action: ProgressBarAction::Set,
                id,
                state: Some(state),
                percent: Some(percent),
                label,
            });
    }

    /// Remove a named progress bar by ID and emit an event
    ///
    /// Returns true if the bar existed and was removed.
    pub fn remove_named_progress_bar(&mut self, id: &str) -> bool {
        if self.named_progress_bars.remove(id).is_some() {
            self.terminal_events
                .push(TerminalEvent::ProgressBarChanged {
                    action: ProgressBarAction::Remove,
                    id: id.to_string(),
                    state: None,
                    percent: None,
                    label: None,
                });
            true
        } else {
            false
        }
    }

    /// Remove all named progress bars and emit an event
    pub fn remove_all_named_progress_bars(&mut self) {
        if !self.named_progress_bars.is_empty() {
            self.named_progress_bars.clear();
            self.terminal_events
                .push(TerminalEvent::ProgressBarChanged {
                    action: ProgressBarAction::RemoveAll,
                    id: String::new(),
                    state: None,
                    percent: None,
                    label: None,
                });
        }
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

    /// Get the grid with scrollback applied (for screenshots/export)
    fn grid_with_scrollback(&self, scrollback_offset: usize) -> Grid {
        let grid = self.active_grid();
        let (cols, rows) = self.size();
        let scrollback_len = grid.scrollback_len();

        let mut view = Grid::new(cols, rows, 0); // No scrollback needed for the view

        for row in 0..rows {
            let abs_row = (scrollback_len + row).saturating_sub(scrollback_offset);
            if abs_row < scrollback_len {
                if let Some(line) = grid.scrollback_line(abs_row) {
                    for (col, cell) in line.iter().enumerate() {
                        view.set(col, row, cell.clone());
                    }
                }
            } else {
                let grid_row = abs_row - scrollback_len;
                if grid_row < rows {
                    if let Some(line) = grid.row(grid_row) {
                        for (col, cell) in line.iter().enumerate() {
                            view.set(col, row, cell.clone());
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
        config.bold_brightening = self.bold_brightening;
        config.faint_text_alpha = self.faint_text_alpha;

        // Use terminal's default background if not specified
        if config.background_color.is_none() {
            config.background_color = Some(self.default_bg.to_rgb());
        }

        let grid = self.grid_with_scrollback(scrollback_offset);
        let cursor = if config.render_cursor && scrollback_offset == 0 {
            Some(&self.cursor)
        } else {
            None
        };
        let graphics = if config.sixel_render_mode != crate::screenshot::SixelRenderMode::Disabled
            && scrollback_offset == 0
        {
            self.all_graphics()
        } else {
            &[]
        };
        crate::screenshot::render_grid(&grid, cursor, graphics, config)
    }

    /// Take a screenshot and save to file
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
        config.bold_brightening = self.bold_brightening;
        config.faint_text_alpha = self.faint_text_alpha;

        // Use terminal's default background if not specified
        if config.background_color.is_none() {
            config.background_color = Some(self.default_bg.to_rgb());
        }

        let grid = self.grid_with_scrollback(scrollback_offset);
        let cursor = if config.render_cursor && scrollback_offset == 0 {
            Some(&self.cursor)
        } else {
            None
        };
        let graphics = if config.sixel_render_mode != crate::screenshot::SixelRenderMode::Disabled
            && scrollback_offset == 0
        {
            self.all_graphics()
        } else {
            &[]
        };
        crate::screenshot::save_grid(&grid, cursor, graphics, path, config)
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
    pub fn set_tmux_control_mode(&mut self, enabled: bool) {
        self.tmux_parser.set_control_mode(enabled);
    }

    /// Check if tmux control mode is enabled
    pub fn is_tmux_control_mode(&self) -> bool {
        self.tmux_parser.is_control_mode()
    }

    /// Enable or disable tmux control mode auto-detection
    pub fn set_tmux_auto_detect(&mut self, enabled: bool) {
        self.tmux_parser.set_auto_detect(enabled);
    }

    /// Check if tmux control mode auto-detection is enabled
    pub fn is_tmux_auto_detect(&self) -> bool {
        self.tmux_parser.is_auto_detect()
    }

    /// Get tmux control protocol notifications
    pub fn tmux_notifications(&self) -> &[crate::tmux_control::TmuxNotification] {
        &self.tmux_notifications
    }

    /// Drain and return tmux control protocol notifications
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

    /// Process incoming data from the PTY
    pub fn process(&mut self, data: &[u8]) {
        if self.is_recording {
            self.record_event(RecordingEventType::Output, data.to_vec());
        }

        if self.synchronized_updates {
            // Buffer data instead of processing it immediately
            self.update_buffer.extend_from_slice(data);

            // Peek at the end of the buffer to see if it contains the disable sequence
            // We check the last 32 bytes to account for sequences split across chunks
            let peek_len = 32.min(self.update_buffer.len());
            let peek_start = self.update_buffer.len() - peek_len;
            if contains_bytes(&self.update_buffer[peek_start..], b"\x1b[?2026l") {
                self.flush_synchronized_updates();
            }
            return;
        }

        if self.tmux_parser.is_control_mode() || self.tmux_parser.is_auto_detect() {
            // Process as tmux control protocol (handles auto-detect internally)
            let notifications = self.tmux_parser.parse(data);
            for notification in notifications {
                match notification {
                    crate::tmux_control::TmuxNotification::TerminalOutput { data } => {
                        // Feed non-control data back to standard VTE parser
                        let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());
                        parser.advance(self, &data);
                        let _ = std::mem::replace(&mut self.parser, parser);
                    }
                    _ => {
                        // Store tmux notification
                        self.tmux_notifications.push(notification);
                    }
                }
            }
        } else {
            // Process as standard terminal output
            let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());
            parser.advance(self, data);
            let _ = std::mem::replace(&mut self.parser, parser);
        }

        self.dispatch_events();
    }

    /// Dispatch pending events to all registered observers.
    /// Uses `events_dispatched_up_to` index to avoid sending duplicate events
    /// when `process()` is called multiple times before `poll_events()`.
    fn dispatch_events(&mut self) {
        if self.observers.is_empty() || self.terminal_events.is_empty() {
            return;
        }

        let start = self.events_dispatched_up_to;
        if start >= self.terminal_events.len() {
            return;
        }

        for event in &self.terminal_events[start..] {
            let category = crate::observer::event_category(event);
            let event_kind = event.kind();
            for entry in &self.observers {
                // Check subscriptions
                if let Some(subs) = entry.observer.subscriptions() {
                    if !subs.contains(&event_kind) {
                        continue;
                    }
                }

                match category {
                    crate::observer::EventCategory::Zone => entry.observer.on_zone_event(event),
                    crate::observer::EventCategory::Command => {
                        entry.observer.on_command_event(event)
                    }
                    crate::observer::EventCategory::Environment => {
                        entry.observer.on_environment_event(event)
                    }
                    crate::observer::EventCategory::Screen => entry.observer.on_screen_event(event),
                }
                entry.observer.on_event(event);
            }
        }

        self.events_dispatched_up_to = self.terminal_events.len();
    }

    /// Reset the terminal to its initial state (RIS)
    pub fn reset(&mut self) {
        let (cols, rows) = self.size();
        let scrollback = self.grid.max_scrollback();

        // Save current tab stops
        let tab_stops = self.tab_stops.clone();

        *self = Self::with_scrollback(cols, rows, scrollback);

        // Restore tab stops
        self.tab_stops = tab_stops;
    }

    /// Mark a row as dirty (needs redrawing)
    pub fn mark_row_dirty(&mut self, row: usize) {
        self.dirty_rows.insert(row);

        // If we have triggers, also add to pending trigger rows
        if self.trigger_registry.has_active_triggers() {
            self.pending_trigger_rows.insert(row);
        }
    }

    /// Mark the entire screen as clean
    pub fn mark_clean(&mut self) {
        self.dirty_rows.clear();
    }

    /// Get all dirty rows
    pub fn get_dirty_rows(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self.dirty_rows.iter().copied().collect();
        rows.sort_unstable();
        rows
    }

    /// Get the bounding box of the dirty region
    pub fn get_dirty_region(&self) -> Option<(usize, usize, usize, usize)> {
        if self.dirty_rows.is_empty() {
            return None;
        }

        let first_row = *self.dirty_rows.iter().min().unwrap();
        let last_row = *self.dirty_rows.iter().max().unwrap();
        let cols = self.grid.cols();

        Some((first_row, 0, last_row, cols.saturating_sub(1)))
    }

    /// Count lines with non-whitespace content
    pub fn count_non_whitespace_lines(&self) -> usize {
        let mut count = 0;
        let grid = self.active_grid();
        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                if line.iter().any(|c| c.c != ' ') {
                    count += 1;
                }
            }
        }
        count
    }

    // === Observer Management ===

    /// Add an observer for push-based event delivery
    pub fn add_observer(
        &mut self,
        observer: std::sync::Arc<dyn crate::observer::TerminalObserver>,
    ) -> crate::observer::ObserverId {
        let id = self.next_observer_id;
        self.next_observer_id += 1;
        self.observers
            .push(crate::observer::ObserverEntry { id, observer });
        id
    }

    /// Remove an observer by ID
    pub fn remove_observer(&mut self, id: crate::observer::ObserverId) -> bool {
        if let Some(pos) = self.observers.iter().position(|o| o.id == id) {
            self.observers.remove(pos);
            true
        } else {
            false
        }
    }

    /// Get the number of registered observers
    pub fn observer_count(&self) -> usize {
        self.observers.len()
    }

    /// Poll for pending events
    pub fn poll_events(&mut self) -> Vec<TerminalEvent> {
        // Drain evicted zones and emit ZoneScrolledOut events
        let evicted = self.grid.drain_evicted_zones();
        for zone in evicted {
            self.terminal_events.push(TerminalEvent::ZoneScrolledOut {
                zone_id: zone.id,
                zone_type: zone.zone_type,
            });
        }
        // Also check alt grid
        let alt_evicted = self.alt_grid.drain_evicted_zones();
        for zone in alt_evicted {
            self.terminal_events.push(TerminalEvent::ZoneScrolledOut {
                zone_id: zone.id,
                zone_type: zone.zone_type,
            });
        }
        self.events_dispatched_up_to = 0;
        std::mem::take(&mut self.terminal_events)
    }

    /// Drain pending bell events
    pub fn drain_bell_events(&mut self) -> Vec<BellEvent> {
        std::mem::take(&mut self.bell_events)
    }

    // === Event Subscription ===

    /// Set the event subscription filter
    pub fn set_event_subscription(&mut self, filter: HashSet<TerminalEventKind>) {
        self.event_subscription = Some(filter);
    }

    /// Clear the event subscription filter (subscribe to all events)
    pub fn clear_event_subscription(&mut self) {
        self.event_subscription = None;
    }

    /// Poll for events that match the current subscription filter
    pub fn poll_subscribed_events(&mut self) -> Vec<TerminalEvent> {
        if let Some(ref filter) = self.event_subscription {
            let events = std::mem::take(&mut self.terminal_events);
            let (matched, remaining): (Vec<_>, Vec<_>) = events.into_iter().partition(|e| {
                let kind = match e {
                    TerminalEvent::BellRang(_) => TerminalEventKind::BellRang,
                    TerminalEvent::TitleChanged(_) => TerminalEventKind::TitleChanged,
                    TerminalEvent::SizeChanged(_, _) => TerminalEventKind::SizeChanged,
                    TerminalEvent::ModeChanged(_, _) => TerminalEventKind::ModeChanged,
                    TerminalEvent::GraphicsAdded(_) => TerminalEventKind::GraphicsAdded,
                    TerminalEvent::HyperlinkAdded { .. } => TerminalEventKind::HyperlinkAdded,
                    TerminalEvent::DirtyRegion(_, _) => TerminalEventKind::DirtyRegion,
                    TerminalEvent::CwdChanged(_) => TerminalEventKind::CwdChanged,
                    TerminalEvent::TriggerMatched(_) => TerminalEventKind::TriggerMatched,
                    TerminalEvent::UserVarChanged { .. } => TerminalEventKind::UserVarChanged,
                    TerminalEvent::ProgressBarChanged { .. } => {
                        TerminalEventKind::ProgressBarChanged
                    }
                    TerminalEvent::BadgeChanged(_) => TerminalEventKind::BadgeChanged,
                    TerminalEvent::ShellIntegrationEvent { .. } => {
                        TerminalEventKind::ShellIntegrationEvent
                    }
                    TerminalEvent::ZoneOpened { .. } => TerminalEventKind::ZoneOpened,
                    TerminalEvent::ZoneClosed { .. } => TerminalEventKind::ZoneClosed,
                    TerminalEvent::ZoneScrolledOut { .. } => TerminalEventKind::ZoneScrolledOut,
                    TerminalEvent::EnvironmentChanged { .. } => {
                        TerminalEventKind::EnvironmentChanged
                    }
                    TerminalEvent::RemoteHostTransition { .. } => {
                        TerminalEventKind::RemoteHostTransition
                    }
                    TerminalEvent::SubShellDetected { .. } => TerminalEventKind::SubShellDetected,
                    TerminalEvent::FileTransferStarted { .. } => {
                        TerminalEventKind::FileTransferStarted
                    }
                    TerminalEvent::FileTransferProgress { .. } => {
                        TerminalEventKind::FileTransferProgress
                    }
                    TerminalEvent::FileTransferCompleted { .. } => {
                        TerminalEventKind::FileTransferCompleted
                    }
                    TerminalEvent::FileTransferFailed { .. } => {
                        TerminalEventKind::FileTransferFailed
                    }
                    TerminalEvent::UploadRequested { .. } => TerminalEventKind::UploadRequested,
                };
                filter.contains(&kind)
            });
            self.terminal_events = remaining;
            matched
        } else {
            self.poll_events()
        }
    }

    /// Poll for CWD change events
    pub fn poll_cwd_events(&mut self) -> Vec<CwdChange> {
        let events = std::mem::take(&mut self.terminal_events);
        let mut cwd_changes = Vec::new();
        let mut remaining = Vec::new();

        for event in events {
            if let TerminalEvent::CwdChanged(change) = event {
                cwd_changes.push(change);
            } else {
                remaining.push(event);
            }
        }

        self.terminal_events = remaining;
        cwd_changes
    }

    /// Poll for upload request events
    ///
    /// Returns all pending UploadRequested events and removes them from the queue.
    pub fn poll_upload_requests(&mut self) -> Vec<String> {
        let events = std::mem::take(&mut self.terminal_events);
        let mut upload_formats = Vec::new();
        let mut remaining = Vec::new();

        for event in events {
            if let TerminalEvent::UploadRequested { format } = event {
                upload_formats.push(format);
            } else {
                remaining.push(event);
            }
        }

        self.terminal_events = remaining;
        upload_formats
    }

    /// Poll for shell integration events
    pub fn poll_shell_integration_events(&mut self) -> Vec<ShellEvent> {
        let events = std::mem::take(&mut self.terminal_events);
        let mut shell_events = Vec::new();
        let mut remaining = Vec::new();

        for event in events {
            if let TerminalEvent::ShellIntegrationEvent {
                event_type,
                command,
                exit_code,
                timestamp,
                cursor_line,
            } = event
            {
                shell_events.push((event_type, command, exit_code, timestamp, cursor_line));
            } else {
                remaining.push(event);
            }
        }

        self.terminal_events = remaining;
        shell_events
    }

    /// Calculate a checksum for a rectangular region of cells
    pub fn calculate_rectangle_checksum(
        &self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> u16 {
        let mut checksum: u32 = 0;
        let grid = self.active_grid();

        for row in top..=bottom {
            if let Some(line) = grid.row(row) {
                for col in left..=right {
                    if let Some(cell) = line.get(col) {
                        checksum = checksum.wrapping_add(cell.c as u32);
                        // Add other attributes to checksum if needed by spec
                    }
                }
            }
        }

        (checksum & 0xFFFF) as u16
    }

    /// Get a rectangular region of cells
    pub fn get_rectangle(
        &self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> Vec<Vec<Cell>> {
        let mut rows = Vec::new();
        let grid = self.active_grid();

        for row in top..=bottom {
            let mut cells = Vec::new();
            if let Some(line) = grid.row(row) {
                for col in left..=right {
                    if let Some(cell) = line.get(col) {
                        cells.push(cell.clone());
                    }
                }
            }
            rows.push(cells);
        }

        rows
    }

    /// Push bytes to the response buffer (to be sent back to PTY)
    pub fn push_response(&mut self, bytes: &[u8]) {
        self.response_buffer.extend_from_slice(bytes);
    }

    /// Fill a rectangular region with a character
    pub fn fill_rectangle(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        ch: char,
    ) {
        let mut cell = Cell::new(ch);
        cell.fg = self.fg;
        cell.bg = self.bg;

        for row in top..=bottom {
            for col in left..=right {
                self.active_grid_mut().set(col, row, cell.clone());
            }
            self.mark_row_dirty(row);
        }
    }

    /// Erase a rectangular region (fill with spaces)
    pub fn erase_rectangle(&mut self, top: usize, left: usize, bottom: usize, right: usize) {
        self.fill_rectangle(top, left, bottom, right, ' ');
    }

    /// Get the visible region bounds
    pub fn get_visible_region(&self) -> (usize, usize, usize, usize) {
        let (cols, rows) = self.size();
        (0, 0, rows.saturating_sub(1), cols.saturating_sub(1))
    }

    /// Get a range of rows as vectors of cells (end is exclusive)
    pub fn get_row_range(&self, start: usize, end: usize) -> Vec<Vec<Cell>> {
        let mut rows = Vec::new();
        let grid = self.active_grid();
        for r in start..end {
            if let Some(line) = grid.row(r) {
                rows.push(line.to_vec());
            }
        }
        rows
    }
}

mod perform;

#[cfg(test)]
mod tests;
