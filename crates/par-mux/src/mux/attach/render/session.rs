//! Session plumbing: seeding a window, feeding pane bytes, the frame
//! cadence, host resize, the status row, the tab strip, and the
//! config-driven display setters.

use super::{DragState, FlushSink, Glyphs, Modal, PaneRect, RenderOptions, WindowSession};
use crate::mux::attach::status::{Segment, StatusRow};
use crate::mux::attach::tabs::TabStrip;
use crate::mux::attach::targets::{active_window_row, initial_attach_session};
use crate::mux::attach::{layout, status};
use par_term_emu_core::cursor::CursorStyle;
use par_term_emu_core::tmux_control::TmuxNotification;
use ratatui::buffer::Cell as RtCell;
use ratatui::style::Color as RtColor;

/// Resolve a render target to `(window, one-of-its-panes)`. Pane targets
/// go through pane-info (its second field is the window); window/session
/// targets through the targeted list queries; none = the target-less
/// attach rule ([`initial_attach_session`]: create-or-single-session,
/// refuse-on-many) resolved to its active window's marked pane.
pub(super) fn resolve_window_and_pane(
    conn: &mut crate::mux::attach::conn::AttachConn,
    target: Option<&str>,
) -> Result<(String, String), String> {
    match target {
        None => {
            let session = initial_attach_session(conn)?;
            let windows = conn
                .send_checked(&format!("list-windows -t {session}"))
                .map_err(|err| format!("list-windows failed: {err}"))?;
            let window = active_window_row(&windows.body).ok_or("the session has no windows")?;
            let pane = marked_pane_of(conn, &window)?;
            Ok((window, pane))
        }
        Some(target) => {
            // A pane: pane-info's second field names the window.
            if let Ok(reply) = conn.send_checked(&format!("pane-info -t {target}")) {
                if reply.ok {
                    if let Some(row) = reply
                        .body
                        .first()
                        .and_then(|l| crate::mux::ipc::PaneInfoRow::parse(l))
                    {
                        return Ok((row.window, row.pane));
                    }
                }
            }
            // A session: its active window. Probed BEFORE the window
            // branch — `list-panes -t` accepts session targets too (the
            // one-rule grammar), so it can no longer prove windowness.
            if let Ok(reply) = conn.send_checked(&format!("list-windows -t {target}")) {
                if reply.ok {
                    let window =
                        active_window_row(&reply.body).ok_or("the session has no windows")?;
                    let pane = marked_pane_of(conn, &window)?;
                    return Ok((window, pane));
                }
            }
            // A window: its marked pane.
            if let Ok(reply) = conn.send_checked(&format!("list-panes -t {target}")) {
                if reply.ok {
                    let pane = marked_pane_of(conn, target)?;
                    return Ok((target.to_string(), pane));
                }
            }
            Err(format!("no such target: {target}"))
        }
    }
}

/// The `*`-marked pane of a window (its active pane), else the first row.
pub(super) fn marked_pane_of(
    conn: &mut crate::mux::attach::conn::AttachConn,
    window: &str,
) -> Result<String, String> {
    let reply = conn
        .send_checked(&format!("list-panes -t {window}"))
        .map_err(|err| format!("list-panes failed: {err}"))?;
    reply
        .body
        .iter()
        .find(|l| l.split_whitespace().nth(2) == Some("*"))
        .or_else(|| reply.body.iter().find(|l| l.starts_with('%')))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string)
        .ok_or_else(|| format!("no panes under {window}"))
}

impl WindowSession {
    /// The display options a renderer built NOW paints with: the
    /// configured [`RenderOptions`], the side-panel width zeroed while
    /// the panel is hidden and the rail's while it is collapsed.
    pub(super) fn effective_render_opts(&self) -> RenderOptions {
        RenderOptions {
            sidebar_width: match (self.sidebar.is_on(), self.sidebar.is_collapsed()) {
                (false, _) => 0,
                (true, true) => crate::mux::attach::panels::SIDEBAR_RAIL_WIDTH,
                (true, false) => self.render_opts.sidebar_width,
            },
            ..self.render_opts.clone()
        }
    }

    /// Recompute the frame geometry from the live host grid, the status
    /// bar's visibility, and the side panel's EFFECTIVE width. The pump
    /// calls this once per tick (and before a parked refit); the toggles
    /// that move the chrome call it too, so their same-tick paint reads
    /// the new layout.
    pub(super) fn refresh_geometry(&mut self) {
        let (cols, rows) = crate::mux::attach::conn::terminal_grid();
        self.set_geometry_host(cols, rows);
    }

