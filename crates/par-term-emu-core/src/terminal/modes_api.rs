//! VT mode accessors and alternate-screen switching (ARC-003): mouse/focus/
//! paste/insert/LNM modes, DECSC/DECRC, margins, cursor style, and tab stops.

use super::Terminal;
use crate::color::Color;
use crate::cursor::CursorStyle;
use crate::debug;
use crate::mouse::{MouseEncoding, MouseMode};

impl Terminal {
    /// Set cursor style programmatically (bypasses DECSCUSR parsing)
    /// Use this when the terminal emulator's UI settings change the cursor style,
    /// rather than sending DECSCUSR escape sequences to the PTY.
    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor.set_style(style);
    }

    /// Check if alternate screen is active
    pub fn is_alt_screen_active(&self) -> bool {
        self.alt_screen_active
    }

    /// Switch to alternate screen
    pub fn use_alt_screen(&mut self) {
        self.enter_alt_screen(true);
    }

    /// Switch to primary screen
    pub fn use_primary_screen(&mut self) {
        self.exit_alt_screen(false);
    }

    /// Enter the alternate screen buffer (DECSET 47/1047/1049).
    /// `clear_alt` blanks the alternate grid on entry (1047/1049); mode 47
    /// preserves the alternate screen's prior contents. No-op when already
    /// on the alternate screen — xterm does not nest alt screens.
    pub(super) fn enter_alt_screen(&mut self, clear_alt: bool) {
        if !self.alt_screen_active {
            // Pending trigger rows index the primary grid; scan them before
            // the visible grid swaps underneath (ARC-064).
            self.flush_pending_trigger_rows();
            debug::log_screen_switch(true, "use_alt_screen");
            // Save current (primary) cursor position before switching
            let primary_cursor = self.cursor;
            self.alt_screen_active = true;
            // Restore alternate screen cursor (or use saved position)
            self.cursor = self.alt_cursor;
            // Save primary cursor for when we switch back
            self.alt_cursor = primary_cursor;
            if clear_alt {
                // Clear the alternate screen buffer to ensure it starts blank
                self.alt_grid.clear();
            }
            self.invalidate_visible_screen();
            // Notify about alt screen entry
            self.events
                .push(crate::terminal::TerminalEvent::ModeChanged(
                    "alternate_screen".to_string(),
                    true,
                ));
        }
    }

    /// Leave the alternate screen buffer (DECRST 47/1047/1049).
    /// `clear_alt` blanks the alternate grid before returning (1047); mode 47
    /// leaves its contents intact. No-op when already on the primary screen.
    pub(super) fn exit_alt_screen(&mut self, clear_alt: bool) {
        if self.alt_screen_active {
            // Pending trigger rows index the alternate grid; scan them
            // before the visible grid swaps back (ARC-064).
            self.flush_pending_trigger_rows();
            debug::log_screen_switch(false, "use_primary_screen");
            if clear_alt {
                self.alt_grid.clear();
            }
            // Save current (alternate) cursor position before switching
            let alt_cursor = self.cursor;
            self.alt_screen_active = false;
            // Restore primary cursor
            self.cursor = self.alt_cursor;
            // Save alternate cursor for when we switch back
            self.alt_cursor = alt_cursor;
            self.invalidate_visible_screen();
            // Reset keyboard protocol flags when exiting alternate screen
            // TUI apps may enable Kitty keyboard protocol and fail to disable it on exit
            if self.keyboard_state.keyboard_flags != 0 {
                self.keyboard_state.keyboard_flags = 0;
                self.events
                    .push(crate::terminal::TerminalEvent::ModeChanged(
                        "keyboard_protocol".to_string(),
                        false,
                    ));
            }
            self.keyboard_state.keyboard_stack_alt.clear();
            // Also reset modifyOtherKeys mode
            if self.keyboard_state.modify_other_keys_mode != 0 {
                self.keyboard_state.modify_other_keys_mode = 0;
                self.events
                    .push(crate::terminal::TerminalEvent::ModeChanged(
                        "modify_other_keys".to_string(),
                        false,
                    ));
            }
            // And focus tracking
            if self.modes.focus_tracking {
                self.modes.focus_tracking = false;
                self.events
                    .push(crate::terminal::TerminalEvent::ModeChanged(
                        "focus_tracking".to_string(),
                        false,
                    ));
            }
            // Notify about alt screen exit
            self.events
                .push(crate::terminal::TerminalEvent::ModeChanged(
                    "alternate_screen".to_string(),
                    false,
                ));
        }
    }

    /// Get mouse mode
    pub fn mouse_mode(&self) -> MouseMode {
        self.modes.mouse_mode
    }

    /// Set mouse mode
    pub fn set_mouse_mode(&mut self, mode: MouseMode) {
        self.modes.mouse_mode = mode;
    }

    /// Get mouse encoding
    pub fn mouse_encoding(&self) -> MouseEncoding {
        self.modes.mouse_encoding
    }

    /// Set mouse encoding
    pub fn set_mouse_encoding(&mut self, encoding: MouseEncoding) {
        self.modes.mouse_encoding = encoding;
    }

    /// Check if focus tracking is enabled
    pub fn focus_tracking(&self) -> bool {
        self.modes.focus_tracking
    }

    /// Set focus tracking
    pub fn set_focus_tracking(&mut self, enabled: bool) {
        self.modes.focus_tracking = enabled;
    }

    /// Save current cursor state
    pub fn save_cursor(&mut self) {
        self.saved_state.save(&self.cursor, &self.attrs);
    }

    /// Restore previously saved cursor state
    pub fn restore_cursor(&mut self) {
        self.saved_state.restore(&mut self.cursor, &mut self.attrs);
    }

    /// Check if bracketed paste is enabled
    pub fn bracketed_paste(&self) -> bool {
        self.modes.bracketed_paste
    }

    /// Set bracketed paste mode
    pub fn set_bracketed_paste(&mut self, enabled: bool) {
        self.modes.bracketed_paste = enabled;
    }

    /// Check if reverse video mode is enabled (DECSCNM)
    pub fn reverse_video(&self) -> bool {
        self.modes.reverse_video
    }

    /// Check if bold brightening is enabled
    /// When enabled, bold text with ANSI colors 0-7 brightens to 8-15
    pub fn bold_brightening(&self) -> bool {
        self.host.bold_brightening
    }

    /// Set bold brightening mode
    pub fn set_bold_brightening(&mut self, enabled: bool) {
        self.host.bold_brightening = enabled;
    }

    /// Get auto-wrap mode state
    pub fn auto_wrap_mode(&self) -> bool {
        self.modes.auto_wrap
    }

    /// Get origin mode state
    pub fn origin_mode(&self) -> bool {
        self.modes.origin_mode
    }

    /// Get application cursor mode state
    pub fn application_cursor(&self) -> bool {
        self.modes.application_cursor
    }

    /// Get application keypad mode state (DECPAM/DECPNM)
    pub fn application_keypad(&self) -> bool {
        self.modes.application_keypad
    }

    /// Get current scroll region (top, bottom)
    pub fn scroll_region(&self) -> (usize, usize) {
        (
            self.margins.scroll_region_top,
            self.margins.scroll_region_bottom,
        )
    }

    /// Get left and right margins
    pub fn left_right_margins(&self) -> (usize, usize) {
        (self.margins.left_margin, self.margins.right_margin)
    }

    /// Get ANSI color by index
    pub fn get_ansi_color(&self, index: usize) -> Option<Color> {
        self.theme.ansi_palette.get(index).cloned()
    }

    /// Get insert mode (IRM) state
    pub fn insert_mode(&self) -> bool {
        self.modes.insert_mode
    }

    /// Get line feed/new line mode (LNM) state
    pub fn line_feed_new_line_mode(&self) -> bool {
        self.modes.line_feed_new_line_mode
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
}
