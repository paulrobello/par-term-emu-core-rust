//! The workspace side panel: its strip geometry, painting, row hit-test,
//! and the toggle/refresh the session drives.

use super::*;

impl PaneRenderer {
    /// The side panel's width (0 = hidden).
    pub(crate) fn sidebar_width(&self) -> u16 {
        self.sidebar_w
    }

    /// Show the side panel at `width` columns (0 hides it). The daemon
    /// never learns of the strip — window_size reports the reduced grid,
    /// and the next refresh-client report re-divides the panes around it.
    pub(crate) fn set_sidebar_width(&mut self, width: u16) {
        if self.sidebar_w != width {
            self.sidebar_w = width;
            self.sidebar_sections = None;
            self.dirty = true;
        }
    }

    /// Replace the panel's sections (the workspace roster query's result)
    /// and mark dirty.
    pub(crate) fn set_sidebar_sections(
        &mut self,
        sections: Option<Vec<super::super::SidebarSection>>,
    ) {
        if self.sidebar_sections != sections {
            self.sidebar_sections = sections;
            self.dirty = true;
        }
    }

    /// The clickable side-panel id under the HOST cell `(x, y)`, if any.
    /// Host row 0 is the tab-strip row (no panel there), and a panel
    /// line composed at row `y` paints at HOST row `y + 1` — the frame
    /// rebases the renderer's rows +1 — so the lookup shifts the host
    /// row down one before matching.
    pub(crate) fn sidebar_row_at(&self, x: u16, y: u16) -> Option<String> {
        if x >= self.sidebar_w || y == 0 {
            return None;
        }
        let sections = self.sidebar_sections.as_ref()?;
        let lines = super::super::compose_sidebar(sections, self.sidebar_w, self.height);
        lines
            .into_iter()
            .find(|line| line.y == y - 1 && line.id.is_some() && x >= line.x && x < line.x_end)
            .and_then(|line| line.id)
    }

    /// Paint the workspace side panel over the strip columns: theme-bg
    /// fill, a dim divider on the strip's right edge, then the sections —
    /// each header in the accent, its rows dim (the active workspace
    /// accent + bold, herdr's emphasis). Clipped to the strip; composed
    /// by [`super::super::compose_sidebar`].
    pub(super) fn paint_sidebar(&mut self) {
        let w = self.sidebar_w;
        if w == 0 {
            return;
        }
        let dim = RtStyle::default().add_modifier(RtModifier::DIM);
        for y in 0..self.height {
            for x in 0..w {
                let cell = &mut self.buffer[(x, y)];
                cell.reset();
                if let Some(bg) = self.bg {
                    cell.set_bg(bg);
                }
                if x + 1 == w {
                    cell.set_symbol(self.glyphs.vertical());
                    cell.set_style(dim);
                }
            }
        }
        let Some(sections) = self.sidebar_sections.clone() else {
            return;
        };
        for line in super::super::compose_sidebar(&sections, w, self.height) {
            // herdr's active treatment: the ACTIVE WORKSPACE's row is a
            // full-width inverted block (the round-6 panel lists only
            // workspaces). The footer chips paint at their own columns
            // (line.x), body rows at col 0.
            let workspace_row = line.id.as_deref().is_some_and(|id| id.starts_with("ws:"));
            if line.active && workspace_row {
                let block = RtStyle::default()
                    .add_modifier(RtModifier::REVERSED)
                    .add_modifier(RtModifier::BOLD);
                for x in 0..w - 1 {
                    let cell = &mut self.buffer[(x, line.y)];
                    cell.reset();
                    if let Some(bg) = self.bg {
                        cell.set_bg(bg);
                    }
                    cell.set_style(block);
                }
            }
            let style = if line.active && workspace_row {
                RtStyle::default()
                    .add_modifier(RtModifier::REVERSED)
                    .add_modifier(RtModifier::BOLD)
            } else {
                dim
            };
            for (j, ch) in line.text.chars().enumerate() {
                let x = line.x + j as u16;
                if x >= w - 1 {
                    break;
                }
                let cell = &mut self.buffer[(x, line.y)];
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
            }
        }
    }
}

impl WindowSession {
    /// Land the view on `ws_id`: its first listed session's active
    /// window, re-seeded (the select+resync contract every switch
    /// follows). An empty workspace only marks the status stale — the
    /// %workspaces-changed the select queued moves the strip's active
    /// marker on the next frame.
    /// Toggle the side panel: flip the strip width on the renderer, then
    /// report the REDUCED/RESTORED grid — `refresh-client -C` — so the
    /// daemon re-divides the panes around it; the `%layout-change`
    /// re-seed repaints. Opening queries the workspace roster for the
    /// section rows.
    pub(super) fn toggle_sidebar(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        self.sidebar_on = !self.sidebar_on;
        self.renderer
            .set_sidebar_width(self.effective_render_opts().sidebar_width);
        // The refit runs on the pump (it owns the flush sink): resize_to
        // reports the new grid, then repaint_all erases the region the
        // old layout vacated — the plain %layout-change re-seed never
        // clears those host cells (the manual-pass ghost-pane report).
        self.pending_grid_refit = true;
        if self.sidebar_on {
            self.refresh_sidebar(conn);
        }
        self.flash = Some(
            if self.sidebar_on {
                "sidebar on"
            } else {
                "sidebar off"
            }
            .to_string(),
        );
        self.draw_status_row();
    }

    /// Re-query the workspace roster into the panel's sections — on
    /// open, and on every status refresh while the panel is up (the
    /// `%workspaces-changed` mark rides the same throttle).
    pub(super) fn refresh_sidebar(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let rows: Vec<(String, String, bool)> = conn
            .send_checked("list-workspaces")
            .ok()
            .filter(|reply| reply.ok)
            .map(|reply| {
                reply
                    .body
                    .iter()
                    .filter_map(|l| super::super::parse_workspace_line(l))
                    .map(|(id, name, active)| (format!("ws:{id}"), name, active))
                    .collect()
            })
            .unwrap_or_default();
        if rows.is_empty() {
            self.renderer.set_sidebar_sections(None);
            return;
        }
        self.renderer
            .set_sidebar_sections(Some(vec![super::super::SidebarSection { rows }]));
    }
}