    /// Recompute the frame geometry over a known `cols` x `rows` host
    /// grid (a chrome toggle keeps the current snapshot's host size;
    /// the pump's per-tick refresh reads the tty).
    pub(super) fn set_geometry_host(&mut self, cols: u16, rows: u16) {
        self.geometry = super::geometry::FrameGeometry::new(
            cols,
            rows,
            self.chrome.status_bar_on,
            self.effective_render_opts().sidebar_width,
        );
    }

    /// Re-derive the chrome fields of the geometry (status bar, side
    /// panel) after a toggle, keeping the snapshot's host size.
    pub(super) fn chrome_geometry_changed(&mut self) {
        let (cols, rows) = (self.geometry.host_cols, self.geometry.host_rows);
        self.set_geometry_host(cols, rows);
    }

    /// Re-fit the renderer's frame to `cols` x `rows` in place (ENH-044)
    /// and push the session's geometry with the side panel's effective
    /// width. Emulators, titles, sections, the overlay, and every display
    /// option stay on the live renderer, so a refit can drop none of them
    /// (the ARC-122 class of regression).
    ///
    /// The frame's diff baseline is dropped even when the size is
    /// unchanged: a refit is followed by a full repaint (`resize_to`'s
    /// `repaint_all` clears the host; a re-seed shows another view), and a
    /// diff against the stale frame would skip every unchanged cell and
    /// leave the cleared host blank there.
    pub(super) fn refit_renderer(&mut self, cols: u16, rows: u16) {
        self.renderer.resize(cols, rows);
        self.renderer.invalidate_frame();
        let mut geometry = self.geometry;
        geometry.sidebar_w = self.effective_render_opts().sidebar_width;
        self.renderer.set_geometry(geometry);
    }

    /// Drain the events a size report queued, IN ORDER, discarding none
    /// (ENH-044). Returns the shown window's first `%layout-change`
    /// triple, if any, and the "race" panes: those whose `%output`
    /// arrived after it.
    ///
    /// Before the layout change every event goes through
    /// [`Self::handle_event`] (output feeds at the old size, which is the
    /// size the daemon produced it at). After it, `%output` is NOT fed:
    /// the daemon's reader applies bytes to the pane terminal before it
    /// pushes them, so a dispatch resize can land between the two and
    /// those bytes were applied at the OLD size — the caller replays the
    /// pane instead. Every other event still goes to `handle_event`; an
    /// `End` outcome parks in `pending_end` for the pump.
    pub(super) fn drain_around_layout(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) -> (Option<(String, String, String)>, Vec<u32>) {
        let mut layout_event = None;
        let mut race: Vec<u32> = Vec::new();
        for event in conn.drain_pending_events() {
            match event {
                TmuxNotification::LayoutChange {
                    window_id,
                    window_layout,
                    window_visible_layout,
                    window_raw_flags,
                } if layout_event.is_none() && window_id == self.window => {
                    layout_event = Some((window_layout, window_visible_layout, window_raw_flags));
                }
                TmuxNotification::Output { pane_id, .. }
                | TmuxNotification::ExtendedOutput { pane_id, .. }
                    if layout_event.is_some() =>
                {
                    if let Some(n) = pane_id.strip_prefix('%').and_then(|n| n.parse().ok()) {
                        if !race.contains(&n) {
                            race.push(n);
                        }
                    }
                }
                other => {
                    if self.handle_event(other) == super::EventOutcome::End {
                        self.pending_end = true;
                    }
                }
            }
        }
        (layout_event, race)
    }

    /// Install `layout` and return the panes to replay: those new to the
    /// renderer (no emulator state yet) and the `race` panes
    /// ([`Self::drain_around_layout`]). A surviving pane's emulator
    /// reflows in `apply_layout` exactly as the daemon's pane did, so it
    /// needs no replay.
    pub(super) fn install_layout(&mut self, layout: Vec<PaneRect>, race: &[u32]) -> Vec<u32> {
        let before: std::collections::HashSet<u32> =
            self.renderer.layout().iter().map(|r| r.pane).collect();
        self.renderer.apply_layout(layout);
        self.renderer
            .layout()
            .iter()
            .map(|r| r.pane)
            .filter(|p| !before.contains(p) || race.contains(p))
            .collect()
    }

    /// Record the session's resolved background (the renderer paints
    /// it; the session's options are the configured record). `None` = the
    /// probe failed; the fill stays terminal-default.
    pub(super) fn set_background(&mut self, bg: Option<RtColor>) {
        self.render_opts.bg = bg;
        self.renderer.set_background(bg);
    }

