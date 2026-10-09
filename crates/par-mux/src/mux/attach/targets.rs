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

/// `session`'s active window id ([`active_window_row`] over its
/// `list-windows` reply), or `None` when the session no longer exists
/// or has no windows.
pub(crate) fn session_active_window(conn: &mut conn::AttachConn, session: &str) -> Option<String> {
    let reply = conn
        .send_checked(&format!("list-windows -t {session}"))
        .ok()
        .filter(|reply| reply.ok)?;
    active_window_row(&reply.body)
}

/// `session`'s window ids (`@N`) in the daemon's order — empty when the
/// query fails or the session is gone.
pub(crate) fn list_window_ids(conn: &mut conn::AttachConn, session: &str) -> Vec<String> {
    conn.send_checked(&format!("list-windows -t {session}"))
        .ok()
        .filter(|reply| reply.ok)
        .map(|reply| {
            reply
                .body
                .iter()
                .filter_map(|l| crate::mux::ipc::WindowRow::parse(l))
                .map(|row| row.id)
                .collect()
        })
        .unwrap_or_default()
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

/// Resolve the attach target to a pane id. A pane target (`%N` or a pane
/// title name) goes straight to the daemon's matcher via `pane-info`; a
/// window (`@N`/name) or session (`$N`/name) target narrows through the
/// targeted `list-windows`/`list-panes` queries to the marked pane. With
/// no `-t`, the newest session's newest pane — the highest pane id in the
/// global roster (ids are monotonic, so the max is newest).
pub(super) fn resolve_target(
    conn: &mut conn::AttachConn,
    target: Option<&str>,
) -> Result<String, String> {
    match target {
        None => {
            let panes = conn
                .send_checked("list-panes")
                .map_err(|err| format!("list-panes failed: {err}"))?;
            // Newest = highest id (ids are monotonic). The bare global
            // roster has no deterministic order, so max by numeric id is
            // the honest spelling of the newest-stand-in rule.
            panes
                .body
                .iter()
                .filter(|l| l.starts_with('%'))
                .filter_map(|l| l.split_whitespace().next())
                .filter_map(|id| {
                    let n: u64 = id[1..].parse().ok()?;
                    Some((n, id))
                })
                .max_by_key(|(n, _)| *n)
                .map(|(_, id)| id.to_string())
                .ok_or_else(|| "no panes exist — create a session first".to_string())
        }
        Some(target) => {
            // Try the daemon-side pane matcher first: ids and pane titles
            // both resolve here, and the failure tells us to try window or
            // session scopes. The reply's first line names the resolved
            // pane (`%N @W ...`), so an ok-but-empty reply does not count.
            if conn
                .send_checked(&format!("pane-info -t {target}"))
                .is_ok_and(|reply| {
                    reply.ok && reply.body.first().is_some_and(|l| l.starts_with('%'))
                })
            {
                return Ok(target.to_string());
            }
            // A window: its marked pane via the targeted list-panes.
            if let Ok(reply) = conn.send_checked(&format!("list-panes -t {target}")) {
                if reply.ok {
                    return marked_pane(&reply.body, target);
                }
            }
            // A session: its active window's marked pane.
            if let Ok(reply) = conn.send_checked(&format!("list-windows -t {target}")) {
                if reply.ok {
                    let window = marked_line(&reply.body, "@", target)?;
                    if let Ok(panes) = conn.send_checked(&format!("list-panes -t {window}")) {
                        if panes.ok {
                            return marked_pane(&panes.body, target);
                        }
                    }
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
