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
pub(super) fn handle_client(
    stream: LocalStream,
    tree: Arc<Mutex<MuxTree>>,
    clients: Clients,
    persist: Option<Sender<(SaveOrigin, PersistState)>>,
    shutdown: Option<Arc<AtomicBool>>,
    abort: ConnectionAbort,
    config: Option<Arc<Mutex<EffectiveConfig>>>,
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
    // The abort handle moves to the registry on first registration — hook
    // connections never register and drop theirs with the frame.
    let mut registration = Registration {
        done: false,
        announced: false,
        abort: Some(abort),
    };

    let mut writer = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    };
    let _ = writer.set_send_timeout(Some(EVICTION_POLL));
    let writer_evicted = Arc::clone(&evicted);
    std::thread::spawn(move || loop {
        match rx.recv_timeout(EVICTION_POLL) {
            Ok(line) => {
                if write_line(&mut writer, &line, &writer_evicted).is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if writer_evicted.load(Ordering::Relaxed) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    });

    let _ = stream.set_recv_timeout(Some(EVICTION_POLL));
    let mut reader = BufReader::new(stream);
    let mut command_number = 0u32;
    loop {
        let mut line = match read_control_line(&mut reader, &evicted) {
            ControlLine::Line(line) => line,
            ControlLine::Closed => break,
            // SEC-104: over-budget accumulation is answered like a
            // malformed command and the connection is closed, whether the
            // line is complete or still unterminated.
            ControlLine::Oversize => {
                registration.ensure(&tree, &clients, client_id, &tx, &evicted);
                command_number += 1;
                let _ = tx.send(emit_block(
                    command_number,
                    "line exceeds 1 MiB budget, closing connection",
                    false,
                ));
                break;
            }
            // A non-UTF-8 line (e.g. send-keys -l carrying Latin-1 bytes)
            // was read through its newline, so the stream stays
            // line-framed, but its contents are unusable. Answer it like a
            // parse error — a numbered %error block — instead of dropping
            // the whole client.
            ControlLine::Undecodable(undecodable_len) => {
                registration.ensure(&tree, &clients, client_id, &tx, &evicted);
                command_number += 1;
                crate::debug_error!(
                    "MUX",
                    "non-UTF-8 command line #{} from client {} ({} bytes)",
                    command_number,
                    client_id,
                    undecodable_len
                );
                if tx
                    .send(emit_block(command_number, "line is not valid UTF-8", false))
                    .is_err()
                {
                    break;
                }
                continue;
            }
        };
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        if line.trim().is_empty() {
            continue;
        }
        // One line, two grammars: [`parse_line`] classifies. A hook report
        // is answered in place (no registration, no command number); a
        // control command — or a parse error — is a numbered dispatch.
        match parse_line(&line) {
            Ok(Line::Hook(report)) => {
                let (reply, broadcast) = crate::mux::hooks::handle_report(&report, &tree);
                if let Some(notification) = broadcast {
                    broadcast_notification(&clients, &notification);
                }
                if tx.send(reply).is_err() {
                    break;
                }
            }
            Ok(Line::Control(command)) => {
                registration.ensure(&tree, &clients, client_id, &tx, &evicted);
                command_number += 1;
                crate::debug_log!(
                    "MUX",
                    "received #{} from client {}: {}",
                    command_number,
                    client_id,
                    summarize_line(&line)
                );
                let ctx = Ctx {
                    tree: &tree,
                    clients: &clients,
                    command_number,
                    shutdown: shutdown.as_deref(),
                    config: config.as_deref(),
                    client_id: Some(client_id),
                };
                let dispatch_started = std::time::Instant::now();
                let reply = dispatch_contained(command, &ctx, persist.as_ref(), Some(&tx));
                if dispatch_started.elapsed() > Duration::from_millis(100) {
                    crate::debug_log!(
                        "MUX",
                        "client {client_id} dispatch of command #{command_number} took {:?}",
                        dispatch_started.elapsed()
                    );
                }
                if reply_is_error(&reply) {
                    // A rejected command writes nothing to any pane and has
                    // no other trace; without this log the rejection is
                    // invisible on both sides of the socket.
                    crate::debug_error!(
                        "MUX",
                        "command #{} from client {} rejected: {}",
                        command_number,
                        client_id,
                        summarize_line(&line)
                    );
                }
                if tx.send(reply).is_err() {
                    break;
                }
                registration.announce_if_sized(&tree, &clients, client_id);
            }
            Err(err) => {
                registration.ensure(&tree, &clients, client_id, &tx, &evicted);
                command_number += 1;
                crate::debug_error!(
                    "MUX",
                    "unparseable command #{} from client {}: {} ({})",
                    command_number,
                    client_id,
                    summarize_line(&line),
                    err
                );
                if tx.send(emit_block(command_number, &err, false)).is_err() {
                    break;
                }
            }
        }
    }
    if registration.done {
        // Every registered connection ends here — a clean disconnect, a
        // socket EOF, and an eviction (its flag ends the read loop) alike.
        clients.lock().retain(|(id, _, _, _)| *id != client_id);
        // The disconnect drops the connection's sizing contribution; the
        // windows it constrained may grow to the remaining viewers'
        // minimum, and the grown windows broadcast `%layout-change` to the
        // clients that are still attached. The displayed view is read
        // first, for the `%client-left` line that follows the layout
        // changes (dispatch's layout-then-lifecycle order).
        let (resized, displayed) = {
            let mut guard = tree.lock();
            let displayed = guard.client_views.get(&client_id).and_then(|view| {
                let session = guard.session_of_window(view.window)?;
                Some((session.to_string(), view.window.to_string()))
            });
            (guard.clear_client_view(client_id), displayed)
        };
        for window_id in resized {
            broadcast_layout_change(&tree, &clients, window_id);
        }
        // Only an announced client is missed: a connection that never
        // reported a size was never introduced to its peers.
        if registration.announced {
            let (session_id, window_id) = displayed.unzip();
            broadcast_notification(
                &clients,
                &TmuxNotification::ClientLeft {
                    client: client_id.to_string(),
                    session_id,
                    window_id,
                },
            );
        }
    }
}