    /// Set the divider/border glyph set (the `border-lines` config and
    /// the border-cycle chord).
    pub(super) fn set_glyphs(&mut self, glyphs: Glyphs) {
        self.render_opts.glyphs = glyphs;
        self.renderer.set_glyphs(glyphs);
    }

    /// Apply the config `border-lines` spelling to the session's glyph
    /// set; reports whether the spelling was recognized (an unknown
    /// value keeps the current set — the caller warns).
    /// Apply a resolved chord set and its file to the live session: the
    /// keys, then the display options. The one application path for the
    /// launch config and every reload, so a default decided in
    /// `Chords::with_defaults` reaches the session the same way either
    /// time (ARC-131). The side panel's width and launch visibility are
    /// the launch path's own. Returns false when `border-lines` names no
    /// known style (the glyphs are left as they were).
    pub(super) fn apply_chords(
        &mut self,
        chords: &crate::mux::config::Chords,
        file: &crate::mux::config::ConfigFile,
    ) -> bool {
        self.prefix = chords.prefix;
        self.literal = chords.prefix;
        self.reload_key = chords.reload;
        self.management = chords.management;
        self.resize_step = chords.resize_step;
        if self.agent_rows != chords.agent_rows {
            self.agent_rows = chords.agent_rows.clone();
            self.rebuild_agent_section();
        }
        // An unknown delivery keeps the default (its warning rides
        // `display_option_warnings`).
        self.notify.delivery = super::notify::NotifyDelivery::parse(&chords.notify_delivery)
            .unwrap_or(super::notify::NotifyDelivery::Terminal);
        self.notify.states = chords.notify_states.clone();
        self.notify.sound = chords.sound.clone();
        self.sidebar_on_launch = chords.sidebar_on_launch;
        self.popups = chords.popups.clone();
        // The border style first, the explicit `pane-borders` flag after
        // it — the style implies a paint mode (herdr = per-pane boxes),
        // and the explicit config key still overrides for any glyph set.
        let known_style = self.set_border_lines(&chords.border_lines);
        // An explicit `pane-borders` key survives every reload; an absent
        // key follows the (possibly reloaded) border style. The chords'
        // own bool cannot tell explicit from absent, so the raw file key
        // decides.
        match file.client.pane_borders {
            Some(on) => self.set_pane_borders(on),
            None => self.set_pane_borders(matches!(self.render_opts.glyphs, Glyphs::Herdr)),
        }
        self.set_show_label_in_border(chords.show_label_in_border);
        self.set_pane_gaps(chords.pane_gaps);
        self.set_scrollbar_gutter(chords.scrollbar_gutter);
        self.cursor.drag_shape = chords.drag_cursor_shape;
        self.set_kitty_enabled(chords.kitty_graphics);
        let eff = crate::mux::config::resolve(file, &crate::mux::config::Overrides::default());
        self.set_border_colors(
            crate::mux::config::parse_hex_color(&eff.border_active_color)
                .map(|(r, g, b)| RtColor::Rgb(r, g, b)),
            crate::mux::config::parse_hex_color(&eff.border_color)
                .map(|(r, g, b)| RtColor::Rgb(r, g, b)),
        );
        self.theme.theme = chords.theme.clone();
        self.theme.dark = chords.theme_dark.clone();
        self.theme.light = chords.theme_light.clone();
        self.apply_theme();
        known_style
    }

    /// Push the effective theme's palette to every painter — the panes'
    /// renderer, the tab strip, the status row. A change dirties the
    /// frame and repaints both chrome rows whole.
    pub(super) fn apply_theme(&mut self) {
        let palette = crate::mux::theme::palette(self.theme.effective());
        if self.renderer.palette() == palette && self.chrome.tab_strip.palette == palette {
            return;
        }
        self.renderer.set_palette(palette);
        self.chrome.tab_strip.palette = palette;
        self.chrome.status_row.palette = palette;
        self.chrome.status_row.invalidate();
        self.draw_status_row();
        self.chrome.tab_strip.invalidate();
        self.draw_tab_strip();
    }

    /// A host background report (the launch probe, or a later OSC 11
    /// reply): the fill follows it and an `auto` theme re-picks.
    pub(super) fn observe_host_background(&mut self, (r, g, b): (u8, u8, u8)) {
        self.set_background(Some(RtColor::Rgb(r, g, b)));
        if self.theme.observe_background((r, g, b)) {
            self.apply_theme();
        }
    }

