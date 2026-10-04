//! The render-mode status bar: the queried state, its one-line
//! composition, and the ratatui row painter.
//!
//! [`StatusState`] is what the client knows (sessions, the shown
//! session's windows, the focused pane's title, the agent roster);
//! [`StatusState::refresh`] re-queries it over the control connection —
//! the throttled re-query the client contract asks for on
//! `%agent-state-changed` / `%agent-telemetry-changed` /
//! `%sessions-changed`. [`StatusRow`] paints the composed line into a
//! one-row ratatui `Buffer` and diffs it, so a status change flushes
//! only the changed cells of the bottom row.
//!
//! Layout, left to right, space-separated: every workspace (`name`, the
//! daemon's active one bold), every session (`$N:name`, the shown one
//! bold), the shown session's windows (`0:name`, the active one bold and
//! `*`-marked), the focused pane's title, and the
//! agent roster as `agent:state` chips. When the focused pane is
//! scrolled client-side, a trailing `[scroll +N]` cue reports the
//! offset.

use super::conn::AttachConn;
use ratatui::buffer::{Buffer, Cell as RtCell};
use ratatui::layout::Rect as RtRect;
use ratatui::style::{Modifier as RtModifier, Style as RtStyle};

/// One styled run of the composed status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    /// The literal text.
    pub text: String,
    /// Bold (the shown session, the active window).
    pub bold: bool,
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
    /// The shown session's windows as `(id, name)`, window order.
    windows: Vec<(String, String)>,
    /// The shown session's active window id, `"@N"`.
    pub active_window: Option<String>,
    /// The focused pane's title.
    pane_title: String,
    /// The agent roster as `(agent, state)` chips, roster order.
    agents: Vec<(String, String)>,
}

impl StatusState {
    /// Re-query everything over `conn` for the view showing `window`
    /// with `focused` as the focused pane id. Every field that resolves
    /// updates; a failed individual query keeps its previous value.
    /// `Err(SessionGone)` when the reply set proves no session owns
    /// `window` any more.
    pub(crate) fn refresh(
        &mut self,
        conn: &mut AttachConn,
        window: &str,
        focused: u32,
    ) -> Result<(), StatusError> {
        // Sessions first: their ids drive the window scan.
        let session_rows = match conn.send_checked("list-sessions") {
            Ok(reply) if reply.ok => reply.body,
            Ok(_) | Err(_) => {
                return Err(StatusError::Query);
            }
        };

        // The workspace roster: the status line's leading segment. The
        // daemon lists in id order and marks the active one, so the reply
        // order IS the display order and the active pick is one find.
        if let Ok(reply) = conn.send_checked("list-workspaces") {
            if reply.ok {
                let rows: Vec<(String, String, bool)> = reply
                    .body
                    .iter()
                    .filter_map(|l| super::parse_workspace_line(l))
                    .collect();
                self.workspaces = rows
                    .iter()
                    .map(|(id, name, _)| (id.clone(), name.clone()))
                    .collect();
                self.active_workspace = rows
                    .iter()
                    .find(|(_, _, active)| *active)
                    .map(|(id, _, _)| id.clone());
            }
        }
        self.sessions = session_rows
            .iter()
            .filter_map(|line| {
                // Workspace-aware shape `+W: wname: $N: name` — the shared
                // parser extracts the session id (the last `$N:` marker).
                super::parse_session_line(line)
            })
            .collect();

        // Which session owns our window now? (It can move or vanish.)
        let mut owner = None;
        for (id, _) in &self.sessions {
            let Ok(reply) = conn.send_checked(&format!("list-windows -t {id}")) else {
                continue;
            };
            if !reply.ok {
                continue;
            }
            if reply
                .body
                .iter()
                .any(|line| line.split_whitespace().next() == Some(window))
            {
                owner = Some(id.clone());
                break;
            }
        }
        let Some(session_id) = owner else {
            return Err(StatusError::SessionGone);
        };
        self.session_id = Some(session_id.clone());

        // The shown session's windows and its active one.
        if let Ok(reply) = conn.send_checked(&format!("list-windows -t {session_id}")) {
            if reply.ok {
                self.windows = reply
                    .body
                    .iter()
                    .filter_map(|line| {
                        let id = line.split_whitespace().next()?;
                        // The name is the line remainder after "id marker".
                        let rest = line
                            .split_whitespace()
                            .skip(2)
                            .collect::<Vec<_>>()
                            .join(" ");
                        let name = if rest.is_empty() {
                            id.to_string()
                        } else {
                            rest
                        };
                        Some((id.to_string(), name))
                    })
                    .collect();
                self.active_window = reply
                    .body
                    .iter()
                    .find(|line| line.split_whitespace().nth(1) == Some("*"))
                    .and_then(|line| line.split_whitespace().next())
                    .map(str::to_string)
                    .or_else(|| self.windows.first().map(|(id, _)| id.clone()));
            }
        }

        // The focused pane's title.
        if let Ok(reply) = conn.send_checked(&format!("pane-title -t %{focused}")) {
            if reply.ok {
                self.pane_title = reply.body.join(" ");
            }
        }

        // The agent roster: `%N <agent> <state> <source> [key=val …]`.
        if let Ok(reply) = conn.send_checked("list-agents") {
            if reply.ok {
                self.agents = reply
                    .body
                    .iter()
                    .filter_map(|line| {
                        let mut fields = line.split_whitespace();
                        let _pane = fields.next()?;
                        let agent = fields.next()?.to_string();
                        let state = fields.next()?.to_string();
                        Some((agent, state))
                    })
                    .collect();
            }
        }
        Ok(())
    }

