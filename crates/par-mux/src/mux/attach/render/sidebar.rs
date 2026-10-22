//! The workspace side panel: its strip geometry, painting, row hit-test,
//! and the toggle/refresh the session drives.

use super::{PaneRenderer, WindowSession};
use crate::mux::attach::panels::{compose_sidebar, compose_sidebar_rail};
use crate::mux::attach::status;
use crate::mux::attach::targets::parse_workspace_line;
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};

use crate::mux::attach::panels::{
    agent_entry_lines, AgentFacts, AgentStatus, SidebarLine, SidebarLineKind, SidebarRow,
    SidebarRowId, SidebarSection, STATE_ICON,
};

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

    /// Set the area split (the workspace area's share of the panel's
    /// rows). Repaints.
    pub(crate) fn set_sidebar_split(&mut self, split: f32) {
        if self.sidebar_split != split {
            self.sidebar_split = split;
            self.dirty = true;
        }
    }

    /// The area split the panel composes with.
    pub(crate) fn sidebar_split(&self) -> f32 {
        self.sidebar_split
    }

    /// Show the panel collapsed to the status rail, or expanded.
    /// Repaints.
    pub(crate) fn set_sidebar_collapsed(&mut self, collapsed: bool) {
        if self.sidebar_collapsed != collapsed {
            self.sidebar_collapsed = collapsed;
            self.dirty = true;
        }
    }

    /// The panel's composed lines at its current width, height, split,
    /// and collapse state — the one source painting and every hit-test
    /// read.
    pub(super) fn sidebar_lines(&self) -> Vec<SidebarLine> {
        let w = self.geometry.sidebar_w;
        let Some(sections) = self.sidebar_sections.as_ref().filter(|_| w > 0) else {
            return Vec::new();
        };
        if self.sidebar_collapsed {
            compose_sidebar_rail(sections, self.height, self.sidebar_split)
        } else {
            compose_sidebar(sections, w, self.height, self.sidebar_split)
        }
    }

    /// The composed rule row, if the panel shows one — the split drag's
    /// anchor.
    pub(crate) fn sidebar_rule_y(&self) -> Option<u16> {
        self.sidebar_lines()
            .into_iter()
            .find(|line| line.kind == SidebarLineKind::Rule)
            .map(|line| line.y)
    }

    /// Whether the panel's HOST cell `(x, y)` is the rule row — the
    /// split drag's handle (the same host-to-frame shift the row
    /// hit-test applies).
    pub(crate) fn sidebar_rule_at(&self, x: u16, y: u16) -> bool {
        let w = self.geometry.sidebar_w;
        if w == 0 || x >= w {
            return false;
        }
        let Some(frame_y) = self.geometry.host_to_frame(x, y).map(|(_, y)| y) else {
            return false;
        };
        self.sidebar_rule_y() == Some(frame_y)
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
        self.sidebar_lines()
            .into_iter()
            .find(|line| line.y == frame_y && line.id.is_some() && x >= line.x && x < line.x_end)
            .and_then(|line| line.id)
    }

    /// Paint the workspace side panel over the strip columns: theme-bg
    /// fill, a dim divider on the strip's right edge, then the composed
    /// lines — each area's header dim + bold (herdr's ` spaces` lead
    /// row), the rule and rows dim. The active workspace/agent entry is
    /// herdr's subtle band: every line of it on [`active_row_bg`], its
    /// first line in the full foreground + bold, later lines still dim,
    /// status glyphs keeping their color. The collapsed rail has no
    /// divider column (the pane area's own edge bounds it): its index
    /// digits paint dim and each status glyph in its state's color.
    pub(super) fn paint_sidebar(&mut self) {
        let w = self.geometry.sidebar_w;
        if w == 0 {
            return;
        }
        let dim = RtStyle::default().add_modifier(RtModifier::DIM);
        let rail = self.sidebar_collapsed;
        // The expanded panel's text stops at its divider column.
        let content_w = if rail { w } else { w - 1 };
        for y in 0..self.height {
            for x in 0..w {
                let cell = &mut self.buffer[(x, y)];
                cell.reset();
                if let Some(bg) = self.bg {
                    cell.set_bg(bg);
                }
                if !rail && x + 1 == w {
                    cell.set_symbol(self.glyphs.vertical());
                    cell.set_style(dim);
                }
            }
        }
        let band = self.palette.map_or_else(
            || active_row_bg(self.bg),
            |p| crate::mux::theme::rt(p.active_row_bg),
        );
        for line in self.sidebar_lines() {
            // herdr's active treatment: the active entry (the active
            // workspace, or the focused pane's agent) sits on a subtle
            // full-width band. The footer chips paint at their own
            // columns (line.x), body rows at col 0.
            if line.active {
                for x in 0..content_w {
                    let cell = &mut self.buffer[(x, line.y)];
                    cell.reset();
                    cell.set_bg(band);
                }
            }
            let style = match (line.active, line.kind) {
                (true, SidebarLineKind::Row) => {
                    RtStyle::default().bg(band).add_modifier(RtModifier::BOLD)
                }
                (_, SidebarLineKind::Badge) => {
                    // The git badge: bright blue (ANSI bright-blue, so it
                    // reads on dark and light hosts alike), band-aware so
                    // the displayed workspace's badge stays legible.
                    let base = RtStyle::default().fg(RtColor::Indexed(12));
                    if line.active {
                        base.bg(band)
                    } else {
                        base
                    }
                }
                (true, _) => dim.bg(band),
                (false, SidebarLineKind::Header) => dim.add_modifier(RtModifier::BOLD),
                (false, _) => dim,
            };
            // The status glyph paints in its state's color: the rail's
            // second column, an expanded workspace row's marker column,
            // and an expanded agent line's state icon.
            let status = line
                .id
                .as_ref()
                .filter(|id| !matches!(id, SidebarRowId::Collapse))
                .map(|id| self.sidebar_row_status(id));
            let glyph = status.map(|st| {
                let glyph = status_style(st, self.palette);
                if line.active {
                    glyph.bg(band)
                } else {
                    glyph
                }
            });
            let ws_line = matches!(line.id, Some(SidebarRowId::Workspace(_)));
            let agent_line = matches!(line.id, Some(SidebarRowId::Pane(_)));
            for (j, ch) in line.text.chars().enumerate() {
                let x = line.x + j as u16;
                if x >= content_w {
                    break;
                }
                let is_glyph = if rail {
                    j == 1
                } else {
                    (ws_line && j == 1) || (agent_line && ch == STATE_ICON)
                };
                let cell = &mut self.buffer[(x, line.y)];
                cell.set_symbol(&ch.to_string());
                // The glyph cell carries the status's own shape, so the
                // working half-circle shows on the rail, the workspace
                // markers, and the agent state icons alike.
                if let Some(st) = status.filter(|_| is_glyph) {
                    cell.set_symbol(st.glyph());
                }
                cell.set_style(match glyph {
                    Some(glyph) if is_glyph => glyph,
                    _ => style,
                });
            }
        }
    }

    /// The status a row id carries in the current sections.
    fn sidebar_row_status(&self, id: &SidebarRowId) -> AgentStatus {
        self.sidebar_sections
            .iter()
            .flatten()
            .flat_map(|section| section.rows.iter())
            .find(|row| &row.id == id)
            .map(|row| row.status)
            .unwrap_or_default()
    }
}