    /// A host report the stdin parser consumed. A late background reply
    /// updates the fill, the `auto` pick, and the daemon's theme record
    /// (as the launch probe does); an appearance report re-picks and
    /// re-queries OSC 11 so the fill follows the host's switch too.
    pub(super) fn on_host_report(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        report: crate::mux::attach::input::HostReport,
    ) {
        use crate::mux::attach::input::HostReport;
        match report {
            HostReport::Background((r, g, b)) => {
                self.observe_host_background((r, g, b));
                let _ = conn.send_forget(&format!(
                    "set-client-colors -f ffffff -b {r:02x}{g:02x}{b:02x}"
                ));
            }
            HostReport::Appearance(appearance) => {
                self.observe_host_appearance(appearance);
                self.notify.outbox.extend_from_slice(b"\x1b]11;?\x1b\\");
            }
            HostReport::KittyGraphics(ok) => self.set_kitty_supported(ok),
        }
    }

    /// An explicit host appearance report (mode 2031 `CSI ? 997 ; 1|2 n`):
    /// it beats background inference for the `auto` pick.
    pub(super) fn observe_host_appearance(&mut self, appearance: crate::mux::theme::Appearance) {
        if self.theme.observe_report(appearance) {
            self.apply_theme();
        }
    }

    pub(super) fn set_border_lines(&mut self, spec: &str) -> bool {
        let glyphs = match spec {
            "" | "unicode" => Glyphs::Unicode,
            "double" => Glyphs::Double,
            "heavy" => Glyphs::Heavy,
            "ascii" => Glyphs::Ascii,
            "herdr" => Glyphs::Herdr,
            _ => return false,
        };
        self.set_glyphs(glyphs);
        // The style owns the paint mode: herdr draws every pane its own
        // complete box, the rest the shared dividers. Applied before the
        // config's explicit `pane-borders` at the init/reload sites, so
        // that key still overrides for any glyph set.
        self.set_pane_borders(matches!(glyphs, Glyphs::Herdr));
        true
    }

    /// The session's pane-border mode (config `pane-borders`): each pane
    /// renders its own complete box with the content inset by the border
    /// cells, replacing the shared-divider look.
    pub(super) fn set_pane_borders(&mut self, on: bool) {
        if self.render_opts.pane_borders != on {
            self.chrome_changed();
        }
        self.render_opts.pane_borders = on;
        self.renderer.set_pane_borders(on);
    }

    /// A per-pane chrome option changed at runtime (the border-cycle
    /// chord, a config reload): the declared chrome rides the size
    /// report, so park a grid refit — the report re-declares and the
    /// daemon re-sizes every pane's PTY to the new interior. Before the
    /// seed there is nothing to refit (the seed's report declares).
    fn chrome_changed(&mut self) {
        if !self.window.is_empty() {
            self.pending.grid_refit = true;
        }
    }

    /// The window size report (`refresh-client -t <pane> -C WxH`) for the
    /// current renderer, carrying the per-pane chrome declaration (`-I`)
    /// when the daemon reserves it. Also arms the renderer's reserved
    /// mode, so its emulators mirror the PTY grids the daemon is about
    /// to size — the rect less the declared chrome.
    pub(super) fn size_report(
        &mut self,
        conn: &crate::mux::attach::conn::AttachConn,
        pane: &str,
    ) -> String {
        self.renderer
            .set_reserved_chrome(conn.has_command_feature("refresh-client", "chrome"));
        let (cols, rows) = self.renderer.window_size();
        format!(
            "refresh-client -t {pane} -C {cols}x{rows}{}",
            conn.chrome_declaration(self.renderer.chrome())
        )
    }

    /// The session's border colors (config `border-active-color` /
    /// `border-color`, `#rrggbb` hex; `None` keeps the built-ins).
    pub(super) fn set_border_colors(&mut self, active: Option<RtColor>, plain: Option<RtColor>) {
        self.render_opts.border_active = active;
        self.render_opts.border_plain = plain;
        self.renderer.set_border_colors(active, plain);
    }

    /// The session's label-in-border mode (config `show-label-in-border`):
    /// each pane's user title renders embedded in its top border edge.
    pub(super) fn set_show_label_in_border(&mut self, on: bool) {
        self.render_opts.show_label_in_border = on;
        self.renderer.set_show_label_in_border(on);
    }

