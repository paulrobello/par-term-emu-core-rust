//! Prefix/chord parsing and target resolution: list-line parsers and
//! the `-t` target lookup against the daemon.

use super::*;

/// Parse the `--prefix` tmux spelling (`C-b`, `C-a`, `C-Space`) or a
/// literal single character into its byte. Delegates to the crate's one
/// prefix grammar in [`crate::mux::config`], so `--prefix` and the config
/// file's chords share spellings and edge cases.
pub(super) fn parse_prefix(spec: &str) -> Option<u8> {
    crate::mux::config::parse_prefix(spec)
}

/// The client-side half of a reload: re-read the canonical config file
/// and re-derive the prefix and reload key through the pure helper
/// ([`crate::mux::config::reload_client_chords`]), `current` the
/// fallback for settings the file does not name.
pub(super) fn reload_client_chords(
    current: crate::mux::config::Chords,
) -> Result<crate::mux::config::Chords, String> {
    let file = crate::mux::config::load_canonical_checked()?;
    crate::mux::config::reload_client_chords(&file, &current)
}

/// One `list-sessions` reply line as `(session_id, name)`, through the
/// shared [`SessionRow`](crate::mux::ipc::SessionRow) grammar (the id is
/// the LAST `$N:` marker, so it never keeps the colon — the `$0:` bug).
pub(crate) fn parse_session_line(line: &str) -> Option<(String, String)> {
    crate::mux::ipc::SessionRow::parse(line).map(|row| (row.id, row.name))
}

/// One `list-workspaces` reply line as `(id, name, active)`, through the
/// shared [`WorkspaceRow`](crate::mux::ipc::WorkspaceRow) grammar.
pub(crate) fn parse_workspace_line(line: &str) -> Option<(String, String, bool)> {
    crate::mux::ipc::WorkspaceRow::parse(line).map(|row| (row.id, row.name, row.active))
}

/// The active window of a `list-windows -t` reply body: the `*`-marked
/// row, else the first row. The one spelling of the active-window rule
/// both attach modes share.
pub(crate) fn active_window_row(body: &[String]) -> Option<String> {
    let rows: Vec<crate::mux::ipc::WindowRow> = body
        .iter()
        .filter_map(|l| crate::mux::ipc::WindowRow::parse(l))
        .collect();
    rows.iter()
        .find(|row| row.active)
        .or_else(|| rows.first())
        .map(|row| row.id.clone())
}

/// One user-initiated query at [`conn::INTERACTIVE_TIMEOUT`] (ARC-137):
/// `Ok(Some(reply))` for an ok reply, `Ok(None)` for a refusal or a
/// transport failure other than a timeout, and `Err(TimedOut)` when the
/// reply outlived the bound — the one outcome a chain must stop on, since
/// each further command adds another abandoned reply.
pub(crate) fn interactive(
    conn: &mut conn::AttachConn,
    command: &str,
) -> std::io::Result<Option<crate::mux::client::Reply>> {
    match conn.send_interactive(command) {
        Ok(reply) if reply.ok => Ok(Some(reply)),
        Ok(_) => Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => Err(err),
        Err(_) => Ok(None),
    }
}

/// [`session_active_window`] that surfaces a timeout instead of folding
/// it into `None`.
pub(crate) fn try_session_active_window(
    conn: &mut conn::AttachConn,
    session: &str,
) -> std::io::Result<Option<String>> {
    Ok(interactive(conn, &format!("list-windows -t {session}"))?
        .and_then(|reply| active_window_row(&reply.body)))
}

/// `session`'s active window id ([`active_window_row`] over its
/// `list-windows` reply), or `None` when the session no longer exists,
/// has no windows, or the query outlived the interactive bound.
pub(crate) fn session_active_window(conn: &mut conn::AttachConn, session: &str) -> Option<String> {
    try_session_active_window(conn, session).ok().flatten()
}

/// `session`'s window ids (`@N`) in the daemon's order — empty when the
/// query fails or the session is gone; `Err(TimedOut)` past the
/// interactive bound.
pub(crate) fn list_window_ids(
    conn: &mut conn::AttachConn,
    session: &str,
) -> std::io::Result<Vec<String>> {
    Ok(interactive(conn, &format!("list-windows -t {session}"))?
        .map(|reply| {
            reply
                .body
                .iter()
                .filter_map(|l| crate::mux::ipc::WindowRow::parse(l))
                .map(|row| row.id)
                .collect()
        })
        .unwrap_or_default())
}

