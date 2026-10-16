//! `Terminal` state sub-structs (ARC-001/ARC-003).
//!
//! The cohesive state groups `Terminal` holds as fields, with their
//! `Default` impls and small helper methods. Every non-private item is
//! re-exported from `terminal/mod.rs` at its historical `crate::terminal::*`
//! path.

use super::notification;
use super::{Bookmark, DamageRegion};
use super::{
    ClipboardEntry, ClipboardHistoryEntry, ClipboardSlot, ClipboardSyncEvent, ClipboardTarget,
    CommandExecution, CwdChange, EventBroker, FileTransferManager, FrameTiming,
    ITermMultipartState, InlineImage, NamedProgressBar, Notification, NotificationConfig,
    NotificationEvent, PerformanceMetrics, ProfilingData, ProgressBar, ProgressBarAction,
    RecordingSession, RegexMatch, RenderingHint, Terminal, TerminalEvent,
};
use crate::cell::CellFlags;
use crate::color::{Color, NamedColor};
use crate::cursor::Cursor;
use crate::graphics::GraphicsStore;
use crate::mouse::{MouseEncoding, MouseEventRecord, MouseMode, MousePosition};
use crate::shell_integration::ShellIntegration;
use crate::sixel;
use crate::terminal::sequences::dcs::DcsKind;
use std::collections::HashMap;
use std::num::NonZeroU32;

/// Character set designation for G0/G1 charset slots.
///
/// VT100 defines multiple character sets; the most commonly used are ASCII
/// and the DEC Special / Line Drawing set used by applications like tmux.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Charset {
    /// Standard ASCII character set (default)
    #[default]
    Ascii,
    /// DEC Special / Line Drawing character set (ESC ( 0 / ESC ) 0)
    DecLineDrawing,
}

impl Charset {
    /// Translate a character according to the DEC Special / Line Drawing table.
    ///
    /// Only characters in the printable ASCII range that appear in the ACS map
    /// are translated; everything else passes through unchanged.
    pub fn translate(self, c: char) -> char {
        if self != Charset::DecLineDrawing {
            return c;
        }
        // ACS byte → Unicode mapping (VT100 manual §3.3.4)
        match c {
            'j' => '┘', // U+2518
            'k' => '┐', // U+2510
            'l' => '┌', // U+250C
            'm' => '└', // U+2514
            'n' => '┼', // U+253C
            'q' => '─', // U+2500
            't' => '├', // U+251C
            'u' => '┤', // U+2524
            'v' => '┴', // U+2534
            'w' => '┬', // U+252C
            'x' => '│', // U+2502
            'a' => '▒', // U+2592
            '`' => '◆', // U+25C6
            'f' => '°', // U+00B0
            'g' => '±', // U+00B1
            '~' => '•', // U+00B7
            'o' => '⎺', // U+23BA
            'p' => '⎻', // U+23BB
            'r' => '⎼', // U+23BC
            's' => '⎽', // U+23BD
            'i' => '␋', // U+240B
            'h' => '▒', // U+2592
            _ => c,
        }
    }
}

/// cap: Terminal notifications queued from processed escape-sequence output.
const DEFAULT_MAX_NOTIFICATIONS: usize = 128;
/// cap: Clipboard sync events queued from processed escape-sequence output.
const DEFAULT_MAX_CLIPBOARD_SYNC_EVENTS: usize = 256;
/// cap: Bytes retained for one queued clipboard sync event.
const DEFAULT_MAX_CLIPBOARD_EVENT_BYTES: usize = 4096;
// =============================================================================
// Cohesive state sub-structs (ARC-001: decomposing the `Terminal` god object)
//
// `Terminal` historically carried ~120 flat fields spanning many unrelated
// concerns. Related fields are grouped into cohesive structs held as fields on
// `Terminal`. Each extraction is behavior-preserving; existing accessor methods
// on `Terminal` delegate to the sub-struct so callers (including the Python
// bindings) are unaffected.
// =============================================================================

/// OSC 52 clipboard-sync state (Feature 30).
///
/// Holds the clipboard-sync event log, per-target history, size caps, and the
/// remote session id. Extracted from `Terminal` for cohesion (ARC-001).
pub(crate) struct ClipboardSyncState {
    /// Clipboard sync events log
    pub(crate) events: Vec<ClipboardSyncEvent>,
    /// Clipboard sync history across targets
    pub(crate) history: HashMap<ClipboardTarget, Vec<ClipboardHistoryEntry>>,
    /// Maximum clipboard sync history entries per target
    pub(crate) max_history: usize,
    /// Maximum clipboard sync events retained for diagnostics
    pub(crate) max_events: usize,
    /// Maximum bytes of clipboard content to persist per event/history entry
    pub(crate) max_event_bytes: usize,
    /// Remote session identifier for clipboard sync
    pub(crate) remote_session_id: Option<String>,
}