    /// The session's gap-band mode (config `pane-gaps`): theme-bg gap
    /// bands between panes, the renderer insetting each pane's rect.
    pub(super) fn set_pane_gaps(&mut self, gaps: u16) {
        if self.render_opts.pane_gaps != gaps {
            self.chrome_changed();
        }
        self.render_opts.pane_gaps = gaps;
        self.renderer.set_pane_gaps(gaps);
    }

    /// The session's scrollbar-gutter mode (config `scrollbar-gutter`):
    /// a right-edge gutter column reserved in every pane rect.
    pub(super) fn set_scrollbar_gutter(&mut self, on: bool) {
        if self.render_opts.scrollbar_gutter != on {
            self.chrome_changed();
        }
        self.render_opts.scrollbar_gutter = on;
        self.renderer.set_scrollbar_gutter(on);
    }

    /// Seed the window: resolve it from the target (a pane/window/session
    /// target narrows as in passthrough; none = the newest session's
    /// active window), report the renderer size against one of its panes
    /// (the `%layout-change` reply carries the layout triple), parse the
    /// layout, then replay every pane's screen into its emulator.
    pub(super) fn seed(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        target: Option<&str>,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        // Registration replay (held panes, zoomed windows) is drained
        // ahead of the queries like passthrough does; the held panes'
        // `%pane-exited` lines seed the exit cue.
        for event in conn.drain_pending_events() {
            if let TmuxNotification::PaneExited { pane_id, exit_code } = event {
                if let Some(pane) = pane_id.strip_prefix('%').and_then(|p| p.parse().ok()) {
                    self.held.insert(pane, exit_code);
                }
            }
        }

        let (window, pane) = resolve_window_and_pane(conn, target)?;
        self.window = window;

        // Size report against the window: the daemon resizes the window
        // to the renderer's grid (the T4.C window-size policy) and the
        // queued `%layout-change` carries the current layout triple —
        // including the Z flag and visible layout when zoomed.
        let report = self.size_report(conn, &pane);
        conn.send_checked(&report)
            .map_err(|err| format!("refresh-client failed: {err}"))?;
        let layout_event = conn
            .drain_pending_events()
            .into_iter()
            .find_map(|event| match event {
                TmuxNotification::LayoutChange {
                    window_id,
                    window_layout,
                    window_visible_layout,
                    window_raw_flags,
                } if window_id == self.window => {
                    Some((window_layout, window_visible_layout, window_raw_flags))
                }
                _ => None,
            })
            .ok_or_else(|| {
                format!(
                    "no %layout-change for {window} — the daemon predates layout broadcast",
                    window = self.window
                )
            })?;
        let layout = layout::parse_layout_triple(&layout_event.0, &layout_event.1, &layout_event.2)
            .map_err(|err| err.to_string())?;
        self.zoomed = layout_event.2.contains('Z');
        self.renderer.apply_layout(layout);
        // A panel up at launch (`sidebar-on-launch`) needs its sections
        // for the first frame.
        if self.sidebar.is_on() {
            self.refresh_sidebar(conn);
        }

        // Replay every visible pane's state into its emulator: grid,
        // scrollback, cursor, and input modes ride the screen-restore
        // byte stream, the same stream passthrough writes to the host.
        self.replay_all_panes(conn);
        // The status bar seeds with the view.
        let focused = self.renderer.focused().unwrap_or(0);
        if let Err(status::StatusError::SessionGone) =
            self.status.refresh(conn, &self.window, focused)
        {
            return Err("the target's session is gone".to_string());
        }
        self.chrome.status_row.invalidate();
        self.draw_status_row();
        self.chrome.tab_strip.invalidate();
        self.draw_tab_strip();
        // First frame: clear + full paint. repaint_all hides the host
        // cursor, so the frame's place_cursor must fire even if the
        // mapped state matches the hidden default.
        self.cursor.invalidate();
        let stale = self.kitty.invalidate();
        if !stale.is_empty() {
            sink.write_raw(&stale);
        }
        sink.repaint_all();
        self.frame(sink);
        Ok(())
    }

    pub(super) fn feed_pane(&mut self, pane_id: &str, data: &[u8]) {
        if let Some(n) = pane_id.strip_prefix('%').and_then(|n| n.parse().ok()) {
            super::trace_input(format_args!("feed pane %{n} {} bytes", data.len()));
            self.renderer.feed_output(n, data);
        }
    }

    /// Replay every visible pane's state into its emulator (the seed).
    pub(super) fn replay_all_panes(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let all: Vec<u32> = self.renderer.layout().iter().map(|r| r.pane).collect();
        self.replay_panes(conn, &all);
    }

