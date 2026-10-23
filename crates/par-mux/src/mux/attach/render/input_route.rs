//! Input routing: plain-byte runs through the prefix scanner and the
//! modal owners, and mouse reports through the overlay hit-testers.

use super::renderer::{pane_in_direction, PaneDir};
use super::{
    DividerHit, DragState, MenuTarget, Modal, PickerState, PromptTarget, Selection, TextDrag,
    WindowSession,
};
use crate::mux::attach::input::SgrMouse;
use crate::mux::attach::layout::PaneRect;
use crate::mux::attach::text_input::{fold_fullwidth_ascii, Utf8Assembler, Utf8Step};
use crate::mux::attach::ManagementKey;
use par_term_emu_core::keyboard::TermKeyEvent;

/// The status flash for a prefix chord typed as a non-ASCII character:
/// chords are ASCII bytes, so an IME left in a non-Latin mode can only
/// ever miss them.
pub(super) fn non_ascii_chord_hint(ch: char) -> String {
    format!("prefix {ch}: chords are ASCII keys (switch the input method to ASCII)")
}

/// What one host mouse report hits — the pure result of
/// [`WindowSession::mouse_hit`], which [`WindowSession::route_mouse`]
/// applies. Coordinates are as each consumer takes them: `TabPress`
/// carries the RAW host column, everything below the strip row the
/// strip-rebased content coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Hit {
    /// Nothing happens: zero coordinates, a modal swallowing the event,
    /// or a point that hits nothing.
    Consumed,
    /// A press on the open context menu's panel row.
    MenuRow(usize),
    /// A press outside the open context menu: close it.
    MenuClose,
    /// A press on the settings dialog at overlay row `row`, inner column
    /// `col`.
    SettingsAt { row: usize, col: usize },
    /// A press outside the settings dialog: close it.
    SettingsClose,
    /// A press on the in-app toast: focus its agent's pane.
    ToastPress { pane: Option<u32> },
    /// The side panel's ` new ` chip: open the new-workspace prompt.
    SidebarNewWorkspace,
    /// The side panel's ` menu ` chip: open the command menu.
    SidebarMenu,
    /// The side panel's chevron: collapse to the rail, or expand it.
    SidebarCollapse,
    /// The agents header's right-aligned sort label: flip the roster's
    /// grouped/priority ordering (herdr's `agent_sort_toggle`).
    SidebarAgentsSort,
    /// A side panel workspace row (`+N`): land on it, or open its
    /// context menu on a right press (`menu`).
    SidebarWorkspace { id: String, menu: bool },
    /// An agent row in the side panel: the pane id (`%N`) to focus.
    SidebarPane { pane: u32 },
    /// Motion or release while a divider drag is in flight.
    Drag { x: u16, y: u16, release: bool },
    /// Motion or release while a text-selection drag is in flight.
    TextDrag { x: u16, y: u16, release: bool },
    /// A picker wheel: move the selection by the delta.
    PickerMove(isize),
    /// A press on the picker's filter line: open the filter box.
    PickerFilter,
    /// A press on picker content row `n` of the visible window.
    PickerRow(usize),
    /// A help panel wheel: scroll the panel by the delta.
    HelpScroll(isize),
    /// A right press on a tab: open that window's (`@N`) context menu.
    TabMenu(String),
    /// A wheel over an overflowing tab strip: shift the visible run.
    TabWheel(isize),
    /// Motion or release on the strip while a tab drag is armed: the
    /// raw host column to drop at.
    TabDrop(u16),
    /// A left press on the tab strip at this raw host column.
    TabPress(u16),
    /// A wheel over a pane: scroll the client's scrollback, or forward
    /// when the pane owns the mouse.
    Wheel {
        rect: PaneRect,
        x: u16,
        cy: u16,
        delta: isize,
    },
    /// Motion or release over a mouse-owning pane: forward it.
    PaneForward(PaneRect),
    /// A press on the side panel's right edge: start a width drag.
    SidebarResize,
    /// A press on the side panel's rule row: start a split drag.
    SidebarSplitDrag,
    /// Motion or release while a sidebar drag (width or split) is in
    /// flight — host coordinates, like the press stored them.
    SidebarDrag { x: u16, y: u16, release: bool },
    /// A press near a divider: start a (pending) drag.
    DragStart { divider: DividerHit, x: u16, y: u16 },
    /// A press on a pane: focus it, forwarding when it owns the mouse.
    PanePress(PaneRect),
}

/// The help panel is modal for the pointer: while it is up every mouse
/// event is consumed — wheels scroll the PANEL (the round-3 defect: they
/// fell through to the pane scrollback / pane forwarding), clicks and
/// drags do nothing.
fn help_hit(mouse: &SgrMouse) -> Hit {
    if mouse.is_wheel_up() {
        Hit::HelpScroll(-3)
    } else if mouse.is_wheel_down() {
        Hit::HelpScroll(3)
    } else {
        Hit::Consumed
    }
}

