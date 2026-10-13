//! The modal switch itself plus the modes without a sibling file: resize
//! mode and the scroll viewport's keys. Help, the pickers, the prompt, and
//! the context menus live in their modal_* siblings.

use super::renderer::{cols_overlap, rows_overlap};
use super::{Modal, WindowSession};
use par_term_emu_core::keyboard::TermKeyEvent;

impl WindowSession {
    /// Make `modal` the one that owns input, replacing any other. A
    /// replaced scroll viewport releases its hold on the focused pane
    /// (snapping it to live), since nothing would be left to exit it.
    pub(super) fn open_modal(&mut self, modal: Modal) {
        if matches!(self.modal, Modal::Scroll) && !matches!(modal, Modal::Scroll) {
            if let Some(id) = self.renderer.focused() {
                self.renderer.exit_scroll_mode(id);
            }
        }
        self.modal = modal;
    }
    /// Enter the sticky resize mode (the `resize` chord): arrows adjust
    /// the focused pane's edges until Enter/Escape/`q` — tmux's resize
    /// step with an explicit mode instead of repeat-time. The flash cue
    /// rides the frame cadence on the status row.
    pub(super) fn enter_resize(&mut self) {
        self.open_modal(Modal::Resize);
        self.chrome.flash(format!(
            "resize — arrows move the edge by {}, Enter/q exits",
            self.resize_step
        ));
    }
    /// One key while resize mode is up: arrows send one resize step for
    /// the focused pane (`resize-pane -t <pane> -L|-R|-U|-D <step>`, the
    /// wire's relative form); Escape and `q` exit; every other key exits
    /// the mode and is consumed (keys must not leak into the pane during
    /// a modal chord).
    pub(super) fn resize_key(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        use par_term_emu_core::keyboard::TermKey;
        match ev.key() {
            TermKey::Up if ev.modifiers == 0 => self.send_resize(conn, "-U"),
            TermKey::Down if ev.modifiers == 0 => self.send_resize(conn, "-D"),
            TermKey::Right if ev.modifiers == 0 => self.send_resize(conn, "-R"),
            TermKey::Left if ev.modifiers == 0 => self.send_resize(conn, "-L"),
            // Escape, q, Enter (as bytes via route_plain), or anything
            // else: the mode exits and the key is consumed.
            _ => self.modal = Modal::None,
        }
    }
    /// One resize step for the focused pane, the wire's relative form.
    /// Best-effort — the %layout-change broadcast the resize queues
    /// re-seeds the window through the pump's parked-layout path (PendingWork).
    pub(super) fn send_resize(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        flag: &str,
    ) {
        let focused = self.focused_pane();
        if focused.is_empty() {
            return;
        }
        // Divider-direction semantics: the arrows move the SHARED divider
        // in the pressed direction regardless of which side is focused
        // (the manual-pass report — the right pane used to invert). The
        // wire's resize-pane GROWS the focused pane, so when the focused
        // pane sits on the far side of its boundary (a neighbor to its
        // left, or above), the wire direction inverts.
        let focused_id: u32 = focused[1..].parse().unwrap_or(u32::MAX);
        let layout = self.renderer.layout();
        let invert = match layout.iter().find(|r| r.pane == focused_id) {
            Some(f) => match flag {
                "-L" | "-R" => layout
                    .iter()
                    .any(|r| r.pane != f.pane && r.x + r.width == f.x && rows_overlap(r, f)),
                "-U" | "-D" => layout
                    .iter()
                    .any(|r| r.pane != f.pane && r.y + r.height == f.y && cols_overlap(r, f)),
                _ => false,
            },
            None => false,
        };
        let flag = match (flag, invert) {
            ("-L", true) => "-R",
            ("-R", true) => "-L",
            ("-U", true) => "-D",
            ("-D", true) => "-U",
            (other, _) => other,
        };
        let command = format!("resize-pane -t {focused} {flag} {}", self.resize_step);
        let _ = conn.send_forget(&command);
    }
    /// Leave scroll mode: clear the hold and snap the focused pane to
    /// live.
    pub(super) fn leave_scroll_mode(&mut self) {
        if matches!(self.modal, Modal::Scroll) {
            self.modal = Modal::None;
        }
        if let Some(id) = self.renderer.focused() {
            self.renderer.exit_scroll_mode(id);
        }
    }
    /// One functional key while scroll mode is up: arrows line-scroll,
    /// PgUp/PgDn page by the pane's height, Home jumps to the top of
    /// history, End and q and Enter exit. Keys never reach the pane
    /// while the viewport is up.
    pub(super) fn scroll_mode_key(&mut self, ev: &TermKeyEvent) {
        use par_term_emu_core::keyboard::TermKey;
        let Some(id) = self.renderer.focused() else {
            return;
        };
        let rows = self
            .renderer
            .pane_terminal(id)
            .map(|t| t.active_grid().rows().max(1) as isize)
            .unwrap_or(1);
        match (ev.key(), ev.modifiers) {
            (TermKey::Up, 0) => {
                self.renderer.scroll_viewport(id, 1);
            }
            (TermKey::Down, 0) => {
                self.renderer.scroll_viewport(id, -1);
            }
            (TermKey::PageUp, 0) => {
                self.renderer.scroll_viewport(id, rows);
            }
            (TermKey::PageDown, 0) => {
                self.renderer.scroll_viewport(id, -rows);
            }
            (TermKey::Home, 0) => {
                let max = self
                    .renderer
                    .pane_terminal(id)
                    .map(|t| t.active_grid().scrollback_len() as isize)
                    .unwrap_or(0);
                self.renderer.scroll_viewport(id, max);
            }
            (TermKey::End, 0) => self.leave_scroll_mode(),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'q') => self.leave_scroll_mode(),
            _ => {}
        }
    }
}