/// The id after (`forward`) or before `current` in `ids`, wrapping at
/// both ends; `None` when `current` is not in `ids`. The cycle rule every
/// window/session/workspace switch in both attach modes follows.
pub(crate) fn next_in_cycle<'a>(
    ids: &'a [String],
    current: &str,
    forward: bool,
) -> Option<&'a String> {
    let position = ids.iter().position(|id| id == current)?;
    let len = ids.len();
    let next = if forward {
        (position + 1) % len
    } else {
        (position + len - 1) % len
    };
    ids.get(next)
}

/// The workspace after (`forward`) or before the daemon's active one in
/// a `list-workspaces` reply body, wrapping; `None` when the roster is
/// empty or names no active workspace.
pub(crate) fn next_workspace(body: &[String], forward: bool) -> Option<String> {
    let rows: Vec<(String, String, bool)> = body
        .iter()
        .filter_map(|l| parse_workspace_line(l))
        .collect();
    let active = rows.iter().find(|(_, _, active)| *active)?.0.clone();
    let ids: Vec<String> = rows.into_iter().map(|(id, _, _)| id).collect();
    next_in_cycle(&ids, &active, forward).cloned()
}

/// The daemon's displayed session (the active workspace's active session)
/// via the bare `switch-client` query, at the interactive bound so a
/// timeout surfaces; `Ok(None)` when the daemon predates the query,
/// nothing is displayed, or the reply outlived the bound.
pub(crate) fn try_displayed_session(
    conn: &mut conn::AttachConn,
) -> std::io::Result<Option<String>> {
    Ok(interactive(conn, "switch-client")?.and_then(|reply| displayed_session_of(&reply)))
}

/// The session id a bare `switch-client` reply names, if any.
pub(crate) fn displayed_session_of(reply: &crate::mux::client::Reply) -> Option<String> {
    let id = reply.body.first()?.trim();
    (id.starts_with('$') && id.len() > 1).then(|| id.to_string())
}

/// The target-less attach rule: a daemon with no sessions gets a default
/// tree created and that session attached; exactly one existing session
/// is attached as-is; several are a refusal that lists them with the
/// `-t` hint. The one rule both attach modes' target-less seed follows.
pub(crate) fn initial_attach_session(conn: &mut conn::AttachConn) -> Result<String, String> {
    let mut sessions = session_ids(conn)?;
    if sessions.is_empty() {
        // `new-workspace -n default` is the one command that builds the
        // whole default tree at once: workspace + session `default`, a
        // window named `1`, one unnamed pane.
        conn.send_checked("new-workspace -n default")
            .map_err(|err| format!("new-workspace failed: {err}"))?;
        sessions = session_ids(conn)?;
    }
    match sessions.as_slice() {
        [one] => Ok(one.0.clone()),
        [] => Err("no sessions exist even after the default-tree create".to_string()),
        many => Err(multi_session_error(many)),
    }
}

/// The attach target after the session picker (`[client]
/// session-picker`): an explicit `target` passes through; with none and
/// `active` (the setting on, both stdio streams terminals — see
/// [`super::picker::picker_active`]), a daemon holding several sessions
/// hands them to `choose`, and its pick becomes the target. `Ok(None)`
/// leaves the target-less rule ([`initial_attach_session`]) to the
/// resolver: the picker is off (no query is sent, so the refusal path is
/// unchanged), or zero or one session exists. An abort is the same
/// [`multi_session_error`] the refusal prints.
pub(crate) fn picked_target(
    query: &mut dyn FnMut(&str) -> std::io::Result<crate::mux::client::Reply>,
    target: Option<&str>,
    active: bool,
    choose: &mut dyn FnMut(&[super::picker::PickerRow]) -> Option<usize>,
) -> Result<Option<String>, String> {
    if let Some(target) = target {
        return Ok(Some(target.to_string()));
    }
    if !active {
        return Ok(None);
    }
    let sessions: Vec<(String, String)> = query("list-sessions")
        .map_err(|err| format!("list-sessions failed: {err}"))?
        .body
        .iter()
        .filter_map(|l| parse_session_line(l))
        .collect();
    if sessions.len() < 2 {
        return Ok(None);
    }
    let counts = window_counts(query);
    let rows: Vec<super::picker::PickerRow> = sessions
        .iter()
        .map(|(id, name)| super::picker::PickerRow {
            id: id.clone(),
            name: name.clone(),
            windows: counts
                .as_ref()
                .map(|counts| counts.iter().filter(|session| *session == id).count()),
        })
        .collect();
    match choose(&rows).and_then(|index| rows.get(index)) {
        Some(row) => Ok(Some(row.id.clone())),
        None => Err(multi_session_error(&sessions)),
    }
}

