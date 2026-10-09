//! Navigation: the management chords and the window, session, and
//! workspace switches, each landing through the select+resync contract.

use super::*;

impl WindowSession {
    /// The management chords in render mode: issue the daemon command for
    /// the focused pane/session, then land the view through the same
    /// re-seed contract `switch_window` follows — split lands on the new
    /// pane (the reply body IS its id, the daemon focuses it), kill lands
    /// on the window's survivor, new-window re-seeds the fresh window.
    pub(super) fn management_chord(
        &mut self,
        key: super::super::ManagementKey,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) {
        let focused = self.focused_pane();
        match key {
            super::super::ManagementKey::SplitRight | super::super::ManagementKey::SplitDown => {
                let flag = if matches!(key, super::super::ManagementKey::SplitRight) {
                    " -h"
                } else {
                    ""
                };
                let Ok(reply) = conn.send_checked(&format!("split-window -t {focused}{flag}"))
                else {
                    return;
                };
                if !reply.ok {
                    return;
                }
                // The reply body is the new pane id; re-seed the window
                // from the fresh layout (the split broadcast rides the
                // reply), THEN focus the fresh pane — the renderer only
                // knows the pane once the reseed installed the new
                // layout, so the focus must come after.
                let new_pane = reply
                    .body
                    .first()
                    .and_then(|id| id.trim().strip_prefix('%'))
                    .and_then(|n| n.parse::<u32>().ok());
                let window = self.window.clone();
                self.reseed_window(conn, &window);
                if let Some(new_pane) = new_pane {
                    self.renderer.focus(new_pane);
                    let _ = conn.send_checked(&format!("select-pane -t %{new_pane}"));
                }
            }
            super::super::ManagementKey::KillPane => {
                let Ok(reply) = conn.send_checked(&format!("kill-pane -t {focused}")) else {
                    return;
                };
                if !reply.ok {
                    return;
                }
                // The window survives with a new active pane, or the view
                // is over (the last pane died; %window-close's session
                // contract ends the view through the status refresh). The
                // survivor focus lands AFTER the re-seed (reseed_window
                // resets focus to the first leaf).
                let window = self.window.clone();
                let survivor = conn
                    .send_checked(&format!("list-panes -t {window}"))
                    .ok()
                    .filter(|reply| reply.ok)
                    .and_then(|reply| super::super::marked_pane(&reply.body, &window).ok());
                if let Some(pane) = survivor {
                    self.reseed_window(conn, &window);
                    if let Some(n) = pane.trim().strip_prefix('%').and_then(|n| n.parse().ok()) {
                        self.renderer.focus(n);
                        let _ = conn.send_checked(&format!("select-pane -t %{n}"));
                    }
                    return;
                }
                // No survivor: the window closed. The pump's next status
                // refresh lands on the session's active window, or ends
                // the view when the session went with it. Ending NOW would
                // skip the terminal restore; flag it and let the pump's
                // normal path decide.
                self.status_dirty = true;
            }
            super::super::ManagementKey::SwapPrev | super::super::ManagementKey::SwapNext => {
                // Swap with the layout-order neighbor; the %layout-change
                // broadcast the swap queues re-seeds the window through
                // the pump's parked-layout path (PendingWork). Fewer than two panes is
                // a no-op.
                let order: Vec<u32> = self.renderer.layout().iter().map(|r| r.pane).collect();
                if order.len() < 2 {
                    return;
                }
                let Some(position) = self
                    .renderer
                    .focused()
                    .and_then(|f| order.iter().position(|p| *p == f))
                else {
                    return;
                };
                let dir = if matches!(key, super::super::ManagementKey::SwapPrev) {
                    -1
                } else {
                    1
                };
                let next = order[((position as i32 + dir).rem_euclid(order.len() as i32)) as usize];
                let focused = self.focused_pane();
                let _ = conn.send_checked(&format!("swap-pane -s {focused} -t %{next}"));
            }
            super::super::ManagementKey::NewWindow => {
                let Some(session) = self.status.session_id.clone() else {
                    return;
                };
                let Ok(reply) = conn.send_checked(&format!("new-window -t {session}")) else {
                    return;
                };
                if !reply.ok {
                    return;
                }
                let Some(window) = reply.body.first().map(|w| w.trim().to_string()) else {
                    return;
                };
                self.land_window(conn, &window);
            }
            super::super::ManagementKey::WorkspaceNext
            | super::super::ManagementKey::WorkspacePrev => {
                let direction = if matches!(key, super::super::ManagementKey::WorkspaceNext) {
                    1
                } else {
                    -1
                };
                self.switch_workspace(conn, direction);
            }
            super::super::ManagementKey::Zoom => {
                // tmux's zoom: the daemon's `resize-pane -Z` toggle. The
                // daemon re-lays the window to the single pane full-grid
                // (children resize), selects away unzooms, and any layout
                // mutation ends the zoom — the %layout-change broadcast
                // re-seeds this client through the pump's pending path.
                // The bool only drives the status row's Z cue; the daemon
                // holds the truth.
                let Ok(reply) = conn.send_checked(&format!("resize-pane -t {focused} -Z")) else {
                    return;
                };
                if !reply.ok {
                    return;
                }
                self.zoomed = !self.zoomed;
                self.flash = Some(if self.zoomed { "zoomed" } else { "unzoomed" }.to_string());
                self.draw_status_row();
            }
            super::super::ManagementKey::RenameWindow | super::super::ManagementKey::RenamePane => {
                self.enter_prompt(if matches!(key, super::super::ManagementKey::RenamePane) {
                    PromptTarget::Pane
                } else {
                    PromptTarget::Window(self.window.clone())
                });
            }
            super::super::ManagementKey::StatusBar => {
                self.status_bar_on = !self.status_bar_on;
                self.chrome_geometry_changed();
                // No flash cue: the flash paints ON the status row, so a
                // hide would flash invisibly — the row's vanishing (or
                // returning) is the feedback. The bottom row's presence
                // changes the content height: park a refit so the report
                // and re-fit follow (the sidebar toggle's contract).
                self.pending.grid_refit = true;
                self.draw_status_row();
            }
            super::super::ManagementKey::BorderCycle => {
                let glyphs = self.render_opts.glyphs.next();
                self.set_glyphs(glyphs);
                // herdr style: every pane draws its own complete box, not
                // the shared dividers — the style owns the paint mode.
                // Through the session-level setter: the chord used to
                // flip the renderer's flag only, so every re-seed (a
                // split is one) restored the session flag and reverted
                // the boxes to shared dividers (the manual-pass report).
                self.set_pane_borders(matches!(glyphs, Glyphs::Herdr));
                self.flash = Some(format!("border style: {}", glyphs.name()));
                self.draw_status_row();
            }
            super::super::ManagementKey::WorkspacePicker => {
                self.enter_ws_picker(conn);
            }
            super::super::ManagementKey::Sidebar => {
                self.toggle_sidebar(conn);
            }
            super::super::ManagementKey::Labels => {
                // Toggle each pane's title in its border: the session's
                // bool flips, the flash cue confirms the new state, and
                // the next frame repaints the borders.
                let on = !self.render_opts.show_label_in_border;
                self.set_show_label_in_border(on);
                self.flash = Some(if on { "labels on" } else { "labels off" }.to_string());
                self.draw_status_row();
            }
        }
    }

