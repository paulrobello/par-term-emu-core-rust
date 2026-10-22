//! The render-mode status bar: the queried state, its one-line
//! composition, and the ratatui row painter.
//!
//! [`StatusState`] is what the client knows (sessions, the shown
//! session's windows, the workspace roster, the agent roster the side
//! panel draws, the focused pane's title);
//! [`StatusState::refresh`] re-queries it over the control connection —
//! one `client-snapshot` round trip when the daemon advertises it
//! (ENH-043), else the per-fact queries — on the events that do not carry
//! their fact (`%sessions-changed`, `%window-add`, …). Events that do
//! (`%agent-state-changed`, `%window-renamed`, …) patch it in place; an
//! agent patch changes the side panel's roster, not this row.
//! [`StatusRow`] paints the composed line into a one-row ratatui
//! `Buffer` and diffs it, so a status change flushes only the changed
//! cells of the bottom row.
//!
//! Layout, left to right: the tab strip (every workspace name, then the
//! shown session's windows, as herdr-style tabs — `|`-separated, the
//! active tab a solid shaded block, the rest dim), every session
//! (`$N:name`, the shown one bold), then the focused pane's title.
//! When the focused pane is scrolled client-side, a trailing
//! `[scroll +N]` cue reports the offset. Agents are not on this row:
//! render mode draws them in the side panel's agents section.

use super::conn::AttachConn;
use super::spell_key;
use crate::mux::ipc::{AgentRow, GitStatusRow, WindowRow, WorktreeRow};
use ratatui::buffer::{Buffer, Cell as RtCell};
use ratatui::layout::Rect as RtRect;
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};

/// One styled run of the composed status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    /// The literal text.
    pub text: String,
    /// Bold (the shown session, the scroll cue).
    pub bold: bool,
    /// Dim (an inactive tab).
    pub dim: bool,
    /// The herdr tab shading: a solid bright block, the strip's
    /// REVERSED base dropped (the active tab).
    pub shaded: bool,
}

/// Why a status refresh did not produce fresh state.
#[derive(Debug)]
pub(crate) enum StatusError {
    /// The window this view shows no longer belongs to any session (the
    /// session was killed, or the window closed): the card's end-the-view
    /// rule.
    SessionGone,
    /// A query failed (transport-level); the caller keeps the stale
    /// state and retries on the next dirty mark.
    Query,
    /// A query outlived [`STATUS_TIMEOUT`]: the refresh stopped there
    /// (later queries would each wait as long), the stale state stands,
    /// and the caller re-marks the status dirty for the next tick.
    TimedOut,
}

/// Reply rows one refresh fetched that other chrome reuses instead of
/// re-querying: the `list-workspaces` roster as `(id, name, active)`,
/// `None` when that query failed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RefreshRows {
    pub workspaces: Option<Vec<(String, String, bool)>>,
}

/// The bound on each status query. The refresh runs on the input pump,
/// so a wedged daemon must cost keystrokes at most this long, not the
/// connection's 10 s reply timeout.
pub(crate) const STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// One bounded status query: `Ok(Some(body))` for an ok reply,
/// `Ok(None)` for an `%error` reply or a non-timeout transport failure,
/// `Err(TimedOut)` when the reply did not arrive within the bound.
pub(crate) fn status_query(
    conn: &mut AttachConn,
    command: &str,
) -> Result<Option<Vec<String>>, StatusError> {
    match conn.send_checked_timeout(command, STATUS_TIMEOUT) {
        Ok(reply) if reply.ok => Ok(Some(reply.body)),
        Ok(_) => Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => Err(StatusError::TimedOut),
        Err(_) => Ok(None),
    }
}

/// One refresh's facts, filled row by row from either wire shape and
/// committed whole by [`StatusState::commit`] (ENH-053). `None` fields
/// keep the state's previous value — the legacy path's per-fact
/// keep-on-failure rule, applied at commit granularity.
#[derive(Debug, Default)]
struct StatusFacts {
    workspaces: Option<Vec<(String, String, bool)>>,
    sessions: Option<Vec<(String, String)>>,
    session_id: Option<String>,
    session_workspace: Option<String>,
    windows: Option<Vec<(String, String)>>,
    active_window: Option<String>,
    pane_title: Option<String>,
    pane_titles: Option<Vec<(u32, String)>>,
    agents: Option<Vec<RosterEntry>>,
    worktrees: Option<Vec<WorktreeRow>>,
}

impl StatusFacts {
    /// One window row's projection — the display name plus the active
    /// pick — shared by both wire shapes.
    fn push_window_row(
        windows: &mut Vec<(String, String)>,
        active: &mut Option<String>,
        row: WindowRow,
    ) {
        if row.active {
            *active = Some(row.id.clone());
        }
        let name = window_display_name(&row.id, &row.name);
        windows.push((row.id, name));
    }

    /// The single agent-row projection, shared by both wire shapes.
    fn agent_entry(row: AgentRow) -> Option<RosterEntry> {
        let workspace = row.workspace().map(str::to_string);
        let project = row.project();
        let message = row.reason();
        Some(RosterEntry {
            pane: pane_number(&row.pane)?,
            agent: row.agent,
            state: row.state,
            workspace,
            project,
            message,
        })
    }

    /// The legacy queries' row shapes as one `StatusFacts`, factored out
    /// so the equivalence test can drive it without a connection. `None`
    /// bodies are the failed queries that keep their previous values.
    fn legacy_from(
        sessions: &[String],
        workspaces: Option<&[String]>,
        owner: (String, Vec<String>),
        title: Option<&[String]>,
        agents: Option<&[String]>,
    ) -> Self {
        let mut facts = Self {
            sessions: Some(
                sessions
                    .iter()
                    .filter_map(|line| super::parse_session_line(line))
                    .collect(),
            ),
            ..Self::default()
        };
        if let Some(body) = workspaces {
            facts.workspaces = Some(
                body.iter()
                    .filter_map(|l| super::parse_workspace_line(l))
                    .collect(),
            );
        }
        let mut windows = Vec::new();
        let mut active_window = None;
        for line in &owner.1 {
            if let Some(row) = WindowRow::parse(line) {
                Self::push_window_row(&mut windows, &mut active_window, row);
            }
        }
        facts.session_workspace = sessions
            .iter()
            .filter_map(|line| crate::mux::ipc::SessionRow::parse(line))
            .find(|row| row.id == owner.0)
            .and_then(|row| row.workspace)
            .map(|(id, _)| id);
        facts.session_id = Some(owner.0);
        facts.active_window = active_window.or_else(|| windows.first().map(|(id, _)| id.clone()));
        facts.windows = Some(windows);
        if let Some(body) = title {
            facts.pane_title = Some(body.join(" "));
        }
        if let Some(body) = agents {
            facts.agents = Some(
                body.iter()
                    .filter_map(|line| AgentRow::parse(line))
                    .filter_map(Self::agent_entry)
                    .collect(),
            );
        }
        facts
    }
}