    /// Replay `panes`' state into their emulators (the seed, and the
    /// panes an in-place refit or re-seed cannot reconcile from the
    /// `%output` stream). The reply body's lines join back EXACTLY: the
    /// wire's body lines split on `\n` (BufRead::lines) and joining
    /// restores the byte stream — appending another `\n` would add a
    /// line feed the pane's restore stream never contained, driving the
    /// freshly positioned cursor one row below the tracked cell (the
    /// round-3 cursor off-by-one) or scrolling the grid when the cursor
    /// sat on the bottom row.
    pub(super) fn replay_panes(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        panes: &[u32],
    ) {
        super::trace_input(format_args!("replay panes {panes:?}"));
        for &id in panes {
            if let Ok(reply) = conn.send_checked(&format!("refresh-client -t %{id}")) {
                if reply.ok {
                    let bytes = reply.body.join("\n").into_bytes();
                    self.renderer.feed_replay(id, &bytes);
                }
            }
        }
    }

    /// The zoom state's effective layout — the daemon's zoom re-lays the
    /// window itself, so the client applies what the broadcast carries;
    /// the cache of the unzoomed tree geometry exists only so the
    /// prefix+arrow navigation can leave a zoom (the daemon unzooms on
    /// the select it issues).
    pub(super) fn tree_layout(&self) -> Vec<PaneRect> {
        if self.zoomed && !self.daemon_layout.is_empty() {
            self.daemon_layout.clone()
        } else {
            self.renderer.layout().to_vec()
        }
    }

    /// A pane id of `window` to hang the size report on: the focused pane
    /// when it still belongs there, else the window's first pane.
    pub(super) fn focused_pane_or_first(
        &self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) -> String {
        let focused = self.focused_pane();
        if let Ok(reply) = conn.send_checked(&format!("list-panes -t {window}")) {
            if reply.ok {
                let panes: Vec<String> = reply
                    .body
                    .iter()
                    .filter_map(|l| l.split_whitespace().next())
                    .filter(|p| p.starts_with('%'))
                    .map(str::to_string)
                    .collect();
                if panes.contains(&focused) {
                    return focused;
                }
                if let Some(first) = panes.first() {
                    return first.clone();
                }
            }
        }
        focused
    }

    /// Draw the status row: fresh state composed and painted into the row
    /// buffer (the pump flushes the diff with the next frame). Hidden,
    /// nothing paints: the row belongs to the pane grid, and the refit's
    /// full repaint is what erased the bar exactly once.
    pub(super) fn draw_status_row(&mut self) {
        let cols = self.geometry.host_cols;
        if self.chrome.status_row.cols() != cols {
            let palette = self.chrome.status_row.palette;
            self.chrome.status_row = StatusRow::new(cols);
            self.chrome.status_row.palette = palette;
        }
        if !self.chrome.status_bar_on {
            return;
        }
        let scroll = if matches!(self.modal, Modal::Scroll) {
            self.renderer
                .focused()
                .map(|id| self.renderer.scroll_offset_of(id))
        } else {
            None
        };
        // The flash cue, the prefix-armed cue, and the zoom marker lead
        // the line and the Help chip RIGHT-ALIGNS at the row's end: all
        // four reserve their widths up front so the composed state
        // truncates into what remains, and the paint clips the content
        // at the chip's left column — the chip can never be pushed off
        // the row (round 6: the chip used to ride at the end of the
        // composed segments, and a full line pushed it off).
        let mut head: Vec<Segment> = Vec::new();
        if self.zoomed {
            head.push(Segment {
                text: " Z |".to_string(),
                bold: true,
                dim: false,
                shaded: false,
            });
        }
        if self.prefix_pending {
            head.push(Segment {
                text: " prefix |".to_string(),
                bold: true,
                dim: false,
                shaded: false,
            });
        }
        if let Some(cue) = self.exit_cue() {
            head.push(Segment {
                text: format!(" {cue} |"),
                bold: true,
                dim: false,
                shaded: false,
            });
        }
        if let Some(flash) = self.chrome.flash_text().map(str::to_owned) {
            head.push(Segment {
                text: format!(" {flash} |"),
                bold: true,
                dim: false,
                shaded: false,
            });
        }
        let chip = status::help_chip(self.prefix, self.management.help);
        let reserved: usize = head
            .iter()
            .chain(std::iter::once(&chip))
            .map(|s| s.text.chars().count())
            .sum();
        let segments = self.status.compose(
            cols.saturating_sub(reserved as u16),
            scroll,
            self.sidebar.is_on(),
        );
        let mut with_head = head;
        with_head.extend(segments);
        self.chrome.status_row.paint(&with_head, Some(&chip));
    }

