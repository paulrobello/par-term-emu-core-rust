//! Session plumbing: seeding a window, feeding pane bytes, the frame
//! cadence, host resize, the status row, the tab strip, and the
//! config-driven display setters.

use super::*;

/// The daemon's displayed session (the active workspace's active session)
/// via the bare `switch-client` query; `None` when the daemon predates it
/// or nothing is displayed.
pub(super) fn displayed_session(conn: &mut crate::mux::attach::conn::AttachConn) -> Option<String> {
    let reply = conn.send_checked("switch-client").ok()?;
    if !reply.ok {
        return None;
    }
    let id = reply.body.first()?.trim();
    (id.starts_with('$') && id.len() > 1).then(|| id.to_string())
}

/// Resolve a render target to `(window, one-of-its-panes)`. Pane targets
/// go through pane-info (its second field is the window); window/session
/// targets through the targeted list queries; none = the displayed
/// session's active window's active pane — the session every other
/// client and the follow broadcasts agree on (card 01a11bd1: the former
/// newest-id stand-in landed on whichever session was created last, a
/// single-pane view while the display showed another). A daemon without
/// the `switch-client` query keeps the newest-id rule.
pub(super) fn resolve_window_and_pane(
    conn: &mut crate::mux::attach::conn::AttachConn,
    target: Option<&str>,
) -> Result<(String, String), String> {
    match target {
        None => {
            let sessions = conn
                .send_checked("list-sessions")
                .map_err(|err| format!("list-sessions failed: {err}"))?;
            // The wire shape is `$N: name`; the id ends at the colon — a
            // whitespace split keeps it (`$0:`), which the daemon's id
            // parser rejects. Newest = highest id (ids are monotonic).
            let newest = sessions
                .body
                .iter()
                .filter_map(|l| super::super::parse_session_line(l).map(|(id, _)| id))
                .filter(|id| id.starts_with('$'))
                .filter_map(|id| {
                    let n: u64 = id[1..].parse().ok()?;
                    Some((n, id))
                })
                .max_by_key(|(n, _)| *n)
                .map(|(_, id)| id);
            let session = displayed_session(conn)
                .or(newest)
                .ok_or("no sessions exist — create one first")?;
            let windows = conn
                .send_checked(&format!("list-windows -t {session}"))
                .map_err(|err| format!("list-windows failed: {err}"))?;
            let window = windows
                .body
                .iter()
                .find(|l| l.split_whitespace().nth(1) == Some("*"))
                .or_else(|| windows.body.iter().find(|l| l.starts_with('@')))
                .and_then(|l| l.split_whitespace().next())
                .ok_or("the session has no windows")?;
            let pane = marked_pane_of(conn, window)?;
            Ok((window.to_string(), pane))
        }
        Some(target) => {
            // A pane: pane-info's second field names the window.
            if let Ok(reply) = conn.send_checked(&format!("pane-info -t {target}")) {
                if reply.ok {
                    if let Some(line) = reply.body.first() {
                        let mut fields = line.split_whitespace();
                        if let (Some(pane), Some(window)) = (fields.next(), fields.next()) {
                            if pane.starts_with('%') && window.starts_with('@') {
                                return Ok((window.to_string(), pane.to_string()));
                            }
                        }
                    }
                }
            }
            // A window: its marked pane.
            if let Ok(reply) = conn.send_checked(&format!("list-panes -t {target}")) {
                if reply.ok {
                    let pane = marked_pane_of(conn, target)?;
                    return Ok((target.to_string(), pane));
                }
            }
            // A session: its active window.
            if let Ok(reply) = conn.send_checked(&format!("list-windows -t {target}")) {
                if reply.ok {
                    let window = reply
                        .body
                        .iter()
                        .find(|l| l.split_whitespace().nth(1) == Some("*"))
                        .or_else(|| reply.body.iter().find(|l| l.starts_with('@')))
                        .and_then(|l| l.split_whitespace().next())
                        .ok_or("the session has no windows")?;
                    let pane = marked_pane_of(conn, window)?;
                    return Ok((window.to_string(), pane));
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
    /// the panel is hidden.
    pub(super) fn effective_render_opts(&self) -> RenderOptions {
        RenderOptions {
            sidebar_width: if self.sidebar_on {
                self.render_opts.sidebar_width
            } else {
                0
            },
            ..self.render_opts.clone()
        }
    }

    /// Replace the renderer with a fresh one over `cols` x `rows`,
    /// built from the session's effective options — the one rebuild
    /// path (resize re-fit, re-seed), so no display option can be lost
    /// across a rebuild.
    pub(super) fn rebuild_renderer(&mut self, cols: u16, rows: u16) {
        self.renderer = PaneRenderer::with_options(cols, rows, &self.effective_render_opts());
    }

    /// Record the session's resolved background (the renderer paints
    /// it; the session's options keep it across rebuilds). `None` = the
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
            self.pending_grid_refit = true;
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
        // ahead of the queries like passthrough does.
        let _replay = conn.drain_pending_events();

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
        if self.sidebar_on {
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
        self.status_row.invalidate();
        self.draw_status_row();
        self.tab_strip.invalidate();
        self.draw_tab_strip();
        // First frame: clear + full paint. repaint_all hides the host
        // cursor, so the frame's place_cursor must fire even if the
        // mapped state matches the hidden default.
        self.cursor_placed = Some(None);
        sink.repaint_all();
        self.frame(sink);
        Ok(())
    }

    pub(super) fn feed_pane(&mut self, pane_id: &str, data: &[u8]) {
        if let Some(n) = pane_id.strip_prefix('%').and_then(|n| n.parse().ok()) {
            self.renderer.feed_output(n, data);
        }
    }

    /// Replay every visible pane's state into its emulator (the seed and
    /// the post-resize re-seed share this). The reply body's lines join
    /// back EXACTLY: the wire's body lines split on `\n` (BufRead::lines)
    /// and joining restores the byte stream — appending another `\n`
    /// would add a line feed the pane's restore stream never contained,
    /// driving the freshly positioned cursor one row below the tracked
    /// cell (the round-3 cursor off-by-one) or scrolling the grid when
    /// the cursor sat on the bottom row.
    pub(super) fn replay_all_panes(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        for rect in self.renderer.layout().to_vec() {
            let pane = format!("%{}", rect.pane);
            if let Ok(reply) = conn.send_checked(&format!("refresh-client -t {pane}")) {
                if reply.ok {
                    let bytes = reply.body.join("\n").into_bytes();
                    self.renderer.feed_output(rect.pane, &bytes);
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
        let (cols, _rows) = super::super::conn::terminal_grid();
        if self.status_row.cols() != cols {
            self.status_row = StatusRow::new(cols);
        }
        if !self.status_bar_on {
            return;
        }
        let scroll = if self.scroll_mode {
            self.renderer
                .focused()
                .map(|id| self.renderer.scroll_offset_of(id))
        } else {
            None
        };
        // The flash cue and the zoom marker lead the line and the Help
        // chip RIGHT-ALIGNS at the row's end: all three reserve their
        // widths up front so the composed state truncates into what
        // remains, and the paint clips the content at the chip's left
        // column — the chip can never be pushed off the row (round 6:
        // the chip used to ride at the end of the composed segments,
        // and a full line pushed it off).
        let mut head: Vec<Segment> = Vec::new();
        if self.zoomed {
            head.push(Segment {
                text: " Z |".to_string(),
                bold: true,
                dim: false,
                shaded: false,
            });
        }
        if let Some(flash) = self.flash.clone() {
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
            self.sidebar_on,
        );
        let mut with_head = head;
        with_head.extend(segments);
        self.status_row.paint(&with_head, Some(&chip));
    }

    /// Flush the status row's changed cells to the host's bottom row.
    /// Returns whether anything flushed (its per-cell CUPs move the host
    /// cursor, so the caller must re-place it). While the bar is hidden
    /// nothing flushes at all: the bottom row is pane real estate, and a
    /// blank-cell flush would erase the pane's freshly painted bottom
    /// row after the pane frame.
    pub(super) fn flush_status_row(&mut self, sink: &mut dyn FlushSink) -> bool {
        if !self.status_bar_on {
            return false;
        }
        let (_cols, rows) = super::super::conn::terminal_grid();
        let bottom = rows.saturating_sub(1);
        let diff = self.status_row.diff();
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
    /// re-divides and re-broadcasts the layout), re-fit, repaint all.
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
        // Reconstruct FIRST: the report below reads the fresh renderer's
        // window_size (the new host grid less the side panel), and the
        // reconstruction carries the panel width it depends on.
        self.rebuild_renderer(cols, rows);
        let report = self.size_report(conn, &pane);
        conn.send_checked(&report)
            .map_err(|err| format!("resize report failed: {err}"))?;
        // The layout broadcast the report queued re-seeds the panes; but
        // drain it here directly so the repaint is synchronous.
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
            });
        if let Some((l, v, f)) = layout_event {
            self.zoomed = f.contains('Z');
            if let Ok(layout) = layout::parse_layout_triple(&l, &v, &f) {
                self.daemon_layout = layout.clone();
                self.renderer.apply_layout(layout);
                for rect in self.renderer.layout().to_vec() {
                    let pane = format!("%{}", rect.pane);
                    if let Ok(reply) = conn.send_checked(&format!("refresh-client -t {pane}")) {
                        if reply.ok {
                            // Join only: an extra trailing `\n` here would
                            // push the replayed cursor one row down (the
                            // replay_all_panes note).
                            let bytes = reply.body.join("\n").into_bytes();
                            self.renderer.feed_output(rect.pane, &bytes);
                        }
                    }
                }
            }
        }
        // Same as the seed: repaint_all hid the cursor, so force the
        // next place_cursor.
        self.cursor_placed = Some(None);
        // repaint_all clears the WHOLE screen — the strip rows too — and
        // the frame below flushes diff-based, so both strips forget
        // their previous frames here (the sidebar-toggle path parks the
        // refit into the pump, which repaints nothing else on those
        // rows).
        self.status_row.invalidate();
        self.draw_status_row();
        self.tab_strip.invalidate();
        self.draw_tab_strip();
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
        const STRIP_ROWS: u16 = 1;
        let mut flushed = false;
        if self.renderer.needs_frame() {
            let diff = self.renderer.render_frame();
            if !diff.is_empty() {
                let rebased: Vec<(u16, u16, RtCell)> = diff
                    .into_iter()
                    .map(|(x, y, cell)| (x, y + STRIP_ROWS, cell))
                    .collect();
                sink.flush(&rebased);
                flushed = true;
            }
        }
        if self.flush_tab_strip(sink) {
            flushed = true;
        }
        if self.flush_status_row(sink) {
            flushed = true;
        }
        let mut cursor = self
            .renderer
            .focused_cursor()
            .map(|(x, y, style)| (x, y + STRIP_ROWS, style));
        // The drag cursor shape (config `drag-cursor-shape`): while a
        // divider drag is live the host cursor carries the resize shape
        // (best-effort DECSCUSR steady block — DECSCUSR has no
        // column-resize value; every drag-end path clears `drag`, so the
        // next frame restores the pane's own shape).
        if self.drag_cursor_shape && matches!(&self.drag, Some(DragState::Active { .. })) {
            cursor = cursor.map(|(x, y, _)| (x, y, CursorStyle::SteadyBlock));
        }
        // A modal overlay (help, picker) covers the focused pane: placing
        // the pane's cursor would draw it through the panel (the
        // manual-pass report). Hide the host cursor while a modal is up;
        // the next unmodaled frame restores placement.
        if self.renderer.overlay.is_some() || self.picker_mode {
            cursor = None;
        }
        if flushed || self.cursor_placed.as_ref() != Some(&cursor) {
            sink.place_cursor(cursor);
            self.cursor_placed = Some(cursor);
        }
    }

    /// Paint the tab strip from the queried window state (the same
    /// state the status row composes from): the shown session's windows
    /// in order, the active one accented. With the side panel up the
    /// strip leads with the panel's title (the lead segment the hit-test
    /// offsets through).
    pub(super) fn draw_tab_strip(&mut self) {
        let (cols, _rows) = super::super::conn::terminal_grid();
        if self.tab_strip.cols() != cols {
            self.tab_strip = TabStrip::new(cols);
        }
        let strip = self.renderer.sidebar_width();
        let lead = (strip > 0).then_some(("workspaces", strip));
        self.tab_strip.paint(
            self.status.windows(),
            self.status.active_window.as_deref(),
            lead,
        );
    }

    /// Flush the tab strip's changed cells to the host's top row.
    /// Returns whether anything flushed (its per-cell CUPs move the host
    /// cursor, so the caller must re-place it).
    pub(super) fn flush_tab_strip(&mut self, sink: &mut dyn FlushSink) -> bool {
        let diff = self.tab_strip.diff();
        if diff.is_empty() {
            return false;
        }
        sink.flush(&diff);
        true
    }
}