/// The owning session id of every window (`list-windows -a` rows are
/// `$S @N <marker> <name>`), or `None` when the query fails.
fn window_counts(
    query: &mut dyn FnMut(&str) -> std::io::Result<crate::mux::client::Reply>,
) -> Option<Vec<String>> {
    let reply = query("list-windows -a").ok().filter(|reply| reply.ok)?;
    Some(
        reply
            .body
            .iter()
            .filter_map(|line| line.split_once(' '))
            .filter(|(session, rest)| {
                session.starts_with('$') && crate::mux::ipc::WindowRow::parse(rest).is_some()
            })
            .map(|(session, _)| session.to_string())
            .collect(),
    )
}

/// The `(id, name)` pairs of every session, in `list-sessions` order.
fn session_ids(conn: &mut conn::AttachConn) -> Result<Vec<(String, String)>, String> {
    let sessions = conn
        .send_checked("list-sessions")
        .map_err(|err| format!("list-sessions failed: {err}"))?;
    Ok(sessions
        .body
        .iter()
        .filter_map(|l| parse_session_line(l))
        .collect())
}

/// The refusal for a target-less attach against several sessions: the
/// roster, then the `-t` hint. One spelling, both attach modes.
pub(crate) fn multi_session_error(sessions: &[(String, String)]) -> String {
    let mut message = String::from("multiple sessions exist — choose one:\n");
    for (id, name) in sessions {
        message.push_str(&format!("  {id}: {name}\n"));
    }
    message.push_str(
        "attach to one with `par-mux attach -t <session>` — with several servers \
         running, name the server too: `par-mux attach <server> -t <session>`",
    );
    message
}

/// Where a switch lands: a window, plus its owning session when the
/// resolver learned it. The resolvers below own the "which target" rules
/// both attach modes share (ARC-139); each mode keeps only its own
/// landing step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Landing {
    pub(crate) window: String,
    pub(crate) session: Option<String>,
}

/// The outcome of resolving a workspace to a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceLanding {
    /// The session to land on.
    Session(String),
    /// The workspace has no sessions: nothing to land on.
    Empty,
    /// The query failed: leave the view put.
    Failed,
}

// Every resolver below runs at the interactive bound (ARC-137) and
// returns `Err(TimedOut)` the moment one of its queries outlives it, so
// the caller stops the chain; `Ok(None)` is "nowhere to land".

/// `session`'s active window as a [`Landing`] that names the session.
pub(crate) fn session_landing(
    conn: &mut conn::AttachConn,
    session: &str,
) -> std::io::Result<Option<Landing>> {
    Ok(
        try_session_active_window(conn, session)?.map(|window| Landing {
            window,
            session: Some(session.to_string()),
        }),
    )
}

/// prefix n/p: the window after (`forward`) or before `shown_window` in
/// `session`'s window list, wrapping; `None` when the shown window is not
/// listed.
pub(crate) fn window_switch_target(
    conn: &mut conn::AttachConn,
    session: &str,
    shown_window: &str,
    forward: bool,
) -> std::io::Result<Option<Landing>> {
    let windows = list_window_ids(conn, session)?;
    Ok(
        next_in_cycle(&windows, shown_window, forward).map(|window| Landing {
            window: window.clone(),
            session: None,
        }),
    )
}