    /// The shown session's windows as `(id, name)`, window order — the
    /// tab strip's source of truth.
    pub(crate) fn windows(&self) -> &[(String, String)] {
        &self.windows
    }

    /// The focused pane's user title (the rename-pane prompt's seed).
    pub(crate) fn pane_title(&self) -> &str {
        &self.pane_title
    }

    /// Compose the status line as styled segments, truncated to `cols`
    /// display columns. `scroll` is the focused pane's client scroll
    /// offset when a scroll view is up.
    pub(crate) fn compose(&self, cols: u16, scroll: Option<usize>) -> Vec<Segment> {
        let mut segments: Vec<Segment> = Vec::new();
        let push = |segments: &mut Vec<Segment>, text: String, bold: bool| {
            if let Some(last) = segments.last_mut() {
                // Merge into the previous run when the style matches so
                // the line stays a handful of segments.
                if last.bold == bold {
                    last.text.push_str(&text);
                    return;
                }
            }
            segments.push(Segment { text, bold });
        };

        // The workspaces segment leads the line: every workspace's name
        // in id order, the active one bold (the "current emphasized"
        // rule the sessions/windows segments follow).
        for (index, (id, name)) in self.workspaces.iter().enumerate() {
            if index > 0 {
                push(&mut segments, " ".to_string(), false);
            }
            let active = self.active_workspace.as_deref() == Some(id.as_str());
            push(&mut segments, name.clone(), active);
        }
        if !self.workspaces.is_empty() {
            push(&mut segments, " | ".to_string(), false);
        }
        for (index, (id, name)) in self.sessions.iter().enumerate() {
            if index > 0 {
                push(&mut segments, " ".to_string(), false);
            }
            let current = self.session_id.as_deref() == Some(id.as_str());
            push(&mut segments, format!("{id}:{name}"), current);
        }
        if !self.windows.is_empty() {
            push(&mut segments, " | ".to_string(), false);
            for (index, (id, name)) in self.windows.iter().enumerate() {
                if index > 0 {
                    push(&mut segments, " ".to_string(), false);
                }
                let window = id.trim_start_matches('@');
                let active = self.active_window.as_deref() == Some(id.as_str());
                let mut text = format!("{window}:{name}");
                if active {
                    text.push('*');
                }
                push(&mut segments, text, active);
            }
        }
        if !self.pane_title.is_empty() {
            push(&mut segments, format!(" | {}", self.pane_title), false);
        }
        if !self.agents.is_empty() {
            push(&mut segments, " | ".to_string(), false);
            for (index, (agent, state)) in self.agents.iter().enumerate() {
                if index > 0 {
                    push(&mut segments, " ".to_string(), false);
                }
                push(&mut segments, format!("{agent}:{state}"), false);
            }
        }
        if let Some(offset) = scroll.filter(|offset| *offset > 0) {
            push(&mut segments, format!(" | [scroll +{offset}]"), true);
        }

        // Truncate to the row width.
        let mut used = 0usize;
        let width = cols as usize;
        segments.retain_mut(|segment| {
            if used >= width {
                return false;
            }
            let taken: String = segment.text.chars().take(width - used).collect();
            used += taken.chars().count();
            segment.text = taken;
            !segment.text.is_empty()
        });
        segments
    }
}

/// The bottom row's paint + diff pair: a one-row ratatui `Buffer` and
/// its previous frame, so a status change flushes only the changed
/// cells.
pub(crate) struct StatusRow {
    cols: u16,
    buffer: Buffer,
    prev: Buffer,
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

