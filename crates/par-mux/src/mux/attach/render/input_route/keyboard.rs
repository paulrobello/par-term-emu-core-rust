//! Plain-byte and key-event routing: the prefix scanner and its chord
//! table, the modal byte owners, and the forwarding path to the focused
//! pane.

use super::super::renderer::{pane_in_direction, PaneDir};
use super::super::{Modal, WindowSession};
use crate::mux::attach::input::InputParser;
use crate::mux::attach::text_input::{fold_fullwidth_ascii, Utf8Assembler, Utf8Step};
use crate::mux::attach::ManagementKey;
use crate::mux::config::{ChordKey, Chords};
use par_term_emu_core::keyboard::TermKeyEvent;

/// Input routing's live state: the prefix chord machinery, the
/// typed-character assemblers, and the stdin tokenizer. Session-level
/// (not per pump call) because a paste or escape sequence can span two
/// stdin bursts, and the pump returns between them every frame tick.
pub(in crate::mux::attach::render) struct InputRouteState {
    /// The reload chord: the key byte matched after the prefix, and the
    /// detach prefix itself — both live-rebindable by the reload.
    pub(in crate::mux::attach::render) prefix: u8,
    pub(in crate::mux::attach::render) reload_key: u8,
    /// The prefix chord is armed — sent, awaiting its second key. The
    /// status row shows the cue while it is up ([`WindowSession::arm_prefix`]
    /// repaints on every flip).
    pub(in crate::mux::attach::render) prefix_pending: bool,
    /// A non-ASCII character is arriving as the prefix chord's key (IME
    /// / CJK input): its remaining bytes are swallowed whole, never
    /// forwarded as a torn UTF-8 tail.
    pub(in crate::mux::attach::render) prefix_utf8: Option<Utf8Assembler>,
    /// Reassembles multi-byte characters typed into the modal text
    /// fields (prompt, picker/help filters, scroll search).
    pub(in crate::mux::attach::render) text_utf8: Utf8Assembler,
    /// The literal prefix byte to forward when the user types prefix
    /// prefix (rebindable, so it is state, not the C_B constant).
    pub(in crate::mux::attach::render) literal: u8,
    /// The stdin tokenizer.
    pub(in crate::mux::attach::render) parser: InputParser,
}

impl InputRouteState {
    /// The chords start at the canonical defaults (ARC-131: one source,
    /// `Chords::with_defaults`); the launch config overrides them.
    pub(in crate::mux::attach::render) fn from_chords(chords: &Chords) -> Self {
        Self {
            prefix: chords.prefix,
            reload_key: chords.reload,
            prefix_pending: false,
            prefix_utf8: None,
            text_utf8: Utf8Assembler::default(),
            literal: chords.prefix,
            parser: InputParser::default(),
        }
    }
}

/// The status flash for a prefix chord typed as a non-ASCII character:
/// chords are ASCII bytes, so an IME left in a non-Latin mode can only
/// ever miss them.
pub(in crate::mux::attach::render) fn non_ascii_chord_hint(ch: char) -> String {
    format!("prefix {ch}: chords are ASCII keys (switch the input method to ASCII)")
}

/// What the byte after the prefix means — the pure result of
/// [`WindowSession::prefix_chord`], which `route_plain` applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mux::attach::render) enum PrefixChord {
    /// The configurable reload chord.
    Reload,
    /// A configurable management chord.
    Management(ManagementKey),
    /// The sticky resize mode's entry key.
    Resize,
    /// The bindings overlay.
    Help,
    /// The window picker.
    Picker,
    /// `d`: detach.
    Detach,
    /// `[`: vi copy mode on the focused pane.
    CopyMode,
    /// `e`: the scrollback in $VISUAL/$EDITOR (copy mode's old `e`;
    /// the vi table owns that letter now).
    EditScrollback,
    /// `n`/`p`/`(`/`)`/`o`: the window, session, and pane switches.
    Switch,
    /// The literal prefix key: send it through.
    Literal,
    /// Anything else: consumed.
    Unbound,
}