    /// prefix W / C-w: the next/previous workspace in id order —
    /// `select-workspace -t +N`, then land the view on the workspace's
    /// session (its active window, re-seeded from fresh replays) through
    /// the same select-then-refresh contract every switch follows. A
    /// workspace with no sessions cannot be landed on: the selection
    /// still moves and the status refresh carries the new active marker.
    pub(super) fn switch_workspace(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        direction: i32,
    ) {
        let Ok(reply) = conn.send_checked("list-workspaces") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let Some(ws_id) = next_workspace(&reply.body, direction > 0) else {
            return;
        };
        let _ = conn.send_checked(&format!("select-workspace -t {ws_id}"));
        self.land_on_workspace(conn, &ws_id);
    }

    /// The pump's status facts in one step: the bar's state, the side
    /// panel's sections (reusing the refresh's workspace rows), and every
    /// visible pane's title. Against a `client-snapshot v1` daemon this is
    /// exactly one round trip (ENH-043). `Err` is the bar refresh's error,
    /// or `TimedOut` when a legacy title query outlived the status bound.
    pub(super) fn refresh_status_facts(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) -> Result<(), status::StatusError> {
        let focused = self.renderer.focused().unwrap_or(0);
        let rows = self.status.refresh(conn, &self.window, focused)?;
        if self.sidebar_on {
            if let Some(roster) = rows.workspaces {
                self.set_sidebar_roster(roster);
            }
        }
        self.refresh_pane_titles(conn)
    }

