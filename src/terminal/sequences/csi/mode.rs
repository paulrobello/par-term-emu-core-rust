//! Mode-related CSI sequence handling (SM/RM)

use crate::debug;
use crate::mouse::{MouseEncoding, MouseMode};
use crate::terminal::Terminal;
use vte::Params;

/// Every DEC private mode the terminal implements (DECSET/DECRST params,
/// ARC-009). The set/reset arms of [`Terminal::set_dec_private_mode`] are
/// this list; adding a mode means one arm there plus one label arm in
/// [`Terminal::dec_mode_label`]. Drives the set/reset/report symmetry test.
#[cfg(test)]
pub(crate) const DEC_PRIVATE_MODES: &[u16] = &[
    1, 6, 7, 9, 25, 47, 69, 80, 1000, 1002, 1003, 1004, 1005, 1006, 1015, 1047, 1048, 1049, 2004,
    2026,
];

impl Terminal {
    pub(crate) fn handle_csi_mode(&mut self, action: char, params: &Params, intermediates: &[u8]) {
        let private = intermediates.contains(&b'?');

        // Specialized handling for synchronized updates to ensure the sequence itself is processed
        // even if buffering is active
        let mut is_sync_update = false;
        if private {
            for param_slice in params {
                if param_slice.first() == Some(&2026) {
                    is_sync_update = true;
                    break;
                }
            }
        }

        if is_sync_update {
            self.sync_state.synchronized_updates = false;
            self.handle_csi_mode_impl(action, params, intermediates);
            // We do NOT restore synchronized_updates here because handle_csi_mode_impl
            // just set it to its new intended value (true for SM, false for RM).
        } else {
            self.handle_csi_mode_impl(action, params, intermediates);
        }
    }