/// The active entry's band: the host background lifted a step toward
/// the foreground — herdr's `active_row_bg`, which on a black panel is
/// `#1c1c29`. A dark host lifts toward a cool tint, a light one darkens;
/// a failed background probe uses herdr's default (catppuccin's
/// `#1e1e2e`).
pub(super) fn active_row_bg(bg: Option<RtColor>) -> RtColor {
    let Some(RtColor::Rgb(r, g, b)) = bg else {
        return RtColor::Rgb(30, 30, 46);
    };
    let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
    if luma < 128 {
        RtColor::Rgb(
            r.saturating_add(28),
            g.saturating_add(28),
            b.saturating_add(41),
        )
    } else {
        RtColor::Rgb(
            r.saturating_sub(25),
            g.saturating_sub(22),
            b.saturating_sub(16),
        )
    }
}

/// A rail status glyph's style: herdr's palette mapped onto the host's
/// indexed colors (so the host theme applies) — working yellow, blocked
/// red, done teal (cyan), idle green, question magenta, no agent dim.
fn status_style(status: AgentStatus, palette: Option<crate::mux::theme::Palette>) -> RtStyle {
    use crate::mux::theme::rt;
    let base = RtStyle::default();
    if let Some(p) = palette {
        return match status {
            AgentStatus::Working => base.fg(rt(p.yellow)),
            AgentStatus::Done => base.fg(rt(p.teal)),
            AgentStatus::Blocked => base.fg(rt(p.red)),
            AgentStatus::Idle => base.fg(rt(p.green)),
            // No preset carries a magenta token, so question reads the
            // host's indexed magenta under themed palettes too.
            AgentStatus::Question => base.fg(RtColor::Indexed(5)),
            AgentStatus::None => base.fg(rt(p.overlay0)),
        };
    }
    match status {
        AgentStatus::Working => base.fg(RtColor::Indexed(3)),
        AgentStatus::Done => base.fg(RtColor::Indexed(6)),
        AgentStatus::Blocked => base.fg(RtColor::Indexed(1)),
        AgentStatus::Idle => base.fg(RtColor::Indexed(2)),
        AgentStatus::Question => base.fg(RtColor::Indexed(5)),
        AgentStatus::None => base.add_modifier(RtModifier::DIM),
    }
}