    /// Refresh the renderer's label store with each visible pane's
    /// effective title — the border labels paint the user `-T` label when
    /// set (the manual-pass report: prefix `$` labels never showed
    /// because the painter read the shell's OSC title only). Called after
    /// the throttled status refresh. Against a `client-snapshot v1` daemon
    /// every title comes from the snapshot, no query. Otherwise the
    /// focused pane's title is the one the status refresh just fetched;
    /// another pane is re-queried only when its recorded title went stale
    /// (ARC-125). A changed title
    /// marks dirty. Stops at the first query that outlives the status
    /// bound.
    pub(super) fn refresh_pane_titles(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) -> Result<(), status::StatusError> {
        if conn.has_command_feature("client-snapshot", "v1") {
            // The snapshot already carried every pane's title: no query.
            let panes: Vec<u32> = self.renderer.layout().iter().map(|r| r.pane).collect();
            for pane in panes {
                let title = self
                    .status
                    .pane_titles()
                    .iter()
                    .find(|(p, _)| *p == pane)
                    .map(|(_, title)| title.trim().to_string())
                    .unwrap_or_default();
                self.renderer.set_user_title(pane, &title);
            }
            return Ok(());
        }
        let focused = self.renderer.focused();
        if let Some(pane) = focused {
            let title = self.status.pane_title().trim().to_string();
            self.renderer.set_user_title(pane, &title);
        }
        let panes: Vec<u32> = self
            .renderer
            .layout()
            .iter()
            .map(|r| r.pane)
            .filter(|pane| Some(*pane) != focused && self.renderer.title_stale(*pane))
            .collect();
        for pane in panes {
            if let Some(body) = status::status_query(conn, &format!("pane-title -t %{pane}"))? {
                self.renderer.set_user_title(pane, body.join(" ").trim());
            }
        }
        Ok(())
    }

    pub(super) fn land_on_workspace(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ws_id: &str,
    ) {
        // Select FIRST: the daemon's active-workspace pointer is what the
        // roster query renders as the side panel's highlight, so a click
        // must move it too — landing used to re-seed the view without
        // selecting, leaving the highlight on the old workspace (the
        // manual-pass round-8 report). Idempotent for the keybind path,
        // which selects before landing.
        let _ = conn.send_checked(&format!("select-workspace -t {ws_id}"));
        let Ok(sessions) = conn.send_checked(&format!("list-sessions -t {ws_id}")) else {
            return;
        };
        if !sessions.ok {
            return;
        }
        let listed: Vec<String> = sessions
            .body
            .iter()
            .filter_map(|l| super::super::parse_session_line(l).map(|(id, _)| id))
            .collect();
        // The session select-workspace resumed (the workspace's active
        // one) — the session the follow broadcast names, so this client
        // and every follower land on the same view (card 01a11bd1: the
        // first-listed session diverged from it). The first listed is the
        // fallback for a daemon without the displayed-session query.
        let resumed = super::session::displayed_session(conn).filter(|s| listed.contains(s));
        let Some(session) = resumed.or_else(|| listed.first().cloned()) else {
            self.status_dirty = true;
            return;
        };
        let Some(window) = session_active_window(conn, &session) else {
            return;
        };
        self.land_window(conn, &window);
    }

