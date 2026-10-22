//! One control client's thread: registration (broadcast enrolment, held
//! state replay, attach announcement) and the read-dispatch-reply loop.

use super::*;

/// A connection's one-time join to the broadcast registry. Deferred until
/// the first control command (or error reply), so hook-only connections
/// never receive pushes; the abort handle moves into the registry then.
/// Registration also replays held state (ENH-037): one `%pane-exited` per
/// held pane and one `%layout-change` per zoomed window, queued on the
/// client's own channel ahead of its first command's reply.
pub(super) struct Registration {
    pub(super) done: bool,
    /// The `%client-attached` announcement has gone out. It waits for the
    /// connection's first size report (`refresh-client -C`) so one-shot
    /// command clients, which never report one, are neither announced nor
    /// missed.
    pub(super) announced: bool,
    pub(super) abort: Option<ConnectionAbort>,
}

impl Registration {
    pub(super) fn ensure(
        &mut self,
        tree: &Arc<Mutex<MuxTree>>,
        clients: &Clients,
        client_id: u64,
        tx: &SyncSender<String>,
        queued: &Arc<AtomicUsize>,
        evicted: &Arc<AtomicBool>,
    ) {
        if self.done {
            return;
        }
        // Snapshot held state and join the broadcast set under one tree-lock
        // hold: a death marked after the snapshot is broadcast to a set that
        // already includes this client. The one overlap — a pane in the
        // snapshot whose broadcast has not yet gone out — delivers
        // `%pane-exited` twice; that is idempotent (the state is "held with
        // code N"), so it is documented rather than de-duplicated. Lock
        // order is `tree` then `clients`, and nothing else nests these two
        // in either direction (`reap_dead_panes` releases the tree before
        // broadcasting; `push_to_clients` takes only `clients`), so this
        // introduces no lock inversion.
        let replay = {
            let guard = tree.lock();
            let lines = held_state_replay_lines(&guard);
            clients.lock().push((
                client_id,
                tx.clone(),
                Arc::clone(queued),
                Arc::clone(evicted),
                self.abort.take().expect("abort is registered once"),
            ));
            lines
        };
        for line in replay {
            if tx.send(line).is_err() {
                break;
            }
        }
        self.done = true;
    }

    /// Announce the connection once it holds a reported view. The push goes
    /// to every registered client but this one, so the joining client never
    /// hears itself.
    fn announce_if_sized(&mut self, tree: &Arc<Mutex<MuxTree>>, clients: &Clients, client_id: u64) {
        if self.announced || !self.done || !tree.lock().client_views.contains_key(&client_id) {
            return;
        }
        self.announced = true;
        push_to_clients_except(
            clients,
            client_id,
            emit(&TmuxNotification::ClientAttached {
                client: client_id.to_string(),
            }),
        );
    }
}