impl WindowSession {
    /// A run of plain bytes through the prefix scanner; non-prefix bytes
    /// forward to the focused pane verbatim (the host already encoded
    /// them). Returns true on detach.
    pub(in crate::mux::attach::render) fn route_plain(
        &mut self,
        bytes: &[u8],
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) -> bool {
        // The copy-mode search line owns the bytes while it is up —
        // before every other router (the pane must see nothing of the
        // query).
        if self.copy_search_line_up() {
            for &byte in bytes {
                self.copy_search_byte(byte);
            }
            return false;
        }
        if self.feed_modal_run(bytes, conn) {
            return false;
        }
        let mut to_send: Vec<u8> = Vec::with_capacity(bytes.len());
        for (at, &raw) in bytes.iter().enumerate() {
            let mut byte = raw;
            // A non-ASCII chord key (IME / CJK input): the prefix stays
            // armed while the character's bytes arrive, then a fullwidth
            // form folds to its ASCII chord byte and anything else is
            // consumed with a hint. No byte of it reaches the pane.
            if let Some(mut utf8) = self.input_route.prefix_utf8.take() {
                match utf8.push(raw) {
                    Utf8Step::Pending => {
                        self.input_route.prefix_utf8 = Some(utf8);
                        continue;
                    }
                    Utf8Step::Char(ch) => match fold_fullwidth_ascii(ch) {
                        Some(folded) => byte = folded,
                        None => {
                            self.arm_prefix(false);
                            self.chrome.flash(non_ascii_chord_hint(ch));
                            self.draw_status_row();
                            continue;
                        }
                    },
                    Utf8Step::Invalid => {
                        self.arm_prefix(false);
                        if raw >= 0x80 {
                            continue;
                        }
                    }
                }
            } else if self.input_route.prefix_pending && raw >= 0x80 {
                let mut utf8 = Utf8Assembler::default();
                if utf8.push(raw) == Utf8Step::Pending {
                    self.input_route.prefix_utf8 = Some(utf8);
                } else {
                    self.arm_prefix(false);
                }
                continue;
            }
            if self.input_route.prefix_pending {
                self.arm_prefix(false);
                match self.prefix_chord(byte) {
                    PrefixChord::Reload => self.reload_config(conn),
                    PrefixChord::Management(key) => self.management_chord(key, conn),
                    PrefixChord::Resize => self.enter_resize(),
                    PrefixChord::Help => self.enter_help(),
                    PrefixChord::Picker => self.enter_picker(conn),
                    PrefixChord::Detach => return true,
                    PrefixChord::CopyMode => {
                        // prefix [ — vi copy mode on the focused pane:
                        // every pane enters (an empty-scrollback or
                        // alt-screen pane roams its visible screen);
                        // the key is consumed either way.
                        self.enter_copy_mode();
                    }
                    PrefixChord::EditScrollback => {
                        // prefix e — the scrollback in $VISUAL/$EDITOR
                        // (formerly copy mode's `e`; the motion table
                        // owns that letter now).
                        self.edit_scrollback(conn);
                    }
                    PrefixChord::Switch => self.prefix_switch(byte, conn),
                    PrefixChord::Literal => to_send.push(byte),
                    PrefixChord::Unbound => {
                        if self.popup_for_key(byte).is_some() {
                            self.open_popup(conn, byte);
                            // The popup owns the rest of the run.
                            self.forward_run(conn, &to_send);
                            let rest = &bytes[at + 1..];
                            return !rest.is_empty() && self.popup_route_plain(conn, rest);
                        }
                    }
                }
            } else if byte == self.input_route.prefix {
                self.arm_prefix(true);
            } else if matches!(self.modal, Modal::Copy(_)) {
                // Copy mode's plain keys: q exits, everything graphic
                // reaches the vi table (the mode owns the keys — nothing
                // leaks into the pane).
                self.copy_mode_byte(byte);
            } else {
                to_send.push(byte);
            }
        }
        self.forward_run(conn, &to_send);
        false
    }

    /// Forward a routed run to the focused pane. Typing snaps the pane's
    /// client scroll back to live.
    fn forward_run(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, to_send: &[u8]) {
        if !to_send.is_empty() {
            if let Some(id) = self.renderer.focused() {
                self.renderer.snap_to_live(id);
            }
            self.forward_keys(conn, self.focused_pane(), to_send);
        }
    }

    /// Flip the prefix-armed state and repaint the status row on a
    /// change: the row's `prefix` cue is the indication the chord is
    /// up (tmux's status-line prefix indication).
    pub(in crate::mux::attach::render) fn arm_prefix(&mut self, on: bool) {
        if !on {
            self.input_route.prefix_utf8 = None;
        }
        if self.input_route.prefix_pending == on {
            return;
        }
        self.input_route.prefix_pending = on;
        self.draw_status_row();
    }

