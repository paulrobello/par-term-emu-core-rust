//! The workspace side panel: its strip geometry, painting, row hit-test,
//! and the toggle/refresh the session drives.

use super::{PaneRenderer, WindowSession};
use crate::mux::attach::panels::compose_sidebar;
use crate::mux::attach::status;
use crate::mux::attach::targets::parse_workspace_line;
use ratatui::style::{Modifier as RtModifier, Style as RtStyle};

use crate::mux::attach::panels::{SidebarRow, SidebarRowId, SidebarSection};

impl PaneRenderer {
    /// The side panel's width (0 = hidden).
    pub(crate) fn sidebar_width(&self) -> u16 {
        self.geometry.sidebar_w
    }

    /// Show the side panel at `width` columns (0 hides it). The daemon
    /// never learns of the strip — window_size reports the reduced grid,
    /// and the next refresh-client report re-divides the panes around it.
    /// Sessions push their whole geometry ([`Self::set_geometry`]).
    #[cfg(test)]
    pub(crate) fn set_sidebar_width(&mut self, width: u16) {
        let mut geometry = self.geometry;
        geometry.sidebar_w = width;
        self.set_geometry(geometry);
    }

    /// Adopt the session's frame geometry (the one owner of the
    /// host/frame/content mapping, ARC-128). A changed panel width drops
    /// the stale sections and repaints.
    pub(crate) fn set_geometry(&mut self, geometry: super::geometry::FrameGeometry) {
        if self.geometry.sidebar_w != geometry.sidebar_w {
            self.sidebar_sections = None;
            self.dirty = true;
        }
        self.geometry = geometry;
    }

