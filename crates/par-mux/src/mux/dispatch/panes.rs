//! Pane command handlers: list, send-keys, kill, split, select, title,
//! info, exited-replay, clear-history, resize, swap, break, join, respawn.

use super::*;

pub(super) fn cmd_list_panes(ctx: &Ctx<'_>, window: Option<Target<WindowId>>) -> Outcome {
    // Wire contract: the bare form replies one line per pane, globally,
    // each just the pane id (`%N`) — the shape every existing client
    // parses. Geometry arrives via %layout-change pushes; there is no -F
    // (Phase 4 T4.E decision — push covers what the -F polling fallback
    // existed for).
    //
    // `list-panes -t <window>` (the attach card's criterion 2): one line
    // per pane of THAT window, in layout leaf order — the same
    // left-to-right/top-to-bottom order `LayoutTree::render` emits leaves
    // in, so `leaf` is the index a client maps tmux layout-string leaves
    // to pane ids by. Fixed positional shape (T4.E): `%N <leaf> <marker>`
    // where marker `*` is the window's active pane and `-` is every other
    // pane — the roster's fixed-prefix rule, and no -F.
    let guard = ctx.tree.lock();
    let Some(window_id) = window.map(|target| guard.resolve_window_target(target)) else {
        let body = guard
            .sessions()
            .iter()
            .filter_map(|s| guard.session(*s))
            .flat_map(|s| s.windows.clone())
            .filter_map(|w| guard.window(w))
            .flat_map(|w| w.panes())
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return Outcome::ok(ctx, &body);
    };
    let window_id = match window_id {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    let Some(window) = guard.window(window_id) else {
        return Outcome::err(ctx, &MuxError::NoSuchWindow(window_id).to_string());
    };
    let body = window
        .panes()
        .into_iter()
        .enumerate()
        .map(|(leaf, pane)| {
            let marker = if pane == window.active { '*' } else { '-' };
            format!("{pane} {leaf} {marker}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    Outcome::ok(ctx, &body)
}

pub(super) fn cmd_send_keys(
    ctx: &Ctx<'_>,
    pane: Target<PaneId>,
    keys: &SendKeysPayload,
) -> Outcome {
    let mut guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    match guard.pane_mut(pane) {
        Some(target) => {
            // Navigation/function keys encode against the pane's live modes
            // (DECCKM, kitty flags); the read guard drops before the write.
            let terminal = target.terminal();
            let bytes = keys.encode(|| terminal.read());
            match target.write(&bytes) {
                Ok(()) => Outcome::ok(ctx, ""),
                Err(err) => Outcome::err(ctx, &err.to_string()),
            }
        }
        None => Outcome::err(ctx, &format!("no such pane: {pane}")),
    }
}

pub(super) fn cmd_kill_pane(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    let pane = {
        let guard = ctx.tree.lock();
        match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    // kill_pane resolves and returns the owning window: afterwards the
    // pane (and its window membership) is gone and cannot be looked up.
    // The lock guard is let-bound so it is gone before the successor
    // lookup below takes the tree again (parking_lot is not reentrant).
    let workspace_fingerprint = workspace_roster_fingerprint(ctx.tree);
    let outcome = ctx.tree.lock().kill_pane(pane);
    match outcome {
        Ok((window_id, removed_session)) => {
            // A window whose last pane was killed is REMOVED by
            // tree.kill_pane (an emptied session goes with it) — its
            // clients learn that through %window-close, the same
            // notification kill-window sends. A surviving window keeps
            // an active pane to name, and gets the layout push.
            // Let-bound (not an `if let` scrutinee) so the lock guard is
            // gone before the else branch's workspace re-lock — the
            // scrutinee temporary would otherwise live through the whole
            // if/else (parking_lot is not reentrant).
            let active = ctx.tree.lock().window(window_id).map(|w| w.active);
            if let Some(active) = active {
                Outcome::ok(ctx, "").with_layout(window_id).notifying(
                    TmuxNotification::WindowPaneChanged {
                        window_id: window_id.to_string(),
                        pane_id: active.to_string(),
                    },
                )
            } else {
                let mut outcome = Outcome::ok(ctx, "").notifying(TmuxNotification::WindowClose {
                    window_id: window_id.to_string(),
                });
                if removed_session.is_some() {
                    // The window's closure emptied the session: the set of
                    // sessions changed, and tmux says so argument-less.
                    outcome = outcome.notifying(TmuxNotification::SessionsChanged);
                    if workspace_roster_changed(ctx.tree, &workspace_fingerprint) {
                        outcome = outcome.notifying(TmuxNotification::WorkspacesChanged);
                    }
                }
                outcome
            }
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

/// Resolve a `split-window -c` / `new-window -c` start directory: an
/// existing directory is used as-is; one that does not exist degrades to
/// home — the command's success may not depend on a directory this
/// process does not control (the restore path's rule for a gone persisted
/// cwd) — and the returned note names both so the pane says where it
/// landed instead of silently starting elsewhere.
pub(super) fn resolve_start_dir(start_dir: Option<&str>) -> (Option<PathBuf>, Option<String>) {
    let Some(raw) = start_dir else {
        return (None, None);
    };
    let dir = Path::new(raw);
    if dir.is_dir() {
        return (Some(dir.to_path_buf()), None);
    }
    let home = dirs::home_dir().unwrap_or_default();
    let note = home.is_dir().then(|| {
        format!(
            "\r\npar-mux: {raw} is gone; pane started in {}\r\n",
            home.display()
        )
    });
    (home.is_dir().then_some(home), note)
}

pub(super) fn cmd_split_window(
    ctx: &Ctx<'_>,
    target: AnyTarget,
    direction: SplitDirection,
    percent: u32,
    before: bool,
    start_dir: Option<&str>,
) -> Outcome {
    let (cwd, note) = resolve_start_dir(start_dir);
    // Two-phase spawn (ARC-022): resolve + reserve under the lock, fork/exec
    // off it, re-lock to insert, wire, and shape the layout. A target killed
    // while the pane spawned fails the insert (the pane is killed tree-side).
    let (plan, factory) = {
        let mut guard = ctx.tree.lock();
        // %N splits that pane, @N its window's active pane, $N the
        // session's active window's active pane, a name the pane-title match.
        let pane = match guard.resolve_split_target(target) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match guard.begin_split(
            pane,
            direction,
            percent as f32 / 100.0,
            cwd.as_deref(),
            before,
        ) {
            Ok(plan) => (plan, guard.factory()),
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match spawn_and_wire(ctx, &*factory, plan, None, note.as_deref()) {
        Ok((new_pane, window_id)) => {
            // split-window focuses the new pane (tmux semantics).
            Outcome::ok(ctx, &new_pane.to_string())
                .with_layout(window_id)
                .notifying(TmuxNotification::WindowPaneChanged {
                    window_id: window_id.to_string(),
                    pane_id: new_pane.to_string(),
                })
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_select_pane(
    ctx: &Ctx<'_>,
    pane: Target<PaneId>,
    title: Option<String>,
) -> Outcome {
    let mut guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    // The title lands first under the same lock: a failed select reports
    // the pane error and nothing else changed.
    let mut title_notification = None;
    if let Some(title) = title {
        let pane_mut = match guard.pane_mut(pane) {
            Some(p) => p,
            None => return Outcome::err(ctx, &MuxError::NoSuchPane(pane).to_string()),
        };
        if pane_mut.set_user_title(&title) {
            // The notification carries the new USER title (empty = cleared);
            // the display title remains user-or-OSC per the precedence rule.
            title_notification = Some(TmuxNotification::PaneTitleChanged {
                pane_id: pane.to_string(),
                title,
            });
        }
    }
    match guard.select_pane(pane) {
        Ok(window_id) => {
            let mut outcome = Outcome::ok(ctx, "").with_layout(window_id).notifying(
                TmuxNotification::WindowPaneChanged {
                    window_id: window_id.to_string(),
                    pane_id: pane.to_string(),
                },
            );
            if let Some(notification) = title_notification {
                outcome = outcome.notifying(notification);
            }
            outcome
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_pane_title(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    // Wire contract: the reply body is exactly one line — the effective
    // title (user `-T` title when set, else the pane terminal's current
    // OSC 0/2 title). An empty body means neither is set.
    let guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    match guard.pane(pane) {
        Some(pane) => Outcome::ok(ctx, &pane.effective_title()),
        None => Outcome::err(ctx, &MuxError::NoSuchPane(pane).to_string()),
    }
}

pub(super) fn cmd_pane_info(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    // Wire contract: one line, `%N @W COLSxROWS [cmd=<base64>]
    // [exited=<code|?>]` — the pane's window, its terminal's current grid
    // size, and, when knowable, the pane's foreground command name
    // (deepest descendant of its child process) for close-confirmation
    // prompts. The cmd token may be absent (Windows table, unreadable
    // argv, a name with control characters or over the cap, or a reaped
    // child). `exited=` rides only for a held-dead pane — `?` when the exit
    // code was unreadable — so a client that connects after `%pane-exited`
    // can still render the remain-on-exit state (ARC-095). Both are
    // whitespace-free `key=value` tail tokens, so older clients keep
    // parsing the fixed prefix.
    let (pane, window, cols, rows, child_pid, dead, exit_code) = {
        let guard = ctx.tree.lock();
        let pane = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let (Some(target), Some(window)) = (guard.pane(pane), guard.window_of_pane(pane)) else {
            return Outcome::err(ctx, &MuxError::NoSuchPane(pane).to_string());
        };
        let (cols, rows) = target.terminal().read().size();
        (
            pane,
            window,
            cols,
            rows,
            target.child_pid(),
            target.dead(),
            target.exit_code(),
        )
    };
    // The process table snapshot reads the whole OS process list; taking
    // it outside the tree lock keeps a slow read from stalling clients.
    let mut line = format!("{pane} {window} {cols}x{rows}");
    if let Some(name) = child_pid
        .and_then(|pid| ProcessTable::snapshot().and_then(|table| table.foreground_command(pid)))
    {
        line.push_str(" cmd=");
        line.push_str(&base64::engine::general_purpose::STANDARD.encode(name));
    }
    if dead {
        line.push_str(" exited=");
        match exit_code {
            Some(code) => line.push_str(&code.to_string()),
            None => line.push('?'),
        }
    }
    Outcome::ok(ctx, &line)
}

/// ENH-042: queue the held panes' `%pane-exited` lines on the issuing
/// client's own channel — ahead of this command's (empty) reply, the same
/// framing registration replay uses. Nothing is broadcast.
pub(super) fn cmd_pane_exited_replay(
    ctx: &Ctx<'_>,
    issuer: Option<&SyncSender<String>>,
) -> Outcome {
    let lines = replay_pane_exited_lines(&ctx.tree.lock());
    if let Some(tx) = issuer {
        for line in lines {
            if tx.send(line).is_err() {
                break;
            }
        }
    }
    Outcome::ok(ctx, "")
}

pub(super) fn cmd_clear_history(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    let guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    match guard.pane(pane) {
        Some(target) => {
            // The clear rides the emulator's own ED 3 path, so graphics
            // teardown and the ScreenCleared event stay in sync with the
            // core instead of a parallel grid-only reimplementation.
            target.with_terminal_mut(|term| term.process(b"\x1b[3J"));
            Outcome::ok(ctx, "")
        }
        None => Outcome::err(ctx, &MuxError::NoSuchPane(pane).to_string()),
    }
}

pub(super) fn cmd_resize_pane(
    ctx: &Ctx<'_>,
    pane: Target<PaneId>,
    adjustment: ResizeAdjustment,
) -> Outcome {
    let outcome = {
        let mut guard = ctx.tree.lock();
        let pane = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match adjustment {
            ResizeAdjustment::Relative { direction, cells } => {
                guard.resize_pane(pane, direction, cells)
            }
            ResizeAdjustment::Absolute { cols, rows } => {
                guard.resize_pane_absolute(pane, cols, rows)
            }
            ResizeAdjustment::Zoom => guard.zoom_pane(pane),
        }
    };
    match outcome {
        Ok(window_id) => Outcome::ok(ctx, "").with_layout(window_id),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_swap_panes(
    ctx: &Ctx<'_>,
    target: Target<PaneId>,
    source: Target<PaneId>,
) -> Outcome {
    let (target, source) = {
        let guard = ctx.tree.lock();
        match (
            guard.resolve_pane_target(target),
            guard.resolve_pane_target(source),
        ) {
            (Ok(target), Ok(source)) => (target, source),
            (Err(err), _) | (_, Err(err)) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().swap_panes(target, source) {
        Ok(window_id) => Outcome::ok(ctx, "").with_layout(window_id),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_break_pane(
    ctx: &Ctx<'_>,
    source: Target<PaneId>,
    name: Option<String>,
) -> Outcome {
    let source = {
        let guard = ctx.tree.lock();
        match guard.resolve_pane_target(source) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    let name = name.unwrap_or_else(|| "0".to_string());
    // Bound to a local so the scrutinee's tree-lock temporary drops here:
    // the match arm below re-locks to read the new window's layout triple.
    let broken = ctx.tree.lock().break_pane(source, &name);
    match broken {
        Ok((window_id, source_window, source_closed)) => {
            // The new window is announced like new-window's; the source's
            // frame only exists while the source does.
            let mut outcome = Outcome::ok(ctx, &window_id.to_string())
                .notifying(window_add_notification(ctx.tree, window_id));
            if source_closed {
                outcome = outcome.notifying(TmuxNotification::WindowClose {
                    window_id: source_window.to_string(),
                });
            } else {
                outcome = outcome.with_layout(source_window);
            }
            outcome
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

pub(super) fn cmd_join_pane(
    ctx: &Ctx<'_>,
    source: Target<PaneId>,
    target: Target<PaneId>,
    direction: SplitDirection,
    percent: u32,
) -> Outcome {
    let (source, target) = {
        let guard = ctx.tree.lock();
        match (
            guard.resolve_pane_target(source),
            guard.resolve_pane_target(target),
        ) {
            (Ok(source), Ok(target)) => (source, target),
            (Err(err), _) | (_, Err(err)) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    let workspace_fingerprint = workspace_roster_fingerprint(ctx.tree);
    // Let-bound so the lock guard drops before the arm's re-lock (the
    // scrutinee temporary would otherwise live through the match —
    // parking_lot is not reentrant).
    let joined = ctx
        .tree
        .lock()
        .join_pane(source, target, direction, percent as f32 / 100.0);
    match joined {
        Ok((target_window, source_window, source_closed, removed_session)) => {
            // Both windows' layouts changed — the destination grew a pane
            // and the source lost one (or closed outright).
            let mut outcome = Outcome::ok(ctx, "").with_layout(target_window);
            if source_closed {
                outcome = outcome.notifying(TmuxNotification::WindowClose {
                    window_id: source_window.to_string(),
                });
            } else {
                outcome = outcome.with_layout(source_window);
            }
            if removed_session.is_some() {
                // The same argument-less cue kill-pane's cascade sends.
                outcome = outcome.notifying(TmuxNotification::SessionsChanged);
                if workspace_roster_changed(ctx.tree, &workspace_fingerprint) {
                    outcome = outcome.notifying(TmuxNotification::WorkspacesChanged);
                }
            }
            outcome
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

/// `respawn-pane -t %N [-k] [-c dir] [command]`: restart the pane's
/// process in place — same pane id, window, and layout; a fresh
/// terminal. Two-phase (ARC-022): resolve and refuse a live pane without
/// `-k` under the lock, spawn off it, re-lock to swap. `%pane-respawned`
/// is the cue clients clear their exited-state chrome on.
pub(super) fn cmd_respawn_pane(
    ctx: &Ctx<'_>,
    pane: Target<PaneId>,
    kill: bool,
    start_dir: Option<&str>,
    command: Option<String>,
) -> Outcome {
    let (cwd, note) = resolve_start_dir(start_dir);
    // Phase 1: resolve, refuse a live pane, snapshot the restart plan.
    let (plan, factory) = {
        let mut guard = ctx.tree.lock();
        let pane = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match guard.begin_respawn(pane, kill, command, cwd.as_deref()) {
            Ok(plan) => {
                // `-k` of a live pane: the old process keeps running and
                // emitting under this id until phase 3 kills it, so stop
                // forwarding it now — its output must not interleave with
                // the replacement's (ARC-089). A failed respawn re-attaches.
                if let Some(old) = guard.pane_mut(plan.pane_id) {
                    old.detach_output();
                }
                (plan, guard.factory())
            }
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    let pane_id = plan.pane_id;
    let command = plan.command.clone();
    // Phases 2 and 3: spawn the replacement off the tree lock with its sink
    // live, then swap it in.
    let outcome = spawn_and_wire(ctx, &*factory, plan, command.as_deref(), note.as_deref());
    if outcome.is_err() {
        // The spawn or the swap failed: the old pane (if it still exists)
        // is the live pane again, so it resumes forwarding as before the
        // command.
        if let Some(pane) = ctx.tree.lock().pane_mut(pane_id) {
            pane.on_output_sink(OutputSink(Arc::new(pane_output_sink(ctx.clients, pane_id))));
        }
    }
    match outcome {
        Ok(pane_id) => Outcome::ok(ctx, "").notifying(TmuxNotification::PaneRespawned {
            pane_id: pane_id.to_string(),
        }),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}
