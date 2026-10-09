//! Input routing: plain-byte runs through the prefix scanner and the
//! modal owners, and mouse reports through the overlay hit-testers.

use super::*;

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
    /// The side panel's ` new ` chip: open the new-workspace prompt.
    SidebarNewWorkspace,
    /// The side panel's ` menu ` chip: open the command menu.
    SidebarMenu,
    /// A side panel workspace row (`+N`): land on it, or open its
    /// context menu on a right press (`menu`).
    SidebarWorkspace { id: String, menu: bool },
    /// Motion or release while a divider drag is in flight.
    Drag { x: u16, y: u16, release: bool },
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
        prefix_pending: &mut bool,
    ) -> bool {
        if self.feed_modal_run(bytes, conn) {
            return false;
        }
        let mut to_send: Vec<u8> = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if *prefix_pending {
                *prefix_pending = false;
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
                    PrefixChord::Unbound => {}
                }
            } else if byte == self.prefix {
                *prefix_pending = true;
            } else if matches!(self.modal, Modal::Scroll) {
                // Scroll mode's plain keys: q and Enter exit (the
                // viewport is a modal view — keys do not leak into the
                // pane).
                if byte == b'q' || byte == b'\r' {
                    self.leave_scroll_mode();
                }
            } else {
                to_send.push(byte);
            }
        }
        // Typing snaps this pane's client scroll back to live.
        if !to_send.is_empty() {
            if let Some(id) = self.renderer.focused() {
                self.renderer.snap_to_live(id);
            }
            super::super::forward_chunked(conn, self.focused_pane(), &to_send);
        }
        false
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
        prefix_pending: &mut bool,
    ) {
        *prefix_pending = false;
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
        super::super::forward_chunked(conn, self.focused_pane(), &bytes);
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
        prefix_pending: &mut bool,
    ) {
        if *prefix_pending {
            *prefix_pending = false;
            self.prefix_pane_arrow(conn, ev);
            return;
        }
        match &self.modal {
            Modal::Scroll => return self.scroll_mode_key(ev),
            Modal::Prompt(_) => return self.prompt_key(conn, ev),
            Modal::Menu(_) => return self.menu_key(ev),
            Modal::Help(_) => return self.help_key(ev),
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
            super::super::forward_chunked(conn, focused, &bytes);
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
                self.flash = Some("no pane in that direction".to_string());
                self.draw_status_row();
                return;
            };
            let _ = conn.send_checked(&format!("swap-pane -s %{focused_id} -t %{next}"));
            self.flash = Some(format!("swapped %{focused_id} with %{next}"));
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
        let _ = conn.send_checked(&format!("select-pane -t %{next}"));
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
        match self.mouse_hit(&mouse) {
            Hit::Consumed => {}
            Hit::MenuRow(row) => self.menu_click(conn, row),
            Hit::SidebarNewWorkspace => self.enter_prompt(PromptTarget::NewWorkspace),
            Hit::SidebarMenu => self.open_menu(MenuTarget::Commands),
            Hit::SidebarWorkspace { id, menu: true } => {
                self.open_menu(MenuTarget::Workspace(id));
            }
            Hit::SidebarWorkspace { id, menu: false } => self.land_on_workspace(conn, &id),
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
            Hit::Wheel { rect, x, cy, delta } => {
                let owns = self.pane_owns_mouse(rect.pane);
                if !owns && self.renderer.wheel_scroll(x, cy, delta) {
                    return; // consumed client-side
                }
                if owns {
                    self.forward_mouse(conn, &rect, &mouse);
                }
            }
            Hit::PaneForward(rect) => self.forward_mouse(conn, &rect, &mouse),
            Hit::DragStart { divider, x, y } => {
                self.drag = Some(DragState::Pending { divider, x, y });
            }
            Hit::PanePress(rect) => {
                self.renderer.focus(rect.pane);
                let _ = conn.send_checked(&format!("select-pane -t %{}", rect.pane));
                if self.pane_owns_mouse(rect.pane) {
                    self.forward_mouse(conn, &rect, &mouse);
                }
            }
        }
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
        // The TOP row stays the tab strip's (the panel begins under it),
        // so a click there falls through to the tab strip even inside
        // the strip's width — the sidebar path used to consume it (the
        // round-5 report).
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
        match &self.modal {
            // The rename prompt is modal for the pointer too: every event
            // is consumed while it is up.
            Modal::Prompt(_) => return Hit::Consumed,
            Modal::Picker(picker) => return self.picker_hit(picker, mouse, x, y),
            Modal::Help(_) => return help_hit(mouse),
            Modal::None | Modal::Scroll | Modal::Menu(_) | Modal::Resize => {}
        }
        if y < self.geometry.strip_rows {
            return self.tab_strip_hit(mouse, rx);
        }
        // Below the strip, content coordinates are host rows minus the
        // strip row.
        self.pane_area_hit(mouse, x, content_y)
    }

    /// The context menu is modal for the pointer: a press on an action
    /// row dispatches it, everything else is consumed while it is up.
    /// The modal centers over the HOST width (the pane layout already
    /// carries the strip offset), so the click maps by the raw column.
    fn menu_hit(&self, mouse: &SgrMouse, x: u16, y: u16) -> Hit {
        if !mouse.release && !mouse.is_motion() && !mouse.is_wheel_up() && !mouse.is_wheel_down() {
            if let Some(row) = self.renderer.overlay_row_at(x, y) {
                return Hit::MenuRow(row);
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
        let Some(id) = self.renderer.sidebar_row_at(x, y) else {
            return Hit::Consumed;
        };
        if id == super::super::SIDEBAR_NEW_ID {
            Hit::SidebarNewWorkspace
        } else if id == super::super::SIDEBAR_MENU_ID {
            Hit::SidebarMenu
        } else if let Some(ws_id) = id.strip_prefix("ws:") {
            Hit::SidebarWorkspace {
                id: ws_id.to_string(),
                menu: mouse.is_right_press(),
            }
        } else {
            Hit::Consumed
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
                .tab_strip
                .hit_test(rx)
                .and_then(|index| self.status.windows().get(index))
                .map_or(Hit::Consumed, |(id, _)| Hit::TabMenu(id.clone()));
        }
        let is_press =
            !mouse.release && !mouse.is_motion() && !mouse.is_wheel_up() && !mouse.is_wheel_down();
        if is_press {
            Hit::TabPress(rx)
        } else {
            Hit::Consumed
        }
    }

    /// The pane area (content coordinates `x`, `cy`): wheels go to the
    /// pane under the pointer (unless a held button owns the pointer);
    /// motion and release continue a drag or forward to a mouse-owning
    /// pane; a press near a divider starts a drag — unless the cell is an
    /// embedded border label (text cells are not drag handles, herdr's
    /// semantics; the plain border segments around a label stay
    /// draggable) — and otherwise focuses the pane.
    fn pane_area_hit(&self, mouse: &SgrMouse, x: u16, cy: u16) -> Hit {
        if mouse.is_wheel_up() || mouse.is_wheel_down() {
            if self.drag.is_some() {
                return Hit::Consumed; // the held button owns the pointer; wheels wait
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
            // Drag/release only matter to a pane that owns the mouse;
            // focus follows press only.
            return match self.renderer.pane_at(x, cy) {
                Some(rect) if self.pane_owns_mouse(rect.pane) => Hit::PaneForward(*rect),
                _ => Hit::Consumed,
            };
        }
        if let Some(divider) = self.renderer.divider_near(x, cy, 1) {
            if !self.renderer.label_cell_at(x, cy) {
                return Hit::DragStart { divider, x, y: cy };
            }
        }
        match self.renderer.pane_at(x, cy) {
            Some(rect) => Hit::PanePress(*rect),
            None => Hit::Consumed,
        }
    }

    /// A press on the tab strip: the ` + ` button opens the new-tab
    /// prompt; any tab column switches to that window through the
    /// existing select+resync contract (daemon-side `select-window`,
    /// then a full re-seed). A click on the already-shown window, or on
    /// a pad/marker column, does nothing. No pane focus, no pane
    /// forwarding.
    pub(super) fn tab_click(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, x: u16) {
        if self.tab_strip.plus_hit(x) {
            self.enter_prompt(PromptTarget::NewWindow);
            return;
        }
        let Some(index) = self.tab_strip.hit_test(x) else {
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

    /// Whether `pane`'s emulator tracks the mouse (the forwarding gate).
    pub(super) fn pane_owns_mouse(&self, pane: u32) -> bool {
        self.renderer
            .pane_terminal(pane)
            .is_some_and(|t| t.mouse_mode() != par_term_emu_core::mouse::MouseMode::Off)
    }

    /// One motion or release while a drag is in flight: motion promotes a
    /// pending press to an active drag and records the pointer's signed
    /// delta from the press point (cells along the divider's axis);
    /// release ends the drag and clears the highlight — a release that
    /// never moved falls through as a click (focus the pane under the
    /// pointer, forward the release when the pane owns the mouse).
    pub(super) fn drag_event(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        x: u16,
        y: u16,
        release: bool,
    ) {
        if release {
            let state = self.drag.take();
            self.renderer.set_drag_divider(None);
            if let Some(DragState::Pending { .. }) = state {
                if let Some(rect) = self.renderer.pane_at(x, y).cloned() {
                    self.renderer.focus(rect.pane);
                    let _ = conn.send_checked(&format!("select-pane -t %{}", rect.pane));
                    let mouse = SgrMouse {
                        cb: 0,
                        col: x + 1,
                        row: y + 1,
                        release: true,
                    };
                    if self.pane_owns_mouse(rect.pane) {
                        self.forward_mouse(conn, &rect, &mouse);
                    }
                }
            }
            return;
        }
        // Motion.
        // Promote only a PENDING press: an already-active drag must stay
        // in place — `take()` here on an Active drag would drop the whole
        // drag state, starving every resize after the first motion (the
        // round-3 "highlight engages, drag does not resize" defect).
        if matches!(&self.drag, Some(DragState::Pending { .. })) {
            if let Some(DragState::Pending { divider, x, y }) = self.drag.take() {
                self.drag = Some(DragState::Active {
                    divider,
                    x,
                    y,
                    applied: 0,
                    pending: 0,
                });
                self.renderer
                    .set_drag_divider(Some((divider.vertical, divider.a, divider.b)));
            }
        }
        if let Some(DragState::Active {
            divider,
            x: px,
            y: py,
            pending,
            ..
        }) = &mut self.drag
        {
            *pending = if divider.vertical {
                i32::from(x) - i32::from(*px)
            } else {
                i32::from(y) - i32::from(*py)
            };
        }
    }

    /// The pump's frame-cadence drag application: one relative
    /// `resize-pane` per unapplied cell of delta, aimed at the boundary's
    /// left/top pane (the daemon re-divides the neighbor). Best-effort —
    /// the %layout-change broadcast re-seeds the window through the
    /// pump's parked-layout path (PendingWork).
    pub(super) fn apply_drag(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Some(DragState::Active {
            divider,
            pending,
            applied,
            ..
        }) = &mut self.drag
        else {
            return;
        };
        let step = *pending - *applied;
        if step == 0 {
            return;
        }
        let flag = if divider.vertical {
            if step > 0 {
                "-R"
            } else {
                "-L"
            }
        } else if step > 0 {
            "-D"
        } else {
            "-U"
        };
        let _ = conn.send_checked(&format!(
            "resize-pane -t %{a} {flag} {n}",
            a = divider.a,
            n = step.abs()
        ));
        *applied = *pending;
    }

    /// Re-encode one host mouse report pane-relative and send it. The
    /// report's row is host 1-based; the pane rects live in content
    /// coordinates (below the strip row), so the strip comes off
    /// first.
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
        let bytes = mouse.reencode_sgr(rel_col, rel_row);
        super::super::forward_chunked(conn, format!("%{}", rect.pane), &bytes);
    }
}