/// prefix ( / ): the session after (`forward`) or before the current one
/// in list-sessions order, landing on its active window. The current
/// session counts only while it still owns `shown_window` (one
/// `list-windows` probe); stale or absent state falls back to the roster
/// head so a direction still moves somewhere deterministic. An empty
/// roster is `None`.
pub(crate) fn session_switch_target(
    conn: &mut conn::AttachConn,
    current: Option<&str>,
    shown_window: &str,
    forward: bool,
) -> std::io::Result<Option<Landing>> {
    let Some(reply) = interactive(conn, "list-sessions")? else {
        return Ok(None);
    };
    let sessions: Vec<String> = reply
        .body
        .iter()
        .filter_map(|l| parse_session_line(l).map(|(id, _)| id))
        .collect();
    let Some(head) = sessions.first().cloned() else {
        return Ok(None);
    };
    let current = match current {
        Some(current)
            if list_window_ids(conn, current)?
                .iter()
                .any(|w| w == shown_window) =>
        {
            current.to_string()
        }
        _ => head,
    };
    let Some(next) = next_in_cycle(&sessions, &current, forward).cloned() else {
        return Ok(None);
    };
    session_landing(conn, &next)
}

/// The session a workspace landing goes to: the one `select-workspace`
/// resumed (the [`displayed_session`], which the follow broadcast names,
/// so every client lands on the same view — card 01a11bd1) when it is
/// listed under `ws_id`, else the first listed.
pub(crate) fn workspace_session(
    conn: &mut conn::AttachConn,
    ws_id: &str,
) -> std::io::Result<WorkspaceLanding> {
    let Some(reply) = interactive(conn, &format!("list-sessions -t {ws_id}"))? else {
        return Ok(WorkspaceLanding::Failed);
    };
    let listed: Vec<String> = reply
        .body
        .iter()
        .filter_map(|l| parse_session_line(l).map(|(id, _)| id))
        .collect();
    let resumed = try_displayed_session(conn)?.filter(|s| listed.contains(s));
    Ok(match resumed.or_else(|| listed.first().cloned()) {
        Some(session) => WorkspaceLanding::Session(session),
        None => WorkspaceLanding::Empty,
    })
}

/// Where a view lands when its shown session is gone (the exit cascade,
/// or an explicit kill): the shown session's active window while the
/// session survives, else the active window of `workspace`'s landing
/// session ([`workspace_session`]), else that of the daemon's active
/// workspace (the `list-workspaces` `active` row, else the first). Reads
/// only: the daemon already moved its pointers, so every client showing
/// the gone view resolves the same landing. `Ok(None)` only when the
/// daemon has nothing left to show; `Err(TimedOut)` past the interactive
/// bound.
pub(crate) fn cascade_landing(
    conn: &mut conn::AttachConn,
    session: Option<&str>,
    workspace: Option<&str>,
) -> std::io::Result<Option<String>> {
    if let Some(session) = session {
        if let Some(window) = try_session_active_window(conn, session)? {
            return Ok(Some(window));
        }
    }
    let mut workspaces: Vec<String> = workspace.map(str::to_string).into_iter().collect();
    if let Some(reply) = interactive(conn, "list-workspaces")? {
        let rows: Vec<(String, String, bool)> = reply
            .body
            .iter()
            .filter_map(|l| parse_workspace_line(l))
            .collect();
        workspaces.extend(
            rows.iter()
                .find(|(_, _, active)| *active)
                .or_else(|| rows.first())
                .map(|(id, _, _)| id.clone()),
        );
    }
    for ws_id in workspaces {
        if let WorkspaceLanding::Session(session) = workspace_session(conn, &ws_id)? {
            if let Some(window) = try_session_active_window(conn, &session)? {
                return Ok(Some(window));
            }
        }
    }
    Ok(None)
}

/// prefix W / C-w: `list-workspaces`, the next/previous workspace in id
/// order ([`next_workspace`]), then `select-workspace` on it; the
/// selected id.
pub(crate) fn workspace_switch_target(
    conn: &mut conn::AttachConn,
    forward: bool,
) -> std::io::Result<Option<String>> {
    let Some(reply) = interactive(conn, "list-workspaces")? else {
        return Ok(None);
    };
    let Some(ws_id) = next_workspace(&reply.body, forward) else {
        return Ok(None);
    };
    interactive(conn, &format!("select-workspace -t {ws_id}"))?;
    Ok(Some(ws_id))
}