impl Default for ClipboardSyncState {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            history: HashMap::new(),
            max_history: 50,
            max_events: DEFAULT_MAX_CLIPBOARD_SYNC_EVENTS,
            max_event_bytes: DEFAULT_MAX_CLIPBOARD_EVENT_BYTES,
            remote_session_id: None,
        }
    }
}

/// Performance metrics and profiling state.
///
/// Extracted from `Terminal` for cohesion (ARC-001).
pub(crate) struct ProfilingState {
    /// Performance metrics tracking (frames rendered, cells updated, etc.)
    pub(crate) metrics: PerformanceMetrics,
    /// Frame timing history (last N frames)
    pub(crate) frame_timings: Vec<FrameTiming>,
    /// Maximum frame timings to keep
    pub(crate) max_frame_timings: usize,
    /// Profiling data (when enabled)
    pub(crate) data: Option<ProfilingData>,
    /// Profiling enabled flag
    pub(crate) enabled: bool,
}

impl Default for ProfilingState {
    fn default() -> Self {
        Self {
            metrics: PerformanceMetrics::default(),
            frame_timings: Vec::new(),
            max_frame_timings: 100, // Keep last 100 frames
            data: None,
            enabled: false,
        }
    }
}

/// Mouse event/position history.
#[derive(Default)]
pub(crate) struct MouseHistoryState {
    pub(crate) mouse_events: Vec<MouseEventRecord>,
    pub(crate) mouse_positions: Vec<MousePosition>,
}

/// Regex search matches and current pattern.
#[derive(Default)]
pub(crate) struct SearchState {
    pub(crate) regex_matches: Vec<RegexMatch>,
    pub(crate) current_regex_pattern: Option<String>,
}

/// Inline image storage (iTerm2/Kitty protocols).
#[derive(Default)]
pub(crate) struct InlineImageState {
    pub(crate) inline_images: Vec<InlineImage>,
}

/// Rendering hints and accumulated damage regions.
#[derive(Default)]
pub(crate) struct RenderingState {
    pub(crate) rendering_hints: Vec<RenderingHint>,
    pub(crate) damage_regions: Vec<DamageRegion>,
}

/// tmux control-protocol parser and notification buffer.
pub(crate) struct TmuxState {
    pub(crate) tmux_parser: crate::tmux_control::TmuxControlParser,
    pub(crate) tmux_notifications: Vec<crate::tmux_control::TmuxNotification>,
}

impl Default for TmuxState {
    fn default() -> Self {
        // Control mode disabled — the default the tmux passthrough gates on.
        Self {
            tmux_parser: crate::tmux_control::TmuxControlParser::new(false),
            tmux_notifications: Vec::new(),
        }
    }
}

/// Terminal notification state.
///
/// Combines the OSC 9/777 notification buffer with the Feature 37 notification
/// config, event log, silence/activity tracking, size cap, and custom triggers.
/// Extracted from `Terminal` for cohesion (ARC-001).
pub(crate) struct NotificationState {
    /// Notifications from OSC 9 / OSC 777 sequences
    pub(crate) notifications: Vec<Notification>,
    /// Notification configuration
    pub(crate) notification_config: NotificationConfig,
    /// Notification events log
    pub(crate) notification_events: Vec<NotificationEvent>,
    /// Last activity timestamp (for silence detection)
    pub(crate) last_activity_time: u64,
    /// Last silence check timestamp
    pub(crate) last_silence_check: u64,
    /// Last activity notification check timestamp (activity monitoring)
    pub(crate) last_activity_check: u64,
    /// Maximum OSC 9/777 notifications retained
    pub(crate) max_notifications: usize,
    /// Custom notification triggers (ID -> message)
    pub(crate) custom_triggers: HashMap<u32, String>,
    /// In-progress Kitty OSC 99 notifications awaiting a `d=1` (done) chunk,
    /// keyed by `i=` id (empty string key for id-less notifications).
    pub(crate) osc99_pending: HashMap<String, notification::PartialNotification>,
}

impl Default for NotificationState {
    fn default() -> Self {
        let now = crate::text_utils::unix_millis();
        Self {
            notifications: Vec::new(),
            notification_config: NotificationConfig::default(),
            notification_events: Vec::new(),
            last_activity_time: now,
            last_silence_check: now,
            last_activity_check: now,
            max_notifications: DEFAULT_MAX_NOTIFICATIONS,
            custom_triggers: HashMap::new(),
            osc99_pending: HashMap::new(),
        }
    }
}

