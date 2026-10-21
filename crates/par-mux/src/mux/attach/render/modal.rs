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
        self.text_utf8 = Default::default();
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
        self.renderer.set_selection(None);
        self.copy_anchor = None;
        self.scroll_query = None;
        self.scroll_matches.clear();
        self.scroll_match_ix = 0;
        if let Some(id) = self.renderer.focused() {
            self.renderer.exit_scroll_mode(id);
        }
    }
    /// One key while scroll mode is up, from either input path (a Key
    /// token or a plain byte): `e` opens the scrollback in the editor
    /// (it needs the connection), everything else is
    /// [`Self::scroll_mode_key`]'s.
    pub(super) fn scroll_mode_input(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        use par_term_emu_core::keyboard::TermKey;
        if ev.key() == TermKey::Char && ev.modifiers == 0 && ev.codepoint == u32::from(b'e') {
            self.edit_scrollback(conn);
            return;
        }
        self.scroll_mode_key(ev);
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
            // The copy-mode extension (the TUI audit's R1): v anchors the
            // line selection, y copies it (OSC 52), / types a search the
            // committed query of which n/N walk.
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'v') => {
                if let Some(id) = self.renderer.focused() {
                    self.toggle_line_selection(id);
                }
            }
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'y') => self.copy_selection(),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'/') => self.begin_scroll_search(),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'n') => self.scroll_search(1),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'N') => self.scroll_search(-1),
            _ => {}
        }
        // The viewport moved: while a selection is up, its head follows.
        if let Some(id) = self.renderer.focused() {
            self.update_selection_head(id);
        }
    }

    /// Toggle the line selection: `v` anchors at the viewport's top line
    /// and the scrolling viewport extends it, a second `v` clears.
    pub(super) fn toggle_line_selection(&mut self, pane: u32) {
        if self.renderer.selection.is_some() {
            self.renderer.set_selection(None);
            self.copy_anchor = None;
            self.chrome.flash("selection cleared".to_string());
            self.draw_status_row();
            return;
        }
        let Some(top) = self.renderer.viewport_top_line(pane) else {
            return;
        };
        self.copy_anchor = Some(top);
        self.renderer.set_selection(Some(super::Selection::Lines {
            pane,
            a: top,
            b: top,
        }));
        self.chrome
            .flash("selecting lines — scroll, then y copies".to_string());
        self.draw_status_row();
    }

    /// While a selection is up, the viewport's top line is its moving
    /// head.
    pub(super) fn update_selection_head(&mut self, pane: u32) {
        let Some(super::Selection::Lines {
            pane: p,
            a: anchor,
            b: _,
        }) = self.renderer.selection
        else {
            return;
        };
        if p != pane {
            return;
        }
        let Some(top) = self.renderer.viewport_top_line(pane) else {
            return;
        };
        self.renderer.set_selection(Some(super::Selection::Lines {
            pane: p,
            a: anchor.min(top),
            b: anchor.max(top),
        }));
    }

    /// y: copy the selected lines' text as an OSC 52 clipboard write —
    /// the host terminal's clipboard policy applies — and clear the
    /// selection. The escape rides the frame's outbox with the
    /// notifications.
    pub(super) fn copy_selection(&mut self) {
        let Some(text) = self.renderer.selection_text() else {
            return;
        };
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let lines = text.lines().count();
        self.notify
            .outbox
            .extend_from_slice(format!("\x1b]52;c;{encoded}\x1b\\").as_bytes());
        self.renderer.set_selection(None);
        self.copy_anchor = None;
        self.chrome.flash(format!("copied {lines} lines"));
        self.draw_status_row();
    }

    /// `/`: the search line lives on the status row's flash; typed bytes
    /// route through [`Self::search_typing_byte`] until Enter or Escape.
    pub(super) fn begin_scroll_search(&mut self) {
        self.scroll_query = Some(String::new());
        self.text_utf8 = Default::default();
        self.chrome.flash("search: ".to_string());
        self.draw_status_row();
    }

    /// One plain byte while the search line is up. Returns whether the
    /// line is still up.
    pub(super) fn search_typing_byte(&mut self, byte: u8) -> bool {
        let Some(query) = self.scroll_query.as_mut() else {
            return false;
        };
        match byte {
            0x1b => {
                self.scroll_query = None;
                self.chrome.flash("search cancelled".to_string());
                self.draw_status_row();
                false
            }
            b'\r' => {
                let query = std::mem::take(&mut self.scroll_query).unwrap_or_default();
                self.run_scroll_search(&query, 1);
                false
            }
            0x7f => {
                query.pop();
                self.chrome.flash(format!("search: {query}"));
                self.draw_status_row();
                true
            }
            b if b.is_ascii_graphic() || b == b' ' => {
                query.push(b as char);
                self.chrome.flash(format!("search: {query}"));
                self.draw_status_row();
                true
            }
            b if b >= 0x80 => {
                if let Some(ch) = self.text_utf8.push_printable(b) {
                    query.push(ch);
                    self.chrome.flash(format!("search: {query}"));
                    self.draw_status_row();
                }
                true
            }
            _ => true,
        }
    }

    /// Find `query` in the focused pane's scrollback + live text and
    /// walk it: `dir` 1 takes the next match at or after the viewport
    /// top (wrapping), -1 the previous; the viewport lands on the match
    /// and the flash names the index.
    fn run_scroll_search(&mut self, query: &str, dir: i32) {
        let Some(pane) = self.renderer.focused() else {
            return;
        };
        if query.is_empty() {
            return;
        }
        // Collect the matches once per query; n/N re-walk them.
        let mut matches: Vec<usize> = Vec::new();
        if let Some(emulator) = self.renderer.pane_terminal(pane) {
            let grid = emulator.active_grid();
            let len = grid.scrollback_len();
            for line in 0..len {
                if let Some(cells) = grid.scrollback_line(line) {
                    let text: String = cells.iter().map(|c| c.c()).collect();
                    if text.to_lowercase().contains(&query.to_lowercase()) {
                        matches.push(line);
                    }
                }
            }
            for row in 0..grid.rows() {
                if grid
                    .row_text(row)
                    .to_lowercase()
                    .contains(&query.to_lowercase())
                {
                    matches.push(len + row);
                }
            }
        }
        if matches.is_empty() {
            self.chrome.flash(format!("no matches: {query}"));
            self.draw_status_row();
            return;
        }
        self.scroll_matches = matches;
        self.scroll_search(dir);
    }

    /// Walk the committed match list (`n` forward, `N` back): land the
    /// viewport on the match line.
    pub(super) fn scroll_search(&mut self, dir: i32) {
        if self.scroll_matches.is_empty() {
            self.chrome.flash("no search".to_string());
            self.draw_status_row();
            return;
        }
        let count = self.scroll_matches.len();
        let next = if dir > 0 {
            (self.scroll_match_ix + 1).min(count.saturating_sub(1))
        } else {
            self.scroll_match_ix.saturating_sub(1)
        };
        self.scroll_match_ix = next;
        let line = self.scroll_matches[next];
        let Some(pane) = self.renderer.focused() else {
            return;
        };
        if let Some(emulator) = self.renderer.pane_terminal(pane) {
            let len = emulator.active_grid().scrollback_len();
            let offset = len.saturating_sub(line);
            let current = self.renderer.scroll_offset_of(pane);
            self.renderer
                .scroll_viewport(pane, offset as isize - current as isize);
        }
        self.chrome
            .flash(format!("match {} of {}", next + 1, count));
        self.draw_status_row();
    }
}
