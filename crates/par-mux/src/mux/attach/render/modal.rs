//! The modal switch itself plus the modes without a sibling file: resize
//! mode and copy mode's keys. Help, the pickers, the prompt, the context
//! menus, and the settings/layouts dialogs live in their modal_* siblings.

use super::renderer::{cols_overlap, rows_overlap};
use super::{copy_mode, Modal, WindowSession};
use par_term_emu_core::keyboard::TermKey;
use par_term_emu_core::keyboard::TermKeyEvent;

use copy_mode::{CopyModeState, SearchDirection, VisualMode};

impl WindowSession {
    /// Run `f` over copy mode's state, restoring it whatever `f` does.
    /// `None` when copy mode is not the modal up (the caller then
    /// treats the key as not its business).
    fn with_copy_state<R>(&mut self, f: impl FnOnce(&mut CopyModeState) -> R) -> Option<R> {
        let taken = std::mem::take(&mut self.modal);
        let Modal::Copy(mut state) = taken else {
            self.modal = taken;
            return None;
        };
        let out = f(&mut state);
        self.modal = Modal::Copy(state);
        Some(out)
    }

    /// Make `modal` the one that owns input, replacing any other. A
    /// replaced copy mode releases its hold on the focused pane
    /// (snapping it to live), since nothing would be left to exit it.
    pub(super) fn open_modal(&mut self, modal: Modal) {
        if matches!(self.modal, Modal::Copy(_)) && !matches!(modal, Modal::Copy(_)) {
            self.exit_copy_state();
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

    /// Release copy mode's pane plumbing without touching `self.modal`:
    /// clear the selection and painted cursor, drop the match list, and
    /// snap the focused pane's viewport to live (the hold drops).
    pub(super) fn exit_copy_state(&mut self) {
        self.renderer.set_selection(None);
        self.renderer.set_copy_cursor(None);
        self.scroll_matches.clear();
        self.scroll_match_ix = 0;
        if let Some(id) = self.renderer.focused() {
            self.renderer.exit_scroll_mode(id);
        }
    }

    /// The pane's feed pushed `pushed` lines into scrollback while copy
    /// mode is open on it: rebase every absolute-line coordinate by the
    /// push (the frozen screen rides along, tmux's frozen-view rule).
    pub(super) fn rebase_copy_state(&mut self, pane: u32, pushed: usize) {
        if matches!(self.renderer.focused(), Some(focused) if focused == pane) {
            self.with_copy_state(|s| s.rebase(pushed));
        }
    }

    /// Leave copy mode: clear the hold and snap the focused pane to
    /// live.
    pub(super) fn leave_copy_mode(&mut self) {
        if matches!(self.modal, Modal::Copy(_)) {
            self.modal = Modal::None;
        }
        self.exit_copy_state();
    }

    /// prefix-[: enter copy mode on the focused pane. Every pane has
    /// something to roam — an empty-scrollback or alt-screen pane roams
    /// its visible screen — so there is no refusal anymore; the view
    /// stays at live and the cursor starts at the pane's host cursor.
    pub(super) fn enter_copy_mode(&mut self) {
        let Some(pane) = self.renderer.focused() else {
            return;
        };
        // Re-entering replaces the state; drop the previous mode's
        // pane plumbing first.
        self.exit_copy_state();
        if !self.renderer.enter_copy_mode(pane) {
            return;
        }
        let Some(term) = self.renderer.pane_terminal(pane) else {
            return;
        };
        let grid = term.active_grid();
        let (cols, rows) = (grid.cols(), grid.rows());
        let sb_len = grid.scrollback_len();
        let mut state = CopyModeState::new();
        state.enter(term.cursor().col, term.cursor().row, cols, rows, sb_len);
        // The frozen screen: the visible live rows at entry.
        state.live_rows = (0..rows).map(|row| grid.row_text(row)).collect();
        self.open_modal(Modal::Copy(state));
        self.draw_status_row();
    }

    /// The text of absolute line `line` under copy mode: scrollback
    /// lines read the grid (immutable history), screen lines the frozen
    /// entry snapshot.
    pub(super) fn copy_line_text(&self, line: usize) -> String {
        let Modal::Copy(state) = &self.modal else {
            return String::new();
        };
        let Some(pane) = self.renderer.focused() else {
            return String::new();
        };
        let renderer = &self.renderer;
        let grid_text = |grid_line: usize| -> Option<String> {
            renderer.pane_terminal(pane).and_then(|term| {
                term.active_grid()
                    .scrollback_line(grid_line)
                    .map(|cells| cells.iter().map(|c| c.c()).collect())
            })
        };
        state.line_text(grid_text, line)
    }

    /// The search line's live prompt on the status row.
    fn flash_search_line(&mut self, query: &str) {
        self.chrome.flash(format!("search: {query}"));
        self.draw_status_row();
    }

    /// Whether copy mode's search line is up (its bytes own the run).
    pub(super) fn copy_search_line_up(&self) -> bool {
        matches!(&self.modal, Modal::Copy(state) if state.is_searching)
    }

    /// One key while copy mode is up, from either input path (a Key
    /// token or a plain byte): the functional keys land here, the
    /// character keys in [`Self::copy_mode_char`].
    pub(super) fn copy_mode_input(
        &mut self,
        _conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        use par_term_emu_core::keyboard::modifiers::CTRL;
        if self.copy_search_key(ev) {
            return;
        }
        match (ev.key(), ev.modifiers) {
            (TermKey::Up, 0) => self.copy_motion(|s| s.move_up()),
            (TermKey::Down, 0) => self.copy_motion(|s| s.move_down()),
            (TermKey::Left, 0) => self.copy_motion(|s| s.move_left()),
            (TermKey::Right, 0) => self.copy_motion(|s| s.move_right()),
            (TermKey::PageUp, 0) => self.copy_motion(CopyModeState::page_up),
            (TermKey::PageDown, 0) => self.copy_motion(CopyModeState::page_down),
            (TermKey::Home, 0) => self.copy_motion(CopyModeState::move_to_line_start),
            (TermKey::End, 0) => self.copy_motion(CopyModeState::move_to_line_end),
            (TermKey::Escape, _) => self.copy_escape(),
            (TermKey::Char, m) if m & CTRL != 0 => match ev.codepoint {
                c if c == u32::from(b'd') => self.copy_motion(CopyModeState::half_page_down),
                c if c == u32::from(b'u') => self.copy_motion(CopyModeState::half_page_up),
                c if c == u32::from(b'f') => self.copy_motion(CopyModeState::page_down),
                c if c == u32::from(b'b') => self.copy_motion(CopyModeState::page_up),
                c if c == u32::from(b'v') => self.copy_visual(VisualMode::Block),
                _ => {}
            },
            (TermKey::Char, 0) => {
                if let Some(ch) = char::from_u32(ev.codepoint) {
                    self.copy_mode_char(ch);
                }
            }
            _ => {}
        }
    }

    /// The search line's Key-token path: Enter commits, Escape cancels,
    /// Backspace deletes, printable chars append. Returns whether the
    /// search line consumed the key.
    fn copy_search_key(&mut self, ev: &TermKeyEvent) -> bool {
        if !self.with_copy_state(|s| s.is_searching).unwrap_or(false) {
            return false;
        }
        match ev.key() {
            TermKey::Escape => {
                self.with_copy_state(|s| s.cancel_search());
                self.chrome.flash("search cancelled".to_string());
                self.draw_status_row();
            }
            TermKey::Enter => self.commit_copy_search(),
            TermKey::Backspace => {
                self.with_copy_state(|s| {
                    s.search_query.pop();
                });
                self.flash_search_line("");
            }
            TermKey::Char if ev.modifiers == 0 => {
                if let Some(ch) = char::from_u32(ev.codepoint) {
                    self.with_copy_state(|s| s.search_query.push(ch));
                    let query = self
                        .with_copy_state(|s| s.search_query.clone())
                        .unwrap_or_default();
                    self.flash_search_line(&query);
                    self.draw_status_row();
                }
            }
            _ => {}
        }
        true
    }

    /// One plain byte while copy mode is up (the route_plain path).
    /// Every byte is consumed: nothing leaks into the pane.
    pub(super) fn copy_mode_byte(&mut self, byte: u8) {
        if matches!(self.modal, Modal::Copy(_)) {
            if self.copy_search_byte(byte) {
                return;
            }
            match byte {
                b'q' => self.leave_copy_mode(),
                b if b.is_ascii_graphic() => self.copy_mode_char(b as char),
                _ => {}
            }
        }
    }

    /// The search line's plain-byte path. Returns whether consumed.
    pub(super) fn copy_search_byte(&mut self, byte: u8) -> bool {
        if !self.with_copy_state(|s| s.is_searching).unwrap_or(false) {
            return false;
        }
        match byte {
            0x1b => {
                self.with_copy_state(|s| s.cancel_search());
                self.chrome.flash("search cancelled".to_string());
                self.draw_status_row();
            }
            b'\r' | b'\n' => self.commit_copy_search(),
            0x7f => {
                self.with_copy_state(|s| {
                    s.search_query.pop();
                });
                let query = self
                    .with_copy_state(|s| s.search_query.clone())
                    .unwrap_or_default();
                self.flash_search_line(&query);
                self.draw_status_row();
            }
            b if b.is_ascii_graphic() || b == b' ' => {
                self.with_copy_state(|s| s.search_query.push(b as char));
                let query = self
                    .with_copy_state(|s| s.search_query.clone())
                    .unwrap_or_default();
                self.flash_search_line(&query);
                self.draw_status_row();
            }
            b if b >= 0x80 => {
                if let Some(ch) = self.text_utf8.push_printable(b) {
                    self.with_copy_state(|s| s.search_query.push(ch));
                    let query = self
                        .with_copy_state(|s| s.search_query.clone())
                        .unwrap_or_default();
                    self.flash_search_line(&query);
                    self.draw_status_row();
                }
            }
            _ => {}
        }
        true
    }

    /// The pending two-key sequences first (`gg`, `m{a-z}`, `'{a-z}`),
    /// then the character key table. Mirrors par-term's handler.
    fn copy_mode_char(&mut self, ch: char) {
        if self.with_copy_state(|s| s.pending_mark_set) == Some(true) {
            self.with_copy_state(|s| {
                s.pending_mark_set = false;
                if ch.is_ascii_lowercase() {
                    s.set_mark(ch);
                }
            });
            return;
        }
        if self.with_copy_state(|s| s.pending_mark_goto) == Some(true) {
            self.with_copy_state(|s| s.pending_mark_goto = false);
            if ch.is_ascii_lowercase() {
                let jumped = self.with_copy_state(|s| s.goto_mark(ch));
                if jumped == Some(true) {
                    self.after_copy_motion();
                }
            }
            return;
        }
        if self.with_copy_state(|s| s.pending_g) == Some(true) {
            self.with_copy_state(|s| {
                s.pending_g = false;
            });
            if ch == 'g' {
                self.copy_motion(CopyModeState::goto_top);
            }
            return;
        }
        match ch {
            'h' => self.copy_motion(CopyModeState::move_left),
            'j' => self.copy_motion(CopyModeState::move_down),
            'k' => self.copy_motion(CopyModeState::move_up),
            'l' => self.copy_motion(CopyModeState::move_right),
            '0' => {
                let counted = self.with_copy_state(|s| s.count.is_some()) == Some(true);
                if counted {
                    self.with_copy_state(|s| s.push_count_digit(0));
                } else {
                    self.copy_motion(CopyModeState::move_to_line_start);
                }
            }
            '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' => {
                if let Some(d) = ch.to_digit(10) {
                    self.with_copy_state(|s| s.push_count_digit(d as u8));
                }
            }
            '$' => self.copy_motion(CopyModeState::move_to_line_end),
            '^' => self.copy_line_motion(CopyModeState::move_to_first_non_blank),
            'w' => self.copy_line_motion(CopyModeState::move_word_forward),
            'b' => self.copy_line_motion(CopyModeState::move_word_backward),
            'e' => self.copy_line_motion(CopyModeState::move_word_end),
            'W' => self.copy_line_motion(CopyModeState::move_big_word_forward),
            'B' => self.copy_line_motion(CopyModeState::move_big_word_backward),
            'E' => self.copy_line_motion(CopyModeState::move_big_word_end),
            'G' => {
                let count = self.with_copy_state(|s| s.count.take()).flatten();
                match count {
                    Some(n) => self.copy_motion(move |s: &mut CopyModeState| {
                        s.goto_line(n.saturating_sub(1));
                    }),
                    None => self.copy_motion(CopyModeState::goto_bottom),
                }
            }
            'g' => {
                self.with_copy_state(|s| s.pending_g = true);
            }
            'v' => self.copy_visual(VisualMode::Char),
            'V' => self.copy_visual(VisualMode::Line),
            'y' => self.yank_copy_selection(),
            '/' => self.begin_copy_search(true),
            '?' => self.begin_copy_search(false),
            'n' => self.copy_search_walk(false),
            'N' => self.copy_search_walk(true),
            'm' => {
                self.with_copy_state(|s| s.pending_mark_set = true);
            }
            '\'' => {
                self.with_copy_state(|s| s.pending_mark_goto = true);
            }
            'H' => self.copy_viewport_jump(0),
            'M' => self.copy_viewport_jump_mid(),
            'L' => self.copy_viewport_jump_bottom(),
            'q' => self.leave_copy_mode(),
            _ => {}
        }
    }

    /// H/M/L: jump to the viewport's top/middle/bottom line — vi's
    /// screen-relative jumps, the viewport follows.
    fn copy_viewport_jump(&mut self, offset_rows: usize) {
        let Some(pane) = self.renderer.focused() else {
            return;
        };
        let Some(top) = self.renderer.viewport_top_line(pane) else {
            return;
        };
        self.with_copy_state(|s| {
            s.cursor_absolute_line = (top + offset_rows).min(s.max_line());
        });
        self.after_copy_motion();
    }

    /// M: the viewport's middle line.
    fn copy_viewport_jump_mid(&mut self) {
        let rows = self.with_copy_state(|s| s.rows).unwrap_or(0);
        self.copy_viewport_jump(rows / 2);
    }

    /// L: the viewport's bottom line.
    fn copy_viewport_jump_bottom(&mut self) {
        let rows = self.with_copy_state(|s| s.rows).unwrap_or(0);
        self.copy_viewport_jump(rows.saturating_sub(1));
    }

    /// One cursor-or-selection change that needs no line text.
    fn copy_motion(&mut self, motion: impl FnOnce(&mut CopyModeState)) {
        self.with_copy_state(motion);
        self.after_copy_motion();
    }

    /// A motion over the cursor line's text (`^`, the word motions).
    fn copy_line_motion(&mut self, motion: impl FnOnce(&mut CopyModeState, &str)) {
        let line = self.with_copy_state(|s| s.cursor_absolute_line);
        let text = self.copy_line_text(line.unwrap_or(0));
        self.with_copy_state(|s| motion(s, &text));
        self.after_copy_motion();
    }

    /// Enter or leave a visual mode; the selection syncs in
    /// [`Self::after_copy_motion`].
    fn copy_visual(&mut self, mode: VisualMode) {
        self.with_copy_state(|s| s.toggle_visual(mode));
        self.after_copy_motion();
    }

    /// Escape exits visual mode first, then copy mode (par-term's rule).
    fn copy_escape(&mut self) {
        let visual = self
            .with_copy_state(|s| s.visual_mode != VisualMode::None)
            .unwrap_or(false);
        if visual {
            self.with_copy_state(CopyModeState::clear_visual);
            self.after_copy_motion();
        } else {
            self.leave_copy_mode();
        }
    }

    /// Post-motion housekeeping: track a resize, sync the painted
    /// selection and cursor, follow the cursor with the viewport, and
    /// redraw the status row (the mode badge's position moved).
    fn after_copy_motion(&mut self) {
        let Some(pane) = self.renderer.focused() else {
            return;
        };
        // A concurrent refit may have resized the pane; keep the state's
        // dimensions current so clamps stay exact.
        if let Some(term) = self.renderer.pane_terminal(pane) {
            let grid = term.active_grid();
            let (cols, rows) = (grid.cols(), grid.rows());
            let sb_len = grid.scrollback_len();
            self.with_copy_state(|s| s.update_dimensions(cols, rows, sb_len));
        }
        let Some((line, col)) = self.with_copy_state(|s| (s.cursor_absolute_line, s.cursor_col))
        else {
            return;
        };
        self.renderer.set_copy_cursor(Some((pane, line, col)));
        let selection = self.with_copy_state(|s| s.selection(pane));
        let offset = self.renderer.scroll_offset_of(pane);
        let follow = self.with_copy_state(|s| s.required_scroll_offset(offset));
        self.renderer.set_selection(selection.flatten());
        if let Some(new_offset) = follow.flatten() {
            let current = self.renderer.scroll_offset_of(pane) as isize;
            let delta = new_offset as isize - current;
            self.renderer.scroll_viewport(pane, delta);
        }
        self.draw_status_row();
    }

    /// y: yank the visual selection (or the cursor line when none is
    /// up) as an OSC 52 clipboard write; with `copy-mode-auto-exit` on,
    /// leave copy mode and flash the size; without, stay up with the
    /// visual cleared.
    fn yank_copy_selection(&mut self) {
        let Some(text) = self.copy_yank_text() else {
            return;
        };
        self.send_osc52(&text);
        self.send_osc52(&text);
        let lines = text.lines().count();
        let chars = text.chars().count();
        let auto_exit = self.copy_mode_auto_exit;
        if auto_exit {
            self.leave_copy_mode();
        } else {
            self.with_copy_state(CopyModeState::clear_visual);
            self.renderer.set_selection(None);
        }
        let msg = if lines > 1 {
            format!("{} lines yanked", lines)
        } else {
            format!("{} chars yanked", chars)
        };
        self.chrome.flash(msg);
        self.draw_status_row();
    }

    /// The yank payload: the visual selection's rows/columns read
    /// through the frozen-aware line accessor, per row trimmed and
    /// newline joined (the mouse copy's text shape).
    fn copy_yank_text(&mut self) -> Option<String> {
        let (mode, anchor, cursor) = self.with_copy_state(|s| {
            (
                s.visual_mode,
                s.selection_anchor,
                (s.cursor_absolute_line, s.cursor_col),
            )
        })?;
        let anchor = anchor?;
        let (top, bottom, left, right) = match mode {
            VisualMode::Char => {
                let ((a_line, a_col), (b_line, b_col)) = super::normalize_span(anchor, cursor);
                (a_line, b_line, a_col, b_col)
            }
            VisualMode::Line => {
                let (a, b) = (anchor.0.min(cursor.0), anchor.0.max(cursor.0));
                (a, b, 0, usize::MAX)
            }
            VisualMode::Block => {
                let top = anchor.0.min(cursor.0);
                let bottom = anchor.0.max(cursor.0);
                let left = anchor.1.min(cursor.1);
                let right = anchor.1.max(cursor.1);
                (top, bottom, left, right)
            }
            VisualMode::None => (cursor.0, cursor.0, 0, usize::MAX),
        };
        let mut text = String::new();
        for line in top..=bottom {
            let row: Vec<char> = self.copy_line_text(line).chars().collect();
            let to = if line == bottom && right != usize::MAX {
                (right + 1).min(row.len())
            } else {
                row.len()
            };
            let slice: String = if left < to {
                row[left.min(row.len())..to].iter().collect()
            } else {
                String::new()
            };
            text.push_str(slice.trim_end());
            text.push('\n');
        }
        Some(text)
    }

    /// `/` and `?`: the search line rides the status row's flash; typed
    /// bytes append to the state's query until Enter or Escape.
    fn begin_copy_search(&mut self, forward: bool) {
        self.with_copy_state(|s| {
            s.start_search(if forward {
                SearchDirection::Forward
            } else {
                SearchDirection::Backward
            })
        });
        let dir = self
            .with_copy_state(|s| s.search_direction)
            .unwrap_or(SearchDirection::Forward);
        let prompt = match dir {
            SearchDirection::Backward => "? ",
            SearchDirection::Forward => ": ",
        };
        self.chrome.flash(format!("search{prompt}"));
        self.draw_status_row();
    }

    /// Enter on the search line: commit the query, collect the pane's
    /// matches once, and land the cursor on the first at-or-after the
    /// copy cursor.
    fn commit_copy_search(&mut self) {
        self.with_copy_state(|s| s.is_searching = false);
        let query = self
            .with_copy_state(|s| s.search_query.clone())
            .unwrap_or_default();
        if query.is_empty() {
            self.chrome.flash("search cancelled".to_string());
            self.draw_status_row();
            return;
        }
        let matches = self.collect_copy_matches(&query);
        if matches.is_empty() {
            self.chrome.flash(format!("no matches: {query}"));
            self.draw_status_row();
            return;
        }
        self.scroll_matches = matches;
        self.scroll_match_ix = 0;
        self.copy_search_walk(false);
    }

    /// Every `(line, column)` where the query occurs (case-insensitive)
    /// across scrollback + the frozen screen.
    fn collect_copy_matches(&mut self, query: &str) -> Vec<(usize, usize)> {
        let query_lower = query.to_lowercase();
        let total = self.with_copy_state(|s| s.total_lines()).unwrap_or(0);
        let mut matches = Vec::new();
        for line in 0..total {
            let text = self.copy_line_text(line);
            let lowered = text.to_lowercase();
            let mut from = 0;
            while let Some(found) = lowered[from..].find(&query_lower) {
                matches.push((line, from + found));
                from += found + query_lower.chars().count();
            }
            let _ = text;
        }
        matches
    }

    /// n/N: walk the committed matches from the copy cursor.
    fn copy_search_walk(&mut self, reverse: bool) {
        let count = self.scroll_matches.len();
        if count == 0 {
            self.chrome.flash("no search".to_string());
            self.draw_status_row();
            return;
        }
        let cursor = self
            .with_copy_state(|s| (s.cursor_absolute_line, s.cursor_col))
            .unwrap_or((0, 0));
        let backward = self
            .with_copy_state(|s| {
                let back = s.search_direction == SearchDirection::Backward;
                back != reverse
            })
            .unwrap_or(false);
        // The first match strictly after (or before) the cursor,
        // wrapping. partition_point counts the prefix the predicate
        // holds over — matches at-or-before (forward) / strictly-before
        // (backward) the cursor.
        let rank = self
            .scroll_matches
            .partition_point(|&(line, col)| match backward {
                false => (line, col) <= cursor,
                true => (line, col) < cursor,
            });
        let next = if backward {
            if rank == 0 {
                count - 1
            } else {
                rank - 1
            }
        } else {
            if rank >= count {
                0
            } else {
                rank
            }
        };
        self.copy_search_jump(next);
    }

    /// Land the copy cursor on match `ix` and flash the position.
    fn copy_search_jump(&mut self, ix: usize) {
        let Some(&(line, col)) = self.scroll_matches.get(ix) else {
            self.chrome.flash("no search".to_string());
            self.draw_status_row();
            return;
        };
        self.with_copy_state(|s| {
            s.cursor_absolute_line = line;
            s.cursor_col = col.min(s.cols.saturating_sub(1));
        });
        self.scroll_match_ix = ix;
        let count = self.scroll_matches.len();
        let msg = format!("match {} of {}", ix + 1, count);
        self.chrome.flash(msg);
        self.after_copy_motion();
    }

    /// The mouse drag's release copy: the dragged span as an OSC 52
    /// write plus a "copied N lines" flash. Copy mode's y is
    /// [`Self::yank_copy_selection`]'s.
    pub(super) fn copy_selection(&mut self) {
        let Some(text) = self.renderer.selection_text() else {
            return;
        };
        self.send_osc52(&text);
        let lines = text.lines().count();
        self.renderer.set_selection(None);
        self.chrome.flash(format!("copied {lines} lines"));
        self.draw_status_row();
    }

    /// `text` as an OSC 52 clipboard write — the host terminal's
    /// clipboard policy applies. The escape rides the frame's outbox
    /// with the notifications.
    pub(super) fn send_osc52(&mut self, text: &str) {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        self.notify
            .outbox
            .extend_from_slice(format!("\x1b]52;c;{encoded}\x1b\\").as_bytes());
    }
}