impl NotificationState {
    /// Queue a notification, evicting the oldest past `max_notifications`
    /// (0 disables buffering).
    pub(crate) fn enqueue(&mut self, notification: Notification) {
        if self.max_notifications == 0 {
            return;
        }

        if self.notifications.len() >= self.max_notifications {
            let excess = self.notifications.len() + 1 - self.max_notifications;
            self.notifications.drain(0..excess);
        }

        self.notifications.push(notification);
    }
}

/// Terminal replay/recording state (Feature 24).
///
/// Extracted from `Terminal` for cohesion (ARC-001).
#[derive(Default)]
pub(crate) struct RecordingState {
    /// Current recording session
    pub(crate) recording_session: Option<RecordingSession>,
    /// Recording active flag
    pub(crate) is_recording: bool,
    /// Recording start timestamp (for relative timing)
    pub(crate) recording_start_time: u64,
}

/// Keyboard protocol state: Kitty flags, per-screen stacks, and modifyOtherKeys mode.
///
/// Extracted from `Terminal` for cohesion (ARC-001).
#[derive(Default)]
pub(crate) struct KeyboardState {
    /// Kitty keyboard protocol flags (progressive enhancement)
    pub(crate) keyboard_flags: u16,
    /// Stack for keyboard protocol flags (main screen)
    pub(crate) keyboard_stack: Vec<u16>,
    /// Stack for keyboard protocol flags (alternate screen)
    pub(crate) keyboard_stack_alt: Vec<u16>,
    /// modifyOtherKeys mode (XTerm extension for enhanced keyboard input)
    /// 0 = disabled, 1 = report modifiers for special keys, 2 = report modifiers for all keys
    pub(crate) modify_other_keys_mode: u8,
}

/// Synchronized update state (DEC 2026).
///
/// Holds the active flag, batched update buffer, and the "explicitly disabled
/// during flush" tracking flag. Extracted from `Terminal` for cohesion (ARC-001).
#[derive(Default)]
pub(crate) struct SyncState {
    /// Synchronized update mode (DEC 2026)
    pub(crate) synchronized_updates: bool,
    /// Buffer for batched updates (when synchronized mode is active)
    pub(crate) update_buffer: Vec<u8>,
    /// Flag to track if synchronized updates were explicitly disabled during a flush
    pub(crate) sync_update_explicitly_disabled: bool,
}

/// Window title, title stack, and answerback string.
///
/// Extracted from `Terminal` for cohesion (ARC-001).
#[derive(Default)]
pub(crate) struct TitleState {
    /// Terminal title
    pub(crate) title: String,
    /// Window title stack for XTWINOPS 22/23 (push/pop title)
    pub(crate) title_stack: Vec<String>,
    /// Answerback string sent in response to ENQ (0x05).
    /// Default: empty (no response) for security
    /// Common values: "par-term", "vt100", or custom identification
    pub(crate) answerback_string: Option<String>,
}

/// Shell-integration core: integration state, host/user tracking, nesting depth,
/// and command-output flag.
///
/// Extracted from `Terminal` for cohesion (ARC-001).
#[derive(Default)]
pub(crate) struct ShellState {
    /// Shell integration state
    pub(crate) shell_integration: ShellIntegration,
    /// Last known hostname (for detecting remote host transitions)
    pub(crate) last_hostname: Option<String>,
    /// Last known username (for detecting remote host transitions)
    pub(crate) last_username: Option<String>,
    /// Current shell nesting depth (for sub-shell detection)
    pub(crate) shell_depth: usize,
    /// Whether we are currently inside command output (between OSC 133 C and D)
    pub(crate) in_command_output: bool,
}

/// Bookmark registry for quick navigation.
///
/// Extracted from `Terminal` for cohesion (ARC-001).
#[derive(Default)]
pub(crate) struct BookmarksState {
    /// Bookmarks for quick navigation
    pub(crate) bookmarks: Vec<Bookmark>,
    /// Next available bookmark ID
    pub(crate) next_bookmark_id: usize,
}

/// ACS (Alternate Character Set) state: G0/G1 slot designations and active slot.
///
/// Extracted from `Terminal` for cohesion (ARC-001).
pub(crate) struct CharsetState {
    /// G0 charset slot designation (ESC ( 0 / ESC ( B)
    pub(crate) g0_charset: Charset,
    /// G1 charset slot designation (ESC ) 0 / ESC ) B)
    pub(crate) g1_charset: Charset,
    /// Active charset slot: 0 = G0, 1 = G1 (toggled by SO/SI)
    pub(crate) active_g: u8,
}

impl Default for CharsetState {
    fn default() -> Self {
        Self {
            g0_charset: Charset::Ascii,
            g1_charset: Charset::Ascii,
            active_g: 0,
        }
    }
}

