//! Input routing: plain-byte runs through the prefix scanner and the
//! modal owners, and mouse reports through the overlay hit-testers.

use super::*;

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
        if self.prompt_mode {
            // The prompt owns the run whole: every byte feeds the input
            // line until Enter commits or Escape cancels; the closing
            // byte and everything after in the run is dropped (the
            // picker's modal-run shape).
            for &byte in bytes {
                if !self.prompt_byte(conn, byte) {
                    return false;
                }
            }
            return false;
        }
        if self.menu.is_some() {
            for &byte in bytes {
                if !self.menu_byte(byte) {
                    return false;
                }
            }
            return false;
        }
        if self.help_mode {
            for &byte in bytes {
                if !self.help_byte(byte) {
                    return false;
                }
            }
            return false;
        }
        if self.picker_mode {
            for &byte in bytes {
                if !self.picker_byte(conn, byte) {
                    return false;
                }
            }
            return false;
        }
        if self.resize_mode {
            // Resize mode owns plain runs: the bytes are typed keys that
            // would leak into the pane, so the run exits the mode and is
            // consumed whole (arrows arrive as Key tokens, not bytes).
            self.resize_mode = false;
            return false;
        }
        let mut to_send: Vec<u8> = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if *prefix_pending {
                *prefix_pending = false;
                // The reload chord matches by byte before the fixed
                // table (configurable; the default C-r does not collide
                // with the literal-key arms).
                if byte == self.reload_key && byte != b'd' {
                    self.reload_config(conn);
                    continue;
                }
                // The management chords match by byte before the fixed
                // table (configurable; the defaults `%`, `"`, `x`, `c`
                // are consumed unbound by the table today).
                let management = match byte {
                    b if b == self.management.split_right => Some(ManagementKey::SplitRight),
                    b if b == self.management.split_down => Some(ManagementKey::SplitDown),
                    b if b == self.management.kill_pane => Some(ManagementKey::KillPane),
                    b if b == self.management.new_window => Some(ManagementKey::NewWindow),
                    b if b == self.management.swap_prev => Some(ManagementKey::SwapPrev),
                    b if b == self.management.swap_next => Some(ManagementKey::SwapNext),
                    b if b == self.management.workspace_next => Some(ManagementKey::WorkspaceNext),
                    b if b == self.management.workspace_prev => Some(ManagementKey::WorkspacePrev),
                    b if b == self.management.zoom => Some(ManagementKey::Zoom),
                    b if b == self.management.rename_window => Some(ManagementKey::RenameWindow),
                    b if b == self.management.rename_pane => Some(ManagementKey::RenamePane),
                    b if b == self.management.border_cycle => Some(ManagementKey::BorderCycle),
                    b if b == self.management.label_toggle => Some(ManagementKey::Labels),
                    b if b == self.management.workspace_picker => {
                        Some(ManagementKey::WorkspacePicker)
                    }
                    b if b == self.management.sidebar => Some(ManagementKey::Sidebar),
                    b if b == self.management.status_bar => Some(ManagementKey::StatusBar),
                    _ => None,
                };
                if let Some(key) = management {
                    self.management_chord(key, conn);
                    continue;
                }
                // The resize chord: a sticky mode — arrows adjust the
                // focused pane's edges until Enter/Escape/q.
                if byte == self.management.resize {
                    self.enter_resize_mode();
                    continue;
                }
                // The help chord: the bindings overlay over the frame.
                if byte == self.management.help {
                    self.enter_help();
                    continue;
                }
                // The picker chord: the session/window modal over the
                // frame.
                if byte == self.management.picker {
                    self.enter_picker(conn);
                    continue;
                }
                match byte {
                    b'd' => return true,
                    b'[' => {
                        // prefix [ — the scroll viewport on the focused
                        // pane. No scrollback means nothing to scroll;
                        // the key is consumed either way.
                        if let Some(id) = self.renderer.focused() {
                            if self.renderer.enter_scroll_mode(id) {
                                self.scroll_mode = true;
                            }
                        }
                    }
                    b'n' | b'p' | b'(' | b')' | b'o' => self.prefix_switch(byte, conn),
                    b if b == self.literal => to_send.push(byte), // literal prefix
                    _ => {}                                       // unbound: consumed
                }
            } else if byte == self.prefix {
                *prefix_pending = true;
            } else if self.scroll_mode {
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

    /// The prefix commands that move the view through the daemon's tree:
    /// `o` cycles panes of the window, `n`/`p` next/prev window, `(`/`)`
    /// prev/next session — every switch is select-then-refresh, the
    /// daemon-side select + resync passthrough dispatches, with the
    /// renderer rebuilding from the fresh replays.
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
        use crate::keyboard::TermKey;
        // Shift+arrow: swap with the pane in that direction. tmux's
        // swap-pane keeps focus following the pane's content, so no
        // re-select is needed — the %layout-change broadcast re-seeds the
        // view with the focused pane in the new cell. The shift bit is
        // matched loosely (a terminal may co-report other modifiers), and
        // both outcomes flash on the status row so a no-op at an edge is
        // never silent (the manual-pass report: the chord felt dead).
        if ev.modifiers & crate::keyboard::modifiers::SHIFT != 0 {
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
    pub(super) fn route_mouse(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        mouse: SgrMouse,
    ) {
        // Window-relative, 0-based host coordinates.
        let Some(x) = mouse.col.checked_sub(1) else {
            return;
        };
        let Some(y) = mouse.row.checked_sub(1) else {
            return;
        };
        let strip = self.renderer.sidebar_width();

        // The context menu is modal for the pointer: a press on an
        // action row dispatches it, everything else is consumed while
        // it is up.
        if self.menu.is_some() {
            if !mouse.release
                && !mouse.is_motion()
                && !mouse.is_wheel_up()
                && !mouse.is_wheel_down()
            {
                // The modal centers over the HOST width (the pane layout
                // already carries the strip offset), so the click maps by
                // the raw column — the picker's guard does the same.
                if let Some(row) = self.renderer.overlay_row_at(x, y) {
                    self.menu_click(conn, row);
                }
            }
            return;
        }

        // The side panel owns its strip below the tab-strip row: a
        // press on a workspace row lands on it (the select+resync every
        // switch follows), a right-press opens the workspace menu, the
        // footer chips open the new-workspace prompt / the command
        // menu, and every other event in the strip is consumed — no
        // pane, divider, or drag lives in the panel. The TOP row stays the tab strip's (the panel
        // begins under it), so a click there falls through to the tab
        // strip even inside the strip's width — the sidebar path used
        // to consume it (the round-5 report).
        if strip > 0 && y > 0 && x < strip {
            if !mouse.release && !mouse.is_motion() {
                if let Some(id) = self.renderer.sidebar_row_at(x, y) {
                    if id == super::super::SIDEBAR_NEW_ID {
                        self.enter_prompt(PromptTarget::NewWorkspace);
                    } else if id == super::super::SIDEBAR_MENU_ID {
                        self.open_menu(MenuTarget::Commands);
                    } else if let Some(ws_id) = id.strip_prefix("ws:") {
                        if mouse.is_right_press() {
                            self.open_menu(MenuTarget::Workspace(ws_id.to_string()));
                        } else {
                            self.land_on_workspace(conn, ws_id);
                        }
                    }
                }
            }
            return;
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
        let x = x.saturating_sub(strip);

        // A drag in flight continues over motion/release regardless of
        // where the pointer is (a pointer that wanders onto the strip
        // row must not strand it); the drag's press stored content
        // coordinates, so the strip row saturates to content row 0.
        if self.drag.is_some() && (mouse.release || mouse.is_motion()) {
            self.drag_event(conn, x, y.saturating_sub(1), mouse.release);
            return;
        }

        // The rename prompt is modal for the pointer too: every event is
        // consumed while it is up.
        if self.prompt_mode {
            return;
        }

        // The picker is modal for the pointer too: wheels move the
        // selection, a click on a content row selects AND activates it
        // (switch through the select+resync), a click on the filter line
        // opens the filter box, and everything else is consumed.
        if self.picker_mode {
            if mouse.is_wheel_up() {
                self.picker_move(-1);
            } else if mouse.is_wheel_down() {
                self.picker_move(1);
            } else if mouse.release || mouse.is_motion() {
                // Drags do nothing in the modal; only a press click.
            } else if let Some(row) = self.renderer.overlay_row_at(x, y) {
                // The panel's row 0 is the filter line only while a
                // filter is open or set; idle, content starts at row 0
                // (the idle placeholder row the old mapping assumed is
                // gone). The footer — the panel's last row — does
                // nothing.
                let filtering = self.picker_filtering || !self.picker_filter.is_empty();
                let filter_lines = usize::from(filtering);
                if filtering && row == 0 {
                    self.picker_filtering = true;
                    self.refresh_picker();
                } else if let Some(content) = row.checked_sub(filter_lines) {
                    let window_len = self.picker_panel_len.saturating_sub(filter_lines + 1);
                    if content < window_len {
                        // Move the cursor to the clicked row and activate
                        // it (start maps the windowed index onto the
                        // filtered list).
                        self.picker_selected = self.picker_start + content;
                        self.refresh_picker();
                        self.picker_activate(conn);
                    }
                }
            }
            return;
        }

        // The help panel is modal for the pointer too: while it is up
        // every mouse event is consumed — wheels scroll the PANEL (the
        // round-3 defect: they fell through to the pane scrollback /
        // pane forwarding), clicks and drags do nothing.
        if self.help_mode {
            if mouse.is_wheel_up() {
                self.help_scroll_by(-3);
            } else if mouse.is_wheel_down() {
                self.help_scroll_by(3);
            }
            return;
        }

        // The tab strip owns the top row: a left press hit-tests the
        // tabs and switches windows through the select+resync contract;
        // a RIGHT press on a tab opens that window's context menu; no
        // pane focus, no pane forwarding, and no drag ever starts
        // there. The hit-test takes the RAW host column (see above).
        if y == 0 {
            let is_press = !mouse.release
                && !mouse.is_motion()
                && !mouse.is_wheel_up()
                && !mouse.is_wheel_down();
            if mouse.is_right_press() {
                if let Some((id, _)) = self
                    .tab_strip
                    .hit_test(rx)
                    .and_then(|index| self.status.windows().get(index))
                {
                    let id = id.clone();
                    self.open_menu(MenuTarget::Tab(id));
                }
            } else if is_press {
                self.tab_click(conn, rx);
            }
            return;
        }
        // Below the strip, content coordinates are host rows minus the
        // strip row.
        let Some(cy) = y.checked_sub(1) else {
            return;
        };

        if mouse.is_wheel_up() || mouse.is_wheel_down() {
            if self.drag.is_some() {
                return; // the held button owns the pointer; wheels wait
            }
            let Some(rect) = self.renderer.pane_at(x, cy).cloned() else {
                return;
            };
            let owns = self.pane_owns_mouse(rect.pane);
            let delta: isize = if mouse.is_wheel_up() { 3 } else { -3 };
            if !owns && self.renderer.wheel_scroll(x, cy, delta) {
                return; // consumed client-side
            }
            if owns {
                self.forward_mouse(conn, &rect, &mouse);
            }
            return;
        }

        if mouse.release || mouse.is_motion() {
            if self.drag.is_some() {
                self.drag_event(conn, x, cy, mouse.release);
                return;
            }
            // Drag/release only matter to a pane that owns the mouse;
            // focus follows press only.
            let Some(rect) = self.renderer.pane_at(x, cy).cloned() else {
                return;
            };
            if self.pane_owns_mouse(rect.pane) {
                self.forward_mouse(conn, &rect, &mouse);
            }
            return;
        }

        // A press: a divider hit starts a drag (never a click-through) —
        // UNLESS the cell is an embedded border label (text cells are not
        // drag handles, herdr's semantics); then it falls through to the
        // focus path below. The plain border segments around a label stay
        // draggable (divider_near still matches the boundary line).
        if let Some(divider) = self.renderer.divider_near(x, cy, 1) {
            if !self.renderer.label_cell_at(x, cy) {
                self.drag = Some(DragState::Pending { divider, x, y: cy });
                return;
            }
        }
        let Some(rect) = self.renderer.pane_at(x, cy).cloned() else {
            return;
        };
        self.renderer.focus(rect.pane);
        let _ = conn.send_checked(&format!("select-pane -t %{}", rect.pane));
        if self.pane_owns_mouse(rect.pane) {
            self.forward_mouse(conn, &rect, &mouse);
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
        if !conn
            .send_checked(&format!("select-window -t {id}"))
            .is_ok_and(|reply| reply.ok)
        {
            return;
        }
        self.reseed_window(conn, &id);
    }

    /// Whether `pane`'s emulator tracks the mouse (the forwarding gate).
    pub(super) fn pane_owns_mouse(&self, pane: u32) -> bool {
        self.renderer
            .pane_terminal(pane)
            .is_some_and(|t| t.mouse_mode() != crate::mouse::MouseMode::Off)
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
    /// pump's pending_layout path.
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
        let rel_col = mouse.col.saturating_sub(1).saturating_sub(rect.x);
        let rel_row = mouse.row.saturating_sub(2).saturating_sub(rect.y);
        let bytes = mouse.reencode_sgr(rel_col, rel_row);
        super::super::forward_chunked(conn, format!("%{}", rect.pane), &bytes);
    }
}