    /// Forward input bytes to `pane` without waiting for the daemon. Bytes
    /// the backpressure bound refused wait in `stdin_backlog`; while any
    /// are parked, new bytes queue behind them so the pane sees the typed
    /// order.
    pub(in crate::mux::attach::render) fn forward_keys(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        pane: String,
        bytes: &[u8],
    ) {
        if self.stdin_backlog.is_empty() {
            let tail = crate::mux::attach::forward_keys(conn, &pane, bytes);
            if !tail.is_empty() {
                self.stdin_backlog.push_back((pane, tail.to_vec()));
            }
        } else {
            self.stdin_backlog.push_back((pane, bytes.to_vec()));
        }
    }

    /// Pump step 3's head: retry the parked bytes in order, stopping at the
    /// first entry the bound refuses again.
    pub(in crate::mux::attach::render) fn flush_stdin_backlog(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) {
        while let Some((pane, bytes)) = self.stdin_backlog.pop_front() {
            let tail = crate::mux::attach::forward_keys(conn, &pane, &bytes);
            if !tail.is_empty() {
                let tail = tail.to_vec();
                self.stdin_backlog.push_front((pane, tail));
                return;
            }
        }
    }

    /// A bracketed-paste body. It never meets the prefix scan or the
    /// chord table: a pending prefix is cancelled (tmux's rule), and the
    /// body forwards to the focused pane — re-framed with the pane's own
    /// `ESC[200~`/`ESC[201~` when its emulator tracks DECSET 2004, after
    /// stripping any embedded terminator. A modal that owns the keyboard
    /// (the rename prompt, a menu, help, the picker) receives the body as
    /// typed text instead; the scroll viewport consumes it.
    pub(in crate::mux::attach::render) fn route_paste(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        body: &[u8],
    ) {
        self.arm_prefix(false);
        if self.feed_modal_run(body, conn) || matches!(self.modal, Modal::Copy(_)) {
            return;
        }
        let Some(id) = self.renderer.focused() else {
            return;
        };
        let body = crate::mux::attach::input::strip_paste_end(body);
        let mut bytes = Vec::with_capacity(body.len() + 12);
        if let Some(term) = self.renderer.pane_terminal(id) {
            bytes.extend_from_slice(term.bracketed_paste_start());
            bytes.extend_from_slice(&body);
            bytes.extend_from_slice(term.bracketed_paste_end());
        } else {
            bytes.extend_from_slice(&body);
        }
        if bytes.is_empty() {
            return;
        }
        self.renderer.snap_to_live(id);
        self.forward_keys(conn, self.focused_pane(), &bytes);
    }

    /// Hand a plain run to the modal that owns the keyboard, if one is
    /// up: the prompt, the menu, help, and the picker each eat bytes
    /// until one closes them (the closing byte and the rest of the run
    /// are dropped); resize mode exits and consumes the run whole (its
    /// arrows arrive as Key tokens, not bytes). The scroll viewport is
    /// not a run owner (see [`Modal`]'s precedence note). Returns whether
    /// a modal owned the run.
    fn feed_modal_run(
        &mut self,
        bytes: &[u8],
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) -> bool {
        // The handler is fixed for the run: a byte that closes this modal
        // ends the run.
        let byte_handler: fn(&mut Self, &mut crate::mux::attach::conn::AttachConn, u8) -> bool =
            match &self.modal {
                Modal::None | Modal::Copy(_) => return false,
                Modal::Resize => {
                    self.modal = Modal::None;
                    return true;
                }
                Modal::Prompt(_) => |s, conn, byte| s.prompt_byte(conn, byte),
                Modal::Menu(_) => |s, conn, byte| s.menu_byte(conn, byte),
                Modal::Help(_) => |s, _, byte| s.help_byte(byte),
                Modal::Settings(_) => |s, conn, byte| s.settings_byte(conn, byte),
                Modal::Layouts(_) => |s, conn, byte| s.layouts_byte(conn, byte),
                Modal::Picker(_) => |s, conn, byte| s.picker_byte(conn, byte),
            };
        for &byte in bytes {
            if !byte_handler(self, conn, byte) {
                break;
            }
        }
        true
    }

