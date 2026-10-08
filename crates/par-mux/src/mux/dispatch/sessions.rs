//! Session, workspace, and server command handlers, plus the workspace
//! roster fingerprint the server's change detection reads.

use super::*;

/// The session/workspace/server group's router; `route_command` sends only
/// this group's variants here.
pub(super) fn route_session_command(ctx: &Ctx<'_>, command: MuxCommand) -> Outcome {
    match command {
        MuxCommand::NewSession {
            name,
            env,
            workspace,
        } => cmd_new_session(ctx, name, env, workspace),
        MuxCommand::RenameSession { session, name } => cmd_rename_session(ctx, session, name),
        MuxCommand::KillSession { session } => cmd_kill_session(ctx, session),
        MuxCommand::ListSessions { workspace } => cmd_list_sessions(ctx, workspace),
        MuxCommand::NewWorkspace { name } => cmd_new_workspace(ctx, name),
        MuxCommand::ListWorkspaces => cmd_list_workspaces(ctx),
        MuxCommand::SelectWorkspace { workspace } => cmd_select_workspace(ctx, workspace),
        MuxCommand::SwitchClient { target } => cmd_switch_client(ctx, target),
        MuxCommand::RenameWorkspace { workspace, name } => {
            cmd_rename_workspace(ctx, workspace, name)
        }
        MuxCommand::KillWorkspace { workspace } => cmd_kill_workspace(ctx, workspace),
        MuxCommand::KillServer => cmd_kill_server(ctx),
        other => unreachable!("route_command sent a non-session command: {other:?}"),
    }
}

