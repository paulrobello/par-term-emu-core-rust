//! Navigation: the management chords and the window, session, and
//! workspace switches, each landing through the select+resync contract.

use super::{Glyphs, Modal, PromptTarget, WindowSession};
use crate::mux::attach::targets::{
    interactive, marked_pane, session_landing, session_switch_target, window_switch_target,
    workspace_session, workspace_switch_target, WorkspaceLanding,
};
use crate::mux::attach::{layout, status, ManagementKey};

/// A user-initiated command outlived [`INTERACTIVE_TIMEOUT`]: the view
/// already flashed "daemon busy", and the chain it belonged to stops here.
///
/// [`INTERACTIVE_TIMEOUT`]: crate::mux::attach::conn::INTERACTIVE_TIMEOUT
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Busy;

impl WindowSession {
    /// Cue a daemon that did not answer a user-initiated command within
    /// the interactive bound: flash "daemon busy" and paint it now, mid
    /// tick. Other error kinds draw nothing (a closed daemon ends the pump
    /// through the notification channel).
    pub(super) fn flash_if_busy(&mut self, err: &std::io::Error) {
        if err.kind() == std::io::ErrorKind::TimedOut {
            self.flash = Some("daemon busy".to_string());
            self.draw_status_row();
        }
    }

    /// One user-initiated command at the interactive bound (ARC-137):
    /// `Ok(Some(reply))` accepted, `Ok(None)` refused or failed, and
    /// `Err(Busy)` (already flashed) when it timed out. The late reply is
    /// discarded on arrival by the client's abandoned-reply count; never
    /// retry a timed-out command, it may still execute.
    pub(super) fn ask(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        command: &str,
    ) -> Result<Option<crate::mux::client::Reply>, Busy> {
        self.busy(interactive(conn, command))
    }

