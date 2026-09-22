//! The control-mode socket server.
//!
//! One Unix socket, one accept loop, one thread-per-client writer. Pane output
//! is pushed to connected clients from the PTY reader callback as bytes arrive
//! — there is no polling anywhere in this path, which is the whole point of
//! the module (see `par-mux.md`). Command dispatch itself lives in
//! [`crate::mux::dispatch`]: this module hands each parsed command over and
//! keeps the accept loop, client threads, and broadcast sinks.

#[cfg(test)]
use crate::mux::command::parse_command;
use crate::mux::command::{parse_line, Line};
use crate::mux::dispatch::{dispatch_command, Ctx};
use crate::mux::emit::{emit, emit_block};
use crate::mux::ids::{PaneId, WindowId};
use crate::mux::ipc::{bind_local_listener, prepare_socket_path, LocalListener, LocalStream};
use crate::mux::pane::ShellPaneFactory;
use crate::mux::persist::{write_state, PersistState};
use crate::mux::tree::MuxTree;
use crate::tmux_control::TmuxNotification;
use interprocess::local_socket::traits::Listener as _;
use interprocess::TryClone as _;
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Write};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// How often the accept loop's idle poll runs the scrape tier — the
/// fallback state pass over panes whose agent reports no state hook
/// (par-mux.md Phase 5 scrape tier).
const SCRAPE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// How often the persist worker re-checks the shutdown flag while idle.
/// Bounds how long the shutdown join waits after the flag is set.
const PERSIST_POLL: Duration = Duration::from_millis(200);

/// Per-client broadcast queue depth in lines (ARC-011). A `%output` line is
/// at most ~4 KiB, so the worst case a stalled client can pin is ~16 MiB;
/// past that it is evicted rather than allowed to grow the daemon without
/// bound (tmux's own policy for a control client that stops draining).
const CLIENT_QUEUE_DEPTH: usize = 4096;

/// Monotonic client ids, so a disconnecting client's broadcast sender can be
/// removed eagerly rather than waiting for the next broadcast to fail.
static CLIENT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Connected clients' broadcast senders, keyed by their monotonic id.
pub(crate) type Clients = Arc<Mutex<Vec<(u64, SyncSender<String>)>>>;

/// A control-mode multiplexer server listening on a Unix socket.
pub struct MuxServer {
    listener: LocalListener,
    path: PathBuf,
    tree: Arc<Mutex<MuxTree>>,
    clients: Clients,
    /// Per-instance shutdown flag (ARC-016): the accept loop polls it, the
    /// binary's signal handler reaches it through [`Self::shutdown_handle`].
    shutdown: Arc<AtomicBool>,
}

impl MuxServer {
    /// Bind to `path`, refusing a path a live server already owns and
    /// reclaiming one only a stale remnant holds.
    ///
    /// Access control is the transport's: mode `0600` on Unix, an owner-only
    /// security descriptor on Windows — see [`crate::mux::ipc`].
    pub fn bind(path: &Path) -> std::io::Result<Self> {
        // The factory exports the socket path so panes it spawns can run
        // hook scripts that report back (the Phase 5 env contract).
        let factory = ShellPaneFactory {
            socket_path: Some(path.to_string_lossy().into_owned()),
            ..ShellPaneFactory::default()
        };
        Self::bind_with_tree(path, MuxTree::new(Box::new(factory)))
    }