    /// Flush the status row's changed cells to the host's bottom row.
    /// Returns whether anything flushed (its per-cell CUPs move the host
    /// cursor, so the caller must re-place it). While the bar is hidden
    /// nothing flushes at all: the bottom row is pane real estate, and a
    /// blank-cell flush would erase the pane's freshly painted bottom
    /// row after the pane frame.
    pub(super) fn flush_status_row(&mut self, sink: &mut dyn FlushSink) -> bool {
        let Some(bottom) = self.geometry.status_row() else {
            return false;
        };
        let diff = self.chrome.status_row.diff();
        if diff.is_empty() {
            return false;
        }
        // Rebase the row-relative cells to the host's bottom row and
        // reuse the sink's per-cell spelling.
        let rebased: Vec<(u16, u16, RtCell)> = diff
            .into_iter()
            .map(|(x, _y, cell)| (x, bottom, cell))
            .collect();
        sink.flush(&rebased);
        true
    }

    pub(super) fn focused_pane(&self) -> String {
        self.renderer
            .focused()
            .map(|n| format!("%{n}"))
            .unwrap_or_default()
    }

    /// Host resize: report the new grid against the window (the daemon
    /// re-divides and re-broadcasts the layout), re-fit in place, replay
    /// only the panes the `%output` stream cannot reconcile, repaint all.
    /// An `%exit` drained here parks in `pending_end` for the pump.
    pub(super) fn resize_to(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        cols: u16,
        rows: u16,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        let pane = self.focused_pane();
        if pane.is_empty() {
            return Ok(());
        }
        // Re-fit FIRST: the report below reads the renderer's window_size
        // (the new host grid less the side panel), and the refit carries
        // the panel width it depends on.
        self.refit_renderer(cols, rows);
        let report = self.size_report(conn, &pane);
        conn.send_checked(&report)
            .map_err(|err| format!("resize report failed: {err}"))?;
        // Drain the layout broadcast the report queued here directly so
        // the repaint is synchronous.
        let (layout_event, race) = self.drain_around_layout(conn);
        if let Some((l, v, f)) = layout_event {
            self.zoomed = f.contains('Z');
            if let Ok(layout) = layout::parse_layout_triple(&l, &v, &f) {
                self.daemon_layout = layout.clone();
                let replay = self.install_layout(layout, &race);
                self.replay_panes(conn, &replay);
            }
        }
        // Same as the seed: repaint_all hid the cursor, so force the
        // next place_cursor.
        self.cursor.invalidate();
        // repaint_all clears the WHOLE screen — the strip rows too — and
        // the frame below flushes diff-based, so both strips forget
        // their previous frames here (the sidebar-toggle path parks the
        // refit into the pump, which repaints nothing else on those
        // rows).
        self.chrome.status_row.invalidate();
        self.draw_status_row();
        self.chrome.tab_strip.invalidate();
        self.draw_tab_strip();
        let stale = self.kitty.invalidate();
        if !stale.is_empty() {
            sink.write_raw(&stale);
        }
        sink.repaint_all();
        self.frame(sink);
        Ok(())
    }