    /// Map a resolver's `Err(TimedOut)` to [`Busy`], flashing the cue.
    pub(super) fn busy<T>(&mut self, result: std::io::Result<T>) -> Result<T, Busy> {
        result.map_err(|err| {
            self.flash_if_busy(&err);
            Busy
        })
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
        let focused = self.focused_pane();
        match key {
            ManagementKey::SplitRight | ManagementKey::SplitDown => {
                let flag = if matches!(key, ManagementKey::SplitRight) {
                    " -h"
                } else {
                    ""
                };
                let Ok(Some(reply)) = self.ask(conn, &format!("split-window -t {focused}{flag}"))
                else {
                    return;
                };
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
                    let _ = self.ask(conn, &format!("select-pane -t %{new_pane}"));
                }
            }
            ManagementKey::KillPane => {
                let Ok(Some(_)) = self.ask(conn, &format!("kill-pane -t {focused}")) else {
                    return;
                };
                // The window survives with a new active pane, or the view
                // is over (the last pane died; %window-close's session
                // contract ends the view through the status refresh). The
                // survivor focus lands AFTER the re-seed (reseed_window
                // resets focus to the first leaf).
                let window = self.window.clone();
                let Ok(listed) = self.ask(conn, &format!("list-panes -t {window}")) else {
                    return;
                };
                let survivor = listed.and_then(|reply| marked_pane(&reply.body, &window).ok());
                if let Some(pane) = survivor {
                    self.reseed_window(conn, &window);
                    if let Some(n) = pane.trim().strip_prefix('%').and_then(|n| n.parse().ok()) {
                        self.renderer.focus(n);
                        let _ = self.ask(conn, &format!("select-pane -t %{n}"));
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
            ManagementKey::SwapPrev | ManagementKey::SwapNext => {
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
                let dir = if matches!(key, ManagementKey::SwapPrev) {
                    -1
                } else {
                    1
                };
                let next = order[((position as i32 + dir).rem_euclid(order.len() as i32)) as usize];
                let focused = self.focused_pane();
                let _ = self.ask(conn, &format!("swap-pane -s {focused} -t %{next}"));
            }
            ManagementKey::NewWindow => {
                let Some(session) = self.status.session_id.clone() else {
                    return;
                };
                let Ok(Some(reply)) = self.ask(conn, &format!("new-window -t {session}")) else {
                    return;
                };
                let Some(window) = reply.body.first().map(|w| w.trim().to_string()) else {
                    return;
                };
                self.land_window(conn, &window);
            }
            ManagementKey::WorkspaceNext | ManagementKey::WorkspacePrev => {
                let direction = if matches!(key, ManagementKey::WorkspaceNext) {
                    1
                } else {
                    -1
                };
                self.switch_workspace(conn, direction);
            }
            ManagementKey::Zoom => {
                // tmux's zoom: the daemon's `resize-pane -Z` toggle. The
                // daemon re-lays the window to the single pane full-grid
                // (children resize), selects away unzooms, and any layout
                // mutation ends the zoom — the %layout-change broadcast
                // re-seeds this client through the pump's pending path.
                // The bool only drives the status row's Z cue; the daemon
                // holds the truth.
                let Ok(Some(_)) = self.ask(conn, &format!("resize-pane -t {focused} -Z")) else {
                    return;
                };
                self.zoomed = !self.zoomed;
                self.flash = Some(if self.zoomed { "zoomed" } else { "unzoomed" }.to_string());
                self.draw_status_row();
            }
            ManagementKey::RenameWindow | ManagementKey::RenamePane => {
                self.enter_prompt(if matches!(key, ManagementKey::RenamePane) {
                    PromptTarget::Pane
                } else {
                    PromptTarget::Window(self.window.clone())
                });
            }
            ManagementKey::StatusBar => {
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
            ManagementKey::BorderCycle => {
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
            ManagementKey::WorkspacePicker => {
                self.enter_ws_picker(conn);
            }
            ManagementKey::Sidebar => {
                self.toggle_sidebar(conn);
            }
            ManagementKey::Labels => {
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
        let target = workspace_switch_target(conn, direction > 0);
        if let Ok(Some(ws_id)) = self.busy(target) {
            self.land_on_workspace(conn, &ws_id);
        }
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
                let focused = self.renderer.focused();
                let sections = Self::sidebar_sections(
                    &roster,
                    self.status.agents(),
                    self.status.workspaces(),
                    focused,
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
        let Ok(Some(_)) = self.ask(conn, &format!("select-pane -t %{pane}")) else {
            return; // the pane is gone, the select was refused, or the daemon is busy
        };
        let Ok(info) = self.ask(conn, &format!("pane-info -t %{pane}")) else {
            return;
        };
        let window = info
            .and_then(|reply| {
                reply
                    .body
                    .first()
                    .and_then(|l| crate::mux::ipc::PaneInfoRow::parse(l))
            })
            .map(|row| row.window);
        match window {
            Some(window) if window != self.window => self.land_window(conn, &window),
            _ => {
                self.status_dirty = true;
            }
        }
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
        // which selects before landing.
        if self
            .ask(conn, &format!("select-workspace -t {ws_id}"))
            .is_err()
        {
            return;
        }
        // The session select-workspace resumed, else the first listed
        // (card 01a11bd1) — the rule both attach modes share.
        let resolved = workspace_session(conn, ws_id);
        let session = match self.busy(resolved) {
            Ok(WorkspaceLanding::Session(session)) => session,
            Ok(WorkspaceLanding::Empty) => {
                self.status_dirty = true;
                return;
            }
            Ok(WorkspaceLanding::Failed) | Err(Busy) => return,
        };
        let landing = session_landing(conn, &session);
        let Ok(Some(landing)) = self.busy(landing) else {
            return;
        };
        self.land_window(conn, &landing.window);
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
                let _ = self.ask(conn, "reload-config");
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
                drag_cursor_shape: self.cursor.drag_shape,
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
        let _ = self.ask(conn, "reload-config");
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
        let _ = self.ask(conn, &format!("select-pane -t %{next}"));
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
        let target = window_switch_target(conn, &session, &self.window, direction > 0);
        let Ok(Some(landing)) = self.busy(target) else {
            return;
        };
        self.land_window(conn, &landing.window);
    }

    /// prefix ( / ): the previous/next session in list-sessions order;
    /// mirror its active window.
    pub(super) fn switch_session(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        direction: i32,
    ) {
        // The current session counts only while it owns the shown
        // window; stale state falls back to the roster head
        // ([`session_switch_target`]).
        let current = self.status.session_id.clone();
        let target = session_switch_target(conn, current.as_deref(), &self.window, direction > 0);
        let Ok(Some(landing)) = self.busy(target) else {
            return;
        };
        self.land_window(conn, &landing.window);
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
    /// refused `switch-client` falls back; a timed-out one flashes and
    /// stops (ARC-137).
    pub(super) fn land_window(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) {
        match self.ask(conn, &format!("switch-client -t {window}")) {
            Ok(Some(_)) => {}
            Ok(None) => {
                let Ok(Some(_)) = self.ask(conn, &format!("select-window -t {window}")) else {
                    return;
                };
            }
            Err(Busy) => return,
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
        self.status_row.invalidate();
        self.draw_status_row();
        self.tab_strip.invalidate();
        self.draw_tab_strip();
        self.pending.clear_view_work();
    }
}