/// Hyperlink storage (OSC 8): ID→URL map, current ID being written, next free ID.
///
/// Extracted from `Terminal` for cohesion (ARC-001).
pub(crate) struct HyperlinkState {
    /// Hyperlink storage: ID -> URL mapping (for deduplication)
    pub(crate) hyperlinks: HashMap<u32, String>,
    /// Current hyperlink ID being written
    pub(crate) current_hyperlink_id: Option<NonZeroU32>,
    /// Next available hyperlink ID
    pub(crate) next_hyperlink_id: u32,
}

impl Default for HyperlinkState {
    fn default() -> Self {
        Self {
            hyperlinks: HashMap::new(),
            current_hyperlink_id: None,
            // Start at 1: hyperlink IDs are `NonZeroU32` on cells (ARC-010
            // niche optimization), so 0 is reserved for "no link" (None).
            next_hyperlink_id: 1,
        }
    }
}

/// OSC 52 clipboard core: current content, read/write-permission flags, per-slot
/// history, and per-slot size cap.
///
/// Extracted from `Terminal` for cohesion (ARC-001). Distinct from the
/// already-extracted `ClipboardSyncState` (Feature 30 event log).
pub(crate) struct ClipboardState {
    /// Clipboard content (OSC 52)
    pub(crate) clipboard_content: Option<String>,
    /// Allow clipboard read operations (security flag for OSC 52 queries)
    pub(crate) allow_clipboard_read: bool,
    /// Allow clipboard write operations (security flag for OSC 52 writes)
    pub(crate) allow_clipboard_write: bool,
    /// Clipboard history (multiple slots)
    pub(crate) clipboard_history: HashMap<ClipboardSlot, Vec<ClipboardEntry>>,
    /// Maximum clipboard history entries per slot
    pub(crate) max_clipboard_history: usize,
}

impl Default for ClipboardState {
    fn default() -> Self {
        Self {
            clipboard_content: None,
            allow_clipboard_read: false,
            allow_clipboard_write: true,
            clipboard_history: HashMap::new(),
            max_clipboard_history: 10,
        }
    }
}

/// DCS / Sixel graphics-decode state.
///
/// Holds the active Sixel parser, the DCS byte buffer, the active flag, and
/// the action character. Extracted from `Terminal` for cohesion (ARC-001).
pub(crate) struct DcsState {
    /// Current Sixel parser (active during DCS)
    pub(crate) sixel_parser: Option<sixel::SixelParser>,
    /// Buffer for DCS data accumulation
    pub(crate) dcs_buffer: Vec<u8>,
    /// DCS active flag
    pub(crate) dcs_active: bool,
    /// DCS action character ('q' for Sixel/XTGETTCAP/DECRQSS)
    pub(crate) dcs_action: Option<char>,
    /// Which DCS sub-protocol is active (disambiguates Sixel/XTGETTCAP/DECRQSS,
    /// which all share action 'q' but differ by intermediate bytes)
    pub(crate) dcs_kind: DcsKind,
    /// SEC-003: the in-flight DCS exceeded `MAX_DCS_BUFFER`; drop the whole
    /// sequence at unhook instead of processing a truncated payload.
    pub(crate) dcs_overflow: bool,
}

impl Default for DcsState {
    fn default() -> Self {
        Self {
            sixel_parser: None,
            dcs_buffer: Vec::new(),
            dcs_active: false,
            dcs_action: None,
            dcs_kind: DcsKind::Other,
            dcs_overflow: false,
        }
    }
}

/// DECSTBM/DECSLRM scroll + left/right margins (ARC-001 sub-struct)
pub(crate) struct MarginState {
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
}

impl MarginState {
    /// Full-screen margins for a `cols`×`rows` terminal — the geometry the
    /// constructor and every margin reset return to.
    pub(crate) fn new(cols: usize, rows: usize) -> Self {
        Self {
            scroll_region_top: 0,
            scroll_region_bottom: rows.saturating_sub(1),
            use_lr_margins: false,
            left_margin: 0,
            right_margin: cols.saturating_sub(1),
        }
    }
}

/// Saved dynamic- and ANSI-palette colors for one XTPUSHCOLORS stack entry (ENH-003)
#[derive(Clone)]
pub(crate) struct ColorPaletteSnapshot {
    /// Default foreground (OSC 10 target)
    pub(crate) default_fg: Color,
    /// Default background (OSC 11 target)
    pub(crate) default_bg: Color,
    /// Cursor color (OSC 12 target)
    pub(crate) cursor_color: Color,
    /// ANSI palette entries 0-15 (OSC 4 target)
    pub(crate) ansi_palette: [Color; 16],
}