    fn handle_csi_mode_impl(&mut self, action: char, params: &Params, intermediates: &[u8]) {
        let private = intermediates.contains(&b'?');
        match action {
            'h' => {
                // Set Mode (SM / DECSET)
                for param_slice in params {
                    let param = param_slice.first().copied().unwrap_or(0);
                    if private {
                        self.handle_decset(param);
                    } else {
                        match param {
                            4 if !self.modes.insert_mode => {
                                self.modes.insert_mode = true;
                                self.events.terminal_events.push(
                                    crate::terminal::TerminalEvent::ModeChanged(
                                        "insert_mode".to_string(),
                                        true,
                                    ),
                                );
                            }
                            20 if !self.modes.line_feed_new_line_mode => {
                                self.modes.line_feed_new_line_mode = true;
                                self.events.terminal_events.push(
                                    crate::terminal::TerminalEvent::ModeChanged(
                                        "line_feed_new_line_mode".to_string(),
                                        true,
                                    ),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
            'l' => {
                // Reset Mode (RM / DECRST)
                for param_slice in params {
                    let param = param_slice.first().copied().unwrap_or(0);
                    if private {
                        self.handle_decrst(param);
                    } else {
                        match param {
                            4 if self.modes.insert_mode => {
                                self.modes.insert_mode = false;
                                self.events.terminal_events.push(
                                    crate::terminal::TerminalEvent::ModeChanged(
                                        "insert_mode".to_string(),
                                        false,
                                    ),
                                );
                            }
                            20 if self.modes.line_feed_new_line_mode => {
                                self.modes.line_feed_new_line_mode = false;
                                self.events.terminal_events.push(
                                    crate::terminal::TerminalEvent::ModeChanged(
                                        "line_feed_new_line_mode".to_string(),
                                        false,
                                    ),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn handle_decset(&mut self, param: u16) {
        let old_label = self.dec_mode_label(param);
        self.set_dec_private_mode(param, true);
        self.emit_mode_changed(param, old_label, true);
    }

    pub(crate) fn handle_decrst(&mut self, param: u16) {
        let old_label = self.dec_mode_label(param);
        self.set_dec_private_mode(param, false);
        self.emit_mode_changed(param, old_label, false);
    }

    /// The current value label of one DEC private mode — the single copy of
    /// the mode table's read side, used for change detection around
    /// [`set_dec_private_mode`]. `None` = unrecognized. 1048 (save/restore)
    /// is an action, not a persistent mode, so it labels `None`.
    pub(crate) fn dec_mode_label(&self, param: u16) -> Option<String> {
        match param {
            1 => Some(format!("app_cursor:{}", self.modes.application_cursor)),
            6 => Some(format!("origin:{}", self.modes.origin_mode)),
            7 => Some(format!("wrap:{}", self.modes.auto_wrap)),
            25 => Some(format!("cursor_visible:{}", self.cursor.visible)),
            69 => Some(format!("lr_margins:{}", self.margins.use_lr_margins)),
            9 | 1000 | 1002 | 1003 => Some(format!("mouse:{:?}", self.modes.mouse_mode)),
            1005 | 1006 | 1015 => Some(format!("mouse_enc:{:?}", self.modes.mouse_encoding)),
            47 | 1047 | 1049 => Some(format!("alt_screen:{}", self.alt_screen_active)),
            1004 => Some(format!("focus_tracking:{}", self.modes.focus_tracking)),
            2004 => Some(format!("bracketed_paste:{}", self.modes.bracketed_paste)),
            2026 => Some(format!(
                "sync_updates:{}",
                self.sync_state.synchronized_updates
            )),
            80 => Some(format!("sixel_display:{}", self.modes.sixel_display_mode)),
            _ => None,
        }
    }

    /// Apply one DEC private mode — DECSET when `enabled`, DECRST when not —
    /// the single copy of the mode table's write side. Asymmetric modes keep
    /// an explicit per-direction body: mouse selection/encoding (each set
    /// param picks its own value, every reset lands on Off/Default), the
    /// alt-screen trio, and the synchronized-update flush on reset.
    pub(crate) fn set_dec_private_mode(&mut self, param: u16, enabled: bool) {
        match param {
            1 => self.modes.application_cursor = enabled,
            6 => {
                // Origin mode homes the cursor in both directions.
                self.modes.origin_mode = enabled;
                self.cursor.goto(0, 0); // Goto (0,0) within scroll region
            }
            7 => self.modes.auto_wrap = enabled,
            25 => self.cursor.visible = enabled,
            69 => self.margins.use_lr_margins = enabled,
            9 if enabled => self.modes.mouse_mode = MouseMode::X10,
            1000 if enabled => self.modes.mouse_mode = MouseMode::Normal,
            1002 if enabled => self.modes.mouse_mode = MouseMode::ButtonEvent,
            1003 if enabled => self.modes.mouse_mode = MouseMode::AnyEvent,
            9 | 1000 | 1002 | 1003 => self.modes.mouse_mode = MouseMode::Off,
            1005 if enabled => self.modes.mouse_encoding = MouseEncoding::Utf8,
            1006 if enabled => self.modes.mouse_encoding = MouseEncoding::Sgr,
            1015 if enabled => self.modes.mouse_encoding = MouseEncoding::Urxvt,
            1005 | 1006 | 1015 => self.modes.mouse_encoding = MouseEncoding::Default,
            47 => {
                if enabled {
                    self.enter_alt_screen(false);
                } else {
                    self.exit_alt_screen(false);
                }
            }
            1047 => {
                if enabled {
                    self.enter_alt_screen(true);
                } else {
                    self.exit_alt_screen(true);
                }
            }
            1048 => {
                if enabled {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if enabled {
                    self.use_alt_screen();
                } else {
                    self.use_primary_screen();
                }
            }
            1004 => self.modes.focus_tracking = enabled,
            2004 => self.modes.bracketed_paste = enabled,
            2026 => {
                self.sync_state.synchronized_updates = enabled;
                if !enabled {
                    self.sync_state.sync_update_explicitly_disabled = true;
                    self.flush_synchronized_updates();
                }
            }
            80 => self.modes.sixel_display_mode = enabled,
            _ => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "CSI",
                    &format!(
                        "Unsupported {}: {}",
                        if enabled { "DECSET" } else { "DECRST" },
                        param
                    ),
                );
            }
        }
    }

    /// Push the ModeChanged event when the mode's label actually moved —
    /// the shared tail of DECSET/DECRST. The alt-screen trio (47/1047/1049)
    /// emits from the screen-switch helpers instead.
    fn emit_mode_changed(&mut self, param: u16, old_label: Option<String>, enabled: bool) {
        if old_label == self.dec_mode_label(param) || matches!(param, 47 | 1047 | 1049) {
            return;
        }
        use crate::terminal::TerminalEvent;
        let mode_name = match param {
            1 => "application_cursor",
            4 => "insert_mode",
            6 => "origin_mode",
            7 => "auto_wrap",
            20 => "line_feed_new_line_mode",
            25 => "cursor_visible",
            69 => "lr_margins",
            9 => "mouse_x10",
            1000 => "mouse_normal",
            1002 => "mouse_button_event",
            1003 => "mouse_any_event",
            1004 => "focus_tracking",
            1005 => "mouse_utf8",
            1006 => "mouse_sgr",
            1015 => "mouse_urxvt",
            1049 => "alternate_screen",
            2004 => "bracketed_paste",
            2026 => "synchronized_updates",
            80 => "sixel_display_mode",
            _ => "unknown",
        };
        self.events
            .terminal_events
            .push(TerminalEvent::ModeChanged(mode_name.to_string(), enabled));
    }
}
