//! Terminal emulator implementation
//!
//! This module provides the main `Terminal` struct and its implementation,
//! split across multiple submodules for maintainability.

// Submodules
pub mod action;
mod apc_filter;
// Fuzz-only re-export (ENH-020): the module is pub(crate), the fuzz crate is
// an external path dependency, and cargo-fuzz is the only builder that sets
// the cfg.
#[cfg(fuzzing)]
pub use apc_filter::fuzz_apc_filter;
pub mod benchmarks;
pub mod clipboard;
mod colors;
pub mod compliance;
pub mod event;
mod event_broker;
// Consumed only by the Python event dicts and the FFI structured events.
#[cfg(any(feature = "python", feature = "ffi"))]
#[doc(hidden)]
pub mod event_fields;
pub mod file_transfer;
mod graphics;
pub mod image;
pub mod macros;
pub mod metrics;
mod mouse_api;
pub mod notification;
pub mod observer;
pub mod progress;
pub mod recording;
pub mod replay;
pub mod replay_snapshot;
pub mod screen;
pub mod search;
pub mod semantic_snapshot;
mod sequences;
pub mod shell_integration;
pub mod snapshot_manager;
mod state;
pub mod trigger;
mod write;
// Themed `impl Terminal` blocks split out of this hub file (ARC-003).
mod damage;
mod events_api;
mod host_config;
mod input;
mod keyboard_api;
mod modes_api;
mod notify_progress;
mod rect_api;
mod sixel_api;
mod title_badge;
mod tmux_api;
mod unicode_api;
mod zone_api;

// Re-export types as they're part of the public API
pub use benchmarks::TerminalBenchmarks;
pub use clipboard::{
    ClipboardEntry, ClipboardHistoryEntry, ClipboardOperation, ClipboardSlot, ClipboardSyncEvent,
    ClipboardTarget,
};
pub use colors::ResolvedCellColors;
pub use compliance::{ComplianceLevel, ComplianceReport, ComplianceTest};
pub use event::{BellEvent, CwdChange, ShellEvent, TerminalEvent, TerminalEventKind};
pub(crate) use event_broker::EventBroker;
pub use event_broker::ObserverDispatchBatch;
#[cfg(test)]
pub(crate) use event_broker::{MAX_BELL_EVENTS, MAX_TERMINAL_EVENTS};