    /// Classify the byte after the prefix. Precedence matters where the
    /// configurable keys collide with the fixed table: reload first (it
    /// never shadows `d`), then the management chords, then resize, help,
    /// and picker, then the fixed `d`/`[`/switch arms, then the literal
    /// prefix; anything else is unbound and consumed.
    pub(in crate::mux::attach::render) fn prefix_chord(&self, byte: u8) -> PrefixChord {
        if byte == self.input_route.reload_key && byte != b'd' {
            return PrefixChord::Reload;
        }
        if let Some(key) = self.management_key(ChordKey::plain(byte)) {
            return PrefixChord::Management(key);
        }
        if ChordKey::plain(byte) == self.management.resize {
            return PrefixChord::Resize;
        }
        if ChordKey::plain(byte) == self.management.help {
            return PrefixChord::Help;
        }
        if ChordKey::plain(byte) == self.management.picker {
            return PrefixChord::Picker;
        }
        match byte {
            b'd' => PrefixChord::Detach,
            b'[' => PrefixChord::CopyMode,
            b'e' => PrefixChord::EditScrollback,
            b'n' | b'p' | b'(' | b')' | b'o' => PrefixChord::Switch,
            b if b == self.input_route.literal => PrefixChord::Literal,
            _ => PrefixChord::Unbound,
        }
    }

    /// The configurable management chord bound to `byte`, if any (first
    /// match wins when two keys share a byte).
    pub(in crate::mux::attach::render) fn management_key(
        &self,
        chord: ChordKey,
    ) -> Option<ManagementKey> {
        let m = &self.management;
        let table = [
            (m.split_right, ManagementKey::SplitRight),
            (m.split_down, ManagementKey::SplitDown),
            (m.kill_pane, ManagementKey::KillPane),
            (m.break_pane, ManagementKey::BreakPane),
            (m.join_pane, ManagementKey::JoinPane),
            (m.palette, ManagementKey::Palette),
            (m.new_window, ManagementKey::NewWindow),
            (m.swap_prev, ManagementKey::SwapPrev),
            (m.swap_next, ManagementKey::SwapNext),
            (m.move_window_left, ManagementKey::MoveWindowLeft),
            (m.move_window_right, ManagementKey::MoveWindowRight),
            (m.tab_strip_left, ManagementKey::TabStripLeft),
            (m.tab_strip_right, ManagementKey::TabStripRight),
            (m.workspace_next, ManagementKey::WorkspaceNext),
            (m.workspace_prev, ManagementKey::WorkspacePrev),
            (m.zoom, ManagementKey::Zoom),
            (m.rename_window, ManagementKey::RenameWindow),
            (m.rename_pane, ManagementKey::RenamePane),
            (m.border_cycle, ManagementKey::BorderCycle),
            (m.label_toggle, ManagementKey::Labels),
            (m.workspace_picker, ManagementKey::WorkspacePicker),
            (m.sidebar, ManagementKey::Sidebar),
            (m.status_bar, ManagementKey::StatusBar),
            (m.respawn, ManagementKey::Respawn),
            (m.notify_jump, ManagementKey::NotifyJump),
            (
                m.layout_even_horizontal,
                ManagementKey::LayoutEvenHorizontal,
            ),
            (m.layout_even_vertical, ManagementKey::LayoutEvenVertical),
            (
                m.layout_main_horizontal,
                ManagementKey::LayoutMainHorizontal,
            ),
            (m.layout_main_vertical, ManagementKey::LayoutMainVertical),
            (m.layout_tiled, ManagementKey::LayoutTiled),
        ];
        table
            .into_iter()
            .find(|(bound, _)| *bound == chord)
            .map(|(_, key)| key)
    }

    /// One functional key (arrows, function keys, modified keys). The
    /// precedence differs from `route_plain`'s: a pending prefix comes
    /// first (the arrows navigate panes directionally, shift+arrows swap
    /// with the neighbor; anything else is unbound — consumed either
    /// way), then the scroll viewport, prompt, menu, help, picker, and
    /// resize mode; otherwise the key is re-encoded against the focused
    /// pane's input modes and forwarded.
    pub(in crate::mux::attach::render) fn route_key(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        if self.input_route.prefix_pending {
            self.arm_prefix(false);
            // An Alt+printable-ASCII key is an Alt chord: bound runs, unbound
            // is consumed (tmux drops it too) rather than reaching the arrows.
            if ev.modifiers & par_term_emu_core::keyboard::modifiers::ALT != 0
                && ev.key() == par_term_emu_core::keyboard::TermKey::Char
            {
                if let Some(byte) = u8::try_from(ev.codepoint).ok().filter(u8::is_ascii_graphic) {
                    if let Some(key) = self.management_key(ChordKey {
                        key: byte,
                        alt: true,
                    }) {
                        self.management_chord(key, conn);
                    }
                    return;
                }
                if let Some(ch) = char::from_u32(ev.codepoint) {
                    self.chrome.flash(non_ascii_chord_hint(ch));
                    self.draw_status_row();
                }
                return;
            }
            self.prefix_pane_arrow(conn, ev);
            return;
        }
        match &self.modal {
            Modal::Copy(_) => return self.copy_mode_input(conn, ev),
            Modal::Prompt(_) => return self.prompt_key(conn, ev),
            Modal::Menu(_) => return self.menu_key(conn, ev),
            Modal::Help(_) => return self.help_key(ev),
            Modal::Settings(_) => return self.settings_key(ev, conn),
            Modal::Layouts(_) => return self.layouts_key(conn, ev),
            Modal::Picker(_) => return self.picker_key(conn, ev),
            Modal::Resize => return self.resize_key(conn, ev),
            Modal::None => {}
        }
        let focused = self.focused_pane();
        let bytes = self
            .renderer
            .focused()
            .and_then(|id| self.renderer.pane_terminal(id))
            .map(|term| par_term_emu_core::keyboard::encode_key(ev, term))
            .unwrap_or_default();
        if !bytes.is_empty() && !focused.is_empty() {
            self.forward_keys(conn, focused, &bytes);
        }
    }