    /// [`Self::bind`] serving an already-built `tree` — the restore path
    /// (D3.5): the daemon startup rebuilt the tree from persisted state via
    /// [`crate::mux::persist::PersistState`], and this server serves it
    /// instead of a fresh one. Every restored pane is wired to push its
    /// output to connected clients exactly as a newly created pane is.
    pub fn bind_with_tree(path: &Path, tree: MuxTree) -> std::io::Result<Self> {
        prepare_socket_path(path)?;
        let listener = bind_local_listener(path)?;

        let clients: Clients = Arc::new(Mutex::new(Vec::new()));
        let tree = Arc::new(Mutex::new(tree));
        wire_all_pane_outputs(&tree, &clients);

        Ok(Self {
            listener,
            path: path.to_path_buf(),
            tree,
            clients,
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The path this server is listening on.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accept connections until the listener is closed or shutdown is
    /// requested, with no on-disk persistence — the embedding choice for
    /// tests and in-process use. The daemon binary runs
    /// [`Self::run_persisting`] instead (D3.3).
    pub fn run(self) {
        self.run_with_state_path(None)
    }

    /// [`Self::run`] with the whole state atomically saved to `state_path`
    /// after every mutating dispatch — the mode the `par-mux` daemon runs
    /// in. On a requested shutdown (Task 3.5) a final save captures content
    /// that arrived since the last structural one, so a clean SIGTERM never
    /// loses the last window. Callers resolve the path with
    /// [`crate::mux::persist::state_file_path`].
    pub fn run_persisting(self, state_path: PathBuf) {
        self.run_with_state_path(Some(state_path.clone()));
        if self.shutdown.load(Ordering::Relaxed) {
            if let Err(err) = crate::mux::persist::save_to(&self.tree.lock(), &state_path) {
                log::error!("par-mux: final state save failed: {err}");
            }
        }
    }

    /// A shareable handle to this server's shutdown flag (ARC-016): storing
    /// `true` stops this instance's accept loop on its next tick. One atomic
    /// store, so a signal handler may write it directly. Instances are
    /// independent — a handle cannot stop another server in the same process.
    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }

    fn run_with_state_path(&self, state_path: Option<PathBuf>) {
        // The loop must be able to NOTICE a shutdown request while idle,
        // but `accept` transparently retries EINTR, so a blocking accept
        // never returns on a signal (observed: the daemon ignored SIGTERM
        // entirely). Nonblocking accept + a short idle sleep is the escape —
        // this poll is on the ACCEPT path only; pane output remains pure
        // push, which is the property the module header states.
        use interprocess::local_socket::ListenerNonblockingMode;
        self.listener
            .set_nonblocking(ListenerNonblockingMode::Accept)
            .expect("the listener was just bound");

        // ARC-003: the persist worker owns serialization + fsync, off the
        // tree lock. Dispatch captures a PersistState under the lock (cheap
        // clones) and sends it here; bursts coalesce to the newest state.
        let (persist_tx, persist_worker) = match state_path
            .clone()
            .map(|path| spawn_persist_worker(path, Arc::clone(&self.shutdown)))
        {
            Some((tx, join)) => (Some(tx), Some(join)),
            None => (None, None),
        };

        // The scrape tier rides this loop as its heartbeat: pattern
        // overrides live beside the state file when there is one, and the
        // idle poll doubles as the 1 s tick without a thread of its own to
        // own a lifecycle for.
        let engine = crate::mux::scrape::ScrapeEngine::load(
            state_path
                .as_ref()
                .and_then(|path| path.parent())
                .map(|dir| dir.join("agent-patterns"))
                .as_deref(),
        );
        let mut last_scrape = std::time::Instant::now();

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                // Tell the clients the daemon is ending deliberately, so
                // they do not have to infer death from a dropped socket.
                broadcast_notification(&self.clients, &TmuxNotification::Exit);
                break;
            }
            match self.listener.accept() {
                Ok(stream) => {
                    // BSD/macOS accepted sockets inherit O_NONBLOCK from the
                    // listening socket; the handler loop is written against
                    // blocking reads, so flip the stream back.
                    use interprocess::local_socket::traits::Stream as _;
                    let _ = stream.set_nonblocking(false);
                    let tree = Arc::clone(&self.tree);
                    let clients = Arc::clone(&self.clients);
                    let persist = persist_tx.clone();
                    std::thread::spawn(move || handle_client(stream, tree, clients, persist));
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    if last_scrape.elapsed() >= SCRAPE_INTERVAL {
                        for notification in crate::mux::scrape::scrape_tick(&self.tree, &engine) {
                            broadcast_notification(&self.clients, &notification);
                        }
                        last_scrape = std::time::Instant::now();
                    }
                }
                // A signal may land mid-accept; that is not a listener fault.
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        // Shutdown ordering (ARC-003): join the persist worker BEFORE
        // `run_persisting`'s final synchronous save, so the worker's
        // in-flight write and the final save never touch the same tmp file
        // concurrently. Joined only on a REQUESTED shutdown — a listener
        // fault breaks the loop without one, leaving the worker detached to
        // die with the process exactly like its client threads.
        drop(persist_tx);
        if self.shutdown.load(Ordering::Relaxed) {
            if let Some(worker) = persist_worker {
                let _ = worker.join();
            }
        }
    }
}

/// Spawn the persist worker (ARC-003): the one thread that serializes and
/// fsyncs state, so no dispatch ever holds the tree lock across a write.
/// Dispatch captures a [`PersistState`] under the lock and sends it here;
/// the worker coalesces bursts to the newest state before writing.
fn spawn_persist_worker(
    path: PathBuf,
    shutdown: Arc<AtomicBool>,
) -> (Sender<PersistState>, JoinHandle<()>) {
    let (tx, rx) = channel::<PersistState>();
    let join = std::thread::spawn(move || {
        persist_worker_loop(rx, &path, &shutdown, write_state);
    });
    (tx, join)
}

/// The worker's loop, generic over the write op so the coalescing test can
/// count writes through a delegating closure instead of the file.
///
/// Never blocks indefinitely: `recv_timeout` returns at least every
/// [`PERSIST_POLL`], and the exit conditions are the channel disconnecting
/// (no senders remain) or the shutdown flag observed while idle.
fn persist_worker_loop<W>(
    rx: Receiver<PersistState>,
    path: &Path,
    shutdown: &AtomicBool,
    mut write: W,
) where
    W: FnMut(&PersistState, &Path) -> Result<(), crate::mux::persist::PersistError>,
{
    let mut write_newest = |mut newest: PersistState| {
        // Coalesce: everything queued behind the newest arrival is strictly
        // newer state; only the last needs writing.
        while let Ok(later) = rx.try_recv() {
            newest = later;
        }
        if let Err(err) = write(&newest, path) {
            log::error!("par-mux: state save to {} failed: {err}", path.display());
        }
    };
    loop {
        match rx.recv_timeout(PERSIST_POLL) {
            Ok(newest) => write_newest(newest),
            Err(RecvTimeoutError::Timeout) => {
                if shutdown.load(Ordering::Relaxed) {
                    // A dispatch may have raced the timeout — drain once
                    // more before exiting.
                    if let Ok(newest) = rx.try_recv() {
                        write_newest(newest);
                    }
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
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
fn handle_client(
    stream: LocalStream,
    tree: Arc<Mutex<MuxTree>>,
    clients: Clients,
    persist: Option<Sender<PersistState>>,
) {
    let client_id = CLIENT_SEQ.fetch_add(1, Ordering::Relaxed);
    // Bounded per ARC-011: a client that stops draining is evicted by
    // [`push_to_clients`] once its queue fills, rather than buffering every
    // `%output` line forever.
    let (tx, rx) = sync_channel::<String>(CLIENT_QUEUE_DEPTH);
    let mut registered = false;

    let mut writer = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    };
    std::thread::spawn(move || {
        while let Ok(line) = rx.recv() {
            if writer.write_all(line.as_bytes()).is_err() || writer.flush().is_err() {
                break;
            }
        }
    });

    let reader = BufReader::new(stream);
    let mut command_number = 0u32;
    for line in reader.lines() {
        let Ok(line) = line else { break };
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
                if !registered {
                    clients.lock().push((client_id, tx.clone()));
                    registered = true;
                }
                command_number += 1;
                let ctx = Ctx {
                    tree: &tree,
                    clients: &clients,
                    command_number,
                };
                let reply = dispatch_command(command, &ctx, persist.as_ref(), Some(&tx));
                if tx.send(reply).is_err() {
                    break;
                }
            }
            Err(err) => {
                if !registered {
                    clients.lock().push((client_id, tx.clone()));
                    registered = true;
                }
                command_number += 1;
                if tx.send(emit_block(command_number, &err, false)).is_err() {
                    break;
                }
            }
        }
    }
    if registered {
        clients.lock().retain(|(id, _)| *id != client_id);
    }
}

/// Execute one parsed-or-not command line and render its reply block, with
/// an issuer channel — the line-level entry the unit tests below drive. The
/// socket path parses once via [`parse_line`] and enters dispatch with the
/// parsed command, so this wrapper exists for line-shaped callers.
#[cfg(test)]
fn dispatch_issued(
    line: &str,
    command_number: u32,
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Clients,
    persist: Option<&Sender<PersistState>>,
    issuer: Option<&SyncSender<String>>,
) -> String {
    match parse_command(line) {
        Ok(command) => {
            let ctx = Ctx {
                tree,
                clients,
                command_number,
            };
            dispatch_command(command, &ctx, persist, issuer)
        }
        Err(err) => emit_block(command_number, &err, false),
    }
}

/// Execute one command with no issuer channel — the in-process shape tests
/// and tooling use.
#[cfg(test)]
fn dispatch(
    line: &str,
    command_number: u32,
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Clients,
    persist: Option<&Sender<PersistState>>,
) -> String {
    dispatch_issued(line, command_number, tree, clients, persist, None)
}

/// Push one line to every connected client, dropping the senders whose
/// client has gone — the single fan-out every broadcast and sink shares.
///
/// ARC-011: the queues are bounded at [`CLIENT_QUEUE_DEPTH`]; a client
/// whose queue is full (it stopped reading the socket) is EVICTED here,
/// tmux's policy for a control client that stops draining. Dropping the
/// sender closes the writer thread's channel, its `recv` errors, the
/// thread exits, and the client observes a clean disconnect.
pub(crate) fn push_to_clients(clients: &Clients, line: String) {
    clients
        .lock()
        .retain(|(id, tx)| match tx.try_send(line.clone()) {
            Ok(()) => true,
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                log::warn!("par-mux: client {id} is not draining; evicting");
                false
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => false,
        });
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
/// path's counterpart of the per-pane wiring each creation command does.
fn wire_all_pane_outputs(tree: &Arc<Mutex<MuxTree>>, clients: &Clients) {
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
            pane.on_output(pane_output_sink(clients, pane_id));
        }
    }
}

/// Push a `%layout-change` for `window_id` to every connected client.
///
/// tmux subscribers re-render their local pane grid from the wire layout
/// string; the visible-layout copy and raw flags mirror tmux's frame shape
/// (same string, empty flags) rather than carrying state Phase 2 does not
/// track. Rendered after the tree lock is released, so a slow client
/// channel never holds up a mutation.
pub(crate) fn broadcast_layout_change(
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Clients,
    window_id: WindowId,
) {
    let line = {
        let guard = tree.lock();
        let Some(window) = guard.window(window_id) else {
            return;
        };
        let layout = window
            .layout
            .render(0, 0, window.cols as usize, window.rows as usize);
        emit(&TmuxNotification::LayoutChange {
            window_id: window_id.to_string(),
            window_layout: layout.clone(),
            window_visible_layout: layout,
            window_raw_flags: String::new(),
        })
    };
    push_to_clients(clients, line);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[cfg(unix)]
    #[test]
    fn graceful_shutdown_pushes_exit_to_clients_before_closing() {
        use crate::mux::ipc::connect_local_stream;

        let mut path = std::env::temp_dir();
        path.push(format!(
            "par-mux-exit-{}-{}.sock",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);
        let server = MuxServer::bind(&path).expect("bind");
        let shutdown = server.shutdown_handle();
        std::thread::spawn(move || server.run());

        let stream = connect_local_stream(&path).expect("connect");
        // Reading happens on a thread with channel deadlines (the
        // interprocess stream exposes no set_read_timeout). Registration
        // must be PROVEN before requesting shutdown: a command round-trip
        // means handle_client has registered this connection's sender, so
        // the shutdown broadcast has someone to reach.
        let (tx, rx) = channel();
        let (reply_seen_tx, reply_seen_rx) = channel();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader, Write};
            let mut writer = stream.try_clone().expect("clone stream");
            writeln!(writer, "list-sessions").expect("write command");
            writer.flush().expect("flush");
            let mut reader = BufReader::new(stream);
            let mut in_reply_block = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line.starts_with("%begin") {
                    in_reply_block = true;
                } else if line.starts_with("%end") || line.starts_with("%error") {
                    in_reply_block = false;
                    let _ = reply_seen_tx.send(());
                } else if !in_reply_block {
                    let _ = tx.send(line);
                }
            }
        });
        reply_seen_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("command reply arrives first");
        shutdown.store(true, Ordering::Relaxed);

        let line = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("%exit arrives before the socket closes");
        assert_eq!(line, "%exit\n", "graceful shutdown pushes %exit first");
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn bind_replaces_a_stale_socket_file_and_sets_mode_0600() {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "par-mux-stale-{}-{}.sock",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"stale junk").expect("write stale file");

        let server = MuxServer::bind(&path).expect("bind replaces a stale socket file");
        assert_eq!(server.path(), path.as_path());

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("socket file exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "socket must be owner-only");

        drop(server);
        let _ = std::fs::remove_file(&path);
    }

    fn harness() -> (Arc<Mutex<MuxTree>>, Clients) {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
            ShellPaneFactory::default(),
        ))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        (tree, clients)
    }

    /// A factory whose panes stay silent, so a capture-range test sees only
    /// the bytes it fed the terminal itself — no shell-prompt races.
    struct SilentPaneFactory;

    impl crate::mux::pane::PaneFactory for SilentPaneFactory {
        fn create_pane(
            &self,
            id: crate::mux::ids::PaneId,
            cols: u16,
            rows: u16,
            _command: Option<&str>,
        ) -> Result<crate::mux::pane::MuxPane, crate::mux::pane::MuxError> {
            ShellPaneFactory::default().create_pane(id, cols, rows, Some("sleep 30"))
        }
    }

    fn quiet_harness() -> (Arc<Mutex<MuxTree>>, Clients) {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(SilentPaneFactory))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        (tree, clients)
    }

    #[test]
    fn bare_new_window_targets_the_newest_session_and_errors_without_one() {
        let (tree, clients) = harness();
        // No sessions yet: bare new-window is an error, like tmux's
        // "no current client" refusal.
        let reply = dispatch("new-window", 1, &tree, &clients, None);
        assert!(reply.contains("%error"), "no sessions: {reply}");

        dispatch("new-session -s first", 2, &tree, &clients, None);
        dispatch("new-session -s second", 3, &tree, &clients, None);
        let second = tree.lock().sessions()[1];
        let first = tree.lock().sessions()[0];

        let reply = dispatch("new-window -n bare", 4, &tree, &clients, None);
        assert!(reply.contains("%end"), "bare new-window succeeds: {reply}");
        // The window landed in the most-recently-created session, not the
        // first one.
        assert_eq!(tree.lock().session(second).unwrap().windows.len(), 2);
        assert_eq!(tree.lock().session(first).unwrap().windows.len(), 1);
    }

    #[test]
    fn new_window_dispatch_creates_a_window_and_wires_its_pane() {
        let (tree, clients) = harness();
        let session_reply = dispatch("new-session -s main", 1, &tree, &clients, None);
        assert!(session_reply.contains("%end"), "new-session succeeds");

        let session_id = tree.lock().sessions()[0];
        let reply = dispatch(
            &format!("new-window -t {session_id}"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "new-window succeeds: {reply}");

        let session = tree.lock().session(session_id).unwrap().windows.clone();
        assert_eq!(session.len(), 2, "session now has two windows");
    }

    #[test]
    fn window_lifecycle_dispatch_round_trips() {
        let (tree, clients) = harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        dispatch(
            &format!("new-window -t {session_id}"),
            2,
            &tree,
            &clients,
            None,
        );
        let window_id = tree.lock().session(session_id).unwrap().windows[1];

        let select = dispatch(
            &format!("select-window -t {window_id}"),
            3,
            &tree,
            &clients,
            None,
        );
        assert!(select.contains("%end"), "select-window succeeds");
        assert_eq!(tree.lock().session(session_id).unwrap().active, 1);

        let rename = dispatch(
            &format!("rename-window -t {window_id} scratch"),
            4,
            &tree,
            &clients,
            None,
        );
        assert!(rename.contains("%end"), "rename-window succeeds");
        assert_eq!(tree.lock().window(window_id).unwrap().name, "scratch");

        let list_windows = dispatch("list-windows", 5, &tree, &clients, None);
        assert!(list_windows.contains("scratch"));

        let list_sessions = dispatch("list-sessions", 6, &tree, &clients, None);
        assert!(list_sessions.contains("main"));

        let kill = dispatch(
            &format!("kill-window -t {window_id}"),
            7,
            &tree,
            &clients,
            None,
        );
        assert!(kill.contains("%end"), "kill-window succeeds");
        assert!(tree.lock().window(window_id).is_none());
    }

    /// Collect every broadcast a non-issuing client receives within a bounded
    /// window after a dispatch, so lifecycle-notification assertions see the
    /// push lines rather than the command's own reply block.
    fn drain_broadcasts(rx: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
        let mut lines = Vec::new();
        while let Ok(line) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
            lines.push(line);
        }
        lines
    }

    #[test]
    fn mutating_dispatches_broadcast_lifecycle_notifications_to_other_clients() {
        let (tree, clients) = harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let first = tree.lock().window(window_id).unwrap().panes()[0];

        // A second, non-issuing client: everything it sees is a broadcast.
        let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
        clients.lock().push((u64::MAX, tx));

        // new-window broadcasts %window-add naming the new window.
        dispatch(
            &format!("new-window -t {session_id} -n logs"),
            2,
            &tree,
            &clients,
            None,
        );
        let second_window = tree.lock().session(session_id).unwrap().windows[1];
        let lines = drain_broadcasts(&rx);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("%window-add") && l.contains(&second_window.to_string())),
            "new-window must broadcast %window-add naming it: {lines:?}"
        );

        // rename-window broadcasts %window-renamed with the new name.
        dispatch(
            &format!("rename-window -t {window_id} scratch"),
            3,
            &tree,
            &clients,
            None,
        );
        let lines = drain_broadcasts(&rx);
        assert!(
            lines.iter().any(|l| l.starts_with("%window-renamed")
                && l.contains(&window_id.to_string())
                && l.contains("scratch")),
            "rename-window must broadcast %window-renamed: {lines:?}"
        );

        // split-window focuses the new pane; select-pane moves focus back.
        dispatch(
            &format!("split-window -t {first} -h"),
            4,
            &tree,
            &clients,
            None,
        );
        let _ = drain_broadcasts(&rx);
        dispatch(&format!("select-pane -t {first}"), 5, &tree, &clients, None);
        let lines = drain_broadcasts(&rx);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("%window-pane-changed") && l.contains(&first.to_string())),
            "select-pane must broadcast %window-pane-changed: {lines:?}"
        );

        // kill-pane changes geometry (and focus, since the active pane died).
        let second_pane = tree.lock().window(window_id).unwrap().panes()[1];
        dispatch(
            &format!("kill-pane -t {second_pane}"),
            6,
            &tree,
            &clients,
            None,
        );
        let lines = drain_broadcasts(&rx);
        assert!(
            lines.iter().any(|l| l.starts_with("%layout-change")),
            "kill-pane must broadcast %layout-change: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.starts_with("%window-pane-changed")),
            "kill-pane of the active pane must broadcast %window-pane-changed: {lines:?}"
        );

        // kill-window broadcasts %window-close naming the window.
        dispatch(
            &format!("kill-window -t {second_window}"),
            7,
            &tree,
            &clients,
            None,
        );
        let lines = drain_broadcasts(&rx);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("%window-close") && l.contains(&second_window.to_string())),
            "kill-window must broadcast %window-close naming it: {lines:?}"
        );
    }

    #[test]
    fn new_session_broadcasts_window_add_and_notifies_the_issuer() {
        let (tree, clients) = harness();
        // The issuing client is NOT in the broadcast set: its channel receives
        // only what dispatch directs to it specifically.
        let (issuer_tx, issuer_rx) = sync_channel(CLIENT_QUEUE_DEPTH);
        dispatch_issued(
            "new-session -s main",
            1,
            &tree,
            &clients,
            None,
            Some(&issuer_tx),
        );
        let lines = drain_broadcasts(&issuer_rx);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("%session-changed") && l.contains("main")),
            "the issuing client must be told %session-changed: {lines:?}"
        );
        // The broadcast set (no other clients here) separately receives
        // %window-add for the session's initial window; that half is covered
        // by mutating_dispatches_broadcast_lifecycle_notifications_to_other_clients.
    }

    #[test]
    fn window_commands_report_an_error_block_for_an_unknown_target() {
        let (tree, clients) = harness();
        let reply = dispatch("select-window -t @999", 1, &tree, &clients, None);
        assert!(
            reply.contains("%error"),
            "unknown window is an error: {reply}"
        );
    }

    #[test]
    fn split_window_dispatch_splits_and_broadcasts_a_layout_change() {
        let (tree, clients) = harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

        // A second client observes the broadcast.
        let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
        clients.lock().push((u64::MAX, tx));

        let reply = dispatch(
            &format!("split-window -t {pane_id} -h -p 25"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "split-window succeeds: {reply}");

        let notification = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("a %layout-change is broadcast");
        assert!(
            notification.contains("%layout-change"),
            "notification: {notification}"
        );
        assert!(
            notification.contains(&window_id.to_string()),
            "names the mutated window: {notification}"
        );

        // The split geometry is real: -p 25 gives the new pane 20 of 80 cols.
        let geo = {
            let guard = tree.lock();
            let window = guard.window(window_id).unwrap();
            window
                .layout
                .geometry(0, 0, window.cols as usize, window.rows as usize)
        };
        let widths: Vec<_> = geo.iter().map(|g| (g.pane, g.width)).collect();
        assert!(
            widths.contains(&(pane_id, 60)),
            "target keeps 60 cols: {widths:?}"
        );
        assert!(
            widths.iter().any(|&(_, width)| width == 20),
            "new pane gets 20 cols: {widths:?}"
        );
    }

    #[test]
    fn pane_commands_dispatch_through_the_tree() {
        let (tree, clients) = harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let first = tree.lock().window(window_id).unwrap().panes()[0];

        // Side by side, so the -R resize below moves the shared divider.
        let split = dispatch(
            &format!("split-window -t {first} -h"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(split.contains("%end"), "split-window succeeds: {split}");
        // The reply body carries the new pane id — a client needs it to
        // address the pane it just created.
        let second = tree.lock().window(window_id).unwrap().panes()[1];
        assert!(
            split.contains(&second.to_string()),
            "reply names the new pane: {split}"
        );

        let select = dispatch(&format!("select-pane -t {first}"), 3, &tree, &clients, None);
        assert!(select.contains("%end"), "select-pane succeeds: {select}");
        assert_eq!(tree.lock().window(window_id).unwrap().active, first);

        let resize = dispatch(
            &format!("resize-pane -t {first} -R 10"),
            4,
            &tree,
            &clients,
            None,
        );
        assert!(resize.contains("%end"), "resize-pane succeeds: {resize}");

        let swap = dispatch(
            &format!("swap-pane -t {first} -s {second}"),
            5,
            &tree,
            &clients,
            None,
        );
        assert!(swap.contains("%end"), "swap-pane succeeds: {swap}");
        assert_eq!(
            tree.lock().window(window_id).unwrap().panes(),
            vec![second, first],
            "the panes traded positions"
        );
    }

    #[test]
    fn pane_commands_report_an_error_block_for_unknown_panes() {
        let (tree, clients) = harness();
        for command in [
            "split-window -t %999",
            "select-pane -t %999",
            "resize-pane -t %999 -R",
            "resize-pane -t %999 -x 40",
            "swap-pane -t %999 -s %998",
        ] {
            let reply = dispatch(command, 1, &tree, &clients, None);
            assert!(reply.contains("%error"), "{command} is an error: {reply}");
        }
    }

    #[test]
    fn resize_pane_absolute_dispatches_and_tracks_the_terminals() {
        let (tree, clients) = harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let first = tree.lock().window(window_id).unwrap().panes()[0];
        let split = dispatch(
            &format!("split-window -t {first} -h"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(split.contains("%end"));
        let second = tree.lock().window(window_id).unwrap().panes()[1];

        // A second client observes the size-driven re-layout.
        let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
        clients.lock().push((u64::MAX, tx));

        let reply = dispatch(
            &format!("resize-pane -t {first} -x 25"),
            3,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "absolute resize succeeds: {reply}");

        let notification = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("a %layout-change follows the size-driven re-layout");
        assert!(
            notification.contains("%layout-change"),
            "notification: {notification}"
        );

        let size_of = |pane| tree.lock().pane(pane).unwrap().terminal().read().size();
        assert_eq!(size_of(first), (25, 24));
        assert_eq!(
            size_of(second),
            (55, 24),
            "the sibling absorbs the difference on the wire too"
        );
    }

    #[test]
    fn refresh_client_size_report_resizes_the_window_and_broadcasts() {
        let (tree, clients) = harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

        let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
        clients.lock().push((u64::MAX, tx));

        let reply = dispatch(
            &format!("refresh-client -t {pane_id} -C 120x40"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "-C report succeeds: {reply}");

        let notification = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("a %layout-change follows the window resize");
        assert!(
            notification.contains("%layout-change") && notification.contains("120x40"),
            "the broadcast carries the new geometry: {notification}"
        );

        let guard = tree.lock();
        let window = guard.window(window_id).unwrap();
        assert_eq!((window.cols, window.rows), (120, 40));
        assert_eq!(
            guard.pane(pane_id).unwrap().terminal().read().size(),
            (120, 40),
            "the pane terminal was re-fitted"
        );
    }

    #[test]
    fn refresh_client_size_report_rejects_an_unknown_pane() {
        let (tree, clients) = harness();
        let reply = dispatch("refresh-client -t %999 -C 120x40", 1, &tree, &clients, None);
        assert!(reply.contains("%error"));
    }

    #[test]
    fn capture_pane_reports_the_pane_screen() {
        let (tree, clients) = harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

        // A freshly spawned pane's screen is empty until the shell writes a
        // prompt; assert the reply is well-formed rather than racing that.
        let reply = dispatch(
            &format!("capture-pane -t {pane_id} -p"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "capture-pane succeeds: {reply}");
    }

    #[test]
    fn capture_pane_rejects_an_unknown_pane() {
        let (tree, clients) = harness();
        let reply = dispatch("capture-pane -t %999 -p", 1, &tree, &clients, None);
        assert!(reply.contains("%error"));
    }

    #[test]
    fn capture_range_slices_history_with_tmux_negative_offsets() {
        // export_scrollback emits newest-first: h1 is the oldest history
        // line, h3 the newest (the line directly above a two-line screen).
        let out = capture_range("h3\nh2\nh1\n", "s1\ns2", Some(-2), Some(-1));
        assert_eq!(out, "h2\nh3");
    }

    #[test]
    fn capture_range_positive_offsets_address_screen_lines() {
        let out = capture_range("h3\nh2\nh1\n", "s1\ns2", Some(0), Some(1));
        assert_eq!(out, "s1\ns2");
    }

    #[test]
    fn capture_range_start_only_defaults_end_to_the_screen_bottom() {
        let out = capture_range("h3\nh2\nh1\n", "s1\ns2", Some(-3), None);
        assert_eq!(out, "h1\nh2\nh3\ns1\ns2");
    }

    #[test]
    fn capture_range_end_only_defaults_start_to_the_first_screen_line() {
        let out = capture_range("h3\nh2\nh1\n", "s1\ns2", None, Some(0));
        assert_eq!(out, "s1");
    }

    #[test]
    fn capture_range_clamps_offsets_beyond_the_buffer() {
        let out = capture_range("h1\n", "s1\ns2", Some(-99), Some(99));
        assert_eq!(out, "h1\ns1\ns2");
    }

    #[test]
    fn capture_range_returns_empty_for_a_reversed_range() {
        let out = capture_range("h2\nh1\n", "s1", Some(1), Some(-1));
        assert_eq!(out, "");
    }

    #[test]
    fn capture_pane_s_and_e_return_the_requested_line_range() {
        let (tree, clients) = quiet_harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

        // Thirty lines on a 24-row pane: L01..L06 scroll into history and
        // L07..L30 stay visible. No trailing newline, so L30 holds the last
        // row and nothing scrolls past it.
        let payload = (1..=30)
            .map(|n| format!("L{n:02}"))
            .collect::<Vec<_>>()
            .join("\r\n");
        tree.lock()
            .pane_mut(pane_id)
            .unwrap()
            .terminal()
            .write()
            .process(payload.as_bytes());

        let reply = dispatch(
            &format!("capture-pane -t {pane_id} -p -S -2 -E -1"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "capture succeeds: {reply}");
        assert!(
            reply.contains("L05"),
            "second-to-last history line: {reply}"
        );
        assert!(reply.contains("L06"), "last history line: {reply}");
        assert!(
            !reply.contains("L04"),
            "-S -2 starts at the second history line back: {reply}"
        );
        assert!(
            !reply.contains("L07"),
            "-E -1 ends above the visible screen: {reply}"
        );
    }

    #[test]
    fn buffer_round_trips_through_set_and_show() {
        let (tree, clients) = harness();
        let empty = dispatch("show-buffer", 1, &tree, &clients, None);
        assert!(empty.contains("%error"), "no buffer yet: {empty}");

        let set = dispatch("set-buffer hello world", 2, &tree, &clients, None);
        assert!(set.contains("%end"), "set-buffer succeeds");

        let show = dispatch("show-buffer", 3, &tree, &clients, None);
        assert!(show.contains("hello world"), "show-buffer: {show}");
    }

    #[test]
    fn paste_buffer_writes_the_buffer_to_the_target_pane() {
        let (tree, clients) = harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let pane_id = tree.lock().session(session_id).unwrap().windows[0];
        let pane_id = tree.lock().window(pane_id).unwrap().panes()[0];

        dispatch("set-buffer echo par-mux-paste", 2, &tree, &clients, None);
        let reply = dispatch(
            &format!("paste-buffer -t {pane_id}"),
            3,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "paste-buffer succeeds: {reply}");
    }

    #[test]
    fn paste_buffer_rejects_an_unknown_pane() {
        let (tree, clients) = harness();
        dispatch("set-buffer hi", 1, &tree, &clients, None);
        let reply = dispatch("paste-buffer -t %999", 2, &tree, &clients, None);
        assert!(reply.contains("%error"));
    }

    #[test]
    fn paste_buffer_with_no_stored_buffer_is_an_error() {
        let (tree, clients) = harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let pane_id = tree.lock().session(session_id).unwrap().windows[0];
        let pane_id = tree.lock().window(pane_id).unwrap().panes()[0];

        let reply = dispatch(
            &format!("paste-buffer -t {pane_id}"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%error"), "no buffer set: {reply}");
    }

    #[test]
    fn mutating_dispatch_saves_state_and_read_only_dispatch_does_not() {
        let (tree, clients) = quiet_harness();
        let target = std::env::temp_dir()
            .join(format!("par-mux-server-state-{}", std::process::id()))
            .join("state.json");
        let _ = std::fs::remove_file(&target);

        let (tx, worker) = spawn_persist_worker(target.clone(), Arc::new(AtomicBool::new(false)));

        dispatch("list-sessions", 1, &tree, &clients, Some(&tx));
        assert!(
            !target.exists(),
            "a read-only dispatch must not touch the state file"
        );

        dispatch("new-session -s main", 2, &tree, &clients, Some(&tx));
        // The worker writes off the dispatch path, so poll for the landing
        // rather than asserting synchronously (the wait_until shape).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !target.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(target.exists(), "a mutating dispatch saves the state file");

        match crate::mux::persist::load_or_quarantine(&target) {
            crate::mux::persist::Loaded::State(state) => {
                assert_eq!(state.format_version, crate::mux::persist::FORMAT_VERSION);
                assert_eq!(state.sessions.len(), 1);
                assert_eq!(state.sessions[0].name, "main");
            }
            other => panic!("expected a readable state file, got {other:?}"),
        }

        // The channel closing (all senders dropped) is the worker's exit.
        drop(tx);
        let _ = worker.join();
        let _ = std::fs::remove_file(&target);
    }

    /// A burst of queued states must coalesce: fewer writes than states,
    /// and the last state written is the newest one sent.
    #[test]
    fn persist_worker_coalesces_a_burst_to_the_newest_state() {
        let (tx, rx) = channel::<PersistState>();
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let last_seen = Arc::new(Mutex::new(None::<u64>));
        let stop = Arc::new(AtomicBool::new(false));

        let write_count = Arc::clone(&writes);
        let seen = Arc::clone(&last_seen);
        let worker = std::thread::spawn(move || {
            persist_worker_loop(
                rx,
                Path::new("/nonexistent-par-mux-coalescing-test"),
                &stop,
                |state: &PersistState, _path| {
                    write_count.fetch_add(1, Ordering::Relaxed);
                    *seen.lock() = Some(state.saved_at_unix_ms);
                    Ok(())
                },
            )
        });

        // Hand-constructed states with distinct capture stamps; the worker
        // cannot keep up with 50 in-flight sends, so writes < sends.
        let burst: u64 = 50;
        for stamp in 1..=burst {
            let state = PersistState {
                format_version: crate::mux::persist::FORMAT_VERSION,
                saved_at_unix_ms: stamp,
                next_ids: (0, 0, 0),
                sessions: Vec::new(),
                buffers: std::collections::HashMap::new(),
            };
            tx.send(state).expect("worker owns the receiver");
        }
        drop(tx);
        worker.join().expect("worker exits when the channel closes");
        assert!(
            (writes.load(Ordering::Relaxed) as u64) < burst,
            "a burst of {burst} states coalesced to {} writes",
            writes.load(Ordering::Relaxed)
        );
        assert_eq!(
            last_seen.lock().as_ref().copied(),
            Some(burst),
            "the newest state is the one written"
        );
    }

    /// ARC-011: a client whose queue fills (it stopped reading the socket)
    /// is evicted from the broadcast set, while a draining sibling still
    /// receives every line pushed past the eviction.
    #[test]
    fn a_stalled_client_is_evicted_and_a_draining_sibling_keeps_every_line() {
        let clients: Clients = Arc::new(Mutex::new(Vec::new()));

        // The stalled client: registered, never drained.
        let (stalled_tx, _stalled_rx) = sync_channel::<String>(CLIENT_QUEUE_DEPTH);
        clients.lock().push((1, stalled_tx));
        // The sibling: drains as lines arrive, like a healthy reader thread.
        let (sibling_tx, sibling_rx) = sync_channel::<String>(CLIENT_QUEUE_DEPTH);
        clients.lock().push((2, sibling_tx));

        let mut received = Vec::new();
        for n in 0..=CLIENT_QUEUE_DEPTH {
            push_to_clients(&clients, format!("line-{n}"));
            while let Ok(line) = sibling_rx.try_recv() {
                received.push(line);
            }
        }

        assert_eq!(
            clients.lock().len(),
            1,
            "the stalled client was evicted; the draining sibling remains"
        );
        while let Ok(line) = sibling_rx.try_recv() {
            received.push(line);
        }
        assert_eq!(
            received.len(),
            CLIENT_QUEUE_DEPTH + 1,
            "the sibling saw every line, including the one that evicted the stalled client"
        );
        assert_eq!(received.first().map(String::as_str), Some("line-0"));
        let last = format!("line-{CLIENT_QUEUE_DEPTH}");
        assert_eq!(received.last().map(String::as_str), Some(last.as_str()));
    }
}
