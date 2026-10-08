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
    let registration = Registration {
        done: false,
        announced: false,
        abort: Some(abort),
    };

    if spawn_writer(&stream, rx, Arc::clone(&evicted)).is_err() {
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
        evicted,
        registration,
        command_number: 0,
    };
    connection.serve(&mut reader);
    connection.teardown();
}

/// Start the connection's writer thread: it drains the client's queue to a
/// clone of the socket until the queue closes, a write fails, or eviction
/// is flagged. Fails only when the socket cannot be cloned.
fn spawn_writer(
    stream: &LocalStream,
    rx: Receiver<String>,
    evicted: Arc<AtomicBool>,
) -> std::io::Result<()> {
    use interprocess::local_socket::traits::Stream as _;

    let mut writer = stream.try_clone()?;
    let _ = writer.set_send_timeout(Some(EVICTION_POLL));
    std::thread::spawn(move || loop {
        match rx.recv_timeout(EVICTION_POLL) {
            Ok(line) => {
                if write_line(&mut writer, &line, &evicted).is_err() {
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
    evicted: Arc<AtomicBool>,
    registration: Registration,
    command_number: u32,
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
        crate::debug_error!(
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
        crate::debug_log!(
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
        };
        let dispatch_started = std::time::Instant::now();
        let reply = dispatch_contained(command, &ctx, self.persist.as_ref(), Some(&self.tx));
        if dispatch_started.elapsed() > Duration::from_millis(100) {
            crate::debug_log!(
                "MUX",
                "client {client_id} dispatch of command #{command_number} took {:?}",
                dispatch_started.elapsed()
            );
        }
        if reply_is_error(&reply) {
            // A rejected command writes nothing to any pane and has no
            // other trace; without this log the rejection is invisible on
            // both sides of the socket.
            crate::debug_error!(
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

    fn on_parse_error(&mut self, err: &str, line: &str) -> Flow {
        let command_number = self.next_command();
        crate::debug_error!(
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
        self.clients.lock().retain(|(id, _, _, _)| *id != client_id);
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
mod flow_tests {
    use super::Flow;

    #[test]
    fn a_failed_send_closes_and_a_landed_one_continues() {
        assert_eq!(Flow::after_send::<()>(Ok(())), Flow::Continue);
        assert_eq!(Flow::after_send(Err(())), Flow::Close);
    }
}
