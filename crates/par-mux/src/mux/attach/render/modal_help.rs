//! The help overlay's state machine: enter/leave, the panel
//! refresh, its keys, wheel, and plain bytes.

use super::{HelpState, Modal, WindowSession};
use crate::mux::attach::panels::{
    compose_help_panel, help_content, help_rows, help_window_start, HELP_OVERLAY_TITLE,
};
use par_term_emu_core::keyboard::TermKeyEvent;

impl WindowSession {
    /// Open the bindings help panel (the `help` chord): the live chord
    /// state's categories composed into the bordered modal (title
    /// `keybinds`, `esc close` badge, footer controls line) painted
    /// centered over the frame. `/` opens a filter-as-you-type box, j/k
    /// and pgup/pgdn scroll, esc/Enter/q close and the prior frame
    /// repaints. Keys never reach the pane while it is up.
    pub(super) fn enter_help(&mut self) {
        self.open_modal(Modal::Help(HelpState::default()));
        self.refresh_help();
    }
    /// Dismiss the help panel: the next frame's pane repaint restores the
    /// covered cells (the frame buffer resets, then panes repaint).
    pub(super) fn leave_help(&mut self) {
        if matches!(self.modal, Modal::Help(_)) {
            self.modal = Modal::None;
        }
        self.renderer.set_overlay(None);
    }
    /// Re-compose the overlay from the live panel state (filter/scroll).
    pub(super) fn refresh_help(&mut self) {
        let Modal::Help(help) = &self.modal else {
            return;
        };
        let (filter, filtering, scroll) = (help.filter.clone(), help.filtering, help.scroll);
        let rows = help_rows(
            self.prefix,
            self.reload_key,
            self.management,
            self.resize_step,
        );
        // The modal's own chrome (2 border rows) and the footer surround
        // the content window; the filter line joins them only while a
        // filter is open or set (the footer already advertises
        // `search /` — no idle placeholder row).
        let filter_lines = usize::from(filtering || !filter.is_empty());
        let visible = usize::from(
            self.renderer
                .window_size()
                .1
                .saturating_sub(3 + filter_lines as u16),
        )
        .max(1);
        let content_len = help_content(&rows, &filter).len();
        let start = help_window_start(content_len, visible, scroll);
        let lines = compose_help_panel(&rows, &filter, filtering, visible, scroll);
        self.renderer.set_overlay(Some((
            HELP_OVERLAY_TITLE,
            lines,
            Some((start, visible, content_len)),
        )));
    }
    /// One key while the help panel is up: `/` opens the filter box
    /// (typing edits it, Enter commits and keeps the filter, Esc closes
    /// the panel), j/k/arrows/pgup/pgdn scroll, esc/Enter/q close. Every
    /// key is consumed — nothing leaks into the pane.
    pub(super) fn help_key(&mut self, ev: &TermKeyEvent) {
        use par_term_emu_core::keyboard::TermKey;
        let Modal::Help(help) = &mut self.modal else {
            return;
        };
        if help.filtering {
            match ev.key() {
                TermKey::Char => {
                    if let Some(ch) = char::from_u32(ev.codepoint) {
                        help.filter.push(ch);
                    }
                }
                TermKey::Escape => {
                    self.leave_help();
                    return;
                }
                _ => help.filtering = false,
            }
            self.refresh_help();
            return;
        }
        match (ev.key(), ev.modifiers) {
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'/') => {
                help.filtering = true;
                self.refresh_help();
            }
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'j') => self.help_scroll_by(1),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'k') => self.help_scroll_by(-1),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'q') => self.leave_help(),
            (TermKey::Up, 0) => self.help_scroll_by(-1),
            (TermKey::Down, 0) => self.help_scroll_by(1),
            (TermKey::PageUp, 0) => self.help_scroll_by(-10),
            (TermKey::PageDown, 0) => self.help_scroll_by(10),
            // Enter (arrives as a byte via route_plain) and everything
            // else close the panel.
            _ => self.leave_help(),
        }
    }
    /// Scroll the help panel's content window (clamped by the compose).
    pub(super) fn help_scroll_by(&mut self, delta: isize) {
        if let Modal::Help(help) = &mut self.modal {
            help.scroll = (help.scroll as isize + delta).max(0) as usize;
        }
        self.refresh_help();
    }
    /// One plain byte while the help panel is up — the same controls the
    /// key path takes, for the byte spellings (Backspace pops the filter,
    /// `/` opens it, Enter commits or closes, `q` closes).
    pub(super) fn help_byte(&mut self, byte: u8) -> bool {
        let Modal::Help(help) = &mut self.modal else {
            return false;
        };
        if help.filtering {
            match byte {
                0x7f => {
                    help.filter.pop();
                    self.refresh_help();
                }
                b'\r' => help.filtering = false,
                b if byte != 0x1b && (b.is_ascii_graphic() || b == b' ') => {
                    help.filter.push(b as char);
                    self.refresh_help();
                }
                b if b >= 0x80 => {
                    if let Some(ch) = self.text_utf8.push_printable(b) {
                        help.filter.push(ch);
                        self.refresh_help();
                    }
                }
                _ => {}
            }
            return matches!(self.modal, Modal::Help(_));
        }
        match byte {
            b'/' => {
                help.filtering = true;
                self.refresh_help();
            }
            b'q' | b'\r' => self.leave_help(),
            _ => {}
        }
        matches!(self.modal, Modal::Help(_))
    }
}