/// Resolve the attach target to a pane id. A pane target (`%N` or a pane
/// title name) goes straight to the daemon's matcher via `pane-info`; a
/// window (`@N`/name) or session (`$N`/name) target narrows through the
/// targeted `list-windows`/`list-panes` queries to the marked pane. With
/// no `-t`, [`initial_attach_session`] picks (or creates) the session and
/// its active window's marked pane is the target.
pub(super) fn resolve_target(
    conn: &mut conn::AttachConn,
    target: Option<&str>,
) -> Result<String, String> {
    match target {
        None => {
            let session = initial_attach_session(conn)?;
            let windows = conn
                .send_checked(&format!("list-windows -t {session}"))
                .map_err(|err| format!("list-windows failed: {err}"))?;
            let window = marked_line(&windows.body, "@", &session)?;
            let panes = conn
                .send_checked(&format!("list-panes -t {window}"))
                .map_err(|err| format!("list-panes failed: {err}"))?;
            marked_pane(&panes.body, &window)
        }
        Some(target) => {
            // Try the daemon-side pane matcher first: ids and pane titles
            // both resolve here, and the failure tells us to try session or
            // window scopes. The reply's first line names the resolved
            // pane (`%N @W ...`), so an ok-but-empty reply does not count.
            if conn
                .send_checked(&format!("pane-info -t {target}"))
                .is_ok_and(|reply| {
                    reply.ok
                        && reply
                            .body
                            .first()
                            .and_then(|l| crate::mux::ipc::PaneInfoRow::parse(l))
                            .is_some()
                })
            {
                return Ok(target.to_string());
            }
            // A session: its active window's marked pane. Probed BEFORE
            // the window branch — `list-panes -t` accepts session
            // targets too (the one-rule grammar), so it can no longer
            // prove windowness.
            if let Ok(reply) = conn.send_checked(&format!("list-windows -t {target}")) {
                if reply.ok {
                    if let Some(window) = active_window_row(&reply.body) {
                        if let Ok(panes) = conn.send_checked(&format!("list-panes -t {window}")) {
                            if panes.ok {
                                return marked_pane(&panes.body, target);
                            }
                        }
                    }
                    return Err(format!("the session {target} has no windows"));
                }
            }
            // A window: its marked pane via the targeted list-panes.
            if let Ok(reply) = conn.send_checked(&format!("list-panes -t {target}")) {
                if reply.ok {
                    return marked_pane(&reply.body, target);
                }
            }
            Err(format!("no such target: {target}"))
        }
    }
}

/// The `*`-marked line from a targeted list reply (`%N <leaf> *` pane rows
/// or `@N * <name>` window rows), else the first line. `sigil` guards the
/// line shape (`%` panes, `@` windows).
pub(super) fn marked_line(body: &[String], sigil: &str, target: &str) -> Result<String, String> {
    body.iter()
        .find(|l| l.starts_with(sigil))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string)
        .ok_or_else(|| format!("empty listing for target {target}"))
}

/// The marked pane id from a `list-panes -t <window>` reply, preferring
/// the `*`-active pane over the first row.
pub(crate) fn marked_pane(body: &[String], target: &str) -> Result<String, String> {
    let active = body
        .iter()
        .find(|l| l.split_whitespace().nth(2) == Some("*"))
        .or_else(|| body.first())
        .and_then(|l| l.split_whitespace().next());
    active
        .map(str::to_string)
        .ok_or_else(|| format!("no panes under target {target}"))
}

#[cfg(test)]
mod tests {
    use super::multi_session_error;

    /// The refusal names every session and the `-t` hint, so the user can
    /// act on it directly — including the server-qualified spelling, so
    /// the suggested form never fails against a second running server.
    #[test]
    fn multi_session_error_lists_every_session_and_the_hint() {
        let message = multi_session_error(&[
            ("$0".to_string(), "att".to_string()),
            ("$1".to_string(), "second".to_string()),
        ]);
        assert!(message.contains("multiple sessions exist"));
        assert!(message.contains("$0: att"));
        assert!(message.contains("$1: second"));
        assert!(message.contains("par-mux attach -t <session>"));
        assert!(message.contains("par-mux attach <server> -t <session>"));
    }
}