/// Group the workspace area's linked worktree workspaces under their
/// repo workspace's row (herdr's indented group children): each child
/// follows its parent in id order, labelled `├─ `/`└─ ` and with the
/// parent's `<name>/` prefix dropped, so the default `<repo>/<branch>`
/// name reads as the branch. `worktrees` is `list-worktrees`' facts
/// (the daemon's `parent` column), never re-derived here; a child whose
/// repo workspace is not open keeps its id-order place, unprefixed.
pub(super) fn group_worktree_rows(
    sections: &mut [SidebarSection],
    worktrees: &[crate::mux::ipc::WorktreeRow],
) {
    let Some(section) = sections.iter_mut().find(|s| s.title == "workspaces") else {
        return;
    };
    let parent_of = |id: &str| {
        worktrees
            .iter()
            .find(|row| row.linked && row.id == id)
            .and_then(|row| row.parent.clone())
    };
    let ids: Vec<String> = section
        .rows
        .iter()
        .filter_map(|row| match &row.id {
            SidebarRowId::Workspace(id) => Some(id.clone()),
            _ => None,
        })
        .collect();
    // A child groups only under a parent row the panel actually shows.
    let grouped = |id: &str| parent_of(id).filter(|parent| ids.contains(parent));
    if !ids.iter().any(|id| grouped(id).is_some()) {
        return;
    }
    let mut rows: Vec<Option<SidebarRow>> = std::mem::take(&mut section.rows)
        .into_iter()
        .map(Some)
        .collect();
    let mut out = Vec::with_capacity(rows.len());
    for i in 0..rows.len() {
        let Some(row) = rows[i].take() else {
            continue;
        };
        let SidebarRowId::Workspace(id) = &row.id else {
            out.push(row);
            continue;
        };
        if grouped(id).is_some() {
            // Its parent's row emits it, wherever the parent sorts.
            rows[i] = Some(row);
            continue;
        }
        let parent_name = row.label.clone();
        let parent_id = id.clone();
        out.push(row);
        let children: Vec<usize> = (0..rows.len())
            .filter(|&j| {
                rows[j].as_ref().is_some_and(|child| match &child.id {
                    SidebarRowId::Workspace(cid) => {
                        grouped(cid).as_deref() == Some(parent_id.as_str())
                    }
                    _ => false,
                })
            })
            .collect();
        let last = children.len().saturating_sub(1);
        for (n, j) in children.into_iter().enumerate() {
            let Some(mut child) = rows[j].take() else {
                continue;
            };
            let branch = child
                .label
                .strip_prefix(&format!("{parent_name}/"))
                .filter(|rest| !rest.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| child.label.clone());
            let glyph = if n == last { "└─" } else { "├─" };
            child.label = format!("{glyph} {branch}");
            out.push(child);
        }
    }
    out.extend(rows.into_iter().flatten());
    section.rows = out;
}

