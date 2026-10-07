//! Broadcast and reply sinks: client fan-out, contained dispatch, pane
//! output wiring, layout/exit replays, and capture ranges.

use super::*;

/// Push one line to every connected client, dropping the senders whose
/// client has gone — the single fan-out every broadcast and sink shares.
///
/// ARC-011: the queues are bounded at [`CLIENT_QUEUE_DEPTH`]; a client
/// whose queue is full (it stopped reading the socket) is EVICTED here,
/// tmux's policy for a control client that stops draining. Eviction also
/// raises the client's flag so its own connection threads tear it down:
/// dropping the queue sender alone cannot free a wedged client — its
/// writer stays blocked in a full socket buffer and its reader parked on
/// input, retaining the queued lines (ENH-012, measured live). The flagged
/// threads exit, the queue drops, and the client observes a clean
/// disconnect within about two [`EVICTION_POLL`]s. On Windows the flag
/// alone is not enough — named pipes never wake a parked reader or writer
/// on a timeout — so eviction also aborts the blocked I/O through the
/// entry's [`ConnectionAbort`].
pub(crate) fn push_to_clients(clients: &Clients, line: String) {
    push_to_clients_skipping(clients, None, line);
}

/// [`push_to_clients`] skipping one client (the announcement's subject, who
/// must not hear about itself). Eviction rules are identical.
pub(super) fn push_to_clients_except(clients: &Clients, skip: u64, line: String) {
    push_to_clients_skipping(clients, Some(skip), line);
}

pub(super) fn push_to_clients_skipping(clients: &Clients, skip: Option<u64>, line: String) {
    clients.lock().retain(|(id, tx, evicted, abort)| {
        if skip == Some(*id) {
            return true;
        }
        match tx.try_send(line.clone()) {
            Ok(()) => true,
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                log::warn!(
                    "par-mux: evicting client {id}: {CLIENT_QUEUE_DEPTH} broadcast lines \
                     undelivered (queue full — stalled, or draining slower than the burst)"
                );
                evicted.store(true, Ordering::Relaxed);
                abort.cancel_blocked_io();
                false
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => false,
        }
    });
}

/// Execute one command with panic containment (QA-113): a panicking
/// handler becomes an error block for the issuer plus an error log,
/// instead of tearing down the client thread with no reply and no trace.
/// The tree lock is a parking_lot guard, so unwinding releases it — the
/// server keeps serving whatever survived the panic.
pub(super) fn dispatch_contained(
    command: crate::mux::command::MuxCommand,
    ctx: &Ctx<'_>,
    persist: Option<&Sender<(SaveOrigin, PersistState)>>,
    issuer: Option<&SyncSender<String>>,
) -> String {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch_command(command, ctx, persist, issuer)
    })) {
        Ok(reply) => reply,
        Err(payload) => {
            let message = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            log::error!(
                "par-mux: command {} panicked: {message}",
                ctx.command_number
            );
            emit_block(ctx.command_number, "internal error", false)
        }
    }
}

/// Push one notification line to every connected client.
///
/// The lifecycle counterpart of [`broadcast_layout_change`]: a client that
/// did not issue the mutating command still learns about the window it
/// affected, which is what a push-driven sync layer consumes.
pub(crate) fn broadcast_notification(clients: &Clients, notification: &TmuxNotification) {
    push_to_clients(clients, emit(notification));
}

/// The per-pane output sink: PTY bytes become `%output` lines pushed to every
/// connected client as they are produced.
///
/// Shared by pane creation (`new-session`/`new-window`/`split-window` wiring)
/// and the restore path — a restored pane pushes output exactly like a
/// freshly created one.
pub(crate) fn pane_output_sink(
    clients: &Clients,
    pane_id: PaneId,
) -> impl Fn(&[u8]) + Send + Sync + 'static {
    let sinks = Arc::clone(clients);
    move |bytes: &[u8]| {
        let line = emit(&TmuxNotification::Output {
            pane_id: pane_id.to_string(),
            data: bytes.to_vec(),
        });
        push_to_clients(&sinks, line);
    }
}

/// Wire every pane in `tree` to push its output to `clients` — the restore
/// path's counterpart of the dispatcher's spawn-and-wire. Restore is exempt
/// from carrying the sink in the spawn context (ARC-103): `persist` spawns
/// every pane before `clients` exists, and no client can connect before the
/// accept loop starts, so no audience exists for the early bytes. They
/// still land in the grid that clients seed from.
pub(super) fn wire_all_pane_outputs(tree: &Arc<Mutex<MuxTree>>, clients: &Clients) {
    let pane_ids: Vec<PaneId> = {
        let guard = tree.lock();
        guard
            .sessions()
            .iter()
            .filter_map(|s| guard.session(*s))
            .flat_map(|s| s.windows.clone())
            .filter_map(|w| guard.window(w))
            .flat_map(|w| w.panes())
            .collect()
    };
    let mut guard = tree.lock();
    for pane_id in pane_ids {
        if let Some(pane) = guard.pane_mut(pane_id) {
            pane.on_output_sink(OutputSink(Arc::new(pane_output_sink(clients, pane_id))));
        }
    }
}