    /// Replace the panel's sections (the workspace roster query's result)
    /// and mark dirty.
    pub(crate) fn set_sidebar_sections(&mut self, sections: Option<Vec<SidebarSection>>) {
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
    pub(crate) fn sidebar_row_at(&self, x: u16, y: u16) -> Option<SidebarRowId> {
        let w = self.geometry.sidebar_w;
        let (x, frame_y) = self.geometry.host_to_frame(x, y)?;
        if x >= w {
            return None;
        }
        let sections = self.sidebar_sections.as_ref()?;
        let lines = compose_sidebar(sections, w, self.height);
        lines
            .into_iter()
            .find(|line| line.y == frame_y && line.id.is_some() && x >= line.x && x < line.x_end)
            .and_then(|line| line.id)
    }

    /// Paint the workspace side panel over the strip columns: theme-bg
    /// fill, a dim divider on the strip's right edge, then the sections —
    /// each header in the accent, its rows dim (the active workspace
    /// accent + bold, herdr's emphasis). Clipped to the strip; composed
    /// by [`compose_sidebar`].
    pub(super) fn paint_sidebar(&mut self) {
        let w = self.geometry.sidebar_w;
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
        for line in compose_sidebar(&sections, w, self.height) {
            // herdr's active treatment: an active row (the active
            // workspace, or the focused pane's agent row) is a full-width
            // inverted block. The footer chips paint at their own columns
            // (line.x), body rows at col 0.
            if line.active {
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
            let style = if line.active {
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
    /// Toggle the side panel: flip the strip width on the renderer, then
    /// report the REDUCED/RESTORED grid — `refresh-client -C` — so the
    /// daemon re-divides the panes around it; the `%layout-change`
    /// re-seed repaints. Opening queries the workspace roster for the
    /// section rows.
    pub(super) fn toggle_sidebar(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        self.sidebar.toggle();
        self.chrome_geometry_changed();
        self.renderer.set_geometry(self.geometry);
        // The refit runs on the pump (it owns the flush sink): resize_to
        // reports the new grid, then repaint_all erases the region the
        // old layout vacated — the plain %layout-change re-seed never
        // clears those host cells (the manual-pass ghost-pane report).
        self.pending.grid_refit = true;
        if self.sidebar.is_on() {
            self.refresh_sidebar(conn);
        }
        self.chrome.flash(
            if self.sidebar.is_on() {
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
        // A `client-snapshot` daemon's refresh already carries the roster
        // (ENH-043); query only before the first refresh filled it.
        let roster: Vec<(String, String, bool)> = if conn
            .has_command_feature("client-snapshot", "v1")
            && !self.status.workspaces().is_empty()
        {
            let active = self.status.active_workspace().map(str::to_string);
            self.status
                .workspaces()
                .iter()
                .map(|(id, name)| (id.clone(), name.clone(), Some(id) == active.as_ref()))
                .collect()
        } else {
            // The roster arrives on a later tick; until then the panel
            // keeps its current sections rather than blanking.
            self.send_then(conn, "list-workspaces", |s, _c, reply| {
                let roster: Vec<(String, String, bool)> = if reply.ok {
                    reply
                        .body
                        .iter()
                        .filter_map(|l| parse_workspace_line(l))
                        .collect()
                } else {
                    Vec::new()
                };
                s.install_sidebar_roster(&roster);
            });
            return;
        };
        self.install_sidebar_roster(&roster);
    }

    fn install_sidebar_roster(&mut self, roster: &[(String, String, bool)]) {
        if !self.sidebar.is_on() {
            return;
        }
        let focused = self.renderer.focused();
        let sections = Self::sidebar_sections(
            roster,
            self.status.agents(),
            self.status.workspaces(),
            focused,
        );
        self.renderer.set_sidebar_sections(sections);
    }

    /// Rebuild the panel's sections from the cached status (workspaces,
    /// active workspace, agent roster) with no round trip: the agent
    /// notifications carry their fact, so the roster patch alone must
    /// reach the panel (ARC-136). Before the first status refresh filled
    /// the workspace cache this only marks the status stale, so the
    /// throttled refresh draws the panel whole.
    pub(super) fn rebuild_agent_section(&mut self) {
        if !self.sidebar.is_on() {
            return;
        }
        if self.status.workspaces().is_empty() {
            self.status_dirty = true;
            return;
        }
        let active = self.status.active_workspace();
        let roster: Vec<(String, String, bool)> = self
            .status
            .workspaces()
            .iter()
            .map(|(id, name)| (id.clone(), name.clone(), Some(id.as_str()) == active))
            .collect();
        let focused = self.renderer.focused();
        let sections = Self::sidebar_sections(
            &roster,
            self.status.agents(),
            self.status.workspaces(),
            focused,
        );
        self.renderer.set_sidebar_sections(sections);
    }

    /// The panel's sections: the workspace rows (`roster` is `(id, name,
    /// active)`), then the agent roster — herdr's grouped panel. Each
    /// agent row labels with its workspace name (`{ws} · {agent}
    /// ({state})`; the bare `{agent} ({state})` when the daemon pre-`ws=`
    /// or the workspace is unknown) and activates when its pane is the
    /// view's focus. Grouped by workspace id in id order (unknown
    /// workspaces last), panes ascending within a group. `None` when
    /// both are empty.
    pub(super) fn sidebar_sections(
        roster: &[(String, String, bool)],
        agents: &[status::RosterEntry],
        ws_names: &[(String, String)],
        focused: Option<u32>,
    ) -> Option<Vec<SidebarSection>> {
        let mut sections: Vec<SidebarSection> = Vec::new();
        if !roster.is_empty() {
            let rows = roster
                .iter()
                .map(|(id, name, active)| SidebarRow {
                    id: SidebarRowId::Workspace(id.clone()),
                    label: name.clone(),
                    active: *active,
                })
                .collect();
            sections.push(SidebarSection { rows });
        }
        if !agents.is_empty() {
            // Group by workspace ID in the daemon's id order (`ws_names` is
            // the workspace cache, id order), panes ascending within a
            // group; rows whose workspace is unknown (pre-`ws=` daemon, or
            // a row a patch inserted before the refresh filled it) trail as
            // one group.
            let group = |ws: Option<&str>| {
                ws.and_then(|ws| ws_names.iter().position(|(id, _)| id == ws))
                    .unwrap_or(ws_names.len())
            };
            let mut keyed: Vec<_> = agents
                .iter()
                .map(|entry| {
                    let ws = entry.workspace.as_deref();
                    let ws_name = ws_names
                        .iter()
                        .find(|(id, _)| Some(id.as_str()) == ws)
                        .map(|(_, name)| name.as_str());
                    let (agent, state) = (&entry.agent, &entry.state);
                    let label = match ws_name {
                        Some(name) => format!("{name} · {agent} ({state})"),
                        None => format!("{agent} ({state})"),
                    };
                    let row = SidebarRow {
                        id: SidebarRowId::Pane(entry.pane),
                        label,
                        active: focused == Some(entry.pane),
                    };
                    ((group(ws), entry.pane), row)
                })
                .collect();
            keyed.sort_by_key(|(key, _)| *key);
            let rows = keyed.into_iter().map(|(_, row)| row).collect();
            sections.push(SidebarSection { rows });
        }
        if sections.is_empty() {
            None
        } else {
            Some(sections)
        }
    }
}