/// Serve one connected client: a writer thread draining a channel, and this
/// thread reading lines. The socket carries two grammars (Phase 5),
/// classified by [`parse_line`]: a JSON hook report (a line whose first
/// non-whitespace byte is `{`) is answered in place; anything else is a tmux
/// control command. Broadcast registration is deferred until the first
/// CONTROL command, so a hook connection — the
/// send-one-line/read-one-reply/close pattern herdr's integration scripts
/// use — never receives pushed notifications. On disconnect, only this
/// client's broadcast sender is removed — the accept loop and the tree are
/// untouched.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_client(
    stream: LocalStream,
    tree: Arc<Mutex<MuxTree>>,
    clients: Clients,
    persist: Option<Sender<(SaveOrigin, PersistState)>>,
    shutdown: Option<Arc<AtomicBool>>,
    abort: ConnectionAbort,
    config: Option<Arc<Mutex<EffectiveConfig>>>,
    output_epoch: OutputEpoch,
) {
    use interprocess::local_socket::traits::Stream as _;

    let client_id = CLIENT_SEQ.fetch_add(1, Ordering::Relaxed);
    // Bounded per ARC-011: a client that stops draining is evicted by
    // [`push_to_clients`] once its queue fills, rather than buffering every
    // `%output` line forever.
    let (tx, rx) = sync_channel::<String>(CLIENT_QUEUE_DEPTH);
    // Eviction's signal to this connection's threads (ENH-012). Both run
    // with send/recv timeouts where the transport supports them (Unix): a
    // wedged writer wakes every poll, sees the flag, and exits — dropping
    // the queue; the reader does the same, and its exit closes the
    // connection the client observes. Named pipes reject I/O timeouts, so
    // on Windows the evictor aborts the blocked calls through
    // [`ConnectionAbort::cancel_blocked_io`] instead — the flag stays the
    // exit signal either way. Without one of the two, the writer stays
    // blocked in a full socket buffer and the queued lines are retained
    // forever (measured live).
    let evicted = Arc::new(AtomicBool::new(false));
    // The queued-line counter (ARC-008): the fan-out increments it on every
    // successful push, the writer decrements it as lines drain. The fan-out
    // sums the counters to enforce the aggregate eviction bound; entries
    // removed from the registry stop counting, so a client that leaves with
    // lines still queued cannot leak its share into the sum.
    let queued = Arc::new(AtomicUsize::new(0));
    // The abort handle moves to the registry on first registration — hook
    // connections never register and drop theirs with the frame.
    let registration = Registration {
        done: false,
        announced: false,
        abort: Some(abort),
    };

    if spawn_writer(
        &stream,
        rx,
        Arc::clone(&queued),
        Arc::clone(&evicted),
        (Arc::clone(&clients), client_id),
    )
    .is_err()
    {
        return;
    }

    let _ = stream.set_recv_timeout(Some(EVICTION_POLL));
    let mut reader = BufReader::new(stream);
    let mut connection = Connection {
        client_id,
        tree,
        clients,
        persist,
        shutdown,
        config,
        tx,
        queued: Arc::clone(&queued),
        evicted,
        registration,
        command_number: 0,
        stream: None,
        output_epoch,
    };
    connection.serve(&mut reader);
    connection.teardown();
}