/// Color theme: OSC-queryable colors + iTerm2-style rendering color prefs (ARC-001 sub-struct)
pub(crate) struct ColorThemeState {
    /// Default foreground color (for OSC 10 queries)
    pub(crate) default_fg: Color,
    /// Default background color (for OSC 11 queries)
    pub(crate) default_bg: Color,
    /// Cursor color (for OSC 12 queries)
    pub(crate) cursor_color: Color,
    /// ANSI color palette (0-15) - modified by OSC 4/104
    pub(crate) ansi_palette: [Color; 16],
    /// Embedder-configured palette (written by `set_ansi_palette_color`).
    /// RIS restores `ansi_palette` from this copy, so OSC 4 drift resets to
    /// the configured colors the way xterm restores its resource palette.
    pub(crate) configured_palette: [Color; 16],
    /// Color stack for XTPUSHCOLORS/XTPOPCOLORS (fg, bg, underline)
    pub(crate) color_stack: Vec<(Color, Color, Option<Color>)>,
    /// Palette stack for XTPUSHCOLORS/XTPOPCOLORS (dynamic + ANSI palette snapshots)
    pub(crate) palette_stack: Vec<ColorPaletteSnapshot>,
    /// High-water mark of palette_stack depth; XTREPORTCOLORS reports it after the
    /// current depth, matching xterm's s->last
    pub(crate) palette_stack_last: usize,
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
    pub(crate) faint_text_alpha: f32,
}

impl Default for ColorThemeState {
    fn default() -> Self {
        Self {
            default_fg: Color::Named(NamedColor::White),
            default_bg: Color::Named(NamedColor::Black),
            cursor_color: Color::Named(NamedColor::White),
            ansi_palette: Terminal::default_ansi_palette(),
            configured_palette: Terminal::default_ansi_palette(),
            color_stack: Vec::new(),
            palette_stack: Vec::new(),
            palette_stack_last: 0,
            // iTerm2 default colors (matching Python implementation)
            link_color: Color::Rgb(0x06, 0x45, 0xad), // RGB(0.023, 0.270, 0.678)
            bold_color: Color::Rgb(0xff, 0xff, 0xff), // RGB(1.0, 1.0, 1.0)
            cursor_guide_color: Color::Rgb(0xa6, 0xe8, 0xff), // RGB(0.650, 0.910, 1.000)
            badge_color: Color::Rgb(0xff, 0x00, 0x00), // RGB(1.0, 0.0, 0.0)
            match_color: Color::Rgb(0xff, 0xff, 0x00), // RGB(1.0, 1.0, 1.0)
            selection_bg_color: Color::Rgb(0xb5, 0xd5, 0xff), // #b5d5ff
            selection_fg_color: Color::Rgb(0x00, 0x00, 0x00), // #000000
            // iTerm2 default rendering control options
            use_bold_color: false,
            use_underline_color: false,
            use_cursor_guide: false,
            use_selected_text_color: false,
            smart_cursor_color: false,
            faint_text_alpha: 0.5, // 50% dimming for SGR 2 (faint/dim) text
        }
    }
}

/// DECSACE attribute-change extent: whether DECCARA/DECRARA change the
/// rectangle or the stream between the two corners (VT420).
///
/// The terminal starts in `Rectangle` to preserve the pre-DECSACE behavior;
/// VT420 powers on in stream mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeChangeExtent {
    /// Stream: everything in reading order from (top,left) to (bottom,right)
    Stream,
    /// Rectangle: strictly the addressed rectangle
    Rectangle,
}

/// VT operational modes toggled by DECSET/DECRST-style sequences (ARC-001 sub-struct)
pub(crate) struct TerminalModes {
    /// Auto wrap mode (DECAWM)
    pub(crate) auto_wrap: bool,
    /// Origin mode (DECOM) - cursor addressing relative to scroll region
    pub(crate) origin_mode: bool,
    /// Insert mode (IRM) - Mode 4: when enabled, new characters are inserted
    pub(crate) insert_mode: bool,
    /// Line Feed/New Line Mode (LNM) - Mode 20: when enabled, LF does CR+LF
    pub(crate) line_feed_new_line_mode: bool,
    /// Character protection mode (DECSCA) - when enabled, new chars are guarded
    pub(crate) char_protected: bool,
    /// Reverse video mode (DECSCNM) - globally inverts fg/bg colors
    pub(crate) reverse_video: bool,
    /// Application cursor keys mode
    pub(crate) application_cursor: bool,
    /// Application keypad mode (DECPAM/DECPNM, ESC = / ESC >)
    pub(crate) application_keypad: bool,
    /// Bracketed paste mode
    pub(crate) bracketed_paste: bool,
    /// Mouse tracking mode
    pub(crate) mouse_mode: MouseMode,
    /// Mouse encoding format
    pub(crate) mouse_encoding: MouseEncoding,
    /// Focus tracking enabled
    pub(crate) focus_tracking: bool,
    /// DECSACE extent for DECCARA/DECRARA attribute changes
    pub(crate) attribute_change_extent: AttributeChangeExtent,
    /// Sixel Display Mode (DECSDM, DECSET 80): when set, sixel graphics paint
    /// at the home position without scrolling; when reset (the default),
    /// they paint at the cursor with the scrolling-mode cursor advance.
    pub(crate) sixel_display_mode: bool,
}

