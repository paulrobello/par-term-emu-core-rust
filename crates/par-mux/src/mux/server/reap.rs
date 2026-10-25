//! Dead-pane reaping: announce exits, then hold or remove each pane.

use super::*;

/// The keep-one-tab respawn's crash-loop guard: a last pane whose
/// process lived less than this is held dead instead of restarted, so a
/// command that exits at once (a broken shell rc, a typo'd agent) cannot
/// respawn on every reap tick.
pub(super) const RESPAWN_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(1);

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
/// window closes when it was the last pane, and an emptied session cascades
/// with it). The removal runs the FULL cascade: a pane that is its
/// workspace's last is removed too, emptying the workspace and cascading it
/// closed — Paul's rule (2026-10-18) that a pane closes when its child
/// exits — unless that workspace opted into keep-one-tab
/// (`respawn_last`, the `set-workspace-respawn` option), which holds its
/// last pane for the respawn below instead. Explicit `kill-*` commands keep
/// their full cascade. The PTY reader flips
/// `is_running` on EOF; this pass (the accept loop's idle tick,
/// [`REAP_INTERVAL`]) observes the death ONCE, records the exit code while
/// the child handle can still be asked, and broadcasts `%pane-exited` — the
/// cue a client uses to show "Process exited (code N)".
///
/// Under auto-remove the `%pane-exited` is broadcast ahead of the removal
/// notifications, so an observer sees the death before the geometry: a
/// surviving window gets `%layout-change` + `%window-pane-changed`, an
/// emptied one gets `%window-close`, then `%sessions-changed` when the
/// cascade removed a session — the same line order an explicit `kill-pane`
/// produces. A held pane gets only its `%pane-exited`. A persisting daemon
/// whose every pane is dead (or gone) exits once the last client is gone
/// (see [`EXIT_EMPTY_GRACE`] and [`MuxTree::all_panes_dead`]) — held panes
/// are for clients that might come back, and with nobody connected and
/// nothing alive the daemon collects itself.
///
/// Keep-one-tab respawn: a workspace with `respawn_last` on (the
/// `set-workspace-respawn` option) restarts its LAST pane instead of
/// holding it — under either remain-on-exit setting — through the
/// `respawn-pane` command itself, after the `%pane-exited` broadcast, so
/// clients see the death, then `%pane-respawned`, on the same pane id. A
/// pane that lived under [`RESPAWN_MIN_AGE`] is held dead instead.
pub(super) fn reap_dead_panes(
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Clients,
    config: Option<&Arc<Mutex<crate::mux::config::EffectiveConfig>>>,
    persist: Option<&Sender<(SaveOrigin, PersistState)>>,
    epoch: &OutputEpoch,
) {
    // No published config (an embedded `run()` server) keeps the library's
    // historical held-dead behavior; the daemon binary always publishes one,
    // and ITS default (the product default) is auto-remove.
    // The idle tick also sweeps expired upload staging (uploads.rs: every
    // verb sweeps too; this tick is what eventually deletes an abandoned
    // upload whose client never sent another verb).
    crate::mux::uploads::sweep_expired();
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
            // Keep-one-tab opt-in: a workspace with `respawn_last` on
            // holds its last pane dead (window, session, and workspace
            // stay; the respawn below restarts it). Every other pane —
            // including a workspace's last — runs the kill-pane cascade.
            // The test and the kill share one lock, so of two same-tick
            // deaths in a two-pane workspace the first is removed and the
            // second, now the last, is held only if the workspace opted
            // in. Let-bound, NOT an `if let` scrutinee: the scrutinee's
            // lock temporary would live through the body (parking_lot is
            // not reentrant) and the window lookup below would deadlock
            // on the tree — the same trap cmd_kill_pane documents.
            let killed = {
                let mut guard = tree.lock();
                if guard.respawns_as_workspace_last_pane(pane_id) {
                    continue;
                }
                guard.kill_pane(pane_id)
            };
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
    // Keep-one-tab respawn, evaluated after the removals: of two same-tick
    // deaths the survivor is the last pane by now. Collected under one
    // lock, dispatched after it drops (the command takes the tree itself).
    let respawns: Vec<PaneId> = {
        let guard = tree.lock();
        just_died
            .iter()
            .map(|&(pane_id, _)| pane_id)
            .filter(|&pane_id| {
                guard.respawns_as_workspace_last_pane(pane_id)
                    && guard
                        .pane(pane_id)
                        .is_some_and(|pane| pane.age() >= RESPAWN_MIN_AGE)
            })
            .collect()
    };
    for pane_id in respawns {
        let ctx = Ctx {
            client_id: None,
            tree,
            clients,
            command_number: 0,
            shutdown: None,
            config: config.map(|config| &**config),
            output_epoch: epoch,
        };
        // The reply has no reader; the command broadcasts
        // `%pane-respawned` and saves the state itself.
        let _ = dispatch_command(
            crate::mux::command::MuxCommand::RespawnPane {
                pane: crate::mux::ids::Target::Id(pane_id),
                kill: false,
                start_dir: None,
                command: None,
            },
            &ctx,
            persist,
            None,
        );
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