    /// Paint `segments` over the whole row (base reversed, per-segment
    /// bold), padding the remainder with spaces so the previous frame's
    /// tail erases.
    pub(crate) fn paint(&mut self, segments: &[Segment]) {
        for x in 0..self.cols {
            let cell = &mut self.buffer[(x, 0)];
            cell.reset();
            cell.set_style(RtStyle::default().add_modifier(RtModifier::REVERSED));
        }
        let mut x = 0u16;
        for segment in segments {
            let style = if segment.bold {
                RtStyle::default()
                    .add_modifier(RtModifier::REVERSED)
                    .add_modifier(RtModifier::BOLD)
            } else {
                RtStyle::default().add_modifier(RtModifier::REVERSED)
            };
            for ch in segment.text.chars() {
                if x >= self.cols {
                    return;
                }
                let cell = &mut self.buffer[(x, 0)];
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
                x += 1;
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
            windows: vec![
                ("@0".to_string(), "main".to_string()),
                ("@1".to_string(), "vim".to_string()),
            ],
            active_window: Some("@0".to_string()),
            pane_title: "~/src".to_string(),
            agents: vec![
                ("claude".to_string(), "working".to_string()),
                ("kimi".to_string(), "blocked".to_string()),
            ],
        }
    }

    fn joined(segments: &[Segment]) -> String {
        segments.iter().map(|s| s.text.as_str()).collect::<String>()
    }

    /// The workspaces segment leads the line: every workspace's name in
    /// id order, the daemon's active one bold.
    #[test]
    fn compose_carries_the_workspaces_segment_in_id_order() {
        let text = joined(&state().compose(120, None));
        assert!(
            text.starts_with("alpha beta | "),
            "workspaces lead the line: {text}"
        );
        assert!(text.contains("alpha beta | $0:work"), "order: {text}");
    }

    /// The active workspace's name segment is bold; the others are not.
    #[test]
    fn compose_bolds_the_active_workspace_only() {
        let segments = state().compose(120, None);
        let bold_text = joined(
            &segments
                .iter()
                .filter(|s| s.bold)
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert!(
            bold_text.contains("alpha"),
            "active workspace bold: {bold_text}"
        );
        assert!(
            !bold_text.contains("beta"),
            "other workspace not bold: {bold_text}"
        );
    }

    /// The composed line carries every fact the card lists: sessions,
    /// the shown session's windows (active `*`-marked), the pane title,
    /// and the agent chips in `agent:state` spelling.
    #[test]
    fn compose_carries_sessions_windows_title_and_chips() {
        let segments = state().compose(120, None);
        let text = joined(&segments);
        assert!(text.contains("$0:work"), "sessions: {text}");
        assert!(text.contains("$1:play"), "sessions: {text}");
        assert!(text.contains("0:main*"), "windows, active marked: {text}");
        assert!(text.contains("1:vim"), "windows: {text}");
        assert!(text.contains("~/src"), "pane title: {text}");
        assert!(text.contains("claude:working"), "agent chips: {text}");
        assert!(text.contains("kimi:blocked"), "agent chips: {text}");
    }

    /// The shown session's segment and the active window's segment are
    /// bold; a non-shown session and inactive window are not.
    #[test]
    fn compose_bolds_the_current_session_and_active_window() {
        let segments = state().compose(120, None);
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
        assert!(
            bold_text.contains("0:main*"),
            "active window bold: {bold_text}"
        );
        assert!(
            !bold_text.contains("1:vim"),
            "inactive window not bold: {bold_text}"
        );
    }

    /// A scroll offset over 0 appends a bold `[scroll +N]` cue; offset 0
    /// appends none.
    #[test]
    fn compose_appends_the_scroll_cue_when_offset() {
        let text = joined(&state().compose(120, Some(7)));
        assert!(text.contains("[scroll +7]"), "cue: {text}");
        let text = joined(&state().compose(120, Some(0)));
        assert!(!text.contains("scroll"), "no cue at live: {text}");
    }

    /// The line truncates to the row width without panicking on a wide
    /// state.
    #[test]
    fn compose_truncates_to_the_row_width() {
        let mut wide = state();
        wide.pane_title = "x".repeat(200);
        let segments = wide.compose(40, None);
        assert_eq!(joined(&segments).chars().count(), 40);
    }

    /// The row painter diffs: the first paint flushes everything, an
    /// identical repaint flushes nothing, and a change flushes only the
    /// changed cells.
    #[test]
    fn status_row_diffs_only_changes() {
        let segments = state().compose(80, None);
        let mut row = StatusRow::new(80);
        row.paint(&segments);
        let first = row.diff();
        assert_eq!(first.len(), 80, "first paint is the whole row");
        assert_eq!(first[0].1, 0, "cell y is row-relative 0");
        row.paint(&segments);
        assert!(row.diff().is_empty(), "identical repaint diffs empty");

        // One agent state change: only that chip's cells move.
        let mut next = state();
        next.agents = vec![("claude".to_string(), "blocked".to_string())];
        row.paint(&next.compose(80, None));
        let diff = row.diff();
        assert!(!diff.is_empty());
        assert!(diff.len() < 80, "a chip change must not repaint the row");
    }

    /// The painter pads the row with reversed spaces, so a shorter line
    /// erases the previous frame's longer tail.
    #[test]
    fn status_row_pads_and_erases_the_tail() {
        let mut row = StatusRow::new(60);
        let mut long = state();
        long.pane_title = String::new();
        long.agents.clear();
        row.paint(&long.compose(60, None));
        row.diff();
        let mut short = long.clone();
        short.pane_title = String::new();
        short.windows.clear();
        row.paint(&short.compose(60, None));
        let diff = row.diff();
        assert!(
            diff.iter().any(|(_, _, cell)| cell.symbol() == " "),
            "the tail erases to padded spaces: {diff:?}"
        );
    }
}
