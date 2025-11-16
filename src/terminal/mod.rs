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
use crate::cell::CellFlags;
use crate::color::{Color, NamedColor};
use crate::cursor::Cursor;
use crate::debug;
use crate::grid::Grid;
use crate::mouse::{MouseEncoding, MouseEvent, MouseMode};
use crate::shell_integration::ShellIntegration;
use crate::sixel;
use std::collections::HashMap;
use vte::{Params, Perform};

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

    /// Process input data
    pub fn process(&mut self, data: &[u8]) {
        debug::log_vt_input(data);

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
    /// * `word_chars` - Optional custom word characters (default: "-_.~:/?#[]@!$&'()*+,;=")
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

        TerminalStats {
            cols,
            rows,
            scrollback_lines: scrollback_len,
            total_cells,
            non_whitespace_lines,
            graphics_count,
            estimated_memory_bytes: estimated_memory,
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
            b'\x07' => {} // Bell - ignore for now
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
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("../tests/terminal_tests.rs");
    include!("../tests/grid_integration_tests.rs");
}
