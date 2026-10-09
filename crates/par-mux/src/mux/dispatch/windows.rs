//! Window command handlers: new, select, kill, rename, list, move, swap.

use super::*;

/// The window group's router; `route_command` sends only this group's
/// variants here.
pub(super) fn route_window_command(ctx: &Ctx<'_>, command: MuxCommand) -> Outcome {
    match command {
        MuxCommand::NewWindow {
            target,
            name,
            start_dir,
        } => cmd_new_window(ctx, target, name, start_dir.as_deref()),
        MuxCommand::SelectWindow { window } => cmd_select_window(ctx, window),
        MuxCommand::KillWindow { window } => cmd_kill_window(ctx, window),
        MuxCommand::RenameWindow { window, name } => cmd_rename_window(ctx, window, name),
        MuxCommand::ListWindows { session, all } => cmd_list_windows(ctx, session, all),
        MuxCommand::MoveWindow { source, index } => cmd_move_window(ctx, source, index),
        MuxCommand::SwapWindows { source, target } => cmd_swap_windows(ctx, source, target),
        other => unreachable!("route_command sent a non-window command: {other:?}"),
    }
}

pub(super) fn cmd_move_window(ctx: &Ctx<'_>, source: Target<WindowId>, index: usize) -> Outcome {
    let source = {
        let guard = ctx.tree.lock();
        match guard.resolve_window_target(source) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().move_window(source, index) {
        // No layout changed — the reorder cue is the argument-less
        // sessions-changed; clients re-query list-windows.
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::SessionsChanged),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_swap_windows(
    ctx: &Ctx<'_>,
    source: Target<WindowId>,
    target: Target<WindowId>,
) -> Outcome {
    let (source, target) = {
        let guard = ctx.tree.lock();
        match (
            guard.resolve_window_target(source),
            guard.resolve_window_target(target),
        ) {
            (Ok(source), Ok(target)) => (source, target),
            (Err(err), _) | (_, Err(err)) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().swap_windows(source, target) {
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::SessionsChanged),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_new_window(
    ctx: &Ctx<'_>,
    target: Option<AnyTarget>,
    name: Option<String>,
    start_dir: Option<&str>,
) -> Outcome {
    let name = name.unwrap_or_else(|| "0".to_string());
    let (cwd, note) = resolve_start_dir(start_dir);
    // Two-phase spawn (ARC-022): resolve + reserve under the lock, fork/exec
    // off it, re-lock to insert and wire. A session killed while the pane
    // spawned fails the insert (the pane is killed tree-side).
    let (plan, factory) = {
        let mut guard = ctx.tree.lock();
        // Bare `new-window` targets the most-recently-created
        // session — ids are monotonic and the registry keeps
        // insertion order, so the last entry is the newest.
        let Some(target) =
            target.or_else(|| guard.sessions().last().copied().map(AnyTarget::Session))
        else {
            return Outcome::err(ctx, "no sessions exist");
        };
        // $N (and names) append; @N/%N insert right after that window.
        let (session, insert_after) = match guard.resolve_new_window_target(target) {
            Ok(resolved) => resolved,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match guard.begin_window(
            session,
            &name,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            cwd.as_deref(),
            insert_after,
        ) {
            Ok(plan) => (plan, guard.factory()),
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match spawn_and_wire(ctx, &*factory, plan, None, note.as_deref()) {
        Ok(window_id) => Outcome::ok(ctx, &window_id.to_string())
            .notifying(window_add_notification(ctx.tree, window_id)),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_select_window(ctx: &Ctx<'_>, window: Target<WindowId>) -> Outcome {
    let (window, active, session_id, selection_moved, resized) = {
        let mut guard = ctx.tree.lock();
        let window = match guard.resolve_window_target(window) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let session_id = guard.session_of_window(window);
        let was_active = session_id
            .and_then(|s| guard.session(s))
            .and_then(|s| s.windows.get(s.active).copied());
        if let Err(err) = guard.select_window(window) {
            return Outcome::err(ctx, &err.to_string());
        }
        // Shared-selection sync: when the move touches the displayed
        // session, every render client attached to it follows to the new
        // active window, and `%session-window-changed` tells them so. A
        // background session's pointer moves silently — clients displaying
        // it are not yanked.
        let displayed = guard.active_session();
        let selection_moved = was_active != Some(window) && displayed == session_id;
        let resized = if selection_moved {
            let Some(session_id) = session_id else {
                return Outcome::err(ctx, "window has no session");
            };
            guard.follow_session_window(session_id, window)
        } else {
            Vec::new()
        };
        (
            window,
            guard.window(window).map(|w| w.active),
            session_id,
            selection_moved,
            resized,
        )
    };
    let mut result = Outcome::ok(ctx, "");
    if selection_moved {
        if let Some(session_id) = session_id {
            result = result.notifying(TmuxNotification::SessionWindowChanged {
                session_id: session_id.to_string(),
                window_id: window.to_string(),
            });
        }
    }
    for window_id in resized {
        result = result.with_layout(window_id);
    }
    if let Some(pane) = active {
        result = result.notifying(TmuxNotification::WindowPaneChanged {
            window_id: window.to_string(),
            pane_id: pane.to_string(),
        });
    }
    result
}

pub(super) fn cmd_kill_window(ctx: &Ctx<'_>, window: Target<WindowId>) -> Outcome {
    let killed = match kill_target(
        ctx,
        window,
        MuxTree::resolve_window_target,
        MuxTree::kill_window,
    ) {
        Ok(killed) => killed,
        Err(outcome) => return outcome,
    };
    let windows = [killed.id];
    if killed.removed.is_none() {
        return notify_window_closes(Outcome::ok(ctx, ""), &windows);
    }
    // The cascade reached the session — same argument-less cue
    // kill-pane's cascade sends, so one handler covers both.
    notify_kill_cascade(Outcome::ok(ctx, ""), &windows, killed.workspaces_changed)
}

pub(super) fn cmd_rename_window(ctx: &Ctx<'_>, window: Target<WindowId>, name: String) -> Outcome {
    let window = {
        let guard = ctx.tree.lock();
        match guard.resolve_window_target(window) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().rename_window(window, &name) {
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::WindowRenamed {
            window_id: window.to_string(),
            name,
        }),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_list_windows(
    ctx: &Ctx<'_>,
    session: Option<Target<SessionId>>,
    all: bool,
) -> Outcome {
    // Wire contract: the bare form replies one line per window, globally,
    // as `@N: name` — the shape every existing client parses.
    //
    // `list-windows -t <session>` (the attach card's criterion 1): one
    // line per window of THAT session, in the session's window order (the
    // order `%sessions-changed` asks clients to re-query), as a fixed
    // positional shape (T4.E): `@N <marker> <name>` — marker `*` for the
    // session's active window, `-` otherwise; the name is the line
    // remainder so a spaced name survives (same rule as the roster's
    // entry and `@N: name`'s own split-on-first-colon). No -F.
    //
    // `list-windows -a` (ARC-125): every session's windows in session-id then
    // window order, each `-t` row prefixed with its session id —
    // `$S @N <marker> <name>` — so a client finds a window's owning
    // session in one round trip.
    let guard = ctx.tree.lock();
    if all {
        let mut sessions = guard.sessions();
        sessions.sort();
        let body = sessions
            .iter()
            .filter_map(|s| guard.session(*s))
            .flat_map(|session| {
                session_window_rows(&guard, session)
                    .into_iter()
                    .map(move |row| format!("{} {row}", session.id))
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Outcome::ok(ctx, &body);
    }
    let Some(session_id) = session.map(|target| guard.resolve_session_target(target)) else {
        let body = guard
            .sessions()
            .iter()
            .filter_map(|s| guard.session(*s))
            .flat_map(|s| s.windows.clone())
            .filter_map(|w| guard.window(w))
            .map(|w| format!("{}: {}", w.id, w.name))
            .collect::<Vec<_>>()
            .join("\n");
        return Outcome::ok(ctx, &body);
    };
    let session_id = match session_id {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    let Some(session) = guard.session(session_id) else {
        return Outcome::err(ctx, &MuxError::NoSuchSession(session_id).to_string());
    };
    let body = session_window_rows(&guard, session).join("\n");
    Outcome::ok(ctx, &body)
}

/// One session's `list-windows -t` rows, `@N <marker> <name>`, in the
/// session's window order.
fn session_window_rows(guard: &MuxTree, session: &crate::mux::tree::MuxSession) -> Vec<String> {
    session
        .windows
        .iter()
        .enumerate()
        .map(|(index, window_id)| {
            let marker = if index == session.active { '*' } else { '-' };
            let name = guard
                .window(*window_id)
                .map(|w| w.name.as_str())
                .unwrap_or_default();
            format!("{window_id} {marker} {name}")
        })
        .collect()
}