/// The queried status state behind the bottom row.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct StatusState {
    /// Every workspace as `(id, name)`, id order, plus the daemon's
    /// active one — the workspaces segment's source.
    workspaces: Vec<(String, String)>,
    active_workspace: Option<String>,
    /// Every session as `(id, name)`, in `list-sessions` order.
    sessions: Vec<(String, String)>,
    /// The session this view's window belongs to, `"$N"`.
    pub session_id: Option<String>,
    /// The workspace that session belongs to, `"+W"` — kept after the
    /// session is gone, so the landing can walk up to the workspace.
    pub session_workspace: Option<String>,
    /// The shown session's windows as `(id, name)`, window order.
    windows: Vec<(String, String)>,
    /// The shown session's active window id, `"@N"`.
    pub active_window: Option<String>,
    /// The focused pane's title.
    pane_title: String,
    /// Every pane title of the shown window as `(pane, title)`, from the
    /// last `client-snapshot` (empty on the legacy query path).
    pane_titles: Vec<(u32, String)>,
    /// The agent roster, pane-id order (the order `list-agents` sorts
    /// by), so a payload patch keeps it.
    agents: Vec<RosterEntry>,
    /// The worktree-backed workspaces (`list-worktrees`' rows, from the
    /// last `client-snapshot`; empty on the legacy query path) — the side
    /// panel groups linked worktree workspaces under their repo's.
    worktrees: Vec<WorktreeRow>,
    /// The workspaces' git badges, from the last `client-snapshot`'s
    /// `git` rows (empty on the legacy query path, for non-repos or
    /// detached-HEAD chains, and from a pre-`git`-row daemon — the
    /// sidebar hides a badge it has no row for).
    git_rows: Vec<GitStatusRow>,
}

/// One agent-roster row the client knows (ARC-140).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RosterEntry {
    pub pane: u32,
    pub agent: String,
    pub state: String,
    /// The owning workspace id (`+N`); `None` when unknown (a pre-`ws`
    /// daemon, or a row a patch inserted before a refresh placed it).
    pub workspace: Option<String>,
    /// The pane's project (the host probe's git work-tree basename);
    /// `None` until a probe sweep reached the pane. A state push carries
    /// no project, so a patch keeps the known one.
    pub project: Option<String>,
    /// The state's message (the blocked reason), decoded from the row's
    /// `reason=` extra or a push's `msg=` token. `None` when the daemon
    /// holds none — the roster renders no line, never a placeholder.
    pub message: Option<String>,
}