impl Default for TerminalModes {
    fn default() -> Self {
        Self {
            auto_wrap: true,
            origin_mode: false,
            insert_mode: false,
            line_feed_new_line_mode: false,
            char_protected: false,
            reverse_video: false,
            application_cursor: false,
            application_keypad: false,
            bracketed_paste: false,
            mouse_mode: MouseMode::Off,
            mouse_encoding: MouseEncoding::Default,
            focus_tracking: false,
            attribute_change_extent: AttributeChangeExtent::Rectangle,
            sixel_display_mode: false,
        }
    }
}

/// Current SGR pen: the colors and flags applied to newly written cells
/// (ARC-002 sub-struct; the capability SGR and DECRQSS handlers take).
#[derive(Debug, Clone, Copy)]
pub(crate) struct TextAttributes {
    /// Current foreground color
    pub(crate) fg: Color,
    /// Current background color
    pub(crate) bg: Color,
    /// Current underline color (SGR 58) - None means use foreground color
    pub(crate) underline_color: Option<Color>,
    /// Current cell flags
    pub(crate) flags: CellFlags,
}

impl Default for TextAttributes {
    fn default() -> Self {
        Self {
            fg: Color::Named(NamedColor::White),
            bg: Color::Named(NamedColor::Black),
            underline_color: None,
            flags: CellFlags::default(),
        }
    }
}

/// DECSC/DECRC saved terminal state: saved cursor + saved SGR colors/flags (ARC-001 sub-struct)
pub(crate) struct SavedCursorState {
    /// Saved cursor position (for save/restore)
    pub(crate) saved_cursor: Option<Cursor>,
    /// Saved foreground color
    pub(crate) saved_fg: Color,
    /// Saved background color
    pub(crate) saved_bg: Color,
    /// Saved underline color (SGR 58)
    pub(crate) saved_underline_color: Option<Color>,
    /// Saved cell flags
    pub(crate) saved_flags: CellFlags,
}

impl Default for SavedCursorState {
    fn default() -> Self {
        Self {
            saved_cursor: None,
            // DECSC's saved SGR baseline mirrors the fresh-terminal colors.
            saved_fg: Color::Named(NamedColor::White),
            saved_bg: Color::Named(NamedColor::Black),
            saved_underline_color: None,
            saved_flags: CellFlags::default(),
        }
    }
}

impl SavedCursorState {
    /// DECSC / SCOSC: snapshot the cursor and the SGR pen.
    pub(crate) fn save(&mut self, cursor: &Cursor, attrs: &TextAttributes) {
        self.saved_cursor = Some(*cursor);
        self.saved_fg = attrs.fg;
        self.saved_bg = attrs.bg;
        self.saved_underline_color = attrs.underline_color;
        self.saved_flags = attrs.flags;
    }

    /// DECRC / SCORC: restore the snapshot; a no-op when nothing was saved.
    pub(crate) fn restore(&self, cursor: &mut Cursor, attrs: &mut TextAttributes) {
        if let Some(saved) = self.saved_cursor {
            *cursor = saved;
            attrs.fg = self.saved_fg;
            attrs.bg = self.saved_bg;
            attrs.underline_color = self.saved_underline_color;
            attrs.flags = self.saved_flags;
        }
    }
}

/// Feature 31 command/CWD execution history (ARC-001 sub-struct)
#[derive(Default)]
pub(crate) struct CommandHistoryState {
    /// Command execution history
    pub(crate) command_history: Vec<CommandExecution>,
    /// Current executing command
    pub(crate) current_command: Option<CommandExecution>,
    /// Working directory change history
    pub(crate) cwd_changes: Vec<CwdChange>,
}

