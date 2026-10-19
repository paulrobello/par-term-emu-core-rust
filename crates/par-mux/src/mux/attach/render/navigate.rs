//! Navigation: the management chords and the window, session, and
//! workspace switches, each landing through the select+resync contract.

use super::{Glyphs, Modal, PromptTarget, WindowSession};
use crate::mux::attach::targets::{
    active_window_row, displayed_session_of, marked_pane, next_in_cycle, next_workspace,
    parse_session_line,
};
use crate::mux::attach::{layout, status, ManagementKey};

impl WindowSession {
    /// Start a user navigation: the latest one wins. Every chain captures
    /// the returned generation and drops its work when a newer navigation
    /// (or a follow reseed) moved the view since it was issued, so a slow
    /// reply never pulls the view back.
    pub(super) fn begin_nav(&mut self) -> u64 {
        self.nav_gen += 1;
        self.nav_gen
    }

    fn nav_current(&self, gen: u64, expected: &str) -> bool {
        self.nav_gen == gen && self.window == expected
    }

    /// Flash "daemon busy" when a write to the daemon timed out. Other
    /// error kinds draw nothing (a closed daemon ends the pump through the
    /// notification channel).
    pub(super) fn flash_if_busy(&mut self, err: &std::io::Error) {
        if err.kind() == std::io::ErrorKind::TimedOut {
            self.chrome.flash("daemon busy".to_string());
            self.draw_status_row();
        }
    }

    /// The management chords in render mode: issue the daemon command for
    /// the focused pane/session, then land the view through the same
    /// re-seed contract `switch_window` follows — split lands on the new
    /// pane (the reply body IS its id, the daemon focuses it), kill lands
    /// on the window's survivor, new-window re-seeds the fresh window.
    pub(super) fn management_chord(
        &mut self,
        key: ManagementKey,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) {
        match key {
            ManagementKey::SplitRight => self.chord_split(conn, true),
            ManagementKey::SplitDown => self.chord_split(conn, false),
            ManagementKey::KillPane => self.chord_kill_pane(conn),
            ManagementKey::BreakPane => self.chord_break_pane(conn),
            ManagementKey::JoinPane => self.chord_join_pane(conn),
            ManagementKey::Palette => self.enter_palette(),
            ManagementKey::SwapPrev => self.chord_swap(conn, -1),
            ManagementKey::SwapNext => self.chord_swap(conn, 1),
            ManagementKey::NewWindow => self.chord_new_window(conn),
            ManagementKey::WorkspaceNext => self.switch_workspace(conn, 1),
            ManagementKey::WorkspacePrev => self.switch_workspace(conn, -1),
            ManagementKey::Zoom => self.chord_zoom(conn),
            ManagementKey::RenameWindow => {
                self.enter_prompt(PromptTarget::Window(self.window.clone()));
            }
            ManagementKey::RenamePane => self.enter_prompt(PromptTarget::Pane),
            ManagementKey::StatusBar => self.toggle_status_bar(),
            ManagementKey::BorderCycle => self.cycle_border_style(),
            ManagementKey::Labels => self.toggle_border_labels(),
            ManagementKey::WorkspacePicker => self.enter_ws_picker(conn),
            ManagementKey::Sidebar => self.toggle_sidebar(conn),
            ManagementKey::Respawn => self.chord_respawn(conn),
        }
    }

    /// The respawn chord: `respawn-pane -t <focused>`, but only when the
    /// focused pane is held dead — never `-k`, so a live pane is never
    /// restarted by a stray chord. The `%pane-respawned` broadcast clears
    /// the cue and resets the pane's screen; a refused respawn flashes.
    fn chord_respawn(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Some(pane) = self.renderer.focused() else {
            return;
        };
        if !self.held.contains_key(&pane) {
            self.chrome.flash("pane is running".to_string());
            self.draw_status_row();
            return;
        }
        self.send_then(conn, &format!("respawn-pane -t %{pane}"), |s, _, reply| {
            if !reply.ok {
                let why = reply.body.first().cloned().unwrap_or_default();
                s.chrome.flash(format!("respawn failed: {why}"));
                s.draw_status_row();
            }
        });
    }