/// The workspaces' git badges: each snapshot `git` row stamps ` branch
/// ↑n ↓n` as its workspace row's continuation line; rows naming unknown
/// workspaces, and workspaces without a row (non-repo, detached HEAD,
/// the key off), leave their rows bare. Pure — the unit-test surface.
pub(super) fn apply_git_badges(
    sections: &mut [SidebarSection],
    git_rows: &[crate::mux::ipc::GitStatusRow],
) {
    let Some(section) = sections.iter_mut().find(|s| s.title == "workspaces") else {
        return;
    };
    for row in git_rows {
        let Some(target) = section
            .rows
            .iter_mut()
            .find(|r| matches!(&r.id, SidebarRowId::Workspace(id) if id == &row.id))
        else {
            continue;
        };
        let mut badge = format!(" {}", row.branch);
        if row.ahead > 0 {
            badge.push_str(&format!(" \u{2191}{}", row.ahead));
        }
        if row.behind > 0 {
            badge.push_str(&format!(" \u{2193}{}", row.behind));
        }
        target.continuation = vec![badge];
        target.badge = true;
    }
}

/// A sidebar drag in flight: the width handle on the strip's right
/// edge, or the split rule between the areas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SidebarDrag {
    /// The press's host column and the width then, what has been
    /// applied, and when — the width change re-reports the grid to
    /// the daemon, so it throttles like a pane divider drag.
    Width {
        anchor_x: u16,
        start_w: u16,
        applied: u16,
        last_applied: Option<std::time::Instant>,
    },
    /// The rule's composed row at the press; dragging moves the rule
    /// by the pointer's row delta, re-expressed as the split ratio.
    /// Local repaint, no daemon round trip.
    Split { anchor_y: u16, start_rule: u16 },
}