    /// The prefix commands that move the view through the daemon's tree:
    /// `o` cycles panes of the window, `n`/`p` next/prev window, `(`/`)`
    /// prev/next session — every switch is select-then-refresh, the
    /// daemon-side select + resync passthrough dispatches, with the
    /// renderer re-laid in place and new panes seeded from replays.
    pub(in crate::mux::attach::render) fn prefix_switch(
        &mut self,
        key: u8,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) {
        match key {
            b'o' => self.cycle_pane(conn),
            b'n' => self.switch_window(conn, 1),
            b'p' => self.switch_window(conn, -1),
            b'(' => self.switch_session(conn, -1),
            b')' => self.switch_session(conn, 1),
            _ => {}
        }
    }

    /// prefix + arrow: select the nearest pane in the arrow's direction
    /// (tmux's directional pane navigation). While zoomed the geometry
    /// is the cached daemon layout, so the arrow can leave the zoom the
    /// way tmux's does — unzoom, then land on the neighbor. A fixed
    /// binding (no config key): the management chords are byte-matched,
    /// and arrows arrive as key events.
    pub(in crate::mux::attach::render) fn prefix_pane_arrow(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        use par_term_emu_core::keyboard::TermKey;
        // Shift+arrow: swap with the pane in that direction. tmux's
        // swap-pane keeps focus following the pane's content, so no
        // re-select is needed — the %layout-change broadcast re-seeds the
        // view with the focused pane in the new cell. The shift bit is
        // matched loosely (a terminal may co-report other modifiers), and
        // both outcomes flash on the status row so a no-op at an edge is
        // never silent (the manual-pass report: the chord felt dead).
        if ev.modifiers & par_term_emu_core::keyboard::modifiers::SHIFT != 0 {
            let dir = match ev.key() {
                TermKey::Up => PaneDir::Up,
                TermKey::Down => PaneDir::Down,
                TermKey::Left => PaneDir::Left,
                TermKey::Right => PaneDir::Right,
                _ => return,
            };
            let Some(focused_id) = self.renderer.focused() else {
                return;
            };
            let tree = self.tree_layout();
            let Some(next) = pane_in_direction(&tree, focused_id, dir) else {
                self.chrome.flash("no pane in that direction".to_string());
                self.draw_status_row();
                return;
            };
            // The daemon applies one connection's lines in order, so nothing
            // below needs the swap's reply.
            if conn
                .send_forget(&format!("swap-pane -s %{focused_id} -t %{next}"))
                .is_err()
            {
                return;
            }
            self.chrome
                .flash(format!("swapped %{focused_id} with %{next}"));
            self.draw_status_row();
            return;
        }
        if ev.modifiers != 0 {
            return;
        }
        let dir = match ev.key() {
            TermKey::Up => PaneDir::Up,
            TermKey::Down => PaneDir::Down,
            TermKey::Left => PaneDir::Left,
            TermKey::Right => PaneDir::Right,
            _ => return,
        };
        let Some(focused) = self.renderer.focused() else {
            return;
        };
        // The full tree geometry: while zoomed, the renderer only holds
        // the zoomed pane's expanded rect.
        let tree = self.tree_layout();
        let Some(next) = pane_in_direction(&tree, focused, dir) else {
            return;
        };
        // The daemon unzooms on the select of a different pane; the
        // broadcast re-seeds the view.
        self.renderer.focus(next);
        let _ = conn.send_forget(&format!("select-pane -t %{next}"));
    }
}
