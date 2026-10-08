//! Dead-pane reaping: announce exits, then hold or remove each pane.

use super::*;

/// One auto-remove removal: the window that held the pane, its active pane
/// when the window survived (the `%window-pane-changed` target), and
/// whether the cascade removed a session (the `%sessions-changed` cue).
pub(super) struct Removal {
    window_id: WindowId,
    surviving_active: Option<PaneId>,
    session_removed: bool,
}

/// Announce panes whose child exited, then either HOLD them (remain-on-exit
/// `= true`: the pane, its window, and its frozen screen stay in the tree so
/// `respawn-pane` can restart the process in place — the recovery path a
/// client offers on a crashed program) or REMOVE them (remain-on-exit
/// `= false`, the default: the kill-pane contract runs — the pane goes, its
/// window closes when it was the last pane, and an emptied session (and its
/// workspace, when that empties too) cascade with it). The PTY reader flips
/// `is_running` on EOF; this pass (the accept loop's idle tick,
/// [`REAP_INTERVAL`]) observes the death ONCE, records the exit code while
/// the child handle can still be asked, and broadcasts `%pane-exited` — the
/// cue a client uses to show "Process exited (code N)".
///
/// Under auto-remove the `%pane-exited` is broadcast ahead of the removal
/// notifications, so an observer sees the death before the geometry: a
/// surviving window gets `%layout-change` + `%window-pane-changed`, an
/// emptied one gets `%window-close`, then `%sessions-changed` when the
/// cascade removed a session, then `%workspaces-changed` when that removal
/// emptied a workspace — the same line order an explicit `kill-pane`
/// produces. A persisting daemon whose every pane is dead (or gone) exits
/// once the last client is gone (see [`EXIT_EMPTY_GRACE`] and
/// [`MuxTree::all_panes_dead`]) — held panes are for clients that might
/// come back, and with nobody connected and nothing alive the daemon
/// collects itself.
pub(super) fn reap_dead_panes(
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Clients,
    config: Option<&Arc<Mutex<crate::mux::config::EffectiveConfig>>>,
    persist: Option<&Sender<(SaveOrigin, PersistState)>>,
) {
    // No published config (an embedded `run()` server) keeps the library's
    // historical held-dead behavior; the daemon binary always publishes one,
    // and ITS default (the product default) is auto-remove.
    let remain_on_exit = config
        .map(|config| config.lock().remain_on_exit)
        .unwrap_or(true);
    let mut just_died: Vec<(PaneId, Option<i32>)> = Vec::new();
    {
        let mut guard = tree.lock();
        // Enumerate first, then poll mutably: poll_running consults the OS
        // child handle when the reader flag still claims alive — on Windows
        // ConPTY that flag never flips after an exit (conhost keeps the pipe
        // open), so is_running alone would never observe the death there.
        let panes: Vec<PaneId> = guard.panes.keys().copied().collect();
        for pane_id in panes {
            let Some(pane) = guard.pane_mut(pane_id) else {
                continue;
            };
            if pane.dead() {
                continue;
            }
            if !pane.poll_running() {
                pane.mark_dead();
                just_died.push((pane_id, pane.exit_code()));
            }
        }
    }
    // Auto-remove (remain-on-exit = false): the kill-pane contract runs
    // after the deaths are known, so `%pane-exited` can be broadcast ahead
    // of the geometry notifications — the same order an explicit kill-pane
    // of a just-died pane produces. The pane is already dead, so the
    // removal's kill is a no-op (SEC-125); the cascade handles the
    // window/session/workspace chain.
    let mut removals: Vec<Removal> = Vec::new();
    let mut workspaces_changed = false;
    if !remain_on_exit && !just_died.is_empty() {
        let fingerprint = workspace_roster_fingerprint(tree);
        for &(pane_id, _) in &just_died {
            // Let-bound, NOT an `if let` scrutinee: the scrutinee's lock
            // temporary would live through the body (parking_lot is not
            // reentrant) and the window lookup below would deadlock on the
            // tree — the same trap cmd_kill_pane documents.
            let killed = tree.lock().kill_pane(pane_id);
            if let Ok((window_id, removed_session)) = killed {
                let active = tree.lock().window(window_id).map(|w| w.active);
                removals.push(Removal {
                    window_id,
                    surviving_active: active,
                    session_removed: removed_session.is_some(),
                });
            }
            // An Err (the pane already removed by an explicit kill-pane or
            // respawn between the death and this pass) removes nothing twice.
        }
        workspaces_changed = workspace_roster_changed(tree, &fingerprint);
        // The removals re-fit their windows' surviving panes.
        crate::mux::tree::deliver_pending_observer_events(tree);
    }
    // The death notices first, queued ahead of any removal geometry.
    for &(pane_id, exit_code) in &just_died {
        broadcast_notification(
            clients,
            &TmuxNotification::PaneExited {
                pane_id: pane_id.to_string(),
                exit_code,
            },
        );
    }
    // Then the removal contract, mirroring the dispatcher's kill-pane tail.
    for removal in &removals {
        match removal.surviving_active {
            Some(active) => {
                broadcast_layout_change(tree, clients, removal.window_id);
                broadcast_notification(
                    clients,
                    &TmuxNotification::WindowPaneChanged {
                        window_id: removal.window_id.to_string(),
                        pane_id: active.to_string(),
                    },
                );
            }
            None => {
                broadcast_notification(
                    clients,
                    &TmuxNotification::WindowClose {
                        window_id: removal.window_id.to_string(),
                    },
                );
                if removal.session_removed {
                    broadcast_notification(clients, &TmuxNotification::SessionsChanged);
                }
            }
        }
    }
    if workspaces_changed {
        broadcast_notification(clients, &TmuxNotification::WorkspacesChanged);
    }
    if !just_died.is_empty() {
        if let Some(tx) = persist {
            // Same off-lock capture discipline as the command path
            // (ARC-032): collect handles under the lock, walk grids after.
            let capture = tree.lock().collect_persist_capture();
            let state = capture.capture();
            let _ = tx.send((SaveOrigin::Reap, state));
        }
    }
}