/// Start the connection's writer thread: it drains the client's queue to a
/// clone of the socket until the queue closes, a write fails, eviction is
/// flagged, or it has written a [`final_item`] — after which it aborts the
/// connection's blocked I/O through its registry entry (`owner`), so a
/// Windows reader parked in `ReadFile` wakes and the connection tears down
/// without waiting for the client to hang up. Fails only when the socket
/// cannot be cloned.
fn spawn_writer(
    stream: &LocalStream,
    rx: Receiver<String>,
    queued: Arc<AtomicUsize>,
    evicted: Arc<AtomicBool>,
    owner: (Clients, u64),
) -> std::io::Result<()> {
    use interprocess::local_socket::traits::Stream as _;

    let mut writer = stream.try_clone()?;
    let _ = writer.set_send_timeout(Some(EVICTION_POLL));
    // The pane a raw stream narrowed this connection to — set by the
    // stream marker in queue order, so everything queued before the
    // stream's reply is delivered unfiltered and everything after it is
    // filtered.
    let mut stream_pane: Option<String> = None;
    std::thread::spawn(move || loop {
        match rx.recv_timeout(EVICTION_POLL) {
            Ok(line) => {
                // Every line that leaves the queue passes here while the
                // entry is live; a counter left above zero only delays the
                // aggregate bound, never underflows (guard below).
                if queued.load(Ordering::Relaxed) > 0 {
                    queued.fetch_sub(1, Ordering::Relaxed);
                }
                let (last, line) = take_final_marker(line);
                let line = match take_stream_marker(line) {
                    (Some(pane), rest) => {
                        stream_pane = Some(pane);
                        rest
                    }
                    (None, line) => line,
                };
                let passes = stream_pane
                    .as_deref()
                    .is_none_or(|pane| pane_stream_passes(&line, pane));
                let wrote = if passes {
                    write_line(&mut writer, &line, &evicted)
                } else {
                    Ok(())
                };
                // The cancel waits until the final line is written: it
                // aborts every pending call on the pipe, the writer's own
                // `WriteFile` included.
                if last {
                    cancel_connection_io(&owner.0, owner.1);
                    break;
                }
                if wrote.is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if evicted.load(Ordering::Relaxed) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    });
    Ok(())
}

/// Whether the read loop keeps reading after a line was handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Close,
}

impl Flow {
    /// `Close` when a send to the client's queue failed — the writer is
    /// gone, so nothing more can reach the client.
    fn after_send<E>(sent: Result<(), E>) -> Self {
        if sent.is_err() {
            Flow::Close
        } else {
            Flow::Continue
        }
    }
}

/// The in-band item that narrows a connection to one pane's raw stream:
/// a NUL (never present in an emitted line) plus the pane id, ahead of the
/// stream's reply block in the same queue item.
fn stream_marker(pane: PaneId) -> String {
    format!("\0{pane}\n")
}

/// Split a queue item into its stream marker's pane id (if it carries
/// one) and the bytes to write.
fn take_stream_marker(item: String) -> (Option<String>, String) {
    match item
        .strip_prefix('\0')
        .and_then(|rest| rest.split_once('\n'))
    {
        Some((pane, rest)) => (Some(pane.to_string()), rest.to_string()),
        None => (None, item),
    }
}

/// The in-band prefix of a connection's last queue item. SOH, like the
/// stream marker's NUL, never starts an emitted line.
const FINAL_MARKER: char = '\u{1}';

/// `line` as the connection's last item: the writer writes it, aborts the
/// connection's blocked I/O, and exits.
fn final_item(line: &str) -> String {
    format!("{FINAL_MARKER}{line}")
}

/// Split a queue item into whether it is the connection's last and the
/// item itself (stream marker, if any, still attached).
fn take_final_marker(item: String) -> (bool, String) {
    match item.strip_prefix(FINAL_MARKER) {
        Some(rest) => (true, rest.to_string()),
        None => (false, item),
    }
}

/// Abort `client_id`'s blocked socket I/O through its registry entry's
/// [`ConnectionAbort`] — a no-op on Unix, where the eviction flag's poll
/// wakes the threads, and a `CancelIoEx` on Windows, where nothing else
/// would.
fn cancel_connection_io(clients: &Clients, client_id: u64) {
    if let Some((.., abort)) = clients.lock().iter().find(|(id, ..)| *id == client_id) {
        abort.cancel_blocked_io();
    }
}

/// What a raw pane stream delivers: reply blocks (the connection's own
/// commands), the pane's `%output`/`%pane-exited`/`%pane-respawned`, and
/// `%exit`. Every other broadcast is dropped at the writer.
pub(super) fn pane_stream_passes(line: &str, pane: &str) -> bool {
    if line.starts_with("%begin ") || line == "%exit\n" {
        return true;
    }
    let names_pane = |rest: Option<&str>| {
        rest.and_then(|rest| rest.strip_prefix(pane))
            .is_some_and(|tail| tail.starts_with(' ') || tail.starts_with('\n'))
    };
    names_pane(line.strip_prefix("%output "))
        || names_pane(line.strip_prefix("%pane-exited "))
        || names_pane(line.strip_prefix("%pane-respawned "))
}

/// Queue `line` as `client_id`'s last item and flag the connection for
/// teardown: its writer drains what is queued (this line included) and
/// then aborts the connection's blocked I/O, so both connection threads
/// exit — on Unix their poll sees the flag; on Windows the abort wakes the
/// reader parked in `ReadFile` — and the socket closes.
fn end_connection_with(clients: &Clients, client_id: u64, line: &str) {
    let guard = clients.lock();
    if let Some((_, tx, queued, evicted, _)) = guard.iter().find(|(id, ..)| *id == client_id) {
        if tx.try_send(final_item(line)).is_ok() {
            queued.fetch_add(1, Ordering::Relaxed);
        }
        evicted.store(true, Ordering::Relaxed);
    }
}

/// A connection narrowed to one pane by `observe-pane`/`control-pane`.
#[derive(Clone, Copy, Debug)]
struct PaneStreamMode {
    /// `control-pane`: later commands still dispatch.
    control: bool,
}

/// How long an ended stream's connection may stay registered before the
/// watch aborts its I/O itself: a client too wedged to drain `%exit` keeps
/// the writer parked in its write, so the writer never reaches its own
/// cancel.
const STREAM_END_GRACE: Duration = Duration::from_secs(5);

/// Watch a streamed pane until it leaves the tree, then end the
/// connection with `%exit`; returns early once the connection itself is
/// gone. Polled at the eviction cadence: a pane's removal has no
/// per-connection signal to park on.
fn spawn_pane_stream_watch(
    tree: Arc<Mutex<MuxTree>>,
    clients: Clients,
    client_id: u64,
    pane: PaneId,
) {
    std::thread::spawn(move || loop {
        std::thread::sleep(EVICTION_POLL);
        if !clients.lock().iter().any(|(id, ..)| *id == client_id) {
            return;
        }
        if tree.lock().pane(pane).is_none() {
            end_connection_with(&clients, client_id, "%exit\n");
            let deadline = std::time::Instant::now() + STREAM_END_GRACE;
            while clients.lock().iter().any(|(id, ..)| *id == client_id) {
                if std::time::Instant::now() >= deadline {
                    cancel_connection_io(&clients, client_id);
                    return;
                }
                std::thread::sleep(EVICTION_POLL);
            }
            return;
        }
    });
}

/// One control connection's reader-side state: what every line handler
/// needs to register, number, dispatch, and reply.
struct Connection {
    client_id: u64,
    tree: Arc<Mutex<MuxTree>>,
    clients: Clients,
    persist: Option<Sender<(SaveOrigin, PersistState)>>,
    shutdown: Option<Arc<AtomicBool>>,
    config: Option<Arc<Mutex<EffectiveConfig>>>,
    tx: SyncSender<String>,
    queued: Arc<AtomicUsize>,
    evicted: Arc<AtomicBool>,
    registration: Registration,
    command_number: u32,
    stream: Option<PaneStreamMode>,
    /// The daemon-wide output epoch — the seed export's race check.
    output_epoch: OutputEpoch,
}

impl Connection {
    /// The read loop: classify each line read and hand it to its handler
    /// until the connection closes or a handler ends it.
    fn serve<R: BufRead>(&mut self, reader: &mut R) {
        loop {
            let flow = match read_control_line(reader, &self.evicted) {
                ControlLine::Line(line) => self.on_line(line),
                ControlLine::Closed => Flow::Close,
                ControlLine::Oversize => self.on_oversize(),
                ControlLine::Undecodable(undecodable_len) => self.on_undecodable(undecodable_len),
            };
            if flow == Flow::Close {
                break;
            }
        }
    }

    /// Register (once) and take the next command number — the shared
    /// prologue of every numbered reply.
    fn next_command(&mut self) -> u32 {
        self.registration.ensure(
            &self.tree,
            &self.clients,
            self.client_id,
            &self.tx,
            &self.queued,
            &self.evicted,
        );
        self.command_number += 1;
        self.command_number
    }

    /// SEC-104: over-budget accumulation is answered like a malformed
    /// command and the connection is closed, whether the line is complete
    /// or still unterminated.
    fn on_oversize(&mut self) -> Flow {
        let command_number = self.next_command();
        let _ = self.tx.send(emit_block(
            command_number,
            "line exceeds 1 MiB budget, closing connection",
            false,
        ));
        Flow::Close
    }

    /// A non-UTF-8 line (e.g. send-keys -l carrying Latin-1 bytes) was read
    /// through its newline, so the stream stays line-framed, but its
    /// contents are unusable. Answer it like a parse error — a numbered
    /// %error block — instead of dropping the whole client.
    fn on_undecodable(&mut self, undecodable_len: usize) -> Flow {
        let command_number = self.next_command();
        par_term_emu_core::debug_error!(
            "MUX",
            "non-UTF-8 command line #{} from client {} ({} bytes)",
            command_number,
            self.client_id,
            undecodable_len
        );
        Flow::after_send(
            self.tx
                .send(emit_block(command_number, "line is not valid UTF-8", false)),
        )
    }

    /// One line, two grammars: [`parse_line`] classifies. A hook report is
    /// answered in place (no registration, no command number); a control
    /// command — or a parse error — is a numbered dispatch.
    fn on_line(&mut self, mut line: String) -> Flow {
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        if line.trim().is_empty() {
            return Flow::Continue;
        }
        match parse_line(&line) {
            Ok(Line::Hook(report)) => self.on_hook(&report),
            Ok(Line::Control(command)) => self.on_control(command, &line),
            Err(err) => self.on_parse_error(&err, &line),
        }
    }

    fn on_hook(&mut self, report: &str) -> Flow {
        let (reply, broadcast) = crate::mux::hooks::handle_report(report, &self.tree);
        if let Some(notification) = broadcast {
            broadcast_notification(&self.clients, &notification);
        }
        Flow::after_send(self.tx.send(reply))
    }

    fn on_control(&mut self, command: crate::mux::command::MuxCommand, line: &str) -> Flow {
        let command_number = self.next_command();
        let client_id = self.client_id;
        if let Some(mode) = self.stream {
            let refusal = if !mode.control {
                Some("observe-pane: this connection is a read-only pane stream")
            } else if matches!(command, crate::mux::command::MuxCommand::PaneStream { .. }) {
                Some("control-pane: this connection is already a pane stream")
            } else {
                None
            };
            if let Some(refusal) = refusal {
                return Flow::after_send(self.tx.send(emit_block(command_number, refusal, false)));
            }
        }
        if let crate::mux::command::MuxCommand::PaneStream { pane, control } = command {
            return self.on_pane_stream(command_number, pane, control);
        }
        par_term_emu_core::debug_log!(
            "MUX",
            "received #{} from client {}: {}",
            command_number,
            client_id,
            summarize_line(line)
        );
        let ctx = Ctx {
            tree: &self.tree,
            clients: &self.clients,
            command_number,
            shutdown: self.shutdown.as_deref(),
            config: self.config.as_deref(),
            client_id: Some(client_id),
            output_epoch: &self.output_epoch,
        };
        let dispatch_started = std::time::Instant::now();
        let reply = dispatch_contained(command, &ctx, self.persist.as_ref(), Some(&self.tx));
        if dispatch_started.elapsed() > Duration::from_millis(100) {
            par_term_emu_core::debug_log!(
                "MUX",
                "client {client_id} dispatch of command #{command_number} took {:?}",
                dispatch_started.elapsed()
            );
        }
        if reply_is_error(&reply) {
            // A rejected command writes nothing to any pane and has no
            // other trace; without this log the rejection is invisible on
            // both sides of the socket.
            par_term_emu_core::debug_error!(
                "MUX",
                "command #{} from client {} rejected: {}",
                command_number,
                client_id,
                summarize_line(line)
            );
        }
        if self.tx.send(reply).is_err() {
            return Flow::Close;
        }
        self.registration
            .announce_if_sized(&self.tree, &self.clients, client_id);
        Flow::Continue
    }

    /// `observe-pane`/`control-pane`: reply with the pane's seed and
    /// narrow this connection to the pane. The seed is read and the
    /// marker-plus-reply item queued under the clients lock — the lock
    /// every `%output` push takes — so no push for this connection lands
    /// between the two: output queued ahead of the reply is already in the
    /// seed, output queued behind it is not.
    fn on_pane_stream(
        &mut self,
        command_number: u32,
        target: crate::mux::ids::Target<PaneId>,
        control: bool,
    ) -> Flow {
        let verb = if control {
            "control-pane"
        } else {
            "observe-pane"
        };
        let streamed = {
            let guard = self.tree.lock();
            // A typed id resolves without an existence check.
            let resolved = guard.resolve_pane_target(target).and_then(|id| {
                guard
                    .pane(id)
                    .map(|pane| (id, pane))
                    .ok_or(crate::mux::pane::MuxError::NoSuchPane(id))
            });
            let (pane_id, pane) = match resolved {
                Ok(found) => found,
                Err(err) => {
                    drop(guard);
                    return Flow::after_send(self.tx.send(emit_block(
                        command_number,
                        &format!("{verb}: {err}"),
                        false,
                    )));
                }
            };
            let exited = pane.dead().then(|| {
                pane.exit_code()
                    .map_or_else(|| " exited=?".to_string(), |code| format!(" exited={code}"))
            });
            let seed_item = |seed: &str, cols: usize, rows: usize| {
                let body = format!(
                    "pane {pane_id} {cols}x{rows}{}\n{seed}",
                    exited.as_deref().unwrap_or_default()
                );
                format!(
                    "{}{}",
                    stream_marker(pane_id),
                    emit_block(command_number, &body, true)
                )
            };
            // The seed must reach this client BEFORE any %output the seed
            // does not already reflect, and every %output enqueue needs
            // the clients lock — so the ORDERING needs the lock, not the
            // export. Export off the lock (a pane with deep scrollback
            // would otherwise pause every pane's fan-out for the whole
            // export), then take the lock and re-check the output epoch:
            // it only bumped if some pane produced output meanwhile, and
            // then the export raced a fan-out — re-export under the lock
            // instead (the original, ordered shape; bounded by one
            // re-export, and a quiet pane never pays it).
            // The epoch baseline is sampled BEFORE the export: any marker
            // processed while the export runs bumps the epoch, so the
            // re-check under the lock sees it and re-exports. (Sampling
            // after the export only covers the lock-acquisition gap — the
            // seed_export_racing_output_stays_ordered failure on the
            // macos runner: a load-slowed export lost markers that raced
            // it while the check compared two equal post-export loads.)
            let before = self.output_epoch.load(std::sync::atomic::Ordering::Acquire);
            let (seed, (cols, rows)) = {
                let terminal = pane.terminal();
                let term = terminal.read();
                (term.export_screen_restore_sequence(), term.size())
            };
            let _clients = self.clients.lock();
            let raced = self.output_epoch.load(std::sync::atomic::Ordering::Acquire) != before;
            let item = if raced {
                let (seed, (cols, rows)) = {
                    let terminal = pane.terminal();
                    let term = terminal.read();
                    (term.export_screen_restore_sequence(), term.size())
                };
                seed_item(&seed, cols, rows)
            } else {
                seed_item(&seed, cols, rows)
            };
            match self.tx.try_send(item) {
                Ok(()) => pane_id,
                Err(_) => return Flow::Close,
            }
        };
        self.stream = Some(PaneStreamMode { control });
        spawn_pane_stream_watch(
            Arc::clone(&self.tree),
            Arc::clone(&self.clients),
            self.client_id,
            streamed,
        );
        Flow::Continue
    }

    fn on_parse_error(&mut self, err: &str, line: &str) -> Flow {
        let command_number = self.next_command();
        par_term_emu_core::debug_error!(
            "MUX",
            "unparseable command #{} from client {}: {} ({})",
            command_number,
            self.client_id,
            summarize_line(line),
            err
        );
        Flow::after_send(self.tx.send(emit_block(command_number, err, false)))
    }

    /// Every registered connection ends here — a clean disconnect, a socket
    /// EOF, and an eviction (its flag ends the read loop) alike. Hook-only
    /// connections never registered and have nothing to undo.
    fn teardown(self) {
        if !self.registration.done {
            return;
        }
        let client_id = self.client_id;
        self.clients.lock().retain(|(id, ..)| *id != client_id);
        // The disconnect drops the connection's sizing contribution; the
        // windows it constrained may grow to the remaining viewers'
        // minimum, and the grown windows broadcast `%layout-change` to the
        // clients that are still attached. The displayed view is read
        // first, for the `%client-left` line that follows the layout
        // changes (dispatch's layout-then-lifecycle order).
        let (resized, displayed) = {
            let mut guard = self.tree.lock();
            let displayed = guard.client_views.get(&client_id).and_then(|view| {
                let session = guard.session_of_window(view.window)?;
                Some((session.to_string(), view.window.to_string()))
            });
            (guard.clear_client_view(client_id), displayed)
        };
        // Windows that grew re-fit their panes under the lock above.
        crate::mux::tree::deliver_pending_observer_events(&self.tree);
        for window_id in resized {
            broadcast_layout_change(&self.tree, &self.clients, window_id);
        }
        // Only an announced client is missed: a connection that never
        // reported a size was never introduced to its peers.
        if self.registration.announced {
            let (session_id, window_id) = displayed.unzip();
            broadcast_notification(
                &self.clients,
                &TmuxNotification::ClientLeft {
                    client: client_id.to_string(),
                    session_id,
                    window_id,
                },
            );
        }
    }
}

#[cfg(test)]
mod pane_stream_tests {
    use super::{
        final_item, pane_stream_passes, stream_marker, take_final_marker, take_stream_marker,
    };
    use crate::mux::ids::PaneId;

    #[test]
    fn the_filter_passes_replies_exit_and_only_the_streamed_panes_lines() {
        for line in [
            "%begin 1 2 1\n",
            "%exit\n",
            "%output %3 abc\n",
            "%pane-exited %3 0\n",
            "%pane-exited %3\n",
            "%pane-respawned %3\n",
        ] {
            assert!(pane_stream_passes(line, "%3"), "{line:?} passes");
        }
        for line in [
            "%output %30 abc\n",
            "%output %4 abc\n",
            "%layout-change @1 x x \n",
            "%sessions-changed\n",
            "%agent-state-changed %3 claude idle\n",
            "%pane-exited %31 0\n",
        ] {
            assert!(!pane_stream_passes(line, "%3"), "{line:?} is dropped");
        }
    }

    #[test]
    fn the_marker_splits_off_ahead_of_the_reply_it_rides_with() {
        let item = format!("{}%begin 1 1 1\n%end 1 1 1\n", stream_marker(PaneId(7)));
        let (pane, rest) = take_stream_marker(item);
        assert_eq!(pane.as_deref(), Some("%7"));
        assert_eq!(rest, "%begin 1 1 1\n%end 1 1 1\n");
        let (none, plain) = take_stream_marker("%output %7 x\n".to_string());
        assert_eq!(none, None);
        assert_eq!(plain, "%output %7 x\n");
    }

    #[test]
    fn the_final_marker_flags_only_the_last_item_and_leaves_its_bytes_intact() {
        let (last, line) = take_final_marker(final_item("%exit\n"));
        assert!(last);
        assert_eq!(line, "%exit\n");
        assert!(
            pane_stream_passes(&line, "%3"),
            "the final %exit passes the filter"
        );
        let (last, line) = take_final_marker("%output %3 x\n".to_string());
        assert!(!last);
        assert_eq!(line, "%output %3 x\n");
        let marked = format!("{}%begin 1 1 1\n", stream_marker(PaneId(3)));
        let (last, line) = take_final_marker(marked.clone());
        assert!(!last, "a stream-marked reply is not a final item");
        assert_eq!(line, marked);
    }
}

#[cfg(test)]
mod flow_tests {
    use super::Flow;

    #[test]
    fn a_failed_send_closes_and_a_landed_one_continues() {
        assert_eq!(Flow::after_send::<()>(Ok(())), Flow::Continue);
        assert_eq!(Flow::after_send(Err(())), Flow::Close);
    }
}