/// Push a `%layout-change` for `window_id` to every connected client.
///
/// tmux subscribers re-render their local pane grid from the wire layout
/// string; the visible-layout copy and raw flags mirror tmux's frame shape.
/// While a pane is zoomed (`resize-pane -Z`) the layout string stays the
/// true (untouched) tree — that is what makes unzoom restore exact — and
/// the zoom shows in the other two fields instead: the visible layout is
/// the zoomed pane alone at full extent, and the raw flags carry `Z`
/// (tmux's zoom flag).
/// Render the `%layout-change` line for `window_id` against `tree` — the
/// line-rendering half of [`broadcast_layout_change`], shared with ENH-037's
/// registration replay so the broadcast and replay forms cannot drift.
/// `None` when the window is gone.
pub(super) fn render_layout_change(tree: &MuxTree, window_id: WindowId) -> Option<String> {
    let window = tree.window(window_id)?;
    let layout = window
        .layout
        .render(0, 0, window.cols as usize, window.rows as usize);
    let (visible_layout, raw_flags) = match window.zoomed {
        Some(pane) => (
            // The single-pane form render_node produces for a leaf.
            format!("0000,{}x{},0,0,{}", window.cols, window.rows, pane.0),
            "Z".to_string(),
        ),
        None => (layout.clone(), String::new()),
    };
    Some(emit(&TmuxNotification::LayoutChange {
        window_id: window_id.to_string(),
        window_layout: layout,
        window_visible_layout: visible_layout,
        window_raw_flags: raw_flags,
    }))
}

pub(crate) fn broadcast_layout_change(
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Clients,
    window_id: WindowId,
) {
    let line = {
        let guard = tree.lock();
        render_layout_change(&guard, window_id)
    };
    if let Some(line) = line {
        push_to_clients(clients, line);
    }
}

/// The `%pane-exited` half of the held-state replay: one line per held
/// pane, sorted by id, each with its trailing newline. Shared by the
/// registration replay and the `pane-exited-replay` command (ENH-042).
/// Called under the tree lock.
pub(crate) fn replay_pane_exited_lines(tree: &MuxTree) -> Vec<String> {
    let mut exits: Vec<(PaneId, String)> = tree
        .panes
        .iter()
        .filter(|(_, pane)| pane.dead())
        .map(|(id, pane)| {
            let line = emit(&TmuxNotification::PaneExited {
                pane_id: id.to_string(),
                exit_code: pane.exit_code(),
            });
            (*id, line)
        })
        .collect();
    exits.sort_by_key(|(id, _)| *id);
    exits.into_iter().map(|(_, line)| line).collect()
}

/// ENH-037: the held state a client that registers now has missed — one
/// `%pane-exited` line per held pane and one `%layout-change` per zoomed
/// window, each sorted by id for deterministic tests. Called under the tree
/// lock; every line carries its trailing newline, ready for the client's
/// own queue ahead of its first command's reply.
pub(super) fn held_state_replay_lines(tree: &MuxTree) -> Vec<String> {
    let mut lines = replay_pane_exited_lines(tree);

    let mut zoomed: Vec<WindowId> = tree
        .sessions()
        .into_iter()
        .filter_map(|s| tree.session(s))
        .flat_map(|s| s.windows.iter().copied())
        .filter(|w| tree.window(*w).is_some_and(|win| win.zoomed.is_some()))
        .collect();
    zoomed.sort();
    for window_id in zoomed {
        lines.extend(render_layout_change(tree, window_id));
    }
    lines
}

/// Resolve a `-S`/`-E` capture range against a pane's composed buffer.
///
/// tmux anchors line `0` at the first line of the visible pane; negative
/// numbers are history lines counted back from there (`-1` is the line
/// directly above the screen) and positive numbers are screen lines below
/// the first. Both bounds are inclusive, offsets beyond the buffer clamp to
/// its edges, and a start past the end yields an empty capture. A `None`
/// bound keeps tmux's defaults — start at the first visible line, end at
/// the last — so `-S -3` alone captures three history lines plus the whole
/// screen.
///
/// History lines are `export_scrollback`'s grid rows while screen lines are
/// `content()`'s logical lines, so offsets count lines of the composed
/// text, not of the raw grid. `scrollback` arrives newest-first, as
/// `export_scrollback` emits it, and is reversed to chronological order
/// before composing with `screen`.
pub(crate) fn capture_range(
    scrollback: &str,
    screen: &str,
    start: Option<i64>,
    end: Option<i64>,
) -> String {
    let mut lines: Vec<&str> = Vec::new();
    if !scrollback.is_empty() {
        // export_scrollback emits newest-first (its loop walks the
        // logical indices backwards), so reverse to chronological before
        // composing with the screen — tmux offsets count oldest-first.
        let mut history: Vec<&str> = scrollback.trim_end_matches('\n').split('\n').collect();
        history.reverse();
        lines.extend(history);
    }
    let history = lines.len() as i64;
    if !screen.is_empty() {
        lines.extend(screen.split('\n'));
    }
    if lines.is_empty() {
        return String::new();
    }

    let screen_len = lines.len() as i64 - history;
    let resolve = |offset: Option<i64>, default: i64| -> usize {
        offset
            .unwrap_or(default)
            .saturating_add(history)
            .clamp(0, lines.len() as i64 - 1) as usize
    };
    let first = resolve(start, 0);
    let last = resolve(end, screen_len - 1);
    if first > last {
        return String::new();
    }
    lines[first..=last].join("\n")
}