/// What the byte after the prefix means — the pure result of
/// [`WindowSession::prefix_chord`], which `route_plain` applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrefixChord {
    /// The configurable reload chord.
    Reload,
    /// A configurable management chord.
    Management(ManagementKey),
    /// The sticky resize mode's entry key.
    Resize,
    /// The bindings overlay.
    Help,
    /// The session/window picker.
    Picker,
    /// `d`: detach.
    Detach,
    /// `[`: the scroll viewport on the focused pane.
    Scroll,
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
    pub(super) fn route_plain(
        &mut self,
        bytes: &[u8],
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) -> bool {
        // The scroll-mode search line owns the bytes while it is up —
        // before every other router (the viewport's pane must see
        // nothing of the query).
        if self.scroll_query.is_some() {
            for &byte in bytes {
                if !self.search_typing_byte(byte) {
                    break;
                }
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
            if let Some(mut utf8) = self.prefix_utf8.take() {
                match utf8.push(raw) {
                    Utf8Step::Pending => {
                        self.prefix_utf8 = Some(utf8);
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
            } else if self.prefix_pending && raw >= 0x80 {
                let mut utf8 = Utf8Assembler::default();
                if utf8.push(raw) == Utf8Step::Pending {
                    self.prefix_utf8 = Some(utf8);
                } else {
                    self.arm_prefix(false);
                }
                continue;
            }
            if self.prefix_pending {
                self.arm_prefix(false);
                match self.prefix_chord(byte) {
                    PrefixChord::Reload => self.reload_config(conn),
                    PrefixChord::Management(key) => self.management_chord(key, conn),
                    PrefixChord::Resize => self.enter_resize(),
                    PrefixChord::Help => self.enter_help(),
                    PrefixChord::Picker => self.enter_picker(conn),
                    PrefixChord::Detach => return true,
                    PrefixChord::Scroll => {
                        // prefix [ — the scroll viewport on the focused
                        // pane. No scrollback means nothing to scroll;
                        // the key is consumed either way.
                        if let Some(id) = self.renderer.focused() {
                            if self.renderer.enter_scroll_mode(id) {
                                self.open_modal(Modal::Scroll);
                            }
                        }
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
            } else if byte == self.prefix {
                self.arm_prefix(true);
            } else if self.scroll_query.is_some() {
                // A `/` earlier in this run opened the search line: the
                // rest of the run is the query.
                self.search_typing_byte(byte);
            } else if matches!(self.modal, Modal::Scroll) {
                // Scroll mode's plain keys: q and Enter exit, the
                // copy-mode letters (v y / n N e) reach the key handler
                // (the viewport is a modal view — keys do not leak into
                // the pane).
                if byte == b'q' || byte == b'\r' {
                    self.leave_scroll_mode();
                } else if byte.is_ascii_graphic() {
                    self.scroll_mode_input(conn, &TermKeyEvent::char_(byte as char, 0));
                }
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
    pub(super) fn arm_prefix(&mut self, on: bool) {
        if !on {
            self.prefix_utf8 = None;
        }
        if self.prefix_pending == on {
            return;
        }
        self.prefix_pending = on;
        self.draw_status_row();
    }

    /// Forward input bytes to `pane` without waiting for the daemon. Bytes
    /// the backpressure bound refused wait in `stdin_backlog`; while any
    /// are parked, new bytes queue behind them so the pane sees the typed
    /// order.
    pub(super) fn forward_keys(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        pane: String,
        bytes: &[u8],
    ) {
        if self.stdin_backlog.is_empty() {
            let tail = crate::mux::attach::forward_keys(conn, &pane, bytes);
            if !tail.is_empty() {
                self.stdin_backlog.push((pane, tail.to_vec()));
            }
        } else {
            self.stdin_backlog.push((pane, bytes.to_vec()));
        }
    }

    /// Pump step 3's head: retry the parked bytes in order, stopping at the
    /// first entry the bound refuses again.
    pub(super) fn flush_stdin_backlog(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        while !self.stdin_backlog.is_empty() {
            let (pane, bytes) = self.stdin_backlog.remove(0);
            let tail = crate::mux::attach::forward_keys(conn, &pane, &bytes);
            if !tail.is_empty() {
                let tail = tail.to_vec();
                self.stdin_backlog.insert(0, (pane, tail));
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
    pub(super) fn route_paste(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        body: &[u8],
    ) {
        self.arm_prefix(false);
        if self.feed_modal_run(body, conn) || matches!(self.modal, Modal::Scroll) {
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
                Modal::None | Modal::Scroll => return false,
                Modal::Resize => {
                    self.modal = Modal::None;
                    return true;
                }
                Modal::Prompt(_) => |s, conn, byte| s.prompt_byte(conn, byte),
                Modal::Menu(_) => |s, _, byte| s.menu_byte(byte),
                Modal::Help(_) => |s, _, byte| s.help_byte(byte),
                Modal::Settings(_) => |s, conn, byte| s.settings_byte(conn, byte),
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
    pub(super) fn prefix_chord(&self, byte: u8) -> PrefixChord {
        if byte == self.reload_key && byte != b'd' {
            return PrefixChord::Reload;
        }
        if let Some(key) = self.management_key(byte) {
            return PrefixChord::Management(key);
        }
        if byte == self.management.resize {
            return PrefixChord::Resize;
        }
        if byte == self.management.help {
            return PrefixChord::Help;
        }
        if byte == self.management.picker {
            return PrefixChord::Picker;
        }
        match byte {
            b'd' => PrefixChord::Detach,
            b'[' => PrefixChord::Scroll,
            b'n' | b'p' | b'(' | b')' | b'o' => PrefixChord::Switch,
            b if b == self.literal => PrefixChord::Literal,
            _ => PrefixChord::Unbound,
        }
    }

    /// The configurable management chord bound to `byte`, if any (first
    /// match wins when two keys share a byte).
    fn management_key(&self, byte: u8) -> Option<ManagementKey> {
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
        ];
        table
            .into_iter()
            .find(|(bound, _)| *bound == byte)
            .map(|(_, key)| key)
    }

    /// One functional key (arrows, function keys, modified keys). The
    /// precedence differs from `route_plain`'s: a pending prefix comes
    /// first (the arrows navigate panes directionally, shift+arrows swap
    /// with the neighbor; anything else is unbound — consumed either
    /// way), then the scroll viewport, prompt, menu, help, picker, and
    /// resize mode; otherwise the key is re-encoded against the focused
    /// pane's input modes and forwarded.
    pub(super) fn route_key(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        if self.prefix_pending {
            self.arm_prefix(false);
            self.prefix_pane_arrow(conn, ev);
            return;
        }
        match &self.modal {
            Modal::Scroll => return self.scroll_mode_input(conn, ev),
            Modal::Prompt(_) => return self.prompt_key(conn, ev),
            Modal::Menu(_) => return self.menu_key(ev),
            Modal::Help(_) => return self.help_key(ev),
            Modal::Settings(_) => return self.settings_key(ev, conn),
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
    pub(super) fn prefix_switch(
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
    pub(super) fn prefix_pane_arrow(
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

    /// One host mouse report. Clicks on the tab strip switch windows;
    /// clicks in the content area focus the pane under the pointer
    /// (select-pane daemon-side, so the daemon's own active-pane state
    /// follows); events forward pane-relative SGR when the pane owns
    /// mouse tracking; the wheel scrolls the client's scrollback when it
    /// does not. A press within one cell of a divider starts a drag: it
    /// must not focus or forward (a drag on a divider is not a
    /// click-through), motion adjusts the adjacent split via the wire's
    /// relative `resize-pane`, and release without any motion falls
    /// through as a click (focus, plus the pane's release when owned).
    ///
    /// [`Self::mouse_hit`] decides what the report hits without touching
    /// state; this applies it.
    pub(super) fn route_mouse(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        mouse: SgrMouse,
    ) {
        super::trace_input(format_args!(
            "mouse cb={} col={} row={} release={}",
            mouse.cb, mouse.col, mouse.row, mouse.release
        ));
        // The menu's hover highlight rides no-button motion (any-event
        // tracking): the row under the pointer lights up; a motion off
        // the panel or onto the header/footer clears it.
        if matches!(self.modal, Modal::Menu(_))
            && mouse.is_motion()
            && !mouse.release
            && mouse.motion_button().is_none()
        {
            if let (Some(x), Some(y)) = (mouse.col.checked_sub(1), mouse.row.checked_sub(1)) {
                self.menu_hover(x, y);
            }
        }
        match self.mouse_hit(&mouse) {
            Hit::Consumed => {}
            Hit::MenuRow(row) => self.menu_click(conn, row),
            Hit::SettingsAt { row, col } => self.settings_click(conn, row, col),
            Hit::SettingsClose => self.leave_settings(),
            Hit::MenuClose => self.leave_menu(),
            Hit::ToastPress { pane } => {
                self.notify.toast_until = None;
                self.renderer.set_toast(None);
                if let Some(pane) = pane {
                    self.focus_pane(conn, pane);
                }
            }
            Hit::SidebarNewWorkspace => self.enter_prompt(PromptTarget::NewWorkspace),
            Hit::SidebarMenu => {
                let anchor = self.renderer.sidebar_menu_chip_anchor();
                self.open_menu(MenuTarget::Commands);
                self.renderer.set_overlay_anchor(anchor);
            }
            Hit::SidebarCollapse => self.toggle_sidebar_collapse(conn),
            Hit::SidebarAgentsSort => self.toggle_agents_sort(conn),
            Hit::SidebarWorkspace { id, menu: true } => {
                self.open_menu(MenuTarget::Workspace(id));
            }
            Hit::SidebarWorkspace { id, menu: false } => self.land_on_workspace(conn, &id),
            Hit::SidebarPane { pane } => self.focus_pane(conn, pane),
            Hit::SidebarResize => self.start_sidebar_width_drag(mouse.col - 1),
            Hit::SidebarSplitDrag => self.start_sidebar_split_drag(mouse.row - 1),
            Hit::SidebarDrag { x, y, release } => self.sidebar_drag_event(x, y, release),
            Hit::Drag { x, y, release } => self.drag_event(conn, x, y, release),
            Hit::PickerMove(delta) => self.picker_move(delta),
            Hit::PickerFilter => {
                if let Modal::Picker(picker) = &mut self.modal {
                    picker.filtering = true;
                }
                self.refresh_picker();
            }
            Hit::PickerRow(content) => {
                // Move the cursor to the clicked row and activate it
                // (start maps the windowed index onto the filtered
                // list).
                if let Modal::Picker(picker) = &mut self.modal {
                    picker.selected = picker.start + content;
                }
                self.refresh_picker();
                self.picker_activate(conn);
            }
            Hit::HelpScroll(delta) => self.help_scroll_by(delta),
            Hit::TabMenu(id) => self.open_menu(MenuTarget::Tab(id)),
            Hit::TabPress(rx) => self.tab_click(conn, rx),
            Hit::TabWheel(delta) => {
                self.chrome.tab_strip.wheel(delta);
                self.draw_tab_strip();
            }
            Hit::TabDrop(rx) => self.tab_drop(conn, rx),
            Hit::Wheel { rect, x, cy, delta } => {
                let forwards = self.pane_accepts_mouse(rect.pane, &mouse);
                if !forwards && self.renderer.wheel_scroll(x, cy, delta) {
                    return; // consumed client-side
                }
                if forwards {
                    self.forward_mouse(conn, &rect, &mouse);
                }
            }
            Hit::PaneForward(rect) => self.forward_mouse(conn, &rect, &mouse),
            Hit::DragStart { divider, x, y } => {
                self.drag = Some(DragState::Pending { divider, x, y });
            }
            Hit::TextDrag { x, y, release } => self.text_drag_event(x, y, release),
            Hit::PanePress(rect) => {
                // A re-click on the focused pane must not re-select it:
                // `select-pane` always broadcasts `%layout-change`, and
                // the re-seed repaints the whole window — htop read it
                // as the menu opening twice. Focus follows press on
                // change alone.
                let changed = self.renderer.focused() != Some(rect.pane);
                super::trace_input(format_args!(
                    "PanePress pane=%{} changed={}",
                    rect.pane, changed
                ));
                self.renderer.focus(rect.pane);
                if changed {
                    let _ = conn.send_forget(&format!("select-pane -t %{}", rect.pane));
                }
                // The mouse-owning pane keeps its report; anything else
                // (or shift, the bypass modifier) arms a text selection
                // instead — the drag sweeps the pane's cells and release
                // copies, tmux's own mouse-copy gating.
                if mouse.is_left_press()
                    && (mouse.shift() || !self.pane_accepts_mouse(rect.pane, &mouse))
                {
                    let (x, cy) = self.geometry.host_to_content_clamped(
                        mouse.col.saturating_sub(1),
                        mouse.row.saturating_sub(1),
                    );
                    if let Some(anchor) = self.renderer.pane_view_cell(&rect, x, cy) {
                        self.text_drag = Some(TextDrag::Pending {
                            pane: rect.pane,
                            anchor,
                        });
                        // A fresh press clears whatever was selected.
                        self.renderer.set_selection(None);
                        return;
                    }
                }
                if self.pane_accepts_mouse(rect.pane, &mouse) {
                    self.forward_mouse(conn, &rect, &mouse);
                }
            }
        }
        // After the dispatch, not before: a release that ends a drag
        // must already see `drag` cleared, or the pointer would keep
        // the resize shape until an unrelated motion report arrives.
        self.update_pointer(&mouse);
    }

    /// The pointer-shape routing (config `drag-cursor-shape`): a live
    /// divider drag, a live sidebar drag, or a hover report ON a resize
    /// handle wants the resize shape — the OSC 22 the next frame writes;
    /// everything else (option off, a modal up, the pointer elsewhere)
    /// restores the terminal's own pointer. Runs after the hit dispatch:
    /// the shape is the pointer's state, and a drag-ending release must
    /// compute its want from the cleared drag.
    fn update_pointer(&mut self, mouse: &SgrMouse) {
        if !self.cursor.drag_shape || !matches!(self.modal, Modal::None) {
            self.pointer.want(None);
            return;
        }
        // A live drag keeps its shape wherever the pointer wanders: a
        // pane divider drag its axis, a sidebar width drag left/right,
        // a sidebar split drag up/down.
        let dragged = self
            .drag
            .as_ref()
            .map(drag_pointer)
            .or_else(|| self.sidebar_drag.as_ref().map(sidebar_drag_pointer));
        let want = match dragged {
            Some(shape) => Some(shape),
            None if mouse.is_motion() && !mouse.release && mouse.motion_button().is_none() => {
                self.hover_resize(mouse.col.saturating_sub(1), mouse.row.saturating_sub(1))
            }
            None => None,
        };
        self.pointer.want(want);
    }

    /// The resize handle under a hovering pointer (host 0-based
    /// coordinates): the side panel's edge column (width, left/right)
    /// and rule row (split, up/down) — the same predicates the press
    /// hit-test applies — then the pane dividers below the strip row in
    /// content coordinates.
    fn hover_resize(&self, x: u16, y: u16) -> Option<&'static str> {
        let strip = self.renderer.sidebar_width();
        if strip > 0 && y > 0 && x < strip {
            use crate::mux::attach::panels::SidebarRowId;
            // The chevron (the panel's last row, beside the divider) is
            // the collapse handle, not a drag handle; the collapsed rail
            // has no divider column.
            if !self.sidebar.is_collapsed()
                && x + 1 == strip
                && self.renderer.sidebar_row_at(x, y) != Some(SidebarRowId::Collapse)
            {
                return Some(super::pointer::RESIZE_COL);
            }
            if self.renderer.sidebar_rule_at(x, y) {
                return Some(super::pointer::RESIZE_ROW);
            }
            return None;
        }
        if y < self.geometry.strip_rows {
            return None;
        }
        let (x, y) = self.geometry.host_to_content_clamped(x, y);
        self.renderer
            .divider_near(x, y, 0)
            .filter(|_| !self.renderer.label_cell_at(x, y))
            .map(|divider| divider_pointer(&divider))
    }

    /// What one host mouse report hits, in the router's precedence order:
    /// the context menu, the side panel's strip, a drag in flight, the
    /// prompt, the picker, the help panel, the tab strip row, then the
    /// pane area. Reads state only — [`Self::route_mouse`] applies it.
    pub(super) fn mouse_hit(&self, mouse: &SgrMouse) -> Hit {
        // Window-relative, 0-based host coordinates.
        let Some(x) = mouse.col.checked_sub(1) else {
            return Hit::Consumed;
        };
        let Some(y) = mouse.row.checked_sub(1) else {
            return Hit::Consumed;
        };
        let strip = self.renderer.sidebar_width();

        if matches!(self.modal, Modal::Menu(_)) {
            return self.menu_hit(mouse, x, y);
        }
        // The settings dialog is modal for the pointer, like the menu:
        // a press on a tab or a choice acts, everything else is consumed.
        if matches!(self.modal, Modal::Settings(_)) {
            let is_press = !mouse.release
                && !mouse.is_motion()
                && !mouse.is_wheel_up()
                && !mouse.is_wheel_down();
            return match (
                is_press,
                self.renderer.overlay_row_at(x, y),
                self.renderer.overlay_col_at(x),
            ) {
                (true, Some(row), Some(col)) => Hit::SettingsAt { row, col },
                // A press outside the box closes the dialog (herdr's
                // click-away dismissal).
                (true, _, _) if !self.renderer.overlay_contains(x, y) => Hit::SettingsClose,
                _ => Hit::Consumed,
            };
        }
        // The in-app toast sits over the panes: a press on it focuses the
        // agent's pane and dismisses the card.
        if matches!(self.modal, Modal::None | Modal::Scroll)
            && !mouse.release
            && !mouse.is_motion()
            && !mouse.is_wheel_up()
            && !mouse.is_wheel_down()
        {
            if let Some(pane) = self
                .geometry
                .host_to_frame(x, y)
                .and_then(|(fx, fy)| self.renderer.toast_pane_at(fx, fy))
            {
                return Hit::ToastPress { pane };
            }
        }
        // The TOP row stays the tab strip's (the panel begins under it),
        // so a click there falls through to the tab strip even inside
        // the strip's width — the sidebar path used to consume it (the
        // round-5 report).
        // A sidebar drag in flight follows the pointer anywhere (a
        // pointer over the panes must not strand it), like the pane
        // divider drags below.
        if self.sidebar_drag.is_some() && (mouse.release || mouse.is_motion()) {
            return Hit::SidebarDrag {
                x,
                y,
                release: mouse.release,
            };
        }
        if strip > 0 && y > 0 && x < strip {
            return self.sidebar_hit(mouse, x, y);
        }
        // Pane coordinates are layout coordinates: the host x less the
        // side panel's strip width. The TOP row is exempt — the strip
        // paints the panel's title at 0..strip when the panel is up and
        // lays the tabs out from it onward in ABSOLUTE host columns, so
        // the tab hit-test takes the RAW host column (the round-6 fix:
        // the rebase used to run before this branch and shift every
        // panel-up top-row click one panel width left, killing the +
        // button's clickability).
        let rx = x;
        let (x, content_y) = self.geometry.host_to_content_clamped(x, y);

        // A drag in flight continues over motion/release regardless of
        // where the pointer is (a pointer that wanders onto the strip
        // row must not strand it); the drag's press stored content
        // coordinates, so the strip row saturates to content row 0.
        if self.drag.is_some() && (mouse.release || mouse.is_motion()) {
            return Hit::Drag {
                x,
                y: content_y,
                release: mouse.release,
            };
        }
        // A text-selection drag in flight follows the pointer the same
        // way — but a modal that owns the pointer (menu, settings,
        // prompt…) takes precedence over a stale armed drag.
        if self.text_drag.is_some()
            && matches!(self.modal, Modal::None | Modal::Scroll)
            && (mouse.release || mouse.is_motion())
        {
            return Hit::TextDrag {
                x,
                y: content_y,
                release: mouse.release,
            };
        }
        match &self.modal {
            // The rename prompt is modal for the pointer too: every event
            // is consumed while it is up.
            Modal::Prompt(_) => return Hit::Consumed,
            Modal::Picker(picker) => return self.picker_hit(picker, mouse, x, y),
            Modal::Help(_) => return help_hit(mouse),
            Modal::None | Modal::Scroll | Modal::Menu(_) | Modal::Settings(_) | Modal::Resize => {}
        }
        if y < self.geometry.strip_rows {
            return self.tab_strip_hit(mouse, rx);
        }
        // Below the strip, content coordinates are host rows minus the
        // strip row.
        self.pane_area_hit(mouse, x, content_y)
    }

    /// The context menu is modal for the pointer: a press on an action
    /// row dispatches it, a press outside the box closes it (click-away,
    /// like the settings dialog), everything else is consumed while it is
    /// up. The modal centers over the HOST width (the pane layout already
    /// carries the strip offset), so the click maps by the raw column.
    fn menu_hit(&self, mouse: &SgrMouse, x: u16, y: u16) -> Hit {
        if !mouse.release && !mouse.is_motion() && !mouse.is_wheel_up() && !mouse.is_wheel_down() {
            if let Some(row) = self.renderer.overlay_row_at(x, y) {
                return Hit::MenuRow(row);
            }
            if !self.renderer.overlay_contains(x, y) {
                return Hit::MenuClose;
            }
        }
        Hit::Consumed
    }

    /// The side panel owns its strip below the tab-strip row: a press on
    /// a workspace row lands on it (the select+resync every switch
    /// follows), a right-press opens the workspace menu, the footer chips
    /// open the new-workspace prompt / the command menu, and every other
    /// event in the strip is consumed — no pane, divider, or drag lives
    /// in the panel.
    fn sidebar_hit(&self, mouse: &SgrMouse, x: u16, y: u16) -> Hit {
        if mouse.release || mouse.is_motion() {
            return Hit::Consumed;
        }
        use crate::mux::attach::panels::SidebarRowId;
        // The chevron sits beside the divider on the panel's last row,
        // and on the rail (no divider) in its glyph column: it wins
        // over both drag handles.
        if self.renderer.sidebar_row_at(x, y) == Some(SidebarRowId::Collapse) {
            return Hit::SidebarCollapse;
        }
        // The strip's right edge — the painted divider column — is the
        // width-drag handle; the rule row is the split-drag handle.
        // Both before the row lookup: neither is a row. The rail has no
        // divider (its edge column is the status glyph's) and a fixed
        // width.
        if !self.sidebar.is_collapsed() && x + 1 == self.renderer.sidebar_width() {
            return Hit::SidebarResize;
        }
        if self.renderer.sidebar_rule_at(x, y) {
            return Hit::SidebarSplitDrag;
        }
        let Some(id) = self.renderer.sidebar_row_at(x, y) else {
            return Hit::Consumed;
        };
        match id {
            SidebarRowId::New => Hit::SidebarNewWorkspace,
            SidebarRowId::Menu => Hit::SidebarMenu,
            SidebarRowId::Collapse => Hit::SidebarCollapse,
            SidebarRowId::AgentsSort => Hit::SidebarAgentsSort,
            SidebarRowId::Pane(pane) => Hit::SidebarPane { pane },
            SidebarRowId::Workspace(id) => Hit::SidebarWorkspace {
                id,
                menu: mouse.is_right_press(),
            },
        }
    }

    /// The picker is modal for the pointer: wheels move the selection, a
    /// click on a content row selects AND activates it (switch through
    /// the select+resync), a click on the filter line opens the filter
    /// box, and everything else — drags, releases, the footer — is
    /// consumed. `x` is the strip-rebased column.
    fn picker_hit(&self, picker: &PickerState, mouse: &SgrMouse, x: u16, y: u16) -> Hit {
        if mouse.is_wheel_up() {
            return Hit::PickerMove(-1);
        }
        if mouse.is_wheel_down() {
            return Hit::PickerMove(1);
        }
        if mouse.release || mouse.is_motion() {
            return Hit::Consumed;
        }
        let Some(row) = self.renderer.overlay_row_at(x, y) else {
            return Hit::Consumed;
        };
        // The panel's row 0 is the filter line only while a filter is
        // open or set; idle, content starts at row 0 (the idle
        // placeholder row the old mapping assumed is gone). The footer —
        // the panel's last row — does nothing.
        let filtering = picker.filtering || !picker.filter.is_empty();
        let filter_lines = usize::from(filtering);
        if filtering && row == 0 {
            return Hit::PickerFilter;
        }
        match row.checked_sub(filter_lines) {
            Some(content) if content < picker.panel_len.saturating_sub(filter_lines + 1) => {
                Hit::PickerRow(content)
            }
            _ => Hit::Consumed,
        }
    }

    /// The tab strip owns the top row: a left press hit-tests the tabs
    /// and switches windows through the select+resync contract; a RIGHT
    /// press on a tab opens that window's context menu; no pane focus, no
    /// pane forwarding, and no drag ever starts there. `rx` is the RAW
    /// host column.
    fn tab_strip_hit(&self, mouse: &SgrMouse, rx: u16) -> Hit {
        if mouse.is_right_press() {
            return self
                .chrome
                .tab_strip
                .hit_test(rx)
                .and_then(|index| self.status.windows().get(index))
                .map_or(Hit::Consumed, |(id, _)| Hit::TabMenu(id.clone()));
        }
        if mouse.is_wheel_up() || mouse.is_wheel_down() {
            // A wheel only matters when the strip hides tabs; anything
            // else falls through consumed (the panes get nothing from
            // the strip row).
            let delta = if mouse.is_wheel_up() { 3 } else { -3 };
            if self.chrome.tab_strip.overflowing() {
                return Hit::TabWheel(delta);
            }
            return Hit::Consumed;
        }
        let is_press =
            !mouse.release && !mouse.is_motion() && !mouse.is_wheel_up() && !mouse.is_wheel_down();
        if is_press {
            Hit::TabPress(rx)
        } else if self.tab_drag_from.is_some() {
            Hit::TabDrop(rx)
        } else {
            Hit::Consumed
        }
    }

    /// The pane area (content coordinates `x`, `cy`): wheels go to the
    /// pane under the pointer (unless a held button owns the pointer);
    /// motion and release continue a drag or forward to a mouse-owning
    /// pane; a press ON a divider cell starts a drag — only the painted
    /// divider column/row itself, never the pane content cells beside it
    /// (a one-cell tolerance hijacked edge clicks from the app: the
    /// manual-pass trace's swallowed htop click) — unless the cell is an
    /// embedded border label (text cells are not drag handles, herdr's
    /// semantics; the plain border segments around a label stay
    /// draggable) — and otherwise focuses the pane.
    fn pane_area_hit(&self, mouse: &SgrMouse, x: u16, cy: u16) -> Hit {
        if mouse.is_wheel_up() || mouse.is_wheel_down() {
            if self.drag.is_some() {
                return Hit::Consumed; // the held button owns the pointer; wheels wait
            }
            if self.text_drag.is_some() {
                return Hit::Consumed; // ditto a selection drag mid-sweep
            }
            return match self.renderer.pane_at(x, cy) {
                Some(rect) => Hit::Wheel {
                    rect: *rect,
                    x,
                    cy,
                    delta: if mouse.is_wheel_up() { 3 } else { -3 },
                },
                None => Hit::Consumed,
            };
        }
        if mouse.release || mouse.is_motion() {
            if self.drag.is_some() {
                return Hit::Drag {
                    x,
                    y: cy,
                    release: mouse.release,
                };
            }
            if self.text_drag.is_some() && matches!(self.modal, Modal::None | Modal::Scroll) {
                return Hit::TextDrag {
                    x,
                    y: cy,
                    release: mouse.release,
                };
            }
            // Drag/release only matter to a pane whose mode accepts
            // them; focus follows press only.
            return match self.renderer.pane_at(x, cy) {
                Some(rect) if self.pane_accepts_mouse(rect.pane, mouse) => Hit::PaneForward(*rect),
                _ => Hit::Consumed,
            };
        }
        if let Some(divider) = self.renderer.divider_near(x, cy, 0) {
            if !self.renderer.label_cell_at(x, cy) {
                return Hit::DragStart { divider, x, y: cy };
            }
        }
        match self.renderer.pane_at(x, cy) {
            Some(rect) => Hit::PanePress(*rect),
            None => Hit::Consumed,
        }
    }

    /// One motion or release while a text-selection drag is in flight.
    /// The first motion promotes the pending press to an active
    /// selection (anchor set, head following the pointer); release ends
    /// it — an ACTIVE drag copies the swept text as an OSC 52 write and
    /// clears, a release that never moved is just the click the press
    /// already handled (focus only, nothing selected, nothing copied).
    /// A pointer outside the drag's own pane clamps to that pane's view,
    /// the same saturation the paint path gives it.
    fn text_drag_event(&mut self, x: u16, y: u16, release: bool) {
        let state = match self.text_drag.take() {
            Some(state) => state,
            // A release with no drag armed (the modal-precedence case):
            // the click already cleared any selection on its press.
            None => {
                return;
            }
        };
        let pane = match &state {
            TextDrag::Pending { pane, .. } | TextDrag::Active { pane } => *pane,
        };
        let Some(rect) = self.renderer.pane_rect(pane) else {
            self.renderer.set_selection(None);
            return;
        };
        let head = self.renderer.pane_view_cell(&rect, x, y);
        match (state, release, head) {
            (TextDrag::Pending { anchor, .. }, false, Some(head)) => {
                self.renderer
                    .set_selection(Some(Selection::Chars { pane, anchor, head }));
                self.text_drag = Some(TextDrag::Active { pane });
            }
            // Motion without a cell (pane vanished mid-drag): drop it.
            (TextDrag::Pending { .. }, false, None) => {}
            // A click, not a drag: focus already followed the press.
            (TextDrag::Pending { .. }, true, _) => {}
            (TextDrag::Active { .. }, false, Some(head)) => {
                if let Some(Selection::Chars { anchor, .. }) = self.renderer.selection {
                    self.renderer
                        .set_selection(Some(Selection::Chars { pane, anchor, head }));
                }
                self.text_drag = Some(TextDrag::Active { pane });
            }
            (TextDrag::Active { .. }, false, None) => {
                self.text_drag = Some(TextDrag::Active { pane })
            }
            (TextDrag::Active { .. }, true, _) => self.copy_selection(),
        }
    }

    /// A press on the tab strip: the ` + ` button opens the new-tab
    /// prompt; any tab column switches to that window through the
    /// existing select+resync contract (daemon-side `select-window`,
    /// then a full re-seed). A click on the already-shown window, or on
    /// a pad/marker column, does nothing. No pane focus, no pane
    /// forwarding.
    pub(super) fn tab_click(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, x: u16) {
        // A press arms a drag regardless of what the press does: motion
        // to another tab and a release reorder.
        self.tab_drag_from = self.chrome.tab_strip.hit_test(x);
        if self.chrome.tab_strip.plus_hit(x) {
            self.enter_prompt(PromptTarget::NewWindow);
            return;
        }
        let Some(index) = self.chrome.tab_strip.hit_test(x) else {
            return;
        };
        let Some((id, _)) = self.status.windows().get(index) else {
            return;
        };
        let id = id.clone();
        if id == self.window {
            return; // the shown window: a no-op click
        }
        self.land_window(conn, &id);
    }

    /// Motion or release on the strip while a tab drag is armed: a drop
    /// on another tab reorders (`move-window -s @from -t <slot>`, the
    /// %sessions-changed reply refreshing the strip); a drop on the
    /// origin tab is just the click the press already handled. The wheel
    /// scroll offset resets with the fresh order.
    pub(super) fn tab_drop(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, x: u16) {
        let Some(from) = self.tab_drag_from.take() else {
            return;
        };
        let Some(to) = self.chrome.tab_strip.hit_test(x) else {
            return;
        };
        if to == from {
            return;
        }
        let Some((id, _)) = self.status.windows().get(from) else {
            return;
        };
        let id = id.clone();
        self.send_then(
            conn,
            &format!("move-window -s {id} -t {to}"),
            |s, _c, reply| {
                if reply.ok {
                    s.chrome.tab_strip.reset_scroll();
                    s.status_dirty = true;
                    s.draw_tab_strip();
                }
            },
        );
    }

    /// Whether the pane's negotiated mouse mode accepts THIS report —
    /// the per-event forwarding gate. The host captures in any-event
    /// mode (crossterm's `EnableMouseCapture` enables 1003), so every
    /// hover arrives as a report, but a pane that negotiated only 1000
    /// (htop, vim's defaults) never asked for motion, and a forwarded
    /// hover (cb 35) reads as button-3 activity to an ncurses app.
    /// Presses go to any tracking pane; releases need 1000+; drag
    /// motion needs 1002+; hover motion needs 1003.
    pub(super) fn pane_accepts_mouse(&self, pane: u32, mouse: &SgrMouse) -> bool {
        use par_term_emu_core::mouse::MouseMode;
        let Some(mode) = self.renderer.pane_terminal(pane).map(|t| t.mouse_mode()) else {
            return false;
        };
        match mode {
            MouseMode::Off => false,
            MouseMode::X10 => mouse.is_press(),
            MouseMode::Normal => !mouse.is_motion(),
            MouseMode::ButtonEvent => mouse.motion_button().is_some() || !mouse.is_motion(),
            MouseMode::AnyEvent => true,
        }
    }

    /// Re-encode one host mouse report pane-relative, in the dialect the
    /// pane negotiated, and send it. The report's row is host 1-based;
    /// the pane rects live in content coordinates (below the strip row),
    /// so the strip comes off first.
    pub(super) fn forward_mouse(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        rect: &PaneRect,
        mouse: &SgrMouse,
    ) {
        // The click in content coordinates: the host report loses its
        // 1-based origin, the side panel's strip width, and the strip
        // row; content_view adds the pane-borders/pane-gaps inset the
        // paint path uses, so the pane sees the cell the user actually
        // clicked (the manual-pass +1-row/+1-col report: the border ring
        // was never subtracted).
        let (x, cy) = self
            .geometry
            .host_to_content_clamped(mouse.col.saturating_sub(1), mouse.row.saturating_sub(1));
        let (inset_x, inset_y, view_w, view_h) = self.renderer.content_view(rect);
        let rel_col = x
            .saturating_sub(rect.x + inset_x)
            .min(view_w.saturating_sub(1));
        let rel_row = cy
            .saturating_sub(rect.y + inset_y)
            .min(view_h.saturating_sub(1));
        // The pane's dialect picks the wire format: a pane that skipped
        // 1006 gets the classic `ESC[M` spelling it can decode — an SGR
        // report it never asked for is misparsed as keystrokes (the
        // manual-pass htop sort-dialog report: htop under iTerm.app's
        // bundled terminfo enables only 1000).
        let encoding = self
            .renderer
            .pane_terminal(rect.pane)
            .map(|t| t.mouse_encoding())
            .unwrap_or(par_term_emu_core::mouse::MouseEncoding::Sgr);
        let bytes = mouse.reencode(encoding, rel_col, rel_row);
        super::trace_input(format_args!(
            "forward pane=%{} bytes={:?}",
            rect.pane,
            String::from_utf8_lossy(&bytes)
        ));
        self.forward_keys(conn, format!("%{}", rect.pane), &bytes);
    }
}

/// The pointer shape for a drag in flight — its divider's axis.
fn drag_pointer(drag: &DragState) -> &'static str {
    match drag {
        DragState::Pending { divider, .. } | DragState::Active { divider, .. } => {
            divider_pointer(divider)
        }
    }
}

/// The pointer shape for a sidebar drag in flight: the width edge drags
/// left/right, the rule row up/down.
fn sidebar_drag_pointer(drag: &super::sidebar::SidebarDrag) -> &'static str {
    match drag {
        super::sidebar::SidebarDrag::Width { .. } => super::pointer::RESIZE_COL,
        super::sidebar::SidebarDrag::Split { .. } => super::pointer::RESIZE_ROW,
    }
}

/// A vertical divider column resizes left/right; a horizontal row
/// up/down.
fn divider_pointer(divider: &DividerHit) -> &'static str {
    if divider.vertical {
        super::pointer::RESIZE_COL
    } else {
        super::pointer::RESIZE_ROW
    }
}