/// Embedder configuration RIS must not touch (ARC-100).
///
/// `reset()` carries this struct across with one swap, so a field added
/// here survives `ESC c` by construction. Settings a program can also
/// change by escape sequence keep a `configured_*` baseline here while the
/// live value stays on `Terminal`; RIS restores the live value from the
/// baseline, never the other way round, so a program's change cannot
/// outlive a reset.
pub(crate) struct HostConfig {
    /// Bold brightening: bold ANSI colors 0-7 render as 8-15.
    pub(crate) bold_brightening: bool,
    /// OSC 1337 file-transfer payload cap, mirrored into
    /// `FileTransferManager` (which enforces it) by `apply_host_config`.
    pub(crate) max_transfer_size: usize,
    /// Host-supplied window position in pixels, for XTWINOPS 13 (`CSI 13 t`).
    pub(crate) window_position: (i32, i32),
    /// Host-supplied iconified state, for XTWINOPS 11 (`CSI 11 t`).
    pub(crate) window_iconified: bool,
    /// Maximum mouse events and positions retained.
    pub(crate) max_mouse_history: usize,
    /// Maximum inline images retained.
    pub(crate) max_inline_images: usize,
    /// Maximum command history entries.
    pub(crate) max_command_history: usize,
    /// Maximum CWD change history entries.
    pub(crate) max_cwd_history: usize,
    /// Conformance level RIS restores; DECSCL changes only the live level.
    pub(crate) configured_conformance_level: crate::conformance_level::ConformanceLevel,
    /// Warning bell volume RIS restores; DECSWBV changes only the live value.
    pub(crate) configured_warning_bell_volume: u8,
    /// Margin bell volume RIS restores; DECSMBV changes only the live value.
    pub(crate) configured_margin_bell_volume: u8,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            bold_brightening: true, // iTerm2 default behavior
            max_transfer_size: crate::terminal::file_transfer::DEFAULT_MAX_TRANSFER_SIZE,
            window_position: (0, 0),
            window_iconified: false,
            max_mouse_history: 100,
            max_inline_images: 100,
            max_command_history: 100,
            max_cwd_history: 50,
            // VT520 for maximum compatibility
            configured_conformance_level: crate::conformance_level::ConformanceLevel::default(),
            configured_warning_bell_volume: 4,
            configured_margin_bell_volume: 4,
        }
    }
}

/// Progress bars (OSC 9;4 + named OSC 934) + bell event counter (ARC-001 sub-struct)
#[derive(Default)]
pub(crate) struct ProgressBellState {
    /// Progress bar state from OSC 9;4 sequences (ConEmu/Windows Terminal style)
    pub(crate) progress_bar: ProgressBar,
    /// Named progress bars from OSC 934 sequences (keyed by ID)
    pub(crate) named_progress_bars: HashMap<String, NamedProgressBar>,
    /// Bell event counter - incremented each time bell (BEL/\x07) is received
    pub(crate) bell_count: u64,
}

impl ProgressBellState {
    /// Set or update a named progress bar and publish the change.
    pub(crate) fn set_named(&mut self, bar: NamedProgressBar, events: &mut EventBroker) {
        let id = bar.id.clone();
        let state = bar.state;
        let percent = bar.percent;
        let label = bar.label.clone();
        self.named_progress_bars.insert(id.clone(), bar);
        events.push(TerminalEvent::ProgressBarChanged {
            action: ProgressBarAction::Set,
            id,
            state: Some(state),
            percent: Some(percent),
            label,
        });
    }