    fn chord_split(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, horizontal: bool) {
        let focused = self.focused_pane();
        let flag = if horizontal { " -h" } else { "" };
        let expected = self.window.clone();
        self.send_then(
            conn,
            &format!("split-window -t {focused}{flag}"),
            move |s, c, reply| {
                // A landing that completed since the split was issued moved
                // the view off the window the new pane belongs to.
                if !reply.ok || s.window != expected {
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
                let window = s.window.clone();
                s.reseed_window(c, &window);
                if let Some(new_pane) = new_pane {
                    s.renderer.focus(new_pane);
                    let _ = c.send_forget(&format!("select-pane -t %{new_pane}"));
                }
            },
        );
    }

    fn chord_kill_pane(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let focused = self.focused_pane();
        let expected = self.window.clone();
        self.send_then(
            conn,
            &format!("kill-pane -t {focused}"),
            move |s, c, reply| {
                if !reply.ok || s.window != expected {
                    return;
                }
                // The window survives with a new active pane, or the view
                // is over (the last pane died; %window-close's session
                // contract ends the view through the status refresh). The
                // survivor focus lands AFTER the re-seed (reseed_window
                // resets focus to the first leaf).
                let window = s.window.clone();
                s.send_then(
                    c,
                    &format!("list-panes -t {window}"),
                    move |s, c, listed| {
                        if s.window != window {
                            return;
                        }
                        let survivor = listed
                            .ok
                            .then(|| marked_pane(&listed.body, &window).ok())
                            .flatten();
                        if let Some(pane) = survivor {
                            s.reseed_window(c, &window);
                            if let Some(n) =
                                pane.trim().strip_prefix('%').and_then(|n| n.parse().ok())
                            {
                                s.renderer.focus(n);
                                let _ = c.send_forget(&format!("select-pane -t %{n}"));
                            }
                            return;
                        }
                        // No survivor: the window closed. The pump's next
                        // status refresh lands on the session's active window,
                        // or ends the view when the session went with it.
                        // Ending NOW would skip the terminal restore; flag it
                        // and let the pump's normal path decide.
                        s.status_dirty = true;
                    },
                );
            },
        );
    }

    /// prefix !: `break-pane -s <focused>` — the focused pane becomes its
    /// session's new window and the view lands on it (the reply body IS
    /// the new window id, like new-window's).
    fn chord_break_pane(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let focused = self.focused_pane();
        self.send_then(conn, &format!("break-pane -s {focused}"), |s, c, reply| {
            if !reply.ok {
                return;
            }
            if let Some(window) = reply.body.first().map(|w| w.trim().to_string()) {
                s.land_window(c, &window);
            }
        });
    }

    /// prefix J: the window picker in join mode — the picked window
    /// receives the focused pane (`join-pane -s <focused> -t <its marked
    /// pane>`). The picker is the session/window roster itself; the
    /// activation routes through the join arm while `join_source` is set,
    /// and every dismissal path clears it.
    fn chord_join_pane(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Some(pane) = self.renderer.focused() else {
            return;
        };
        self.join_source = Some(pane);
        self.enter_picker(conn);
    }

    /// The join arm: move `source` next to `window`'s marked pane
    /// (`join-pane -s <source> -t <marked>`), then land the view on the
    /// destination window — tmux's join-pane follows the joined pane. A
    /// refused join flashes instead of landing.
    pub(super) fn join_pane_into(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        source: u32,
        window: String,
    ) {
        self.send_then(
            conn,
            &format!("list-panes -t {window}"),
            move |s, c, listed| {
                if !listed.ok {
                    return;
                }
                let Some(target) = marked_pane(&listed.body, &window).ok() else {
                    return;
                };
                s.send_then(
                    c,
                    &format!("join-pane -s %{source} -t {target}"),
                    move |s, c, reply| {
                        if !reply.ok {
                            let why = reply.body.first().cloned().unwrap_or_default();
                            s.chrome.flash(format!("join failed: {why}"));
                            s.draw_status_row();
                            return;
                        }
                        s.land_window(c, &window);
                    },
                );
            },
        );
    }

    /// Swap with the layout-order neighbor; the %layout-change
    /// broadcast the swap queues re-seeds the window through
    /// the pump's parked-layout path (PendingWork). Fewer than two panes is
    /// a no-op.
    fn chord_swap(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, dir: i32) {
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
        let next = order[((position as i32 + dir).rem_euclid(order.len() as i32)) as usize];
        let focused = self.focused_pane();
        let _ = conn.send_forget(&format!("swap-pane -s {focused} -t %{next}"));
    }

    fn chord_new_window(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Some(session) = self.status.session_id.clone() else {
            return;
        };
        self.send_then(conn, &format!("new-window -t {session}"), |s, c, reply| {
            if !reply.ok {
                return;
            }
            if let Some(window) = reply.body.first().map(|w| w.trim().to_string()) {
                s.land_window(c, &window);
            }
        });
    }

    /// tmux's zoom: the daemon's `resize-pane -Z` toggle. The
    /// daemon re-lays the window to the single pane full-grid
    /// (children resize), selects away unzooms, and any layout
    /// mutation ends the zoom — the %layout-change broadcast
    /// re-seeds this client through the pump's pending path.
    /// The bool only drives the status row's Z cue; the daemon
    /// holds the truth.
    fn chord_zoom(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let focused = self.focused_pane();
        // Assigned, not toggled: the %layout-change the zoom queues may
        // reseed (setting the cue from the layout flags) before this reply
        // is dispatched, and a toggle would then flip it back.
        let zoomed = !self.zoomed;
        self.send_then(
            conn,
            &format!("resize-pane -t {focused} -Z"),
            move |s, _c, reply| {
                if !reply.ok {
                    return;
                }
                s.zoomed = zoomed;
                s.chrome
                    .flash(if zoomed { "zoomed" } else { "unzoomed" }.to_string());
                s.draw_status_row();
            },
        );
    }

    /// No flash cue: the flash paints ON the status row, so a
    /// hide would flash invisibly — the row's vanishing (or
    /// returning) is the feedback. The bottom row's presence
    /// changes the content height: park a refit so the report
    /// and re-fit follow (the sidebar toggle's contract).
    fn toggle_status_bar(&mut self) {
        self.chrome.status_bar_on = !self.chrome.status_bar_on;
        self.chrome_geometry_changed();
        self.pending.grid_refit = true;
        self.draw_status_row();
    }

    /// herdr style: every pane draws its own complete box, not
    /// the shared dividers — the style owns the paint mode.
    /// Through the session-level setter: the chord used to
    /// flip the renderer's flag only, so every re-seed (a
    /// split is one) restored the session flag and reverted
    /// the boxes to shared dividers (the manual-pass report).
    fn cycle_border_style(&mut self) {
        let glyphs = self.render_opts.glyphs.next();
        self.set_glyphs(glyphs);
        self.set_pane_borders(matches!(glyphs, Glyphs::Herdr));
        self.chrome
            .flash(format!("border style: {}", glyphs.name()));
        self.draw_status_row();
    }

    /// Toggle each pane's title in its border: the session's
    /// bool flips, the flash cue confirms the new state, and
    /// the next frame repaints the borders.
    fn toggle_border_labels(&mut self) {
        let on = !self.render_opts.show_label_in_border;
        self.set_show_label_in_border(on);
        self.chrome
            .flash(if on { "labels on" } else { "labels off" }.to_string());
        self.draw_status_row();
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
        let gen = self.begin_nav();
        let expected = self.window.clone();
        self.send_then(conn, "list-workspaces", move |s, c, reply| {
            if !reply.ok || !s.nav_current(gen, &expected) {
                return;
            }
            let Some(ws_id) = next_workspace(&reply.body, direction > 0) else {
                return;
            };
            // In-order daemon: the landing's queries observe this select.
            let _ = c.send_forget(&format!("select-workspace -t {ws_id}"));
            s.land_on_workspace(c, &ws_id);
        });
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
        if self.sidebar.is_on() {
            if let Some(roster) = rows.workspaces {
                let focused = self.renderer.focused();
                let sections = Self::sidebar_sections(
                    &roster,
                    self.status.agents(),
                    self.status.workspaces(),
                    focused,
                    &self.agent_rows,
                );
                self.renderer.set_sidebar_sections(sections);
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

    /// Focus an agent's pane from a side-panel agent-row click: move the
    /// daemon's shared selection with `select-pane` + `pane-info` lookup,
    /// then land the view on the pane's window when it is not already
    /// shown (the select+resync contract every switch follows).
    /// Same-window clicks just move focus; the throttled status refresh
    /// repaints the panel highlight. A refused `select-pane` (the pane is
    /// gone) stops there: no `pane-info`, no landing. A timed-out one
    /// flashes "daemon busy" and stops the same way (ARC-137).
    pub(super) fn focus_pane(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        pane: u32,
    ) {
        // In-order daemon: the pane-info below observes the select. A gone
        // pane yields no pane-info row, which lands in the status_dirty arm.
        if conn
            .send_forget(&format!("select-pane -t %{pane}"))
            .is_err()
        {
            return;
        }
        let gen = self.begin_nav();
        let expected = self.window.clone();
        self.send_then(
            conn,
            &format!("pane-info -t %{pane}"),
            move |s, c, reply| {
                // A landing that completed since the click moved the view;
                // this one must not pull it back.
                if !s.nav_current(gen, &expected) {
                    return;
                }
                let window = reply
                    .ok
                    .then(|| {
                        reply
                            .body
                            .first()
                            .and_then(|l| crate::mux::ipc::PaneInfoRow::parse(l))
                    })
                    .flatten()
                    .map(|row| row.window);
                match window {
                    Some(window) if window != s.window => s.land_window(c, &window),
                    _ => s.status_dirty = true,
                }
            },
        );
    }

    /// Land the view on `ws_id`: the session `select-workspace` resumed
    /// (else the first listed one), at its active window, re-seeded (the
    /// select+resync contract every switch follows). An empty workspace
    /// only marks the status stale — the %workspaces-changed the select
    /// queued moves the strip's active marker on the next frame.
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
        // which selects before landing. In-order daemon: the queries below
        // observe it.
        if conn
            .send_forget(&format!("select-workspace -t {ws_id}"))
            .is_err()
        {
            return;
        }
        // The session select-workspace resumed, else the first listed
        // (card 01a11bd1) — the rule both attach modes share.
        let gen = self.begin_nav();
        let expected = self.window.clone();
        self.send_then(
            conn,
            &format!("list-sessions -t {ws_id}"),
            move |s, c, listed| {
                if !listed.ok || !s.nav_current(gen, &expected) {
                    return;
                }
                let sessions: Vec<String> = listed
                    .body
                    .iter()
                    .filter_map(|l| parse_session_line(l).map(|(id, _)| id))
                    .collect();
                s.send_then(c, "switch-client", move |s, c, displayed| {
                    if !s.nav_current(gen, &expected) {
                        return;
                    }
                    let resumed = displayed
                        .ok
                        .then(|| displayed_session_of(&displayed))
                        .flatten()
                        .filter(|id| sessions.contains(id));
                    let Some(session) = resumed.or_else(|| sessions.first().cloned()) else {
                        // An empty workspace only marks the status stale.
                        s.status_dirty = true;
                        return;
                    };
                    s.send_then(
                        c,
                        &format!("list-windows -t {session}"),
                        move |s, c, windows| {
                            if !windows.ok || !s.nav_current(gen, &expected) {
                                return;
                            }
                            if let Some(window) = active_window_row(&windows.body) {
                                s.land_window(c, &window);
                            }
                        },
                    );
                });
            },
        );
    }

    /// The reload chord in render mode: the same client-side rebind the
    /// passthrough session performs, plus a status-row flash, plus the
    /// daemon's `reload-config` — best-effort either way.
    pub(super) fn reload_config(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        match self.load_client_file() {
            Ok(file) => {
                let text = self.apply_client_file(&file, "config reloaded");
                self.chrome.flash(text);
            }
            Err(err) => self.chrome.flash(format!("reload failed: {err}")),
        }
        let _ = conn.send_forget("reload-config");
    }

    /// `%config-changed`: some client's reload made the daemon re-read
    /// the config, so re-apply this client's `[client]` settings through
    /// the reload chord's own rebind. Never sends `reload-config` — that
    /// would re-broadcast to every client. A file whose `[client]`
    /// section matches the last one applied is a no-op (the client whose
    /// reload caused the broadcast already applied it).
    pub(super) fn on_config_changed(&mut self) {
        // A file that fails to load here was readable when the daemon
        // re-read it a moment ago: a mid-write race, which the writer's
        // next reload resolves; stay quiet rather than flash a transient.
        let Ok(file) = self.load_client_file() else {
            return;
        };
        if file.client == self.applied_client {
            return;
        }
        let text = self.apply_client_file(&file, "config re-applied");
        self.chrome.flash(text);
        if matches!(self.modal, Modal::Settings(_)) {
            self.refresh_settings();
        }
    }

    /// The config file the reload paths read — the file the settings
    /// dialog writes when the attach knows it (the canonical path in
    /// practice). It drives both the chord rebind and the pane-borders
    /// resolution, so each reload loads it once.
    fn load_client_file(&self) -> Result<crate::mux::config::ConfigFile, String> {
        match &self.settings_path {
            Some(path) => crate::mux::config::load_file(path).map(Option::unwrap_or_default),
            None => crate::mux::config::load_canonical_checked(),
        }
    }

    /// Re-resolve the `[client]` settings from `file` over the live ones
    /// and apply them; returns the status-row text (`ok_text`, a display
    /// warning, or the error). Records the section either way, so the
    /// broadcast of this client's own reload never repeats its outcome.
    fn apply_client_file(
        &mut self,
        file: &crate::mux::config::ConfigFile,
        ok_text: &str,
    ) -> String {
        // A config width edit overrides a dragged/persisted width, but
        // an unrelated reload must preserve that client's local width.
        let width_changed = file.client.sidebar_width != self.applied_client.sidebar_width;
        self.applied_client = file.client.clone();
        match crate::mux::config::reload_client_chords(
            file,
            &crate::mux::config::Chords {
                prefix: self.prefix,
                reload: self.reload_key,
                management: self.management,
                resize_step: self.resize_step,
                pane_borders: self.render_opts.pane_borders,
                show_label_in_border: self.render_opts.show_label_in_border,
                pane_gaps: self.render_opts.pane_gaps,
                scrollbar_gutter: self.render_opts.scrollbar_gutter,
                drag_cursor_shape: self.cursor.drag_shape,
                border_lines: self.render_opts.glyphs.name().to_string(),
                sidebar_width: self.render_opts.sidebar_width,
                // Launch-only: the live panel's visibility stays as it
                // is; the session keeps the configured value for the
                // settings dialog.
                sidebar_on_launch: self.sidebar_on_launch,
                agent_rows: self.agent_rows.clone(),
                notify_delivery: self.notify.delivery.name().to_string(),
                notify_states: self.notify.states.clone(),
            },
        ) {
            Ok(chords) => {
                if width_changed && file.client.sidebar_width.is_some() {
                    self.render_opts.sidebar_width = chords.sidebar_width;
                    self.chrome_geometry_changed();
                    self.persist_sidebar();
                }
                let known_border = self.apply_chords(&chords, file);
                let warning = crate::mux::config::display_option_warnings(&chords)
                    .into_iter()
                    .next();
                match warning {
                    Some(warning) => warning,
                    None if known_border => ok_text.to_string(),
                    None => format!(
                        "border-lines {:?} unknown — using {}",
                        chords.border_lines,
                        self.render_opts.glyphs.name()
                    ),
                }
            }
            Err(err) => format!("reload failed: {err}"),
        }
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
        let _ = conn.send_forget(&format!("select-pane -t %{next}"));
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
        let gen = self.begin_nav();
        let expected = self.window.clone();
        self.send_then(
            conn,
            &format!("list-windows -t {session}"),
            move |s, c, reply| {
                if !reply.ok || !s.nav_current(gen, &expected) {
                    return;
                }
                let windows = window_ids(&reply.body);
                if let Some(next) = next_in_cycle(&windows, &expected, direction > 0) {
                    s.land_window(c, next);
                }
            },
        );
    }

    /// prefix ( / ): the previous/next session in list-sessions order;
    /// mirror its active window.
    pub(super) fn switch_session(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        direction: i32,
    ) {
        // The current session counts only while it owns the shown
        // window; stale state falls back to the roster head.
        let current = self.status.session_id.clone();
        let gen = self.begin_nav();
        let expected = self.window.clone();
        self.send_then(conn, "list-sessions", move |s, c, reply| {
            if !reply.ok || !s.nav_current(gen, &expected) {
                return;
            }
            let sessions: Vec<String> = reply
                .body
                .iter()
                .filter_map(|l| parse_session_line(l).map(|(id, _)| id))
                .collect();
            let Some(head) = sessions.first().cloned() else {
                return;
            };
            let Some(current) = current else {
                s.land_on_session_after(c, &sessions, &head, direction > 0, (gen, &expected));
                return;
            };
            s.send_then(
                c,
                &format!("list-windows -t {current}"),
                move |s, c, owned| {
                    if !s.nav_current(gen, &expected) {
                        return;
                    }
                    let owns = owned.ok && window_ids(&owned.body).contains(&expected);
                    let from = if owns { current } else { head };
                    s.land_on_session_after(c, &sessions, &from, direction > 0, (gen, &expected));
                },
            );
        });
    }

    /// The tail of a session switch: the session after (or before) `from`
    /// in `sessions`, landed on its active window.
    fn land_on_session_after(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        sessions: &[String],
        from: &str,
        forward: bool,
        (gen, expected): (u64, &str),
    ) {
        let Some(next) = next_in_cycle(sessions, from, forward).cloned() else {
            return;
        };
        let expected = expected.to_string();
        self.send_then(
            conn,
            &format!("list-windows -t {next}"),
            move |s, c, reply| {
                if !reply.ok || !s.nav_current(gen, &expected) {
                    return;
                }
                if let Some(window) = active_window_row(&reply.body) {
                    s.land_window(c, &window);
                }
            },
        );
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
    /// write from a lagging follow could pull the display back. Only a
    /// refused `switch-client` falls back. The reseed runs when the reply
    /// lands, unless another landing moved the view first.
    pub(super) fn land_window(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) {
        let gen = self.begin_nav();
        let expected = self.window.clone();
        let target = window.to_string();
        self.send_then(
            conn,
            &format!("switch-client -t {window}"),
            move |s, c, reply| {
                if !s.nav_current(gen, &expected) {
                    return;
                }
                if reply.ok {
                    s.reseed_window(c, &target);
                    return;
                }
                s.send_then(
                    c,
                    &format!("select-window -t {target}"),
                    move |s, c, reply| {
                        if reply.ok && s.nav_current(gen, &expected) {
                            s.reseed_window(c, &target);
                        }
                    },
                );
            },
        );
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
        super::trace_input(format_args!("reseed_window {window}"));
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
        if self.sidebar.is_on() {
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
        self.cursor.invalidate();
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
        self.chrome.status_row.invalidate();
        self.draw_status_row();
        self.chrome.tab_strip.invalidate();
        self.chrome.tab_strip.reset_scroll();
        self.draw_tab_strip();
        self.pending.clear_view_work();
    }
}

/// The window ids (`@N`) of a `list-windows` reply body, in order.
fn window_ids(body: &[String]) -> Vec<String> {
    body.iter()
        .filter_map(|l| crate::mux::ipc::WindowRow::parse(l))
        .map(|row| row.id)
        .collect()
}