    /// Frame the pending output at cadence: panes, the tab strip, the
    /// status row, then the cursor — positioned at the focused pane's
    /// tracked cell (mapped through rect origin + scroll offset; hidden
    /// when the view is scrolled off live or the pane hid its cursor via
    /// DECTCEM). A flushed frame always repositions (the diff's last
    /// per-cell CUP left the terminal cursor wherever the last diff cell
    /// sits), while a quiet pump re-emits only on a state change.
    ///
    /// Coordinate spaces: the renderer's frame covers the rows BETWEEN
    /// the strip and the status row, so its diff and cursor rebase +1
    /// host row; the strip flushes at host row 0 and the status row at
    /// the host's bottom row.
    pub(super) fn frame(&mut self, sink: &mut dyn FlushSink) {
        if !self.notify.outbox.is_empty() {
            sink.write_raw(&std::mem::take(&mut self.notify.outbox));
        }
        // The pointer's shape change rides the same raw path (OSC 22).
        self.pointer.emit(sink);
        let geometry = self.geometry;
        let mut flushed = false;
        if self.renderer.needs_frame() {
            let mut diff = self.renderer.render_frame();
            if !diff.is_empty() {
                // Rebase the owned diff in place: no second vector, no
                // cell clones (ARC-132).
                for (x, y, _) in &mut diff {
                    (*x, *y) = geometry.frame_to_host(*x, *y);
                }
                sink.flush(&diff);
                flushed = true;
            }
        }
        if self.flush_tab_strip(sink) {
            flushed = true;
        }
        if self.flush_status_row(sink) {
            flushed = true;
        }
        let covered =
            self.renderer.overlay.is_some() || self.modal.hides_cursor() || self.popup_active();
        if self.frame_graphics(sink, covered) {
            flushed = true;
        }
        // An open popup owns the cursor: its tracked cell inside the box
        // (hidden while it opens or hides its own cursor).
        let mut cursor = if self.popup_active() {
            self.renderer.popup_cursor()
        } else {
            self.renderer.focused_cursor()
        }
        .map(|(x, y, style)| {
            let (hx, hy) = geometry.frame_to_host(x, y);
            (hx, hy, style)
        });
        // The drag cursor shape (config `drag-cursor-shape`): while a
        // divider drag is live the host cursor carries the resize shape
        // (best-effort DECSCUSR steady block — DECSCUSR has no
        // column-resize value; every drag-end path clears `drag`, so the
        // next frame restores the pane's own shape). The POINTER carries
        // the axis-correct resize shape via OSC 22 (see `update_pointer`)
        // — this text-cursor swap is the fallback cue for terminals
        // without OSC 22.
        if self.cursor.drag_shape && matches!(&self.drag, Some(DragState::Active { .. })) {
            cursor = cursor.map(|(x, y, _)| (x, y, CursorStyle::SteadyBlock));
        }
        // A modal overlay (help, picker) covers the focused pane: placing
        // the pane's cursor would draw it through the panel (the
        // manual-pass report). Hide the host cursor while a modal is up;
        // the next unmodaled frame restores placement.
        let modal_up = self.renderer.overlay.is_some() || self.modal.hides_cursor();
        if modal_up {
            cursor = None;
        }
        // A pane that hides its cursor (DECTCEM) still gets the hidden
        // host cursor parked on its tracked cell: the host terminal
        // anchors IME preedit and the candidate window there (herdr
        // parks the same way). Not under a modal: the panel owns the
        // screen there.
        let anchor = if cursor.is_none() && !modal_up && !self.popup_active() {
            self.renderer
                .focused_ime_anchor()
                .map(|(x, y)| geometry.frame_to_host(x, y))
        } else {
            None
        };
        let emitted = self
            .cursor
            .decide(cursor, flushed, std::time::Instant::now());
        if emitted {
            sink.place_cursor(cursor);
        }
        if let (true, Some((x, y))) = (self.cursor.decide_anchor(anchor, emitted), anchor) {
            sink.anchor_hidden_cursor(x, y);
        }
    }

    /// Paint the tab strip from the queried window state (the same
    /// state the status row composes from): the shown session's windows
    /// in order, the active one accented. With the side panel up the
    /// strip leads with the panel's title (the lead segment the hit-test
    /// offsets through) — the panel itself opens directly with its rows.
    pub(super) fn draw_tab_strip(&mut self) {
        let cols = self.geometry.host_cols;
        if self.chrome.tab_strip.cols() != cols {
            let palette = self.chrome.tab_strip.palette;
            self.chrome.tab_strip = TabStrip::new(cols);
            self.chrome.tab_strip.palette = palette;
        }
        // The rail is too narrow for the title; its columns stay blank.
        let strip = self.renderer.sidebar_width();
        let title = if self.sidebar.is_collapsed() {
            ""
        } else {
            "workspaces"
        };
        let lead = (strip > 0).then_some((title, strip));
        // An open popup's scratch window is not a tab of this view.
        let windows: Vec<(String, String)> = self
            .status
            .windows()
            .iter()
            .filter(|(id, _)| Some(id.as_str()) != self.popup_window())
            .cloned()
            .collect();
        self.chrome
            .tab_strip
            .paint(&windows, self.status.active_window.as_deref(), lead);
    }

    /// Flush the tab strip's changed cells to the host's top row.
    /// Returns whether anything flushed (its per-cell CUPs move the host
    /// cursor, so the caller must re-place it).
    pub(super) fn flush_tab_strip(&mut self, sink: &mut dyn FlushSink) -> bool {
        let diff = self.chrome.tab_strip.diff();
        if diff.is_empty() {
            return false;
        }
        sink.flush(&diff);
        true
    }
}
