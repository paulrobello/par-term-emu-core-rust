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
                // reply), THEN focus the fresh pane — reseed_window
                // rebuilds the renderer, which resets focus to the first
                // leaf, so the focus must come after.
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
                // the pump's pending_layout path. Fewer than two panes is
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
                // No flash cue: the flash paints ON the status row, so a
                // hide would flash invisibly — the row's vanishing (or
                // returning) is the feedback. The bottom row's presence
                // changes the content height: park a refit so the report
                // and re-fit follow (the sidebar toggle's contract).
                self.pending_grid_refit = true;
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
        let rows: Vec<(String, String, bool)> = reply
            .body
            .iter()
            .filter_map(|l| super::super::parse_workspace_line(l))
            .collect();
        if rows.is_empty() {
            return;
        }
        let Some(current) = rows.iter().position(|(_, _, active)| *active) else {
            return;
        };
        let next = (current as i32 + direction).rem_euclid(rows.len() as i32) as usize;
        let (ws_id, _, _) = &rows[next];
        let _ = conn.send_checked(&format!("select-workspace -t {ws_id}"));
        self.land_on_workspace(conn, ws_id);
    }

    /// Re-query every pane's effective title into the renderer's label
    /// store — the border labels paint the user `-T` label when set (the
    /// manual-pass report: prefix `$` labels never showed because the
    /// painter read the shell's OSC title only). Called on seed and on
    /// the throttled status refresh; a changed title marks dirty.
    pub(super) fn refresh_pane_titles(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let panes: Vec<u32> = self.renderer.layout().iter().map(|r| r.pane).collect();
        for pane in panes {
            if let Ok(reply) = conn.send_checked(&format!("pane-title -t %{pane}")) {
                if reply.ok {
                    let title = reply.body.join(" ");
                    self.renderer.set_user_title(pane, title.trim());
                }
            }
        }
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
        let Ok(windows) = conn.send_checked(&format!("list-windows -t {session}")) else {
            return;
        };
        if !windows.ok {
            return;
        }
        let window = windows
            .body
            .iter()
            .find(|l| l.split_whitespace().nth(1) == Some("*"))
            .or_else(|| windows.body.first())
            .and_then(|l| l.split_whitespace().next());
        let Some(window) = window else {
            return;
        };
        self.land_window(conn, window);
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
                self.prefix = chords.prefix;
                self.reload_key = chords.reload;
                self.management = chords.management;
                self.resize_step = chords.resize_step;
                if !self.set_border_lines(&chords.border_lines) {
                    self.flash = Some(format!(
                        "border-lines {:?} unknown — using {}",
                        chords.border_lines,
                        self.render_opts.glyphs.name()
                    ));
                }
                // An explicit `pane-borders` key survives every reload;
                // an absent key follows the (possibly reloaded) border
                // style — herdr implies per-pane boxes. The chords' own
                // bool cannot tell explicit from absent, so the raw file
                // key decides.
                match file.client.pane_borders {
                    Some(on) => self.set_pane_borders(on),
                    None => self.set_pane_borders(matches!(self.render_opts.glyphs, Glyphs::Herdr)),
                }
                self.set_pane_gaps(chords.pane_gaps);
                self.set_scrollbar_gutter(chords.scrollbar_gutter);
                self.set_show_label_in_border(chords.show_label_in_border);
                self.drag_cursor_shape = chords.drag_cursor_shape;
                let eff =
                    crate::mux::config::resolve(&file, &crate::mux::config::Overrides::default());
                self.set_border_colors(
                    crate::mux::config::parse_hex_color(&eff.border_active_color)
                        .map(|(r, g, b)| RtColor::Rgb(r, g, b)),
                    crate::mux::config::parse_hex_color(&eff.border_color)
                        .map(|(r, g, b)| RtColor::Rgb(r, g, b)),
                );
                self.literal = chords.prefix;
                self.flash = Some("config reloaded".to_string());
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
        // The status state knows the shown session's windows.
        if self.status.session_id.is_none() {
            return;
        }
        // Re-query for the fresh order: the status state may be stale.
        let Ok(reply) = conn.send_checked(&format!(
            "list-windows -t {}",
            self.status.session_id.clone().unwrap_or_default()
        )) else {
            return;
        };
        if !reply.ok {
            return;
        }
        let windows: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|w| w.starts_with('@'))
            .map(str::to_string)
            .collect();
        let Some(position) = windows.iter().position(|w| *w == self.window) else {
            return;
        };
        let next = windows[(position as i32 + direction).rem_euclid(windows.len() as i32) as usize]
            .clone();
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
        // Which session owns the shown window right now?
        let Ok(windows) = conn.send_checked(&format!(
            "list-windows -t {}",
            self.status.session_id.clone().unwrap_or_default()
        )) else {
            return;
        };
        let owns_window = windows
            .body
            .iter()
            .any(|l| l.split_whitespace().next() == Some(self.window.as_str()));
        let current = if owns_window {
            sessions
                .iter()
                .position(|s| Some(s.as_str()) == self.status.session_id.as_deref())
        } else {
            // Stale state: fall back to the head so a direction still
            // moves somewhere deterministic.
            Some(0)
        };
        let Some(current) = current else {
            return;
        };
        let next = sessions
            [(current as i32 + direction).rem_euclid(sessions.len() as i32) as usize]
            .clone();
        let Ok(windows) = conn.send_checked(&format!("list-windows -t {next}")) else {
            return;
        };
        if !windows.ok {
            return;
        }
        let window = windows
            .body
            .iter()
            .find(|l| l.split_whitespace().nth(1) == Some("*"))
            .or_else(|| windows.body.first())
            .and_then(|l| l.split_whitespace().next());
        let Some(window) = window else {
            return;
        };
        self.land_window(conn, window);
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
    /// (or the window is in the same session), re-fit the renderer from a
    /// fresh layout report, replay every pane, and mark everything dirty
    /// — the render-mode resync.
    pub(super) fn reseed_window(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) {
        let pane = self.focused_pane_or_first(conn, window);
        // Rebuild at the CURRENT renderer's frame extent — the host grid
        // the seed and resize_to derived — then report the fresh
        // renderer's window_size: that size is the reported extent (the
        // grid less the side panel), not the buffer's width, and
        // constructing from it ratcheted the view one panel-width
        // narrower on every re-seed while the panel was up, each report
        // then shrinking again (the manual-pass click-shrinks-the-panes
        // report).
        let (host_cols, host_rows) = self.renderer.frame_size();
        // The rebuild carries every session display option (background,
        // borders, labels, chrome, the side panel's effective width,
        // border colors): a hand re-apply list here dropped options three
        // times before (the round-3 prefix n/p black band and border
        // mode, the split-while-panel-open width, ARC-122's colors).
        self.rebuild_renderer(host_cols, host_rows);
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
        self.scroll_mode = false;
        if let Some((l, v, f)) =
            conn.drain_pending_events()
                .into_iter()
                .find_map(|event| match event {
                    TmuxNotification::LayoutChange {
                        window_id,
                        window_layout,
                        window_visible_layout,
                        window_raw_flags,
                    } if window_id == window => {
                        Some((window_layout, window_visible_layout, window_raw_flags))
                    }
                    _ => None,
                })
        {
            self.zoomed = f.contains('Z');
            if let Ok(layout) = layout::parse_layout_triple(&l, &v, &f) {
                self.renderer.apply_layout(layout);
            }
        }
        self.replay_all_panes(conn);
        // The re-seed replaced the renderer's buffers: the next frame's
        // diff repaints every cell, and the cursor guard must reset so a
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
        // Stale parked state from the replaced view: a follow aimed at the
        // old window is done (this reseed IS the follow landing), a parked
        // layout triple was parsed against the old renderer's pane set.
        self.pending_follow_window = None;
        self.pending_follow_session = None;
        self.pending_layout = None;
    }
}