    /// The reload chord in render mode: the same client-side rebind the
    /// passthrough session performs, plus a status-row flash, plus the
    /// daemon's `reload-config` — best-effort either way.
    pub(super) fn reload_config(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        // The file drives both the chord rebind and the pane-borders
        // resolution below, so it loads once, here.
        let file = match crate::mux::config::load_canonical_checked() {
            Ok(file) => file,
            Err(err) => {
                self.flash = Some(format!("reload failed: {err}"));
                let _ = conn.send_checked("reload-config");
                return;
            }
        };
        match crate::mux::config::reload_client_chords(
            &file,
            &crate::mux::config::Chords {
                prefix: self.prefix,
                reload: self.reload_key,
                management: self.management,
                resize_step: self.resize_step,
                pane_borders: self.render_opts.pane_borders,
                show_label_in_border: self.render_opts.show_label_in_border,
                pane_gaps: self.render_opts.pane_gaps,
                scrollbar_gutter: self.render_opts.scrollbar_gutter,
                drag_cursor_shape: self.drag_cursor_shape,
                border_lines: self.render_opts.glyphs.name().to_string(),
                sidebar_width: self.render_opts.sidebar_width,
                // Launch-only: resolved for the shape, never applied to
                // the live panel below.
                sidebar_on_launch: self.sidebar_on,
            },
        ) {
            Ok(chords) => {
                self.flash = Some(if self.apply_chords(&chords, &file) {
                    "config reloaded".to_string()
                } else {
                    format!(
                        "border-lines {:?} unknown — using {}",
                        chords.border_lines,
                        self.render_opts.glyphs.name()
                    )
                });
            }
            Err(err) => {
                self.flash = Some(format!("reload failed: {err}"));
            }
        }
        let _ = conn.send_checked("reload-config");
    }

    /// prefix o: select the next pane in the window's layout-leaf order,
    /// daemon-side, then re-seed — the render-mode spelling of
    /// select-then-refresh.
    pub(super) fn cycle_pane(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let order: Vec<u32> = self.renderer.layout().iter().map(|r| r.pane).collect();
        if order.len() < 2 {
            return;
        }
        let current = self
            .renderer
            .focused()
            .and_then(|f| order.iter().position(|p| *p == f))
            .unwrap_or(0);
        let next = order[(current + 1) % order.len()];
        self.renderer.focus(next);
        let _ = conn.send_checked(&format!("select-pane -t %{next}"));
    }

    /// prefix n/p: move to the next/previous window of the shown session
    /// and mirror it (daemon-side select-window, then re-seed from fresh
    /// replays).
    pub(super) fn switch_window(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        direction: i32,
    ) {
        // The status state knows the shown session; re-query its windows
        // for the fresh order (the status state may be stale).
        let Some(session) = self.status.session_id.clone() else {
            return;
        };
        let windows = list_window_ids(conn, &session);
        let Some(next) = next_in_cycle(&windows, &self.window, direction > 0).cloned() else {
            return;
        };
        self.land_window(conn, &next);
    }