pub(super) fn cmd_new_session(
    ctx: &Ctx<'_>,
    name: Option<String>,
    env: Vec<(String, String)>,
    workspace: Option<Target<WorkspaceId>>,
) -> Outcome {
    let name = name.unwrap_or_else(|| "0".to_string());
    let env: std::collections::BTreeMap<String, String> = env.into_iter().collect();
    // Two-phase spawn (ARC-022): reserve ids under the lock, run the
    // fork/exec OFF it — a slow spawn must not stall every other client —
    // then re-lock to insert and wire. The in-flight session is invisible
    // until the insert lands. The workspace target resolves here (under
    // the lock, against the tree): an unknown one fails the command before
    // anything spawns.
    let (plan, factory) = {
        let mut guard = ctx.tree.lock();
        let workspace_id = match guard.resolve_new_session_workspace(workspace) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        (
            guard.begin_session_at(workspace_id, &name, DEFAULT_COLS, DEFAULT_ROWS, &env),
            guard.factory(),
        )
    };
    let window_id = plan.window_id;
    let outcome = spawn_and_wire(ctx, &*factory, plan, None, None)
        .map(|session_id| (session_id, vec![window_id]));
    match outcome {
        Ok((session_id, window_ids)) => {
            let mut result = Outcome::ok(ctx, &session_id.to_string());
            for window in window_ids {
                result = result.notifying(window_add_notification(ctx.tree, window));
            }
            // The session set changed on the create side too — one cue for
            // both directions, so a client's "re-query list-sessions"
            // handling is written once.
            result = result.notifying(TmuxNotification::SessionsChanged);
            result.telling_issuer(TmuxNotification::SessionChanged {
                session_id: session_id.to_string(),
                name,
            })
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_kill_server(ctx: &Ctx<'_>) -> Outcome {
    // The same path SIGTERM takes: raise the flag and let the accept loop
    // notice on its next tick, which runs the final state save and sends
    // `%exit` to every client. The reply goes out before the loop exits.
    match ctx.shutdown {
        Some(flag) => {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Outcome::ok(ctx, "")
        }
        None => Outcome::err(ctx, "kill-server: no running server to stop"),
    }
}

pub(super) fn cmd_rename_session(
    ctx: &Ctx<'_>,
    session: Target<SessionId>,
    name: String,
) -> Outcome {
    let session = {
        let guard = ctx.tree.lock();
        match guard.resolve_session_target(session) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().rename_session(session, &name) {
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::SessionRenamed {
            session_id: session.to_string(),
            name,
        }),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_kill_session(ctx: &Ctx<'_>, session: Target<SessionId>) -> Outcome {
    let killed = match kill_target(
        ctx,
        session,
        MuxTree::resolve_session_target,
        MuxTree::kill_session,
    ) {
        Ok(killed) => killed,
        Err(outcome) => return outcome,
    };
    // The same line order kill-window's cascade produces: one
    // %window-close per killed window, then the session-set cue —
    // plus the workspace cue when the session's death emptied its
    // workspace away.
    let mut outcome = notify_window_closes(Outcome::ok(ctx, ""), &killed.removed)
        .notifying(TmuxNotification::SessionsChanged);
    if killed.workspaces_changed {
        outcome = outcome.notifying(TmuxNotification::WorkspacesChanged);
    }
    outcome
}

pub(super) fn cmd_list_sessions(ctx: &Ctx<'_>, workspace: Option<Target<WorkspaceId>>) -> Outcome {
    // Wire contract: bare `list-sessions` lists EVERY workspace's sessions,
    // one line each, extended from the old `$N: name` shape with a
    // workspace prefix: `+W: wname: $N: name`. A consumer finds the
    // session id at the LAST ` $<digits>:` marker; the workspace fields
    // precede it. `list-sessions -t <workspace>` (id or name) restricts
    // the listing to that one workspace, same line shape. Sessions are
    // listed in workspace order, sessions in workspace-list order.
    let guard = ctx.tree.lock();
    let workspace_id = match workspace {
        Some(target) => match guard.resolve_workspace_target(target) {
            Ok(id) if guard.workspace(id).is_some() => Some(id),
            Ok(id) => return Outcome::err(ctx, &MuxError::NoSuchWorkspace(id).to_string()),
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        },
        None => None,
    };
    let mut lines: Vec<String> = Vec::new();
    let mut workspaces = guard.workspaces();
    workspaces.sort();
    for ws_id in workspaces {
        let Some(ws) = guard.workspace(ws_id) else {
            continue;
        };
        if let Some(filter) = workspace_id {
            if filter != ws_id {
                continue;
            }
        }
        for session_id in &ws.sessions {
            let Some(session) = guard.session(*session_id) else {
                continue;
            };
            lines.push(format!(
                "{}: {}: {}: {}",
                ws.id, ws.name, session.id, session.name
            ));
        }
    }
    Outcome::ok(ctx, &lines.join("\n"))
}

/// `new-workspace [-n name]`: create the workspace, select it, and reply
/// with its `+N` id. `%workspaces-changed` is the roster cue — one
/// argument-less broadcast covering add, close, select, and rename, the
/// same re-query convention `%sessions-changed` set (clients re-run
/// `list-workspaces` rather than parse a diff).
pub(super) fn cmd_new_workspace(ctx: &Ctx<'_>, name: Option<String>) -> Outcome {
    let name = name.unwrap_or_else(|| "main".to_string());
    // A workspace with no sessions cannot be landed on — the panel's
    // ` new ` chip must create a USABLE one: the workspace plus its
    // first session/window spawning the default shell, so the
    // select+land the client runs after the create has something to
    // land on (the manual-pass report: a created workspace highlighted
    // in the side panel while the view stayed on the old one's tabs).
    let (id, plan, factory) = {
        let mut guard = ctx.tree.lock();
        let id = guard.new_workspace(&name);
        (
            id,
            guard
                .begin_session_at(id, &name, DEFAULT_COLS, DEFAULT_ROWS, &Default::default())
                .with_window_name("1"),
            guard.factory(),
        )
    };
    let window_id = plan.window_id;
    let outcome = spawn_and_wire(ctx, &*factory, plan, None, None)
        .map(|session_id| (session_id, vec![window_id]));
    match outcome {
        Ok((_session_id, window_ids)) => {
            // The reply stays the WORKSPACE id (the wire contract the
            // client's landing parses); the session/window cues ride the
            // notifications.
            let mut result = Outcome::ok(ctx, &id.to_string())
                .notifying(TmuxNotification::WorkspacesChanged)
                .notifying(TmuxNotification::SessionsChanged);
            for window in window_ids {
                result = result.notifying(window_add_notification(ctx.tree, window));
            }
            result
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_list_workspaces(ctx: &Ctx<'_>) -> Outcome {
    // Wire contract: one `+N: name` line per workspace, in id order; the
    // daemon's active workspace's line ends with a space and `active`.
    let guard = ctx.tree.lock();
    let active = guard.active_workspace();
    let mut ids = guard.workspaces();
    ids.sort();
    let body = ids
        .iter()
        .filter_map(|id| guard.workspace(*id))
        .map(|ws| {
            if Some(ws.id) == active {
                format!("{}: {} active", ws.id, ws.name)
            } else {
                format!("{}: {}", ws.id, ws.name)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Outcome::ok(ctx, &body)
}

pub(super) fn cmd_select_workspace(ctx: &Ctx<'_>, workspace: Target<WorkspaceId>) -> Outcome {
    let (workspace, session, window, moved, resized) = {
        let mut guard = ctx.tree.lock();
        let workspace = match guard.resolve_workspace_target(workspace) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let previous = guard.active_workspace();
        let was_session = guard.active_session();
        let was_window = was_session
            .and_then(|s| guard.session(s))
            .and_then(|s| s.windows.get(s.active).copied());
        if let Err(err) = guard.select_workspace(workspace) {
            return Outcome::err(ctx, &err.to_string());
        }
        let session = guard.active_session();
        let window = session
            .and_then(|s| guard.session(s))
            .and_then(|s| s.windows.get(s.active).copied());
        // Shared-selection sync: when the switch moves the displayed view,
        // render clients attached to the left workspace's sessions follow
        // to the new displayed window and `%client-session-changed` tells
        // them so — the `client` field carries the workspace the display
        // moved to, since par-mux has no per-client names. Re-selecting
        // the active workspace (same displayed session and window) is a
        // no-op.
        let moved = was_window != window;
        let resized = match (moved, window) {
            (true, Some(window)) => match previous {
                Some(previous) => guard.follow_workspace_views(previous, window),
                None => guard.follow_workspace_views(workspace, window),
            },
            _ => Vec::new(),
        };
        (workspace, session, window, moved, resized)
    };
    let mut result = Outcome::ok(ctx, "").notifying(TmuxNotification::WorkspacesChanged);
    if moved {
        if let (Some(session), Some(_)) = (session, window) {
            result = result.notifying(TmuxNotification::ClientSessionChanged {
                client: workspace.to_string(),
                session_id: session.to_string(),
                name: ctx
                    .tree
                    .lock()
                    .session(session)
                    .map(|s| s.name.clone())
                    .unwrap_or_default(),
            });
        }
    }
    for window_id in resized {
        result = result.with_layout(window_id);
    }
    result
}

/// `switch-client [-t <session|window|pane>]`: the displayed-session
/// pointer's read and write. Bare, the reply is the displayed session's
/// `$N` (empty when nothing exists) — what a no-target attach lands on.
/// With a target, the target's session becomes the displayed one (and a
/// window/pane target's window its active window), so a client landing on
/// a sibling session moves the pointer every other path reads — the
/// no-target attach, `select-workspace`'s resume, the follow broadcasts,
/// and the persisted state. A target in the already-displayed session is
/// exactly `select-window` (same `%session-window-changed` /
/// `%window-pane-changed` tab-switch cues). A cross-session switch sends
/// `%client-session-changed` when the displayed window moved — render
/// clients viewing the previously displayed workspace follow, the same
/// cue `select-workspace` sends — plus `%workspaces-changed` when the
/// workspace moved and `%window-pane-changed` for a targeted window.
pub(super) fn cmd_switch_client(ctx: &Ctx<'_>, target: Option<AnyTarget>) -> Outcome {
    let Some(target) = target else {
        let guard = ctx.tree.lock();
        let body = guard
            .active_session()
            .map(|s| s.to_string())
            .unwrap_or_default();
        return Outcome::ok(ctx, &body);
    };
    let switched = {
        let mut guard = ctx.tree.lock();
        let (session, window) = match guard.resolve_new_window_target(target) {
            Ok(resolved) => resolved,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        // Already the displayed session: a window target is exactly a
        // select-window (its %session-window-changed / %window-pane-changed
        // are what tab-switch consumers track); a session target is a no-op.
        if guard.active_session() == Some(session) {
            drop(guard);
            return match window {
                Some(window) => cmd_select_window(ctx, Target::Id(window)),
                None => Outcome::ok(ctx, ""),
            };
        }
        let previous_workspace = guard.active_workspace();
        let was_window = guard
            .active_session()
            .and_then(|s| guard.session(s))
            .and_then(|s| s.windows.get(s.active).copied());
        if let Some(window) = window {
            if let Err(err) = guard.select_window(window) {
                return Outcome::err(ctx, &err.to_string());
            }
        }
        if let Err(err) = guard.display_session(session) {
            return Outcome::err(ctx, &err.to_string());
        }
        let workspace = guard.active_workspace();
        let shown = guard
            .session(session)
            .and_then(|s| s.windows.get(s.active).copied());
        let moved = was_window != shown;
        let resized = match (moved, shown, previous_workspace) {
            (true, Some(shown), Some(previous)) => guard.follow_workspace_views(previous, shown),
            _ => Vec::new(),
        };
        let name = guard
            .session(session)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let pane_changed = window.and_then(|w| guard.window(w).map(|win| (w, win.active)));
        (
            session,
            name,
            workspace,
            moved && shown.is_some(),
            workspace != previous_workspace,
            resized,
            pane_changed,
        )
    };
    let (session, name, workspace, moved, workspace_moved, resized, pane_changed) = switched;
    let mut result = Outcome::ok(ctx, "");
    for window_id in resized {
        result = result.with_layout(window_id);
    }
    if workspace_moved {
        result = result.notifying(TmuxNotification::WorkspacesChanged);
    }
    if moved {
        result = result.notifying(TmuxNotification::ClientSessionChanged {
            client: workspace.map(|w| w.to_string()).unwrap_or_default(),
            session_id: session.to_string(),
            name,
        });
    }
    if let Some((window, pane)) = pane_changed {
        result = result.notifying(TmuxNotification::WindowPaneChanged {
            window_id: window.to_string(),
            pane_id: pane.to_string(),
        });
    }
    result
}

pub(super) fn cmd_rename_workspace(
    ctx: &Ctx<'_>,
    workspace: Target<WorkspaceId>,
    name: String,
) -> Outcome {
    let workspace = {
        let guard = ctx.tree.lock();
        match guard.resolve_workspace_target(workspace) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().rename_workspace(workspace, &name) {
        // Renames change what list-workspaces renders; same cue.
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::WorkspacesChanged),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_kill_workspace(ctx: &Ctx<'_>, workspace: Target<WorkspaceId>) -> Outcome {
    let killed = match kill_target(
        ctx,
        workspace,
        MuxTree::resolve_workspace_target,
        MuxTree::kill_workspace,
    ) {
        Ok(killed) => killed,
        Err(outcome) => return outcome,
    };
    // kill-session's line order, workspace-flavored: one
    // %window-close per killed window, then the session-set cue,
    // then the workspace-roster cue — unconditional, since the killed
    // workspace itself left the roster.
    notify_window_closes(Outcome::ok(ctx, ""), &killed.removed)
        .notifying(TmuxNotification::SessionsChanged)
        .notifying(TmuxNotification::WorkspacesChanged)
}

/// Snapshot the workspace roster for [`workspace_roster_changed`]: the
/// sorted id set plus the active pointer, enough to tell "the workspace
/// set (or its selection) changed" from "it did not".
pub(crate) fn workspace_roster_fingerprint(
    tree: &Arc<Mutex<MuxTree>>,
) -> (Vec<WorkspaceId>, Option<WorkspaceId>) {
    let guard = tree.lock();
    let mut ids = guard.workspaces();
    ids.sort();
    (ids, guard.active_workspace())
}

/// True when the workspace roster's fingerprint changed across a kill
/// cascade — a session's death can empty and remove its workspace, and
/// that removal is the workspace-roster cue clients re-query on.
pub(crate) fn workspace_roster_changed(
    tree: &Arc<Mutex<MuxTree>>,
    before: &(Vec<WorkspaceId>, Option<WorkspaceId>),
) -> bool {
    &workspace_roster_fingerprint(tree) != before
}