    /// Remove a named progress bar; publishes and returns true only when it existed.
    pub(crate) fn remove_named(&mut self, id: &str, events: &mut EventBroker) -> bool {
        if self.named_progress_bars.remove(id).is_some() {
            events.push(TerminalEvent::ProgressBarChanged {
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

    /// Remove every named progress bar; publishes only when any existed.
    pub(crate) fn remove_all_named(&mut self, events: &mut EventBroker) {
        if !self.named_progress_bars.is_empty() {
            self.named_progress_bars.clear();
            events.push(TerminalEvent::ProgressBarChanged {
                action: ProgressBarAction::RemoveAll,
                id: String::new(),
                state: None,
                percent: None,
                label: None,
            });
        }
    }
}

/// Unicode width configuration + normalization form (ARC-001 sub-struct)
#[derive(Default)]
pub(crate) struct UnicodeConfigState {
    /// Unicode width configuration for character width calculations
    pub(crate) width_config: crate::unicode_width_config::WidthConfig,
    /// Unicode normalization form for text stored in cells
    pub(crate) normalization_form: crate::unicode_normalization_config::NormalizationForm,
}

/// Security flags: OSC 7 acceptance + insecure-sequence disable (ARC-001 sub-struct)
pub(crate) struct SecurityFlagsState {
    /// Accept OSC 7 directory tracking sequences
    pub(crate) accept_osc7: bool,
    /// Disable potentially insecure escape sequences
    pub(crate) disable_insecure_sequences: bool,
    /// Maximum total OSC data length in bytes before a sequence is rejected as
    /// a memory-exhaustion guard (QA-012/SEC-003). Defaults to 1 MiB; the
    /// pre-SEC-003 default of 128 MiB only bounded the *dispatch* check while
    /// vte's internal buffer grew unbounded. Enforced incrementally by the
    /// guard in [`Terminal::advance_parser`], which stops feeding payload
    /// bytes to vte once the in-flight OSC exceeds this. Deployments pushing
    /// larger inline images (iTerm2/Kitty base64) can raise it via
    /// [`Terminal::set_max_osc_data_length`].
    pub(crate) max_osc_data_length: usize,
    /// SEC-003 incremental OSC guard: previous byte was ESC (candidate OSC
    /// start).
    pub(crate) osc_seen_esc: bool,
    /// SEC-003 incremental OSC guard: inside an OSC payload.
    pub(crate) osc_in_osc: bool,
    /// SEC-003 incremental OSC guard: payload bytes fed for the in-flight OSC.
    pub(crate) osc_in_flight: usize,
    /// SEC-003 incremental OSC guard: set when the in-flight OSC exceeded
    /// `max_osc_data_length`; the next `osc_dispatch` is dropped whole.
    pub(crate) osc_discard_dispatch: bool,
}

impl Default for SecurityFlagsState {
    fn default() -> Self {
        Self {
            accept_osc7: true,
            disable_insecure_sequences: false,
            max_osc_data_length: DEFAULT_MAX_OSC_DATA_LENGTH,
            osc_seen_esc: false,
            osc_in_osc: false,
            osc_in_flight: 0,
            osc_discard_dispatch: false,
        }
    }
}

/// Default max OSC data length: 1 MiB (SEC-003; was 128 MiB, which only
/// checked at dispatch after vte had already buffered the payload).
/// cap: Payload bytes accepted for one OSC sequence from terminal output.
pub const DEFAULT_MAX_OSC_DATA_LENGTH: usize = 1024 * 1024;

/// OSC 1337 badge format string + session variables for evaluation (ARC-001 sub-struct)
pub(crate) struct BadgeState {
    /// Badge format string (from OSC 1337 SetBadgeFormat)
    /// Contains template with \(variable) placeholders
    pub(crate) badge_format: Option<String>,
    /// Session variables for badge format evaluation
    pub(crate) session_variables: crate::badge::SessionVariables,
}

impl BadgeState {
    /// The badge format evaluated against the session variables, or `None`
    /// when no format is set.
    pub(crate) fn evaluate(&self) -> Option<String> {
        self.badge_format
            .as_ref()
            .map(|format| crate::badge::evaluate_badge_format(format, &self.session_variables))
    }

    /// Set a user variable, publishing `UserVarChanged` only when the value changed.
    pub(crate) fn set_user_var(&mut self, name: String, value: String, events: &mut EventBroker) {
        let old_value = self.session_variables.custom.get(&name).cloned();
        let changed = old_value.as_deref() != Some(&value);
        self.session_variables
            .custom
            .insert(name.clone(), value.clone());
        if changed {
            events.push(TerminalEvent::UserVarChanged {
                name,
                value,
                old_value,
            });
        }
    }
}

/// Unified graphics/inline-image/file machinery: graphics store, Sixel limits,
/// cell pixel dimensions, iTerm2 multipart transfer state, file transfer manager.
/// (ARC-001 sub-struct)
#[doc(hidden)]
pub struct GraphicsState {
    /// Unified graphics storage (Sixel, iTerm2, Kitty)
    #[doc(hidden)]
    pub graphics_store: GraphicsStore,
    /// Sixel resource limits (per-terminal, for decoding)
    pub(crate) sixel_limits: sixel::SixelLimits,
    /// Cell dimensions in pixels (width, height) for sixel graphics
    /// Default (1, 2) is for text-mode TUI with half-block rendering
    /// Pixel renderers should set actual cell dimensions
    #[doc(hidden)]
    pub cell_dimensions: (u32, u32),
    /// iTerm2 multi-part image transfer state (MultipartFile/FilePart protocol)
    pub(crate) iterm_multipart_buffer: Option<ITermMultipartState>,
    /// File transfer manager for tracking file downloads and uploads
    pub(crate) file_transfer_manager: FileTransferManager,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            // GraphicsStore::default() is with_limits(GraphicsLimits::default())
            graphics_store: GraphicsStore::default(),
            sixel_limits: sixel::SixelLimits::default(),
            cell_dimensions: (1, 2), // Default for TUI half-block rendering
            iterm_multipart_buffer: None,
            file_transfer_manager: FileTransferManager::default(),
        }
    }
}