    /// prefix ( / ): the previous/next session in list-sessions order;
    /// mirror its active window.
    pub(super) fn switch_session(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        direction: i32,
    ) {
        let Ok(reply) = conn.send_checked("list-sessions") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let sessions: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| super::super::parse_session_line(l).map(|(id, _)| id))
            .collect();
        if sessions.is_empty() {
            return;
        }
        // Which session owns the shown window right now? Stale state
        // falls back to the head so a direction still moves somewhere
        // deterministic.
        let shown = self.status.session_id.clone().unwrap_or_default();
        let owns_window = list_window_ids(conn, &shown).contains(&self.window);
        let current = if owns_window {
            shown
        } else {
            sessions[0].clone()
        };
        let Some(next) = next_in_cycle(&sessions, &current, direction > 0).cloned() else {
            return;
        };
        let Some(window) = session_active_window(conn, &next) else {
            return;
        };
        self.land_window(conn, &window);
    }

    /// A user-initiated landing on `window`: `switch-client -t <window>`
    /// selects it AND makes its session the daemon's displayed one, then
    /// the view re-seeds. A landing used to `select-window` only — silent
    /// for a background session — so the displayed pointer stayed on the
    /// session last displayed, and every reader of it (the next no-target
    /// attach, the persisted state a restore serves, the follow broadcast
    /// other clients obey) landed back there (card 01a11bd1). A daemon
    /// without `switch-client` keeps the select-window behavior; a landing
    /// both refuse (the window is gone) leaves the view put. Follow
    /// reseeds must NOT come through here: the daemon already moved, and a
    /// write from a lagging follow could pull the display back.
    pub(super) fn land_window(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) {
        let landed =
            |reply: std::io::Result<crate::mux::client::Reply>| reply.is_ok_and(|reply| reply.ok);
        if !landed(conn.send_checked(&format!("switch-client -t {window}")))
            && !landed(conn.send_checked(&format!("select-window -t {window}")))
        {
            return;
        }
        self.reseed_window(conn, window);
    }

    /// Point the whole view at `window`: daemon-side select already done
    /// (or the window is in the same session), re-fit the renderer in
    /// place from a fresh layout report, replay the panes new to the
    /// renderer (every pane of a different window; on the same window
    /// only the ones it lacked, plus the output-race panes), and mark
    /// everything dirty — the render-mode resync.
    pub(super) fn reseed_window(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) {
        let pane = self.focused_pane_or_first(conn, window);
        // Re-fit at the CURRENT renderer's frame extent — the host grid
        // the seed and resize_to derived — then report the renderer's
        // window_size: that size is the reported extent (the grid less
        // the side panel), not the buffer's width, and deriving the frame
        // from it ratcheted the view one panel-width narrower on every
        // re-seed while the panel was up (the manual-pass
        // click-shrinks-the-panes report). The frame size is unchanged
        // here, so the in-place refit only re-pushes the geometry; every
        // display option stays on the live renderer (ENH-044).
        let (host_cols, host_rows) = self.renderer.frame_size();
        self.refit_renderer(host_cols, host_rows);
        if self.sidebar_on {
            self.refresh_sidebar(conn);
        }
        let report = self.size_report(conn, &pane);
        if conn.send_checked(&report).is_err() {
            return;
        }
        self.window = window.to_string();
        // The cue resets with the view, then takes the new window's zoom
        // truth from the layout triple the size report queues below.
        self.zoomed = false;
        // The emulators survive the re-seed now, so leaving scroll mode
        // also clears the focused pane's hold rather than relying on a
        // fresh emulator.
        if matches!(self.modal, Modal::Scroll) {
            self.leave_scroll_mode();
        }
        let (layout_event, race) = self.drain_around_layout(conn);
        if let Some((l, v, f)) = layout_event {
            self.zoomed = f.contains('Z');
            if let Ok(layout) = layout::parse_layout_triple(&l, &v, &f) {
                let replay = self.install_layout(layout, &race);
                self.replay_panes(conn, &replay);
            }
        }
        // The re-seed re-laid the panes: the next frame's diff repaints
        // what moved, and the cursor guard must reset so a
        // changed position/shape re-emits even when the recorded state
        // coincidentally matches the old window's.
        self.cursor_placed = Some(None);
        // Fresh facts for the new view; a session gone mid-switch ends
        // the view through the pump's normal path on the next mark.
        let focused = self.renderer.focused().unwrap_or(0);
        self.status_dirty = true;
        if matches!(
            self.status.refresh(conn, window, focused),
            Err(status::StatusError::SessionGone)
        ) {
            // The window vanished between the select and the query: leave
            // the view rendering its last frame; the next %sessions-changed
            // (or this switch's own burst) re-evaluates.
            return;
        }
        self.status_row.invalidate();
        self.draw_status_row();
        self.tab_strip.invalidate();
        self.draw_tab_strip();
        self.pending.clear_view_work();
    }
}