/// How often the roster re-queries while a panel agent's project is
/// unknown — a third of the host probe's 30 s sweep.
pub(super) const PROJECT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// The panel width bounds: the footer chips need 12 columns; 60 keeps
/// the panes usable on every host this targets.
pub(super) const SIDEBAR_WIDTH_MIN: u16 = 12;
pub(super) const SIDEBAR_WIDTH_MAX: u16 = 60;

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

    /// The panel's chevron: collapse it to the status rail, or expand
    /// the rail back. The strip width changes, so the grid re-reports
    /// (the toggle's parked refit); the state persists.
    pub(super) fn toggle_sidebar_collapse(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) {
        if !self.sidebar.is_on() {
            return;
        }
        let collapsed = !self.sidebar.is_collapsed();
        self.sidebar.set_collapsed(collapsed);
        self.renderer.set_sidebar_collapsed(collapsed);
        self.chrome_geometry_changed();
        self.renderer.set_geometry(self.geometry);
        self.pending.grid_refit = true;
        self.refresh_sidebar(conn);
        self.draw_tab_strip();
        self.persist_sidebar();
    }

    /// Write the panel's dragged/toggled state to the client prefs file
    /// (a real attach only — the path is unset in unit-test sessions).
    /// A failed write flashes; the live panel keeps its state.
    pub(super) fn persist_sidebar(&mut self) {
        let Some(path) = self.sidebar.prefs_path() else {
            return;
        };
        let prefs = crate::mux::attach::prefs::ClientPrefs {
            sidebar_width: Some(self.render_opts.sidebar_width),
            sidebar_split: Some(self.renderer.sidebar_split()),
            sidebar_collapsed: Some(self.sidebar.is_collapsed()),
        };
        if let Err(err) = crate::mux::attach::prefs::store(path, &prefs) {
            self.chrome.flash(format!("sidebar state not saved: {err}"));
        }
    }

    /// Whether the roster is due a re-query for a missing project: the
    /// panel is up, an agent's project is unknown, and the last poll is
    /// [`PROJECT_POLL_INTERVAL`] old (the poll restarts the clock). A
    /// project-less pane outside a repo still gets its directory's name,
    /// so the poll stops once the probe reached every agent.
    pub(super) fn project_poll_due(&mut self, now: std::time::Instant) -> bool {
        let missing = self.sidebar.is_on()
            && self
                .status
                .agents()
                .iter()
                .any(|entry| entry.project.is_none());
        if missing && now.duration_since(self.project_poll) >= PROJECT_POLL_INTERVAL {
            self.project_poll = now;
            return true;
        }
        false
    }

    /// Whether the git badge's snapshot re-query is due: the key is on,
    /// the panel is up, and the last poll is the configured
    /// `[client] git-poll-secs` (default 10) old. The status stream is
    /// event-driven, so without this poll a repo that changed while
    /// idle would keep a stale badge until the next event; the poll
    /// restarts the clock.
    pub(super) fn git_poll_due(&mut self, now: std::time::Instant) -> bool {
        let interval = std::time::Duration::from_secs(self.git_poll_secs.max(1));
        let due = self.git_status
            && self.sidebar.is_on()
            && now.duration_since(self.git_poll) >= interval;
        if due {
            self.git_poll = now;
        }
        due
    }

    /// Press on the strip's right edge: start a width drag.
    pub(super) fn start_sidebar_width_drag(&mut self, x: u16) {
        if !self.sidebar.is_on() || self.sidebar.is_collapsed() {
            return;
        }
        self.sidebar_drag = Some(SidebarDrag::Width {
            anchor_x: x,
            start_w: self.render_opts.sidebar_width,
            applied: self.render_opts.sidebar_width,
            last_applied: None,
        });
    }

    /// Press on the rule row: start a split drag anchored to the rule's
    /// composed row.
    pub(super) fn start_sidebar_split_drag(&mut self, y: u16) {
        let Some(start_rule) = self.renderer.sidebar_rule_y() else {
            return;
        };
        self.sidebar_drag = Some(SidebarDrag::Split {
            anchor_y: y,
            start_rule,
        });
    }

    /// One motion or release while a sidebar drag is in flight (host
    /// coordinates). The split follows the pointer directly — a local
    /// recompose; the width re-reports the reduced grid, so it applies
    /// at the pane drags' cadence with the remainder flushed on
    /// release.
    pub(super) fn sidebar_drag_event(&mut self, x: u16, y: u16, release: bool) {
        let Some(drag) = self.sidebar_drag else {
            return;
        };
        match drag {
            SidebarDrag::Split {
                anchor_y,
                start_rule,
            } => {
                // Each area keeps 3 rows (the compose's own clamp); the
                // ratio that rounds back to the dragged row is rule/height.
                let height = self.renderer.frame_height();
                if height >= 6 {
                    let rule = (i32::from(start_rule) + i32::from(y) - i32::from(anchor_y))
                        .clamp(3, i32::from(height) - 3);
                    self.renderer
                        .set_sidebar_split(rule as f32 / f32::from(height));
                }
                if release {
                    self.sidebar_drag = None;
                    self.persist_sidebar();
                }
            }
            SidebarDrag::Width {
                anchor_x,
                start_w,
                applied,
                last_applied,
            } => {
                let target = (i32::from(start_w) + i32::from(x) - i32::from(anchor_x))
                    .clamp(i32::from(SIDEBAR_WIDTH_MIN), i32::from(SIDEBAR_WIDTH_MAX))
                    as u16;
                let due = last_applied
                    .is_none_or(|at| at.elapsed() >= super::drag::DRAG_SEND_MIN_INTERVAL);
                if release || (due && target != applied) {
                    self.apply_sidebar_width(target);
                    self.sidebar_drag = Some(SidebarDrag::Width {
                        anchor_x,
                        start_w,
                        applied: target,
                        last_applied: Some(std::time::Instant::now()),
                    });
                }
                if release {
                    self.sidebar_drag = None;
                    self.persist_sidebar();
                }
            }
        }
    }

    /// Apply a dragged panel width: the render option, the geometry, and
    /// the parked grid re-report (`refresh-client -C`) the pump applies —
    /// the same plumbing a toggle runs.
    fn apply_sidebar_width(&mut self, w: u16) {
        if self.render_opts.sidebar_width == w {
            return;
        }
        self.render_opts.sidebar_width = w;
        self.chrome_geometry_changed();
        self.renderer.set_geometry(self.geometry);
        self.pending.grid_refit = true;
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
            let active = self.status.chrome_workspace().map(str::to_string);
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
        self.install_panel_sections(roster);
    }

    /// The one builder every panel refresh goes through: the sections
    /// from `roster` and the cached agent roster, with linked worktree
    /// workspaces grouped under their repo's ([`group_worktree_rows`]).
    pub(super) fn install_panel_sections(&mut self, roster: &[(String, String, bool)]) {
        let focused = self.renderer.focused();
        let mut sections = Self::sidebar_sections(
            roster,
            self.status.agents(),
            self.status.workspaces(),
            focused,
            &self.agent_rows,
        );
        if let Some(sections) = sections.as_mut() {
            group_worktree_rows(sections, self.status.worktrees());
            apply_git_badges(sections, self.status.git_rows());
        }
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
        let active = self.status.chrome_workspace();
        let roster: Vec<(String, String, bool)> = self
            .status
            .workspaces()
            .iter()
            .map(|(id, name)| (id.clone(), name.clone(), Some(id.as_str()) == active))
            .collect();
        self.install_panel_sections(&roster);
    }

    /// The panel's sections: the workspace rows (`roster` is `(id, name,
    /// active)`), then the agent roster — herdr's grouped panel. Each
    /// agent entry renders its `agent_rows` lines ([`agent_entry_lines`]:
    /// by default the state icon, workspace name, and project, then the
    /// agent) and activates when its pane is the view's focus. Grouped by
    /// workspace id in id order (unknown workspaces last), panes
    /// ascending within a group. `None` when both are empty.
    pub(super) fn sidebar_sections(
        roster: &[(String, String, bool)],
        agents: &[status::RosterEntry],
        ws_names: &[(String, String)],
        focused: Option<u32>,
        agent_rows: &[Vec<String>],
    ) -> Option<Vec<SidebarSection>> {
        let mut sections: Vec<SidebarSection> = Vec::new();
        if !roster.is_empty() {
            // A workspace's rail glyph is its agents' most urgent state.
            let workspace_status = |ws: &str| {
                agents
                    .iter()
                    .filter(|entry| entry.workspace.as_deref() == Some(ws))
                    .map(|entry| AgentStatus::from_state(&entry.state))
                    .max()
                    .unwrap_or_default()
            };
            let rows = roster
                .iter()
                .map(|(id, name, active)| SidebarRow {
                    id: SidebarRowId::Workspace(id.clone()),
                    label: name.clone(),
                    active: *active,
                    status: workspace_status(id),
                    continuation: Vec::new(),
                    badge: false,
                })
                .collect();
            sections.push(SidebarSection {
                title: "workspaces",
                rows,
            });
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
                    let mut lines = agent_entry_lines(
                        agent_rows,
                        &AgentFacts {
                            pane: entry.pane,
                            agent: &entry.agent,
                            state: &entry.state,
                            workspace: ws_name,
                            project: entry.project.as_deref(),
                            message: entry.message.as_deref(),
                        },
                    );
                    let label = lines.remove(0);
                    let row = SidebarRow {
                        id: SidebarRowId::Pane(entry.pane),
                        label,
                        active: focused == Some(entry.pane),
                        status: AgentStatus::from_state(&entry.state),
                        continuation: lines,
                        badge: false,
                    };
                    ((group(ws), entry.pane), row)
                })
                .collect();
            keyed.sort_by_key(|(key, _)| *key);
            let rows = keyed.into_iter().map(|(_, row)| row).collect();
            sections.push(SidebarSection {
                title: "agents",
                rows,
            });
        }
        if sections.is_empty() {
            None
        } else {
            Some(sections)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws_row(id: &str) -> SidebarRow {
        SidebarRow {
            id: SidebarRowId::Workspace(id.to_string()),
            label: "repo".to_string(),
            active: false,
            status: AgentStatus::None,
            continuation: Vec::new(),
            badge: false,
        }
    }

    /// A workspace row's status is its agents' most urgent state: a
    /// question (a dialog asking the user to choose) outranks a blocked
    /// peer.
    #[test]
    fn workspace_row_status_ranks_question_over_blocked() {
        let agents = vec![
            crate::mux::attach::status::RosterEntry {
                pane: 1,
                agent: "a".to_string(),
                state: "blocked".to_string(),
                workspace: Some("+0".to_string()),
                project: None,
                message: None,
            },
            crate::mux::attach::status::RosterEntry {
                pane: 2,
                agent: "b".to_string(),
                state: "question".to_string(),
                workspace: Some("+0".to_string()),
                project: None,
                message: None,
            },
        ];
        let sections = crate::mux::attach::render::WindowSession::sidebar_sections(
            &[("+0".to_string(), "main".to_string(), false)],
            &agents,
            &[("+0".to_string(), "main".to_string())],
            None,
            &[vec!["agent".to_string()]],
        )
        .expect("sections");
        assert_eq!(sections[0].rows[0].status, AgentStatus::Question);
    }

    /// Badges ride every named workspace's row, with the herdr arrows;
    /// a row naming an unknown workspace, and workspaces without a row,
    /// leave their rows bare.
    #[test]
    fn git_badges_land_on_their_workspace_rows() {
        let badge = |id: &str, ahead: u32, behind: u32| crate::mux::ipc::GitStatusRow {
            id: id.to_string(),
            branch: "main".to_string(),
            ahead,
            behind,
        };
        let mut sections = vec![SidebarSection {
            title: "workspaces",
            rows: vec![ws_row("+0"), ws_row("+1")],
        }];
        apply_git_badges(&mut sections, &[badge("+0", 2, 0), badge("+1", 0, 3)]);
        assert_eq!(sections[0].rows[0].continuation, vec![" main \u{2191}2"]);
        assert_eq!(sections[0].rows[1].continuation, vec![" main \u{2193}3"]);
        assert!(sections[0].rows[0].badge && sections[0].rows[1].badge);
        // A row naming an unknown workspace is skipped (the builder
        // rebuilds the sections fresh each refresh, so no stale badge can
        // persist across renders).
        apply_git_badges(&mut sections, &[badge("+9", 1, 1)]);
        assert_eq!(
            sections[0].rows[0].continuation,
            vec![" main \u{2191}2"],
            "a foreign workspace's row never lands"
        );
        apply_git_badges(&mut sections, &[]);
        assert_eq!(
            sections[0].rows[0].continuation,
            vec![" main \u{2191}2"],
            "the stamper never clears — the rebuild drops stale badges"
        );
    }
}