/// A window row's display name: an unnamed window shows its id, and a
/// spaced name collapses its whitespace.
fn window_display_name(id: &str, name: &str) -> String {
    if name.trim().is_empty() {
        id.to_string()
    } else {
        name.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

/// The numeric id of a `%N` pane id.
fn pane_number(pane: &str) -> Option<u32> {
    pane.strip_prefix('%')?.parse().ok()
}

impl StatusState {
    /// Re-query everything over `conn` for the view showing `window`
    /// with `focused` as the focused pane id. Every field that resolves
    /// updates; a failed individual query keeps its previous value.
    /// `Err(SessionGone)` when an ok reply proves no session owns
    /// `window` any more; `Err(TimedOut)` stops at the first query that
    /// outlives [`STATUS_TIMEOUT`].
    ///
    /// Against a daemon advertising `client-snapshot v1` (ENH-043) the
    /// whole read is one `client-snapshot -t <window>` round trip;
    /// otherwise the per-fact legacy round runs.
    pub(crate) fn refresh(
        &mut self,
        conn: &mut AttachConn,
        window: &str,
        focused: u32,
    ) -> Result<RefreshRows, StatusError> {
        if conn.has_command_feature("client-snapshot", "v1") {
            self.refresh_snapshot(conn, window, focused)
        } else {
            self.refresh_legacy(conn, window, focused)
        }
    }

    /// The single-round-trip refresh (ENH-043). A transport failure is
    /// `Query`; an `%error` reply (unknown window, or no owning session)
    /// is `SessionGone` with `session_id` left at its previous value, the
    /// pump's landing contract.
    fn refresh_snapshot(
        &mut self,
        conn: &mut AttachConn,
        window: &str,
        focused: u32,
    ) -> Result<RefreshRows, StatusError> {
        let reply = match conn
            .send_checked_timeout(&format!("client-snapshot -t {window}"), STATUS_TIMEOUT)
        {
            Ok(reply) => reply,
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
                return Err(StatusError::TimedOut)
            }
            Err(_) => return Err(StatusError::Query),
        };
        if !reply.ok {
            return Err(StatusError::SessionGone);
        }
        let workspaces = self.apply_snapshot(&reply.body)?;
        self.pane_title = self
            .pane_titles
            .iter()
            .find(|(pane, _)| *pane == focused)
            .map(|(_, title)| title.clone())
            .unwrap_or_default();
        Ok(RefreshRows {
            workspaces: Some(workspaces),
        })
    }

    /// Assign every `Some` field of `facts`, in the refresh's current
    /// order; `None` fields leave the state alone. Returns the workspace
    /// roster as `(id, name, active)` for `RefreshRows`.
    fn commit(&mut self, facts: StatusFacts) -> Option<Vec<(String, String, bool)>> {
        let StatusFacts {
            workspaces,
            sessions,
            session_id,
            session_workspace,
            windows,
            active_window,
            pane_title,
            pane_titles,
            agents,
            worktrees,
        } = facts;
        let triple = workspaces.clone();
        if let Some(parsed) = workspaces {
            self.workspaces = parsed
                .iter()
                .map(|(id, name, _)| (id.clone(), name.clone()))
                .collect();
            self.active_workspace = parsed
                .iter()
                .find(|(_, _, active)| *active)
                .map(|(id, _, _)| id.clone());
        }
        if let Some(sessions) = sessions {
            self.sessions = sessions;
        }
        if let Some(session_id) = session_id {
            self.session_id = Some(session_id);
        }
        if let Some(session_workspace) = session_workspace {
            self.session_workspace = Some(session_workspace);
        }
        if let Some(windows) = windows {
            self.active_window =
                active_window.or_else(|| windows.first().map(|(id, _)| id.clone()));
            self.windows = windows;
        } else if let Some(active) = active_window {
            self.active_window = Some(active);
        }
        if let Some(pane_titles) = pane_titles {
            self.pane_titles = pane_titles;
        }
        if let Some(pane_title) = pane_title {
            self.pane_title = pane_title;
        }
        if let Some(agents) = agents {
            self.agents = agents;
        }
        if let Some(worktrees) = worktrees {
            self.worktrees = worktrees;
        }
        triple
    }

    /// Apply one `client-snapshot` reply body. Rows are classified by
    /// their first word; unknown kinds are ignored (the additive rule).
    /// A missing or non-`1` `snapshot` header is `Query`; no `*` session
    /// row is `SessionGone`. Either error leaves every field untouched.
    /// Returns the workspace roster as `(id, name, active)` for the side
    /// panel. `pane_title` is the caller's to set from `pane_titles`.
    fn apply_snapshot(
        &mut self,
        body: &[String],
    ) -> Result<Vec<(String, String, bool)>, StatusError> {
        if body.first().map(|l| l.trim_end()) != Some("snapshot 1") {
            return Err(StatusError::Query);
        }
        let mut facts = StatusFacts::default();
        let mut owner = None;
        let mut owner_workspace = None;
        // The git rows commit straight onto the state (not through
        // `StatusFacts`' keep-previous rule): a snapshot without one is
        // the daemon's answer "no badges", never a failed query.
        let mut git_rows = Vec::new();
        for line in &body[1..] {
            let Some((kind, rest)) = line.split_once(' ') else {
                continue;
            };
            match kind {
                "workspace" => {
                    let mut parts = rest.splitn(3, ' ');
                    let (Some(id), Some(marker)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    let name = parts.next().unwrap_or_default();
                    facts.workspaces.get_or_insert_with(Vec::new).push((
                        id.to_string(),
                        name.to_string(),
                        marker == "*",
                    ));
                }
                "session" => {
                    let mut parts = rest.splitn(4, ' ');
                    let (Some(id), Some(marker), Some(ws)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    let name = parts.next().unwrap_or_default();
                    if marker == "*" {
                        owner = Some(id.to_string());
                        owner_workspace = Some(ws.to_string());
                    }
                    facts
                        .sessions
                        .get_or_insert_with(Vec::new)
                        .push((id.to_string(), name.to_string()));
                }
                "window" => {
                    let Some(row) = WindowRow::parse(rest) else {
                        continue;
                    };
                    StatusFacts::push_window_row(
                        facts.windows.get_or_insert_with(Vec::new),
                        &mut facts.active_window,
                        row,
                    );
                }
                "pane" => {
                    let (id, title) = rest.split_once(' ').unwrap_or((rest, ""));
                    if let Some(pane) = pane_number(id) {
                        facts
                            .pane_titles
                            .get_or_insert_with(Vec::new)
                            .push((pane, title.to_string()));
                    }
                }
                "agent" => {
                    if let Some(row) = AgentRow::parse(rest) {
                        if let Some(entry) = StatusFacts::agent_entry(row) {
                            facts.agents.get_or_insert_with(Vec::new).push(entry);
                        }
                    }
                }
                "worktree" => {
                    if let Some(row) = WorktreeRow::parse(rest) {
                        facts.worktrees.get_or_insert_with(Vec::new).push(row);
                    }
                }
                "git" => {
                    // The snapshot is the git rows' only emitter and the
                    // reply is authoritative: a body without one clears
                    // the badges (the repos left, or the key was
                    // disabled daemon-side). One row per workspace.
                    if let Some(row) = GitStatusRow::parse(rest) {
                        git_rows.push(row);
                    }
                }
                _ => {}
            }
        }
        // "Nothing half-applied": the owner check gates the commit.
        let Some(owner) = owner else {
            return Err(StatusError::SessionGone);
        };
        facts.session_id = Some(owner);
        facts.session_workspace = owner_workspace;
        // The snapshot is authoritative: empty lists are real facts, not
        // failed queries.
        facts.workspaces.get_or_insert_with(Vec::new);
        facts.sessions.get_or_insert_with(Vec::new);
        facts.windows.get_or_insert_with(Vec::new);
        facts.pane_titles.get_or_insert_with(Vec::new);
        facts.agents.get_or_insert_with(Vec::new);
        facts.worktrees.get_or_insert_with(Vec::new);
        self.git_rows = git_rows;
        Ok(self.commit(facts).unwrap_or_default())
    }

    /// The legacy per-fact refresh, for daemons without `client-snapshot`.
    /// One round per query (ARC-125): `list-sessions`, `list-workspaces`
    /// (its rows are returned for the side panel to reuse rather than
    /// re-query), one `list-windows -a` that both finds the owning
    /// session and lists its windows (a daemon without the `all` token
    /// gets the per-session scan, reusing the owner's reply),
    /// `pane-title` for the focused pane, and `list-agents`.
    ///
    /// The queries fill a [`StatusFacts`] and commit once at the end
    /// (ENH-053): a `TimedOut` at any step returns before the commit, so
    /// the stale state stands whole, matching the snapshot path and the
    /// `StatusError::TimedOut` doc.
    fn refresh_legacy(
        &mut self,
        conn: &mut AttachConn,
        window: &str,
        focused: u32,
    ) -> Result<RefreshRows, StatusError> {
        // Sessions first: their ids drive the window scan.
        let Some(session_rows) = status_query(conn, "list-sessions")? else {
            return Err(StatusError::Query);
        };

        // The workspace roster: the status line's leading segment. The
        // daemon lists in id order and marks the active one, so the reply
        // order IS the display order and the active pick is one find.
        let workspaces = status_query(conn, "list-workspaces")?;

        // Which session owns our window now (it can move or vanish), and
        // that session's window rows. The scan needs this round's parsed
        // session rows (see find_owner's doc).
        let parsed_sessions: Vec<(String, String)> = session_rows
            .iter()
            .filter_map(|line| super::parse_session_line(line))
            .collect();
        let owner = self.find_owner(conn, window, &parsed_sessions)?;

        // The focused pane's title.
        let title = status_query(conn, &format!("pane-title -t %{focused}"))?;

        // The agent roster: `%N <agent> <state> <source> [key=val …]`.
        let agents = status_query(conn, "list-agents")?;

        let facts = StatusFacts::legacy_from(
            &session_rows,
            workspaces.as_deref(),
            owner,
            title.as_deref(),
            agents.as_deref(),
        );
        Ok(RefreshRows {
            workspaces: self.commit(facts),
        })
    }

    /// The session owning `window` and its `list-windows -t` rows. One
    /// `list-windows -a` when the daemon advertises it; otherwise the
    /// per-session `-t` scan, whose matching reply doubles as the rows.
    /// `SessionGone` only when the replies came back and none listed the
    /// window; a failed `-a` is `Query`.
    /// The session owning `window` and its `list-windows -t` rows. One
    /// `list-windows -a` when the daemon advertises it; otherwise the
    /// per-session `-t` scan over `sessions` (the round's parsed
    /// `list-sessions` rows — commit-at-end means the state does not hold
    /// them yet, ENH-053), whose matching reply doubles as the rows.
    /// `SessionGone` only when the replies came back and none listed the
    fn find_owner(
        &self,
        conn: &mut AttachConn,
        window: &str,
        sessions: &[(String, String)],
    ) -> Result<(String, Vec<String>), StatusError> {
        if conn.has_command_feature("list-windows", "all") {
            let Some(body) = status_query(conn, "list-windows -a")? else {
                return Err(StatusError::Query);
            };
            // `-a` rows are `$S ` + a `-t` row.
            let tagged: Vec<(&str, &str)> = body
                .iter()
                .filter_map(|line| line.split_once(' '))
                .collect();
            let owner = tagged.iter().find_map(|(session, row)| {
                (WindowRow::parse(row)?.id == window).then(|| session.to_string())
            });
            let Some(owner) = owner else {
                return Err(StatusError::SessionGone);
            };
            let rows = tagged
                .iter()
                .filter(|(session, _)| *session == owner)
                .map(|(_, row)| row.to_string())
                .collect();
            return Ok((owner, rows));
        }
        for (id, _) in sessions {
            let Some(body) = status_query(conn, &format!("list-windows -t {id}"))? else {
                continue;
            };
            if body
                .iter()
                .any(|line| WindowRow::parse(line).is_some_and(|row| row.id == window))
            {
                return Ok((id.clone(), body));
            }
        }
        Err(StatusError::SessionGone)
    }

    /// The shown session's windows as `(id, name)`, window order — the
    /// tab strip's source of truth.
    pub(crate) fn windows(&self) -> &[(String, String)] {
        &self.windows
    }

    /// The agent roster in pane-id order — the side panel's agents
    /// section's source of truth.
    pub(crate) fn agents(&self) -> &[RosterEntry] {
        &self.agents
    }

    /// The workspace roster as `(id, name)`, id order — the side
    /// panel's and the workspace prompts'/menus' source of truth.
    pub(crate) fn workspaces(&self) -> &[(String, String)] {
        &self.workspaces
    }

    /// The worktree-backed workspaces the last snapshot reported, id
    /// order — the side panel's grouping source.
    pub(crate) fn worktrees(&self) -> &[WorktreeRow] {
        &self.worktrees
    }

    /// The workspaces' git badges, `None` per workspace when the daemon
    /// serves none for it (non-repo, detached HEAD, disabled) — the
    /// slice is empty when no workspace has a badge.
    pub(crate) fn git_rows(&self) -> &[GitStatusRow] {
        self.git_rows.as_slice()
    }

    /// The focused pane's user title (the rename-pane prompt's seed).
    pub(crate) fn pane_title(&self) -> &str {
        &self.pane_title
    }

    /// Replace the focused pane's title from a `%pane-title-changed`
    /// payload, without a query.
    pub(crate) fn set_pane_title(&mut self, title: &str) {
        self.pane_title = title.to_string();
    }

    /// The shown window's pane titles as `(pane, title)` from the last
    /// `client-snapshot`, layout order.
    pub(crate) fn pane_titles(&self) -> &[(u32, String)] {
        &self.pane_titles
    }

    /// The workspace the client's chrome highlights: the DISPLAYED
    /// session's own workspace, falling back to the daemon's active one
    /// (an older daemon whose session rows carry no workspace). The
    /// initial `attach -t` used to land on the displayed session's
    /// windows while the status segment and side panel shaded the
    /// daemon's active workspace — the TUI audit's F1.
    pub(crate) fn chrome_workspace(&self) -> Option<&str> {
        self.session_workspace
            .as_deref()
            .or(self.active_workspace.as_deref())
    }

    /// Patch one pane's roster entry from a notification payload:
    /// `Some(state)` upserts it (`%agent-state-changed`), `None` removes
    /// it (`%agent-released`). `ws` is the payload's owning-workspace
    /// token (`%agent-state-changed`'s `ws=`, ENH-061); empty from a
    /// pre-ENH-061 daemon. The roster stays in pane-id order.
    /// Returns `true` when the patch inserted a row the payload could not
    /// place in a workspace (no `ws=` token): the caller marks the status
    /// stale so the next throttled refresh fills it.
    pub(crate) fn patch_agent(
        &mut self,
        pane: &str,
        agent: &str,
        state: Option<&str>,
        ws: &str,
        message: &str,
    ) -> bool {
        let Some(pane) = pane_number(pane) else {
            return false;
        };
        match state {
            Some(state) => match self.agents.binary_search_by_key(&pane, |e| e.pane) {
                Ok(index) => {
                    let entry = &mut self.agents[index];
                    entry.agent = agent.to_string();
                    entry.state = state.to_string();
                    if !ws.is_empty() {
                        entry.workspace = Some(ws.to_string());
                    }
                    // An empty message means the daemon holds none (the
                    // push token only rides when set), so the patch
                    // mirrors the roster refresh: absent replaces.
                    entry.message = (!message.is_empty()).then(|| message.to_string());
                    false
                }
                Err(index) => {
                    self.agents.insert(
                        index,
                        RosterEntry {
                            pane,
                            agent: agent.to_string(),
                            state: state.to_string(),
                            workspace: (!ws.is_empty()).then(|| ws.to_string()),
                            project: None,
                            message: (!message.is_empty()).then(|| message.to_string()),
                        },
                    );
                    ws.is_empty()
                }
            },
            None => {
                self.agents.retain(|e| e.pane != pane);
                false
            }
        }
    }

    /// Give every roster entry `project` (test seam for the project
    /// poll).
    #[cfg(test)]
    pub(crate) fn set_projects_for_test(&mut self, project: &str) {
        for entry in &mut self.agents {
            entry.project = Some(project.to_string());
        }
    }

    /// Apply a `client-snapshot` body (test seam for the render session).
    #[cfg(test)]
    pub(crate) fn apply_snapshot_for_test(&mut self, body: &[String]) {
        self.apply_snapshot(body).expect("snapshot applies");
    }

    /// Patch a window's tab name from a `%window-renamed` payload. A
    /// window this view does not list is ignored.
    pub(crate) fn patch_window_name(&mut self, window: &str, name: &str) {
        if let Some(entry) = self.windows.iter_mut().find(|(id, _)| id == window) {
            entry.1 = window_display_name(window, name);
        }
    }

    /// Compose the status line as styled segments, truncated to `cols`
    /// display columns. `scroll` is the focused pane's client scroll
    /// offset when a scroll view is up. With the side panel open
    /// (`sidebar`), the strip's roster replaces the all-workspaces and
    /// sessions segments: the line leads with the ACTIVE workspace's
    /// label, then the shown session's tabs — herdr's top bar.
    pub(crate) fn compose(&self, cols: u16, scroll: Option<usize>, sidebar: bool) -> Vec<Segment> {
        let mut segments: Vec<Segment> = Vec::new();
        let push =
            |segments: &mut Vec<Segment>, text: String, bold: bool, dim: bool, shaded: bool| {
                let text = super::chrome_safe(&text).into_owned();
                if let Some(last) = segments.last_mut() {
                    // Merge into the previous run when the style matches so
                    // the line stays a handful of segments.
                    if last.bold == bold && last.dim == dim && last.shaded == shaded {
                        last.text.push_str(&text);
                        return;
                    }
                }
                segments.push(Segment {
                    text,
                    bold,
                    dim,
                    shaded,
                });
            };

        // The tab strip leads the line, herdr-style: every workspace and
        // window as a padded name tab — the active one a solid shaded
        // block, the rest dim, `|` between tabs (the "current emphasized"
        // rule the sessions segment still follows with bold). With the
        // side panel up the roster lives in the strip, so the line leads
        // with just the ACTIVE workspace's label before the tabs.
        if sidebar {
            // The shaded block means "the selected workspace" and must
            // agree with the side panel's highlighted row (the manual-
            // pass report: with the active window's tab also shaded, the
            // strip read as selecting a different workspace). The active
            // window's tab carries the bold current treatment instead.
            if let Some(name) = self
                .workspaces
                .iter()
                .find(|(id, _)| Some(id.as_str()) == self.chrome_workspace())
                .map(|(_, name)| name)
            {
                push(&mut segments, format!(" {name} "), true, false, true);
            }
        } else {
            for (index, (id, name)) in self.workspaces.iter().enumerate() {
                if index > 0 {
                    push(&mut segments, " | ".to_string(), false, false, false);
                }
                let active = self.chrome_workspace() == Some(id.as_str());
                push(&mut segments, format!(" {name} "), false, !active, active);
            }
            if !self.workspaces.is_empty() {
                push(&mut segments, " | ".to_string(), false, false, false);
            }
            for (index, (id, name)) in self.sessions.iter().enumerate() {
                if index > 0 {
                    push(&mut segments, " ".to_string(), false, false, false);
                }
                let current = self.session_id.as_deref() == Some(id.as_str());
                push(&mut segments, format!("{id}:{name}"), current, false, false);
            }
        }
        if !self.windows.is_empty() {
            push(&mut segments, " | ".to_string(), false, false, false);
            for (index, (id, name)) in self.windows.iter().enumerate() {
                if index > 0 {
                    push(&mut segments, " | ".to_string(), false, false, false);
                }
                let active = self.active_window.as_deref() == Some(id.as_str());
                if sidebar {
                    // The shaded block stays reserved for the workspace
                    // label in this mode; the active tab is the bold
                    // current treatment the sessions segment follows.
                    push(&mut segments, format!(" {name} "), active, !active, false);
                } else {
                    push(&mut segments, format!(" {name} "), false, !active, active);
                }
            }
        }
        if !self.pane_title.is_empty() {
            push(
                &mut segments,
                format!(" | {}", self.pane_title),
                false,
                false,
                false,
            );
        }
        if let Some(offset) = scroll.filter(|offset| *offset > 0) {
            push(
                &mut segments,
                format!(" | [scroll +{offset}]"),
                true,
                false,
                false,
            );
        }

        // Truncate to the row width.
        let mut used = 0usize;
        let width = cols as usize;
        segments.retain_mut(|segment| {
            if used >= width {
                return false;
            }
            let (taken, taken_width) =
                super::text_input::truncate_to_width(&segment.text, width - used);
            used += taken_width;
            segment.text = taken.to_string();
            !segment.text.is_empty()
        });
        segments
    }
}

/// The end-of-bar Help chip: the live help keybind, then the chip's
/// label — ` C-b? Help ` — the keybind spelled the way the keybinds
/// panel spells the same chord. The renderer reserves the chip's width
/// before composing the rest of the bar, so it survives every
/// truncation; with the bar hidden it is not drawn at all.
pub(crate) fn help_chip(prefix: u8, help: u8) -> Segment {
    Segment {
        text: format!(" {}{} Help ", spell_key(prefix), spell_key(help)),
        bold: true,
        dim: false,
        shaded: false,
    }
}

/// The bottom row's paint + diff pair: a one-row ratatui `Buffer` and
/// its previous frame, so a status change flushes only the changed
/// cells.
pub(crate) struct StatusRow {
    cols: u16,
    buffer: Buffer,
    prev: Buffer,
    /// The color theme (config `theme`); `None` = the built-in reversed
    /// row with the indexed bright-blue shading.
    pub(crate) palette: Option<crate::mux::theme::Palette>,
}

/// One status-row cell's style. Built-in: the REVERSED base, the shaded
/// block dark-on-bright-blue (indexed, so the host theme applies). A
/// theme: text on surface0, dim as overlay0, the shaded block panel_bg
/// on the accent.
fn row_style(
    palette: Option<crate::mux::theme::Palette>,
    shaded: bool,
    bold: bool,
    dim: bool,
) -> RtStyle {
    use crate::mux::theme::rt;
    let Some(p) = palette else {
        if shaded {
            return RtStyle::default()
                .fg(RtColor::Indexed(0))
                .bg(RtColor::Indexed(12))
                .add_modifier(RtModifier::BOLD);
        }
        let base = RtStyle::default().add_modifier(RtModifier::REVERSED);
        return if bold {
            base.add_modifier(RtModifier::BOLD)
        } else if dim {
            base.add_modifier(RtModifier::DIM)
        } else {
            base
        };
    };
    if shaded {
        return RtStyle::default()
            .fg(rt(p.panel_bg))
            .bg(rt(p.accent))
            .add_modifier(RtModifier::BOLD);
    }
    let base = RtStyle::default().bg(rt(p.surface0));
    if bold {
        base.fg(rt(p.text)).add_modifier(RtModifier::BOLD)
    } else if dim {
        base.fg(rt(p.overlay0))
    } else {
        base.fg(rt(p.text))
    }
}

impl StatusRow {
    /// A row `cols` wide, initially blank (the first diff paints all of
    /// it).
    pub(crate) fn new(cols: u16) -> Self {
        let area = RtRect::new(0, 0, cols, 1);
        Self {
            cols,
            buffer: Buffer::empty(area),
            prev: Buffer::empty(area),
            palette: None,
        }
    }

    /// The row's width.
    pub(crate) fn cols(&self) -> u16 {
        self.cols
    }

    /// Forget the previous frame: the next diff paints the whole row
    /// again (after a host repaint flood, a resize, or a full-screen
    /// clear the caller knows wiped the row).
    pub(crate) fn invalidate(&mut self) {
        self.prev = Buffer::empty(RtRect::new(0, 0, self.cols, 1));
    }

    /// Paint `segments` from the row's left (base reversed, per-segment
    /// bold), padding the remainder with spaces so the previous frame's
    /// tail erases. `chip`, when given, paints RIGHT-ALIGNED at the
    /// row's right edge (the Help chip): the content clips at the
    /// chip's left column, so the chip can never be pushed off the row.
    pub(crate) fn paint(&mut self, segments: &[Segment], chip: Option<&Segment>) {
        // The chip's columns: [cols - chip_width, cols). Content stops
        // before them.
        let chip_cols = chip.map(|chip| chip.text.chars().count() as u16);
        let content_limit = chip_cols.map_or(self.cols, |w| self.cols - w);
        for x in 0..self.cols {
            let cell = &mut self.buffer[(x, 0)];
            cell.reset();
            cell.set_style(row_style(self.palette, false, false, false));
        }
        let mut x = 0u16;
        'content: for segment in segments {
            // The herdr tab shading: the active tab drops the strip's
            // REVERSED base for a solid bright block (dark on bright
            // blue); inactive tabs dim, the rest the base.
            let style = row_style(
                self.palette,
                segment.shaded,
                segment.bold && !segment.dim,
                segment.dim,
            );
            // A wide (CJK) glyph spans two columns: the next glyph starts
            // past its right half, which keeps the base style.
            for ch in segment.text.chars() {
                let width = super::text_input::char_width(ch) as u16;
                if width == 0 {
                    continue;
                }
                if x + width > content_limit {
                    break 'content;
                }
                let cell = &mut self.buffer[(x, 0)];
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
                if width == 2 {
                    self.buffer[(x + 1, 0)].set_style(style);
                }
                x += width;
            }
        }
        if let Some(chip) = chip {
            let Some(chip_cols) = chip_cols else {
                return;
            };
            if chip_cols == 0 || chip_cols > self.cols {
                return;
            }
            let start = self.cols - chip_cols;
            let style = row_style(self.palette, chip.shaded, chip.bold, chip.dim);
            for (offset, ch) in chip.text.chars().enumerate() {
                let x = start + offset as u16;
                if x >= self.cols {
                    break;
                }
                let cell = &mut self.buffer[(x, 0)];
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
            }
        }
    }

    /// The changed cells against the previous paint, `(x, 0, cell)`;
    /// the painted row becomes the next baseline. Cell `y` is 0 — the
    /// caller offsets it to the host's bottom row.
    pub(crate) fn diff(&mut self) -> Vec<(u16, u16, RtCell)> {
        let diff = self
            .prev
            .diff(&self.buffer)
            .into_iter()
            .map(|(x, y, cell)| (x, y, cell.clone()))
            .collect::<Vec<_>>();
        self.prev = self.buffer.clone();
        diff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> StatusState {
        StatusState {
            workspaces: vec![
                ("+0".to_string(), "alpha".to_string()),
                ("+1".to_string(), "beta".to_string()),
            ],
            active_workspace: Some("+0".to_string()),
            sessions: vec![
                ("$0".to_string(), "work".to_string()),
                ("$1".to_string(), "play".to_string()),
            ],
            session_id: Some("$0".to_string()),
            session_workspace: Some("+0".to_string()),
            windows: vec![
                ("@0".to_string(), "main".to_string()),
                ("@1".to_string(), "vim".to_string()),
            ],
            active_window: Some("@0".to_string()),
            pane_title: "~/src".to_string(),
            pane_titles: Vec::new(),
            agents: vec![entry(1, "claude", "working"), {
                let mut kimi = entry(2, "kimi", "blocked");
                kimi.message = Some("wait".to_string());
                kimi
            }],
            worktrees: Vec::new(),
            git_rows: Vec::new(),
        }
    }

    /// A roster entry with no known workspace.
    fn entry(pane: u32, agent: &str, state: &str) -> RosterEntry {
        RosterEntry {
            pane,
            agent: agent.to_string(),
            state: state.to_string(),
            workspace: None,
            project: None,
            message: None,
        }
    }

    /// The blocked reason decodes from the row's `reason=` base64 extra;
    /// a row without one keeps the message empty.
    #[test]
    fn agent_reason_decodes_into_the_entry_message() {
        let row = crate::mux::ipc::AgentRow::parse("%2 kimi blocked hook reason=d2FpdA==")
            .expect("parses");
        let entry = super::StatusFacts::agent_entry(row).expect("entry");
        assert_eq!(entry.message.as_deref(), Some("wait"));
        let bare = crate::mux::ipc::AgentRow::parse("%2 kimi blocked hook").expect("parses");
        assert_eq!(
            super::StatusFacts::agent_entry(bare)
                .expect("entry")
                .message,
            None
        );
    }

    fn lines(body: &[&str]) -> Vec<String> {
        body.iter().map(|l| l.to_string()).collect()
    }

    /// The `client-snapshot` body describing [`state`].
    fn snapshot_body() -> Vec<String> {
        lines(&[
            "snapshot 1",
            "workspace +0 * alpha",
            "workspace +1 - beta",
            "session $0 * +0 work",
            "session $1 - +1 play",
            "window @0 * main",
            "window @1 - vim",
            "pane %1 ~/src",
            "pane %2",
            "agent %1 claude working hook",
            "agent %2 kimi blocked hook reason=d2FpdA==",
        ])
    }

    /// A full snapshot body yields the same state the legacy fixture
    /// describes, plus every pane's title.
    #[test]
    fn apply_snapshot_fills_the_same_state_as_the_legacy_queries() {
        let mut got = StatusState::default();
        let roster = got.apply_snapshot(&snapshot_body()).expect("applies");
        got.pane_title = "~/src".to_string();
        let mut want = state();
        want.pane_titles = vec![(1, "~/src".to_string()), (2, String::new())];
        assert_eq!(got, want);
        assert_eq!(
            roster,
            vec![
                ("+0".to_string(), "alpha".to_string(), true),
                ("+1".to_string(), "beta".to_string(), false),
            ]
        );
    }

    /// The additive rule: an unknown row kind is ignored.
    #[test]
    fn apply_snapshot_ignores_an_unknown_row_kind() {
        let mut body = snapshot_body();
        body.insert(3, "future x y".to_string());
        body.push("future".to_string());
        let mut got = StatusState::default();
        got.apply_snapshot(&body).expect("applies");
        let mut plain = StatusState::default();
        plain.apply_snapshot(&snapshot_body()).unwrap();
        assert_eq!(got, plain);
    }

    /// The snapshot's `git` rows commit the badges; a snapshot without
    /// one clears them (the reply is authoritative).
    #[test]
    fn apply_snapshot_commits_the_git_row_and_clears_without_one() {
        let mut body = snapshot_body();
        body.push("git +0 main 2 1".to_string());
        body.push("git +8 dev 0 3".to_string());
        let mut got = StatusState::default();
        got.apply_snapshot(&body).expect("applies");
        assert_eq!(got.git_rows().len(), 2, "one row per workspace");
        assert_eq!(
            got.git_rows().iter().find(|r| r.id == "+0").map(|r| (
                r.branch.as_str(),
                r.ahead,
                r.behind
            )),
            Some(("main", 2, 1))
        );
        got.apply_snapshot(&snapshot_body()).expect("applies");
        assert!(got.git_rows().is_empty(), "no rows clears the badges");
    }

    /// ENH-053: the legacy row shapes for `snapshot_body()`'s contents
    /// commit the same state the snapshot path does — `pane_titles`
    /// excluded, the legacy path has no such query.
    #[test]
    fn legacy_and_snapshot_commit_identical_state() {
        let mut from_snapshot = StatusState::default();
        from_snapshot
            .apply_snapshot(&snapshot_body())
            .expect("applies");

        let body = snapshot_body();
        // The legacy `list-workspaces` row shape: `+N: name [active]`.
        let workspaces: Vec<String> = body
            .iter()
            .filter(|l| l.starts_with("workspace "))
            .map(|l| {
                let rest = &l["workspace ".len()..];
                let (id, rest) = rest.split_once(' ').expect("fixture shape");
                let (marker, name) = rest.split_once(' ').expect("fixture shape");
                format!(
                    "{}: {}{}",
                    id,
                    name,
                    if marker == "*" { " active" } else { "" }
                )
            })
            .collect();
        let sessions: Vec<String> = body
            .iter()
            .filter(|l| l.starts_with("session "))
            .map(|l| {
                // The legacy `list-sessions` row shape: `+W: wname: $N: name`.
                let mut parts = l["session ".len()..].splitn(4, ' ');
                let id = parts.next();
                let _marker = parts.next();
                let ws_id = parts.next();
                let name = parts.next();
                let (Some(id), Some(ws_id), Some(name)) = (id, ws_id, name) else {
                    unreachable!("fixture shape");
                };
                let ws_name = workspaces
                    .iter()
                    .find(|row| row.starts_with(&format!("{ws_id}: ")))
                    .map(|row| {
                        row[format!("{ws_id}: ").len()..]
                            .split(" active")
                            .next()
                            .unwrap_or_default()
                            .to_string()
                    })
                    .unwrap_or_default();
                format!("{ws_id}: {ws_name}: {id}: {name}")
            })
            .collect();
        let window_rows: Vec<String> = body
            .iter()
            .filter(|l| l.starts_with("window "))
            .map(|l| l["window ".len()..].to_string())
            .collect();
        let agent_rows: Vec<String> = body
            .iter()
            .filter(|l| l.starts_with("agent "))
            .map(|l| l["agent ".len()..].to_string())
            .collect();
        let facts = StatusFacts::legacy_from(
            &sessions,
            Some(&workspaces),
            ("$0".to_string(), window_rows),
            Some(&["~/src".to_string()]),
            Some(&agent_rows),
        );
        let mut from_legacy = StatusState::default();
        from_legacy.commit(facts);

        // The legacy path has no pane-title-per-pane query, and the
        // snapshot caller derives pane_title from pane_titles itself
        // (refresh_snapshot) — mirror both here.
        from_snapshot.pane_titles.clear();
        from_snapshot.pane_title = "~/src".to_string();
        assert_eq!(from_legacy, from_snapshot);
    }

    /// No `*` session row means no session owns the window: SessionGone,
    /// and `session_id` keeps its previous value (the landing contract).
    #[test]
    fn apply_snapshot_without_an_owner_is_session_gone() {
        let body: Vec<String> = snapshot_body()
            .into_iter()
            .map(|l| l.replace("session $0 * ", "session $0 - "))
            .collect();
        let mut got = state();
        let before = got.clone();
        assert!(matches!(
            got.apply_snapshot(&body),
            Err(StatusError::SessionGone)
        ));
        assert_eq!(got.session_id.as_deref(), Some("$0"));
        assert_eq!(got, before, "nothing half-applied");
    }

    /// A missing or unknown-version header is a Query failure.
    #[test]
    fn apply_snapshot_with_a_bad_header_is_query() {
        let mut got = state();
        let mut body = snapshot_body();
        body[0] = "snapshot 2".to_string();
        assert!(matches!(got.apply_snapshot(&body), Err(StatusError::Query)));
        body.remove(0);
        assert!(matches!(got.apply_snapshot(&body), Err(StatusError::Query)));
        assert!(matches!(got.apply_snapshot(&[]), Err(StatusError::Query)));
        assert_eq!(got, state());
    }

    /// Payload patches: an agent upsert keeps pane-id order, a release
    /// removes the chip, and a window rename updates its tab.
    #[test]
    fn payload_patches_update_agents_and_window_names() {
        let mut got = state();
        assert!(
            !got.patch_agent("%2", "kimi", Some("working"), "", ""),
            "update"
        );
        assert!(
            !got.patch_agent("%0", "pi", Some("blocked"), "+1", ""),
            "an insert carrying its ws needs no refresh (ENH-061)"
        );
        assert_eq!(
            got.agents,
            vec![
                RosterEntry {
                    pane: 0,
                    agent: "pi".to_string(),
                    state: "blocked".to_string(),
                    workspace: Some("+1".to_string()),
                    project: None,
                    message: None,
                },
                entry(1, "claude", "working"),
                entry(2, "kimi", "working"),
            ]
        );
        assert!(!got.patch_agent("%1", "claude", None, "", ""), "release");
        assert!(got.agents.iter().all(|e| e.pane != 1));
        got.patch_window_name("@1", "  edit  me ");
        assert_eq!(got.windows[1], ("@1".to_string(), "edit me".to_string()));
        got.patch_window_name("@9", "ignored");
        assert_eq!(got.windows.len(), 2);
    }

    fn joined(segments: &[Segment]) -> String {
        segments.iter().map(|s| s.text.as_str()).collect::<String>()
    }

    /// The workspaces segment leads the line: every workspace's name as
    /// a padded tab in id order, then the sessions segment.
    #[test]
    fn compose_carries_the_workspaces_segment_in_id_order() {
        let text = joined(&state().compose(120, None, false));
        assert!(
            text.starts_with(" alpha  |  beta  | "),
            "workspaces lead the line: {text}"
        );
        assert!(text.contains("| $0:work"), "order: {text}");
    }

    /// The active workspace's tab is the shaded block; the others dim.
    #[test]
    fn compose_shades_the_active_workspace_only() {
        let segments = state().compose(120, None, false);
        let shaded = joined(
            &segments
                .iter()
                .filter(|s| s.shaded)
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert!(
            shaded.contains(" alpha "),
            "active workspace shaded: {shaded}"
        );
        assert!(!shaded.contains(" beta "), "other not shaded: {shaded}");
        let dim = joined(
            &segments
                .iter()
                .filter(|s| s.dim)
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert!(dim.contains(" beta "), "inactive tab dim: {dim}");
    }

    /// The chrome highlights the DISPLAYED session's workspace, not the
    /// daemon's active one (the TUI audit's F1: the initial `attach -t`
    /// lands on the session's windows while the shading used to follow
    /// the daemon-wide pointer). With no session workspace the daemon's
    /// active one still shades.
    #[test]
    fn compose_shades_the_displayed_sessions_workspace() {
        let mut got = state();
        got.session_workspace = Some("+1".to_string());
        let segments = got.compose(120, None, false);
        let shaded = joined(
            &segments
                .iter()
                .filter(|s| s.shaded)
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert!(
            shaded.contains(" beta "),
            "the displayed session's workspace shades: {shaded}"
        );
        assert!(
            !shaded.contains(" alpha "),
            "the daemon-active workspace does not: {shaded}"
        );
        let mut fallback = state();
        fallback.session_workspace = None;
        let segments = fallback.compose(120, None, false);
        let shaded = joined(
            &segments
                .iter()
                .filter(|s| s.shaded)
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert!(
            shaded.contains(" alpha "),
            "no session workspace falls back to the daemon's: {shaded}"
        );
    }

    /// The composed line carries every fact the card lists: sessions,
    /// the shown session's windows (active `*`-marked), and the pane
    /// title — and NOT the agent roster, which lives in the side panel.
    #[test]
    fn compose_carries_sessions_windows_title_and_chips() {
        let segments = state().compose(120, None, false);
        let text = joined(&segments);
        assert!(text.contains("$0:work"), "sessions: {text}");
        assert!(text.contains("$1:play"), "sessions: {text}");
        assert!(text.contains(" main "), "windows, active tab: {text}");
        assert!(text.contains(" vim "), "windows: {text}");
        assert!(text.contains("~/src"), "pane title: {text}");
        assert!(!text.contains("claude:working"), "agent chips: {text}");
        assert!(!text.contains("kimi:blocked"), "agent chips: {text}");
    }

    /// The shown session's segment is bold; the active window's tab is
    /// shaded, an inactive tab dim. A non-shown session is neither.
    #[test]
    fn compose_bolds_the_current_session_and_shades_the_active_window() {
        let segments = state().compose(120, None, false);
        let bold_text = joined(
            &segments
                .iter()
                .filter(|s| s.bold)
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert!(
            bold_text.contains("$0:work"),
            "current session bold: {bold_text}"
        );
        assert!(
            !bold_text.contains("$1:play"),
            "other session not bold: {bold_text}"
        );
        let shaded = joined(
            &segments
                .iter()
                .filter(|s| s.shaded)
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert!(shaded.contains(" main "), "active window shaded: {shaded}");
        assert!(!shaded.contains(" vim "), "inactive not shaded: {shaded}");
        let dim = joined(
            &segments
                .iter()
                .filter(|s| s.dim)
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert!(dim.contains(" vim "), "inactive window dim: {dim}");
    }

    /// With the side panel up, the line leads with the ACTIVE
    /// workspace's label and drops the all-workspaces/sessions rosters
    /// (they live in the strip).
    #[test]
    fn compose_sidebar_mode_leads_with_the_active_workspace() {
        let segments = state().compose(120, None, true);
        let text = joined(&segments);
        assert!(text.starts_with(" alpha "), "label leads: {text}");
        assert!(!text.contains("$1:play"), "sessions dropped: {text}");
        assert!(text.contains(" main "), "tabs remain: {text}");
        // The shaded block is the selected workspace — agreeing with the
        // side panel's highlighted row; the active window's tab takes the
        // bold current treatment instead (the manual-pass report: with
        // the tab shaded too, the strip read as selecting another
        // workspace).
        let shaded = |needle: &str| segments.iter().any(|s| s.shaded && s.text.contains(needle));
        let bolded = |needle: &str| segments.iter().any(|s| s.bold && s.text.contains(needle));
        assert!(shaded("alpha"), "the ws label is the shaded block: {text}");
        assert!(!shaded("main"), "the active tab is not shaded: {text}");
        assert!(bolded("main"), "the active tab is bold: {text}");
    }

    /// A scroll offset over 0 appends a bold `[scroll +N]` cue; offset 0
    /// appends none.
    #[test]
    fn compose_appends_the_scroll_cue_when_offset() {
        let text = joined(&state().compose(120, Some(7), false));
        assert!(text.contains("[scroll +7]"), "cue: {text}");
        let text = joined(&state().compose(120, Some(0), false));
        assert!(!text.contains("scroll"), "no cue at live: {text}");
    }

    /// The line truncates to the row width without panicking on a wide
    /// state.
    #[test]
    fn compose_truncates_to_the_row_width() {
        let mut wide = state();
        wide.pane_title = "x".repeat(200);
        let segments = wide.compose(40, None, false);
        assert_eq!(joined(&segments).chars().count(), 40);
    }

    /// The row painter diffs: the first paint flushes everything, an
    /// identical repaint flushes nothing, and a change flushes only the
    /// changed cells.
    #[test]
    fn status_row_diffs_only_changes() {
        let segments = state().compose(80, None, false);
        let mut row = StatusRow::new(80);
        row.paint(&segments, None);
        let first = row.diff();
        assert_eq!(first.len(), 80, "first paint is the whole row");
        assert_eq!(first[0].1, 0, "cell y is row-relative 0");
        row.paint(&segments, None);
        assert!(row.diff().is_empty(), "identical repaint diffs empty");

        // One pane-title change: only that segment's cells move. (Agent
        // state left the bar — the side panel renders it.)
        let mut next = state();
        next.pane_title = "~/src/other".to_string();
        row.paint(&next.compose(80, None, false), None);
        let diff = row.diff();
        assert!(!diff.is_empty());
        assert!(diff.len() < 80, "a title change must not repaint the row");
    }

    /// A theme drops the REVERSED base for the palette: text on
    /// surface0, the shaded block panel_bg on the accent.
    #[test]
    fn status_row_uses_the_theme_palette() {
        use crate::mux::theme::{palette, rt};
        let p = palette("tokyo-night").expect("preset");
        let mut row = StatusRow::new(20);
        row.palette = Some(p);
        let plain = Segment {
            text: "ab".into(),
            bold: false,
            dim: false,
            shaded: false,
        };
        let shaded = Segment {
            text: "cd".into(),
            bold: false,
            dim: false,
            shaded: true,
        };
        row.paint(&[plain, shaded], None);
        let cell = &row.buffer[(0, 0)];
        assert_eq!((cell.fg, cell.bg), (rt(p.text), rt(p.surface0)));
        assert!(!cell.modifier.contains(RtModifier::REVERSED));
        let cell = &row.buffer[(2, 0)];
        assert_eq!((cell.fg, cell.bg), (rt(p.panel_bg), rt(p.accent)));
        assert_eq!(row.buffer[(19, 0)].bg, rt(p.surface0), "padding");
    }

    /// The painter pads the row with reversed spaces, so a shorter line
    /// erases the previous frame's longer tail.
    #[test]
    fn status_row_pads_and_erases_the_tail() {
        let mut row = StatusRow::new(60);
        let mut long = state();
        long.pane_title = String::new();
        long.agents.clear();
        row.paint(&long.compose(60, None, false), None);
        row.diff();
        let mut short = long.clone();
        short.pane_title = String::new();
        short.windows.clear();
        row.paint(&short.compose(60, None, false), None);
        let diff = row.diff();
        assert!(
            diff.iter().any(|(_, _, cell)| cell.symbol() == " "),
            "the tail erases to padded spaces: {diff:?}"
        );
    }

    /// The end-of-bar Help chip: the live help keybind, then the chip's
    /// label — ` C-b? Help ` — the keybind spelled the way the keybinds
    /// panel spells it. Bold. The renderer reserves its width before
    /// composing the rest of the bar, so it survives every truncation.
    #[test]
    fn help_chip_spells_the_live_keybind() {
        let chip = super::help_chip(0x02, b'?');
        assert_eq!(chip.text, " C-b? Help ");
        assert!(chip.bold);
        let remapped = super::help_chip(0x02, b'h');
        assert_eq!(remapped.text, " C-bh Help ");
    }

    /// The chip paints RIGHT-ALIGNED at the row's right edge and the
    /// content clips at the chip's left column, so a long line can never
    /// push the chip off the row (the reservation's paint half).
    #[test]
    fn paint_right_aligns_the_chip_and_clips_content_at_it() {
        let mut row = StatusRow::new(40);
        let long = Segment {
            text: "x".repeat(60),
            bold: false,
            dim: false,
            shaded: false,
        };
        let chip = super::help_chip(0x02, b'?'); // " C-b? Help " = 11 cols
        row.paint(&[long], Some(&chip));
        row.diff();
        // The chip's 11 chars end the row: cols 29..40.
        for (offset, ch) in chip.text.chars().enumerate() {
            let cell = &row.buffer[(29 + offset as u16, 0)];
            assert_eq!(cell.symbol(), ch.to_string(), "chip col {}", 29 + offset);
        }
        // The content clipped at the chip's left column (40 - 11 = 29).
        for x in [0u16, 15, 28] {
            assert_eq!(row.buffer[(x, 0)].symbol(), "x", "content col {x}");
        }
        // ...and an identical repaint diffs nothing (the diff baseline
        // includes the chip).
        row.paint(
            &[Segment {
                text: "y".repeat(60),
                bold: false,
                dim: false,
                shaded: false,
            }],
            Some(&chip),
        );
        let diff = row.diff();
        assert!(
            diff.iter().all(|(x, _, _)| *x < 29),
            "only the clipped content cells repaint: {diff:?}"
        );
    }
}