/// Maximum undrained device-query response bytes retained (SEC-207). A
/// parse pass that would grow the queue past this drops the replies it
/// generated beyond it, so a query flood (`ESC[6n` …) into a terminal whose
/// host never drains responses cannot grow memory without bound. Bytes the
/// embedder pushes (`push_response`, uploads) are never dropped. A host that
/// drains every read never reaches it.
/// cap: Undrained device-query response bytes retained from processed output.
pub(crate) const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub use file_transfer::{
    FileTransfer, FileTransferManager, TransferDirection, TransferId, TransferStatus,
};
pub(crate) use image::ITermMultipartState;
pub use image::{ImageFormat, ImagePlacement, ImageProtocol, InlineImage};
pub use macros::MacroEngine;
pub use metrics::{
    BenchmarkCategory, BenchmarkResult, BenchmarkSuite, EscapeSequenceProfile, FrameTiming,
    PerformanceMetrics, ProfileCategory, ProfilingData, TerminalStats,
};
pub use notification::{
    Notification, NotificationAlert, NotificationConfig, NotificationEvent, NotificationTrigger,
    Urgency,
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
pub use semantic_snapshot::{
    diff_screen_lines, diff_snapshots, Bookmark, CommandInfo, CwdChangeInfo, DiffChangeType,
    ExportFormat, LineDiff, ScrollbackStats, SemanticSnapshot, SnapshotDiff, SnapshotScope,
    ZoneInfo,
};
pub use shell_integration::{CommandExecution, CommandOutput, ShellIntegrationStats};
// State sub-structs live in `state` (ARC-003); re-exported so every
// `crate::terminal::*` path keeps resolving.
pub(crate) use state::{
    ApcPrefilterState, BadgeState, BellVolumeState, BookmarksState, CharsetState, ClipboardState,
    ClipboardSyncState, ColorPaletteSnapshot, ColorThemeState, CommandHistoryState, DcsState,
    HostConfig, HyperlinkState, InlineImageState, KeyboardState, MarginState, MouseHistoryState,
    NotificationState, PointerShapeState, ProfilingState, ProgressBellState, RecordingState,
    RenderingState, SavedCursorState, SearchState, SecurityFlagsState, ShellState, SyncState,
    TerminalModes, TextAttributes, TitleState, TmuxState, UnicodeConfigState,
};
pub use state::{AttributeChangeExtent, Charset, GraphicsState, DEFAULT_MAX_OSC_DATA_LENGTH};
// Generic helpers relocated to `crate::text_utils` (ARC-009); re-exported so
// the public `crate::terminal::*` paths keep resolving.
pub use crate::text_utils::{cells_to_text, html_escape, unix_millis};
pub use trigger::{
    ActionResult, Trigger, TriggerAction, TriggerEngine, TriggerHighlight, TriggerId, TriggerMatch,
    TriggerRegistry, TriggerSplitCommand, TriggerSplitDirection, TriggerSplitTarget,
};

// Imports
use crate::cell::CellFlags;
use crate::color::{Color, NamedColor};
use crate::cursor::Cursor;
use crate::graphics::kitty::KittyParser;
use crate::graphics::GraphicsStore;
use crate::grid::Grid;
use crate::terminal::apc_filter::ApcFilterState;

const CLIPBOARD_TRUNCATION_SUFFIX: &str = " [truncated]";
/// Hard upper limit for clipboard content (10 MB), regardless of configured max_bytes
/// cap: Clipboard content bytes accepted from an OSC 52 sequence.
const MAX_CLIPBOARD_CONTENT_SIZE: usize = 10_485_760;

/// Truncate clipboard content in place to `max_bytes` (0 clears it), enforcing
/// the 10 MB hard cap and appending a truncation marker when cut.
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

/// A terminal emulator covering VT100/VT220/VT320/VT420/VT520 sequences with
/// iTerm2 feature parity: scrollback, true color, Sixel/iTerm2/Kitty graphics,
/// mouse reporting, and shell integration.
///
/// State is grouped into `pub(crate)` sub-structs by feature area (ARC-001).
/// Feed input bytes through [`Terminal::process`] and query grid/cursor state
/// through the accessor methods.
pub struct Terminal {
    /// The primary terminal grid
    #[doc(hidden)]
    pub grid: Grid,
    /// Alternate screen grid
    pub(crate) alt_grid: Grid,
    /// Last damage generation observed by the built-in default consumer
    /// (`get_dirty_rows`/`mark_clean`). External consumers keep their own
    /// remembered generations and are unaffected by `mark_clean`.
    pub(crate) default_consumer_gen: u64,
    /// Whether we're using the alternate screen
    pub(crate) alt_screen_active: bool,
    /// Cursor position and state
    pub(crate) cursor: Cursor,
    /// Saved cursor for alternate screen
    pub(crate) alt_cursor: Cursor,
    /// Current SGR pen: fg/bg/underline color + cell flags (ARC-002 sub-struct)
    pub(crate) attrs: TextAttributes,
    /// DECSC/DECRC saved terminal state: cursor + SGR colors/flags (ARC-001 sub-struct)
    pub(crate) saved_state: SavedCursorState,
    /// Window title, title stack, answerback string (ARC-001 sub-struct)
    pub(crate) title_state: TitleState,
    pub(crate) pointer_state: PointerShapeState,
    /// Synchronized update mode, buffer, disable-during-flush flag (ARC-001 sub-struct)
    pub(crate) sync_state: SyncState,
    /// Shell integration state, host/user, depth, command-output flag (ARC-001 sub-struct)
    pub(crate) shell_state: ShellState,
    /// DECSTBM/DECSLRM scroll + left/right margins (ARC-001 sub-struct)
    pub(crate) margins: MarginState,
    /// VT operational modes toggled by DECSET/DECRST-style sequences (ARC-001 sub-struct)
    pub(crate) modes: TerminalModes,
    /// Tab stops (columns where tab stops are set)
    pub(crate) tab_stops: Vec<bool>,
    /// Keyboard protocol flags, stacks, modifyOtherKeys mode (ARC-001 sub-struct)
    pub(crate) keyboard_state: KeyboardState,
    /// Response buffer for device queries (DA/DSR/etc). Parse-generated
    /// replies are capped at [`MAX_RESPONSE_BYTES`] per parse pass.
    pub(crate) response_buffer: Vec<u8>,
    /// Hyperlinks map, current ID, next ID (ARC-001 sub-struct)
    pub(crate) hyperlink_state: HyperlinkState,
    /// Unified graphics storage + Sixel limits + cell pixel dimensions +
    /// iTerm2 multipart transfer state + file transfer manager (ARC-001 sub-struct)
    #[doc(hidden)]
    pub graphics: GraphicsState,
    /// Sixel parser, DCS buffer, active flag, action char (ARC-001 sub-struct)
    pub(crate) dcs_state: DcsState,
    /// OSC 52 clipboard content, read flag, history, cap (ARC-001 sub-struct)
    pub(crate) clipboard_state: ClipboardState,
    /// Color theme: OSC-queryable colors + iTerm2-style rendering color prefs (ARC-001 sub-struct)
    pub(crate) theme: ColorThemeState,
    /// Notifications, config, events, silence/activity tracking (ARC-001 sub-struct)
    pub(crate) notifications_state: NotificationState,
    /// Progress bars + bell counter (ARC-001 sub-struct)
    pub(crate) progress_state: ProgressBellState,
    /// VTE parser instance (maintains state across process() calls)
    pub(crate) parser: vte::Parser,
    /// Kitty TGP APC pre-filter: filter state, payload accumulator, and
    /// reusable passthrough buffer (ARC-001 sub-struct)
    pub(crate) apc: ApcPrefilterState,
    /// SEC-003: scratch buffer reused by the incremental OSC guard in
    /// [`Terminal::advance_parser`] so guarded chunks (those containing ESC or
    /// mid-OSC state) do not allocate per call.
    pub(crate) osc_scratch: Vec<u8>,
    /// Long-lived Kitty TGP parser; reset between unrelated transmissions.
    pub(crate) kitty_parser: KittyParser,
    /// DECAWM delayed wrap: set after printing in last column
    pub(crate) pending_wrap: bool,
    /// Pixel width of the text area (XTWINOPS 14)
    #[doc(hidden)]
    pub pixel_width: usize,
    /// Pixel height of the text area (XTWINOPS 14)
    #[doc(hidden)]
    pub pixel_height: usize,
    /// Embedder configuration carried whole across RIS (ARC-100)
    pub(crate) host: HostConfig,
    /// Security flags: OSC 7 acceptance + insecure-sequence disable (ARC-001 sub-struct)
    pub(crate) security_state: SecurityFlagsState,
    /// Terminal conformance level (VT100/VT220/VT320/VT420/VT520)
    pub(crate) conformance_level: crate::conformance_level::ConformanceLevel,
    /// VT520 DECSWBV/DECSMBV warning + margin bell volumes (ARC-001 sub-struct)
    pub(crate) bell_volumes: BellVolumeState,
    /// tmux control-protocol state (ARC-001 sub-struct)
    pub(crate) tmux: TmuxState,
    /// Event queue, observer registry, and dispatch bookkeeping (ARC-002 broker)
    pub(crate) events: EventBroker,
    /// Current selection state
    pub(crate) selection: Option<Selection>,
    /// Bookmarks and next bookmark ID (ARC-001 sub-struct)
    pub(crate) bookmarks_state: BookmarksState,
    /// Performance metrics and profiling state (ARC-001 sub-struct)
    pub(crate) profiling: ProfilingState,
    /// Mouse event/position history (ARC-001 sub-struct)
    pub(crate) mouse_history: MouseHistoryState,
    /// Rendering hints and damage regions (ARC-001 sub-struct)
    pub(crate) rendering: RenderingState,
    /// Regex search state (ARC-001 sub-struct)
    pub(crate) search: SearchState,
    /// Inline image storage (ARC-001 sub-struct)
    pub(crate) inline_image_state: InlineImageState,

    // === Feature 30: OSC 52 Clipboard Sync ===
    /// OSC 52 clipboard-sync state (ARC-001 sub-struct)
    pub(crate) clipboard_sync: ClipboardSyncState,

    // === Feature 31: Shell Integration++ ===
    /// Command/CWD execution history (ARC-001 sub-struct)
    pub(crate) command_history_state: CommandHistoryState,

    // === Feature 24: Terminal Replay/Recording ===
    /// Recording session, active flag, start timestamp (ARC-001 sub-struct)
    pub(crate) recording_state: RecordingState,

    // === Feature 38: Macro Recording and Playback ===
    /// Macro library and playback state (ARC-001 sub-struct)
    pub(crate) macros: macros::MacroState,

    // === Answerback String (ENQ response) ===
    /// Answerback handled by `title_state` (ARC-001 sub-struct)

    /// Unicode width configuration + normalization form (ARC-001 sub-struct)
    pub(crate) unicode_state: UnicodeConfigState,

    // === Badge Support (OSC 1337 SetBadgeFormat) ===
    /// OSC 1337 badge format + session variables (ARC-001 sub-struct)
    pub(crate) badge_state: BadgeState,

    // === Feature 18: Triggers & Automation ===
    /// Trigger & automation state (ARC-001 sub-struct; owned by `trigger`, ARC-102)
    pub(crate) triggers: trigger::TriggerState,

    // === ACS (Alternate Character Set) state ===
    /// G0/G1 charset designations and active slot (ARC-001 sub-struct)
    pub(crate) charset_state: CharsetState,
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
    /// Create a new terminal with the default 10,000-line scrollback.
    pub fn new(cols: usize, rows: usize) -> Self {
        Self::with_scrollback(cols, rows, 10000)
    }

    /// Create a new terminal with custom scrollback size.
    ///
    /// Field defaults live with each sub-struct's `Default` impl (ARC-006);
    /// this constructor states only the geometry-derived fields: the grids,
    /// tab stops, margins, badge session variables, and pixel dimensions.
    pub fn with_scrollback(cols: usize, rows: usize, scrollback: usize) -> Self {
        // Initialize tab stops at every 8 columns
        let mut tab_stops = vec![false; cols];
        for i in (0..cols).step_by(8) {
            tab_stops[i] = true;
        }

        Self {
            grid: Grid::new(cols, rows, scrollback),
            alt_grid: Grid::new(cols, rows, 0), // Alt screen has no scrollback
            default_consumer_gen: 0,
            alt_screen_active: false,
            cursor: Cursor::new(),
            alt_cursor: Cursor::new(),
            attrs: TextAttributes::default(),
            saved_state: SavedCursorState::default(),
            title_state: TitleState::default(),
            pointer_state: PointerShapeState::default(),
            sync_state: SyncState::default(),
            shell_state: ShellState::default(),
            margins: MarginState::new(cols, rows),
            modes: TerminalModes::default(),
            tab_stops,
            keyboard_state: KeyboardState::default(),
            response_buffer: Vec::new(),
            hyperlink_state: HyperlinkState::default(),
            graphics: GraphicsState::default(),
            dcs_state: DcsState::default(),
            clipboard_state: ClipboardState::default(),
            theme: ColorThemeState::default(),
            progress_state: ProgressBellState::default(),
            parser: vte::Parser::new(),
            apc: ApcPrefilterState::default(),
            osc_scratch: Vec::new(),
            kitty_parser: KittyParser::new(),
            pending_wrap: false,
            // Initialize pixel dimensions with reasonable defaults (10x20 per cell)
            // This ensures CSI 14 t queries return valid pixel dimensions after resize
            pixel_width: cols * 10,
            pixel_height: rows * 20,
            host: HostConfig::default(),
            security_state: SecurityFlagsState::default(),
            // VT520 conformance level - default to VT520 for maximum compatibility
            conformance_level: crate::conformance_level::ConformanceLevel::default(),
            // VT520 bell volume controls - default to moderate volume (4)
            bell_volumes: BellVolumeState::default(),
            tmux: TmuxState::default(),
            events: EventBroker::default(),
            selection: None,
            bookmarks_state: BookmarksState::default(),
            profiling: ProfilingState::default(),
            mouse_history: MouseHistoryState::default(),
            rendering: RenderingState::default(),
            search: SearchState::default(),
            inline_image_state: InlineImageState::default(),
            clipboard_sync: ClipboardSyncState::default(),
            command_history_state: CommandHistoryState::default(),
            notifications_state: NotificationState::default(),
            recording_state: RecordingState::default(),
            macros: macros::MacroState::default(),
            unicode_state: UnicodeConfigState::default(),
            badge_state: BadgeState {
                badge_format: None,
                session_variables: crate::badge::SessionVariables::with_dimensions(
                    cols as u16,
                    rows as u16,
                ),
            },
            triggers: trigger::TriggerState::default(),
            charset_state: CharsetState::default(),
        }
    }

    /// Return the currently-active character set (G0 or G1 based on SO/SI state).
    #[inline]
    pub(crate) fn active_charset(&self) -> Charset {
        if self.charset_state.active_g == 1 {
            self.charset_state.g1_charset
        } else {
            self.charset_state.g0_charset
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
                lines.push(crate::text_utils::cells_to_text(line));
            }
        }
        lines
    }

    /// Get the cursor
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Get terminal dimensions (of the ACTIVE screen)
    ///
    /// Returns (cols, rows) for whichever screen buffer is currently active
    /// to avoid stale dimensions when the alternate screen is in use.
    pub fn size(&self) -> (usize, usize) {
        let g = self.active_grid();
        (g.cols(), g.rows())
    }

    /// Resize the terminal
    ///
    /// Ends with the same tail as [`Terminal::process`]: zones the reflow
    /// pushed off the scrollback floor are queued as `ZoneScrolledOut` and
    /// delivered to observers before this returns.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.resize_internal(cols, rows);
        self.finish_applied_actions();
    }

    /// [`Terminal::resize`] without invoking observer callbacks: the
    /// counterpart of [`Terminal::process_deferred`] for callers that hold
    /// an exclusive lock around the resize. Deliver the returned batch after
    /// releasing that lock.
    #[doc(hidden)]
    pub fn resize_deferred(&mut self, cols: usize, rows: usize) -> ObserverDispatchBatch {
        self.resize_internal(cols, rows);
        self.flush_evicted_zones();
        let batch = self.take_observer_dispatch_batch();
        self.cap_terminal_events();
        batch
    }

    fn resize_internal(&mut self, cols: usize, rows: usize) {
        crate::debug_log!("TERMINAL_RESIZE", "Requested resize to {}x{}", cols, rows);

        // Reflow moves rows across the scrollback boundary; pending trigger
        // indices cannot follow that, so scan before it happens (ARC-064).
        self.flush_pending_trigger_rows();

        // Both grids re-mark every row below; sync first so those stamps
        // outrun any generation a consumer captured earlier (ENH-025).
        self.sync_damage_generations();

        let old_cols = self.grid.cols().max(1);
        let old_rows = self.grid.rows().max(1);

        self.grid.resize(cols, rows);
        // The alt screen never reflows (see Grid::resize_without_reflow).
        self.alt_grid.resize_without_reflow(cols, rows);
        // Both grids' damage bitsets were resized and fully marked by their
        // resize calls: a resize reflows/redraws the whole screen, and a
        // damage-driven renderer (FFI readback) must repaint all of it.

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
        self.badge_state
            .session_variables
            .set_dimensions(cols as u16, rows as u16);

        crate::debug_trace!(
            "TERMINAL_RESIZE",
            "Applied resize: primary={}x{}, alt={}x{}, pixels={}x{}",
            self.grid.cols(),
            self.grid.rows(),
            self.alt_grid.cols(),
            self.alt_grid.rows(),
            self.pixel_width,
            self.pixel_height
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
        self.margins.scroll_region_top = 0;
        self.margins.scroll_region_bottom = rows.saturating_sub(1);
        crate::debug_log!(
            "TERMINAL_RESIZE",
            "Reset scroll region to full screen: 0-{}",
            self.margins.scroll_region_bottom
        );

        // Clamp left/right margins to new width
        self.margins.left_margin = self.margins.left_margin.min(cols.saturating_sub(1));
        self.margins.right_margin = self.margins.right_margin.min(cols.saturating_sub(1));
        if self.margins.left_margin > self.margins.right_margin {
            self.margins.left_margin = 0;
            self.margins.right_margin = cols.saturating_sub(1);
        }

        // Ensure cursor is within bounds
        let (active_cols, active_rows) = self.size();
        self.cursor.col = self.cursor.col.min(active_cols.saturating_sub(1));
        self.cursor.row = self.cursor.row.min(active_rows.saturating_sub(1));
        self.alt_cursor.col = self.alt_cursor.col.min(active_cols.saturating_sub(1));
        self.alt_cursor.row = self.alt_cursor.row.min(active_rows.saturating_sub(1));

        // Update session variables for badge evaluation
        self.badge_state
            .session_variables
            .set_dimensions(cols as u16, rows as u16);

        self.record_resize(cols, rows);
    }

    /// Check if synchronized updates mode is enabled
    pub fn synchronized_updates(&self) -> bool {
        self.sync_state.synchronized_updates
    }

    /// Flush the synchronized update buffer
    pub fn flush_synchronized_updates(&mut self) {
        if !self.sync_state.update_buffer.is_empty() {
            let buffer = std::mem::take(&mut self.sync_state.update_buffer);
            crate::debug_log!("SYNC_UPDATE", "Flushing buffer ({} bytes)", buffer.len());
            // Process the buffered data without synchronized mode
            let saved_mode = self.sync_state.synchronized_updates;
            self.sync_state.sync_update_explicitly_disabled = false;
            self.sync_state.synchronized_updates = false;
            self.process(&buffer);

            // Restore only if it was originally enabled and not explicitly disabled
            if saved_mode
                && !self.sync_state.sync_update_explicitly_disabled
                && !self.sync_state.synchronized_updates
            {
                self.sync_state.synchronized_updates = true;
            }
        }
    }

    /// Get the grid with scrollback applied (for screenshots/export)
    #[cfg(feature = "screenshot")]
    pub(crate) fn grid_with_scrollback(&self, scrollback_offset: usize) -> Grid {
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
        self.hyperlink_state.hyperlinks.get(&id).cloned()
    }

    /// Run the Kitty APC pre-filter on `data` and feed the non-APC remainder
    /// to the VTE parser.
    ///
    /// vte 0.15 does not deliver APC payload bytes to `Perform`, so Kitty
    /// graphics sequences (`ESC _ G ... ST`) must be intercepted before they
    /// reach the parser. Each completed Kitty APC payload is forwarded to
    /// [`KittyParser::parse_chunk`]; once the final chunk arrives (the APC
    /// did not request more chunks via `m=1`), [`KittyParser::build_graphic`]
    /// commits the result into [`GraphicsStore`]. Errors during parsing
    /// reset the Kitty parser and discard the payload — they never panic.
    fn filter_apc_and_advance(&mut self, data: &[u8]) {
        // Fast path (ARC-008): if the APC filter is idle and this chunk has no
        // ESC byte, no APC sequence can begin here — feed the bytes straight to
        // vte, skipping the passthrough copy and the filter pass entirely.
        if self.apc.filter == ApcFilterState::Outside && !data.contains(&0x1b) {
            self.advance_parser(data);
            return;
        }

        // Filter path. Reuse the passthrough buffer's capacity across calls
        // (ARC-008) instead of reallocating `Vec::with_capacity(data.len())`
        // on every `process()`. Take it out of `self` so feeding it to vte
        // below doesn't alias a `&mut self` borrow.
        let mut passthrough = std::mem::take(&mut self.apc.passthrough);
        passthrough.clear();
        // Record (passthrough_offset, payload) pairs so each APC is processed
        // with the cursor state produced by the passthrough bytes that
        // precede it in the stream (e.g. Herdr's CSI row;col H before each a=p).
        let mut completed_with_offsets: Vec<(usize, Vec<u8>)> = Vec::new();

        apc_filter::feed(
            &mut self.apc.filter,
            &mut self.apc.buffer,
            data,
            &mut passthrough,
            |apc, offset| {
                completed_with_offsets.push((offset, apc.payload.to_vec()));
            },
        );

        // Process each APC with the cursor state from its preceding
        // passthrough bytes, then advance the remaining tail.
        let mut prev_offset = 0;
        for (offset, payload_bytes) in completed_with_offsets {
            if offset > prev_offset {
                self.advance_parser(&passthrough[prev_offset..offset]);
            }
            prev_offset = offset;
            // Kitty payloads are ASCII text (key=value pairs + base64). Any
            // non-UTF-8 byte indicates a malformed APC; reset and skip.
            let payload = match std::str::from_utf8(&payload_bytes) {
                Ok(s) => s,
                Err(_) => {
                    self.kitty_parser.reset();
                    continue;
                }
            };

            match self.kitty_parser.parse_chunk(payload) {
                Ok(true) => {
                    // More chunks expected (m=1) — keep state, await next APC.
                }
                Ok(false) => {
                    // Final chunk: handle query (a=q) inline by emitting an
                    // APC OK reply on response_buffer; non-query actions go to
                    // build_graphic. See Kitty TGP spec, "Querying support".
                    if self.kitty_parser.action == crate::graphics::kitty::KittyAction::Query {
                        if self.kitty_parser.quietness < 2 {
                            let response = match self.kitty_parser.image_id {
                                Some(id) => format!("\x1b_Gi={};OK\x1b\\", id),
                                None => "\x1b_G;OK\x1b\\".to_string(),
                            };
                            self.response_buffer.extend_from_slice(response.as_bytes());
                        }
                        self.kitty_parser.reset();
                    } else {
                        // Errors here are non-fatal.
                        let position = (self.cursor.col, self.cursor.row);
                        match self
                            .kitty_parser
                            .build_graphic(position, &mut self.graphics.graphics_store)
                        {
                            Ok(crate::graphics::kitty::KittyGraphicResult::Graphic(
                                mut graphic,
                            )) => {
                                let (cell_width, cell_height) = self.graphics.cell_dimensions;
                                graphic.set_cell_dimensions(cell_width, cell_height);
                                let span_rows = graphic.cell_span(cell_width, cell_height).1;
                                let image_id = graphic.id;
                                self.graphics.graphics_store.add_graphic(graphic);
                                // Graphic landed on screen: notify like the Sixel
                                // (DCS) and iTerm2 inline paths do, so observers
                                // and streaming graphics subscribers hear kitty
                                // placements too. Row is the placement row,
                                // captured before any cursor move.
                                self.events
                                    .push(crate::terminal::TerminalEvent::GraphicsAdded {
                                        image_id,
                                        row: position.1,
                                    });
                                if !self.kitty_parser.suppress_cursor_move {
                                    // Kitty TGP: a placement at the cursor moves it
                                    // to the first line below the image unless C=1
                                    // suppresses the move. Written through the
                                    // normal newline path so the scroll region and
                                    // scrollback promotion apply exactly as they
                                    // do for a multi-row text write. Virtual
                                    // placements (U=1) never move the cursor.
                                    for _ in 0..span_rows {
                                        self.write_char('\n');
                                    }
                                    self.write_char('\r');
                                }
                            }
                            Ok(
                                crate::graphics::kitty::KittyGraphicResult::VirtualPlacement {
                                    ..
                                }
                                | crate::graphics::kitty::KittyGraphicResult::None,
                            )
                            | Err(_) => {}
                        }
                        self.kitty_parser.reset();
                    }
                }
                Err(_) => {
                    // Malformed chunk — reset and continue.
                    self.kitty_parser.reset();
                }
            }
        }

        // Feed the remaining passthrough tail to vte, then return the
        // (capacity-reused) buffer to the field for the next call.
        if prev_offset < passthrough.len() {
            self.advance_parser(&passthrough[prev_offset..]);
        }
        self.apc.passthrough = passthrough;
    }

    /// Feed bytes to the vte parser. `vte::Parser::advance` needs `&mut self`
    /// (as the `Perform` impl) and `&mut parser`, but `parser` is a field of
    /// `self` — temporarily move it out to satisfy the borrow checker.
    ///
    /// SEC-003: enforces `max_osc_data_length` incrementally. vte 0.15 has no
    /// per-byte OSC hook (its internal `osc_raw` Vec grows unbounded under
    /// the `std` feature), so the guard lives here, at the only layer we
    /// control: payload bytes past the cap are never fed to vte, terminators
    /// always are (so vte's own state machine never desyncs), and the
    /// resulting truncated dispatch is dropped whole via
    /// `osc_discard_dispatch` in `osc_dispatch_impl`.
    fn advance_parser(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let needs_guard = self.security_state.osc_in_osc
            || self.security_state.osc_seen_esc
            || bytes.contains(&0x1b);
        if !needs_guard {
            let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());
            parser.advance(self, bytes);
            self.parser = parser;
            return;
        }

        let mut kept = std::mem::take(&mut self.osc_scratch);
        kept.clear();
        for &b in bytes {
            if self.osc_guard_step(b) {
                kept.push(b);
            }
        }
        let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());
        parser.advance(self, &kept);
        self.parser = parser;
        self.osc_scratch = kept;
    }

    /// One step of the SEC-003 incremental OSC guard. Updates the tracking
    /// state and returns `false` when the byte is an over-cap OSC payload
    /// byte that must not reach vte. Mirrors vte's OSC state transitions: the
    /// payload ends at BEL, ESC, CAN, or SUB.
    fn osc_guard_step(&mut self, b: u8) -> bool {
        let s = &mut self.security_state;
        if s.osc_seen_esc {
            if b == 0x1b {
                // Consecutive ESC: vte restarts the escape; stay armed.
                return true;
            }
            s.osc_seen_esc = false;
            if b == b']' {
                s.osc_in_osc = true;
                s.osc_in_flight = 0;
            }
            return true;
        }
        if s.osc_in_osc {
            match b {
                0x1b | 0x07 | 0x18 | 0x1a => {
                    // BEL/ESC terminate (vte dispatches), CAN/SUB abort
                    // (no dispatch): either way the tracked sequence ends.
                    s.osc_in_osc = false;
                    s.osc_in_flight = 0;
                    if b == 0x18 || b == 0x1a {
                        s.osc_discard_dispatch = false;
                    }
                    return true;
                }
                _ => {
                    if s.osc_in_flight >= s.max_osc_data_length {
                        if !s.osc_discard_dispatch {
                            s.osc_discard_dispatch = true;
                            crate::debug_log!(
                                "SECURITY",
                                "OSC payload exceeds max_osc_data_length ({} bytes), dropping sequence",
                                s.max_osc_data_length
                            );
                        }
                        return false;
                    }
                    s.osc_in_flight += 1;
                    return true;
                }
            }
        }
        if b == 0x1b {
            s.osc_seen_esc = true;
        }
        true
    }

    /// Process incoming data from the PTY
    pub fn process(&mut self, data: &[u8]) {
        if !self.process_internal(data) {
            return;
        }
        self.finish_applied_actions();
    }

    /// Post-parse tail shared by [`Terminal::process`] and
    /// [`Terminal::apply_action`]/[`Terminal::apply_actions`]: queue evicted
    /// zones as `ZoneScrolledOut`, deliver pending events to observers
    /// inline, then cap the event queue.
    pub(crate) fn finish_applied_actions(&mut self) {
        self.flush_evicted_zones();
        self.dispatch_events();
        self.cap_terminal_events();
    }

    /// Drop the replies a parse pass added past [`MAX_RESPONSE_BYTES`].
    /// `before` is the buffer length when the pass began: bytes already
    /// queued (embedder pushes, earlier passes) are never removed, so only
    /// output-driven growth is bounded. The escape-sequence handlers write
    /// `response_buffer` directly (they take it as a `&mut Vec<u8>`
    /// capability), so every parse entry point ends here.
    pub(crate) fn cap_parsed_responses(&mut self, before: usize) {
        self.response_buffer
            .truncate(before.max(MAX_RESPONSE_BYTES));
    }

    /// Process incoming data from the PTY without invoking observer
    /// callbacks synchronously (ARC-001).
    ///
    /// Identical to [`Terminal::process`] for all internal state (grid,
    /// cursor, event queue, dispatched-index bookkeeping); the only
    /// difference is that observer delivery is deferred instead of run
    /// inline. Returns an [`ObserverDispatchBatch`] that the caller must
    /// deliver via [`ObserverDispatchBatch::deliver`] — ideally *after*
    /// releasing any exclusive lock held around this call.
    ///
    /// Intended for callers that hold an exclusive lock around `process()`
    /// (e.g. `PtySession`'s `RwLock<Terminal>` write guard in its reader
    /// thread). Observer callbacks can be slow or re-entrant (a
    /// `PyCallbackObserver` re-enters Python under the GIL); running them
    /// while the exclusive lock is held blocks every concurrent reader
    /// (streaming clients, Python queries) — the exact problem the
    /// `Mutex`→`RwLock` migration was meant to solve. Call this instead of
    /// `process()`, drop the lock, then call `deliver()` on the batch.
    pub fn process_deferred(&mut self, data: &[u8]) -> ObserverDispatchBatch {
        if !self.process_internal(data) {
            return ObserverDispatchBatch::default();
        }
        self.flush_evicted_zones();
        let batch = self.take_observer_dispatch_batch();
        self.cap_terminal_events();
        batch
    }

    /// Shared parsing/state-mutation body for [`Terminal::process`] and
    /// [`Terminal::process_deferred`]. Returns `false` if the caller should
    /// skip dispatch/cap-eviction for this call — mirrors `process()`'s
    /// original early return when data is being buffered for synchronized-
    /// update coalescing rather than parsed immediately.
    fn process_internal(&mut self, data: &[u8]) -> bool {
        let responses_before = self.response_buffer.len();
        if self.recording_state.is_recording {
            self.record_event(RecordingEventType::Output, data.to_vec());
        }

        if self.sync_state.synchronized_updates {
            // Buffer data instead of processing it immediately
            self.sync_state.update_buffer.extend_from_slice(data);

            // Peek at the end of the buffer to see if it contains the disable sequence
            // We check the last 32 bytes to account for sequences split across chunks
            let peek_len = 32.min(self.sync_state.update_buffer.len());
            let peek_start = self.sync_state.update_buffer.len() - peek_len;
            if contains_bytes(&self.sync_state.update_buffer[peek_start..], b"\x1b[?2026l") {
                self.flush_synchronized_updates();
            }
            return false;
        }

        if self.tmux.tmux_parser.is_control_mode() || self.tmux.tmux_parser.is_auto_detect() {
            // Process as tmux control protocol (handles auto-detect internally)
            let notifications = self.tmux.tmux_parser.parse(data);
            for notification in notifications {
                match notification {
                    crate::tmux_control::TmuxNotification::TerminalOutput { data } => {
                        // Feed non-control data back to standard VTE parser
                        // (with Kitty APC pre-filtering)
                        self.filter_apc_and_advance(&data);
                    }
                    _ => {
                        // Store tmux notification
                        self.tmux.tmux_notifications.push(notification);
                    }
                }
            }
        } else {
            // Process as standard terminal output (with Kitty APC pre-filtering)
            self.filter_apc_and_advance(data);
        }
        self.cap_parsed_responses(responses_before);

        true
    }

    /// Evict the oldest terminal events when the queue exceeds the cap
    /// (ARC-006). Bounds memory under sustained output when the host polls
    /// infrequently; shifts `events_dispatched_up_to` so observer dispatch
    /// stays consistent with the moved positions. Front (oldest) events are
    /// dropped — preferentially ones already dispatched to observers.
    fn cap_terminal_events(&mut self) {
        self.events.cap_pending();
    }

    /// Dispatch pending events to all registered observers, inline.
    /// Uses `events_dispatched_up_to` index to avoid sending duplicate events
    /// when `process()` is called multiple times before `poll_events()`.
    ///
    /// Used by `process()`, which callers may invoke while holding an
    /// exclusive lock — see [`Terminal::process_deferred`] and
    /// [`ObserverDispatchBatch`] for the ARC-001 alternative that lets
    /// callers deliver observer callbacks after releasing that lock.
    fn dispatch_events(&mut self) {
        self.take_observer_dispatch_batch().deliver();
    }

    /// Extract the not-yet-dispatched events plus a snapshot of the
    /// currently registered observers into an owned [`ObserverDispatchBatch`],
    /// advancing `events_dispatched_up_to` so the same events aren't
    /// re-dispatched later. Pure bookkeeping — no observer callback runs
    /// here, so this is safe to call while holding an exclusive lock (unlike
    /// [`ObserverDispatchBatch::deliver`]).
    fn take_observer_dispatch_batch(&mut self) -> ObserverDispatchBatch {
        self.events.take_dispatch_batch()
    }

    /// Reset the terminal to its initial state (RIS).
    ///
    /// xterm semantics: RIS clears the *terminal* — grids, scrollback,
    /// cursor, SGR, modes, margins, charsets, keyboard protocol,
    /// hyperlinks, graphics, title, selection — but not the embedder's
    /// configuration.
    ///
    /// The crate-private `HostConfig` is carried whole with one swap (ARC-100): the OSC
    /// 1337 transfer cap, bold brightening, window position/iconified
    /// state, and the mouse/inline-image/command/cwd history caps. The
    /// conformance level and bell volumes, which programs can also set
    /// (DECSCL/DECSWBV/DECSMBV), restore to the host-configured baseline,
    /// not the live value. The fields below the swap are still carried one
    /// by one (moving them into `HostConfig` is ARC-102): security policy
    /// ([`set_accept_osc7`](Self::set_accept_osc7),
    /// [`set_disable_insecure_sequences`](Self::set_disable_insecure_sequences),
    /// [`set_max_osc_data_length`](Self::set_max_osc_data_length)), file-media and clipboard policy,
    /// graphics/clipboard limits, the answerback string, theme colors,
    /// unicode config, observers, event subscriptions, triggers, macros,
    /// notification config, the badge format, active recordings, tmux
    /// control flags, pixel dimensions, and profiling (ARC-058). DECSTR
    /// (CSI ! p) is the softer [`Terminal::soft_reset`], which resets
    /// modes without touching the screen.
    pub fn reset(&mut self) {
        let (cols, rows) = self.size();
        let scrollback = self.grid.max_scrollback();

        // xterm keeps user-configured tab stops across RIS.
        let tab_stops = self.tab_stops.clone();

        let mut fresh = Self::with_scrollback(cols, rows, scrollback);

        std::mem::swap(&mut fresh.host, &mut self.host);
        fresh.apply_host_config();

        // Security policy survives; the in-flight OSC-guard fields reset
        // with the parser state they track.
        fresh.security_state.accept_osc7 = self.security_state.accept_osc7;
        fresh.security_state.disable_insecure_sequences =
            self.security_state.disable_insecure_sequences;
        fresh.security_state.max_osc_data_length = self.security_state.max_osc_data_length;

        // Kitty host policy survives; parser transmission state resets.
        fresh.kitty_parser.allow_file_media = self.kitty_parser.allow_file_media;
        fresh.kitty_parser.retain_temp_files = self.kitty_parser.retain_temp_files;

        // Graphics limits survive; store contents reset with the grid.
        fresh.graphics.sixel_limits = self.graphics.sixel_limits;
        fresh.graphics.cell_dimensions = self.graphics.cell_dimensions;
        fresh.graphics.graphics_store =
            GraphicsStore::with_limits(*self.graphics.graphics_store.limits());

        // Answerback is embedder identification, not terminal state.
        fresh.title_state.answerback_string = self.title_state.answerback_string.take();

        // Clipboard policy and caps survive; content and history reset.
        fresh.clipboard_state.allow_clipboard_read = self.clipboard_state.allow_clipboard_read;
        fresh.clipboard_state.allow_clipboard_write = self.clipboard_state.allow_clipboard_write;
        fresh.clipboard_state.max_clipboard_history = self.clipboard_state.max_clipboard_history;
        fresh.clipboard_sync.max_history = self.clipboard_sync.max_history;
        fresh.clipboard_sync.max_events = self.clipboard_sync.max_events;
        fresh.clipboard_sync.max_event_bytes = self.clipboard_sync.max_event_bytes;
        fresh.clipboard_sync.remote_session_id = self.clipboard_sync.remote_session_id.take();

        // Theme survives wholesale, except the OSC-driven parts: palette
        // drift resets to the configured copy and the XTPUSHCOLORS stacks
        // clear.
        std::mem::swap(&mut fresh.theme, &mut self.theme);
        fresh.theme.ansi_palette = fresh.theme.configured_palette;
        fresh.theme.color_stack.clear();
        fresh.theme.palette_stack.clear();
        fresh.theme.palette_stack_last = 0;

        std::mem::swap(&mut fresh.unicode_state, &mut self.unicode_state);

        // Observers keep their registration and ID sequence, and the
        // subscription filter survives; buffered events reset with
        // everything else they describe.
        fresh.events.carry_observers_from(&mut self.events);

        // The trigger registry survives; highlights, action results, and
        // pending scan rows reset.
        fresh.triggers.carry_registry_from(&mut self.triggers);

        fresh.macros.carry_from(&mut self.macros);

        std::mem::swap(
            &mut fresh.notifications_state.notification_config,
            &mut self.notifications_state.notification_config,
        );
        fresh.notifications_state.max_notifications = self.notifications_state.max_notifications;
        std::mem::swap(
            &mut fresh.notifications_state.custom_triggers,
            &mut self.notifications_state.custom_triggers,
        );

        std::mem::swap(
            &mut fresh.badge_state.badge_format,
            &mut self.badge_state.badge_format,
        );

        // An in-flight recording keeps recording.
        std::mem::swap(&mut fresh.recording_state, &mut self.recording_state);

        // tmux control-mode flag and auto-detect survive; buffered
        // notifications reset.
        std::mem::swap(&mut fresh.tmux.tmux_parser, &mut self.tmux.tmux_parser);

        fresh.pixel_width = self.pixel_width;
        fresh.pixel_height = self.pixel_height;
        std::mem::swap(&mut fresh.profiling, &mut self.profiling);

        // RIS swaps in a fresh Terminal whose grid counters start at zero;
        // carry the old maximum so consumers holding pre-reset generations
        // still see the reset screen as fully dirty (ENH-025).
        let max_gen = self.grid.generation().max(self.alt_grid.generation());

        *self = fresh;
        self.tab_stops = tab_stops;

        self.grid.raise_generation(max_gen);
        self.invalidate_visible_screen();
    }

    /// Push a freshly swapped-in [`HostConfig`] into the state that
    /// enforces or mirrors it (ARC-100).
    ///
    /// Two things only: a cap enforced by a public sub-type that keeps its
    /// own copy (`FileTransferManager`; any future cap of that kind goes
    /// here too), and the live values of the program-settable settings,
    /// which restart from the host-configured baseline.
    fn apply_host_config(&mut self) {
        self.graphics
            .file_transfer_manager
            .set_max_transfer_size(self.host.max_transfer_size);
        self.conformance_level = self.host.configured_conformance_level;
        self.bell_volumes.warning = self.host.configured_warning_bell_volume;
        self.bell_volumes.margin = self.host.configured_margin_bell_volume;
    }

    /// Soft terminal reset (DECSTR, `CSI ! p`) per VT510.
    ///
    /// Resets the state a program can set through escapes — cursor
    /// visibility (DECTCEM on) and position (home), IRM/DECOM/DECAWM/
    /// DECCKM/DECKPAM, selective erase (DECSCA), scroll margins (DECSTBM
    /// full screen), charsets (G0-G3 ASCII, GL=G0), SGR defaults, DECSDM,
    /// DECSACE, pushed colors, and the DECSC saved state (home, default
    /// attributes) — while leaving screen content, scrollback, and all
    /// embedder configuration untouched.
    pub fn soft_reset(&mut self) {
        let (cols, rows) = self.size();

        self.cursor.col = 0;
        self.cursor.row = 0;
        self.cursor.visible = true;
        self.modes.insert_mode = false;
        self.modes.origin_mode = false;
        self.modes.auto_wrap = true;
        self.modes.application_cursor = false;
        self.modes.application_keypad = false;
        self.modes.char_protected = false;
        self.modes.sixel_display_mode = false;
        self.modes.attribute_change_extent = AttributeChangeExtent::Rectangle;
        self.margins = MarginState::new(cols, rows);

        // SGR default — the fresh-terminal baseline.
        self.attrs.fg = Color::Named(NamedColor::White);
        self.attrs.bg = Color::Named(NamedColor::Black);
        self.attrs.underline_color = None;
        self.attrs.flags = CellFlags::default();

        self.charset_state = CharsetState::default();

        // DECSC saved state: home with default attributes.
        self.saved_state = SavedCursorState {
            saved_cursor: Some(Cursor::new()),
            ..SavedCursorState::default()
        };

        // Pushed colors are discarded.
        self.theme.color_stack.clear();
        self.theme.palette_stack.clear();
        self.theme.palette_stack_last = 0;

        self.pending_wrap = false;
    }

    /// Push bytes to the response buffer (to be sent back to PTY)
    pub fn push_response(&mut self, bytes: &[u8]) {
        self.response_buffer.extend_from_slice(bytes);
    }
}

mod perform;

#[cfg(test)]
mod tests;
