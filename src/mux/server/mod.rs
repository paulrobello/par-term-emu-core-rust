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
use crate::mux::config::EffectiveConfig;
use crate::mux::dispatch::{
    dispatch_command, workspace_roster_changed, workspace_roster_fingerprint, Ctx,
};
use crate::mux::emit::{emit, emit_block};
use crate::mux::ids::{PaneId, WindowId};
use crate::mux::ipc::{
    accept_connection, bind_local_listener, prepare_socket_path, ConnectionAbort, LocalListener,
    LocalStream,
};
use crate::mux::pane::{OutputSink, ShellPaneFactory};
use crate::mux::persist::{write_job, PersistState, SaveOrigin};
use crate::mux::tree::MuxTree;
use crate::tmux_control::TmuxNotification;
// The Windows LocalListener wrapper exposes accept/set_nonblocking as
// inherent methods, so the trait import is only reachable — and only used —
// on Unix.
#[cfg(unix)]
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

/// How often the accept loop's idle poll runs the pane reaper — the pass
/// that closes panes whose child exited (tmux semantics: a pane dies with
/// its process). The PTY reader flips `is_running` on EOF within
/// milliseconds of the exit; this poll bounds the close latency.
const REAP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// How often the persist worker re-checks the shutdown flag while idle.
/// Bounds how long the shutdown join waits after the flag is set.
const PERSIST_POLL: Duration = Duration::from_millis(200);

/// How long a persisting daemon tolerates an empty tree — zero sessions,
/// or only dead panes — before exiting, CONNECTED CLIENTS NOTWITHSTANDING
/// (tmux's `exit-empty`: the last session dying under an attached client
/// still ends the server, and the client learns through the broadcast
/// %exit). The grace keeps the two races it could lose won instead: a
/// session created (or a pane respawned) within the window resets the
/// clock, and a logout's SIGTERM — which follows pane deaths within
/// moments — beats the grace, so the reboot-race resurrection
/// ([`SaveOrigin::Shutdown`]) stays intact. `daemon.exit-empty = false`
/// (the live config) holds the daemon past the grace however long it sits
/// empty.
#[cfg(not(test))]
const EXIT_EMPTY_GRACE: Duration = Duration::from_secs(5);

/// Tests run the accept loop for real; a 5 s grace would dominate every
/// test's runtime.
#[cfg(test)]
const EXIT_EMPTY_GRACE: Duration = Duration::from_millis(300);

/// Per-client broadcast queue depth in lines (ARC-011). A `%output` line
/// carries one PTY read (up to 16 KiB raw, roughly doubled by escape
/// encoding), so a stalled client pins at most ~128 MiB before eviction;
/// past that it is disconnected — and the disconnect frees the queue
/// (ENH-012) — rather than growing the daemon without bound (tmux's own
/// policy for a control client that stops draining).
/// cap: Broadcast lines queued per control-socket client before the daemon evicts it.
const CLIENT_QUEUE_DEPTH: usize = 4096;

/// Per-line byte budget for the control-socket read loop (SEC-104). A line
/// is accumulated until its newline, so without a budget a client streaming
/// an unterminated line grows the daemon's memory until the socket closes.
/// Over budget answers one `%error` block and closes the connection — same
/// exposure class as the queue depth above (peer-euid-verified same user),
/// so this bounds accidental growth, not an adversary.
/// cap: Bytes accumulated from one control-socket client line before the daemon closes it.
const MAX_CONTROL_LINE_BYTES: usize = 1024 * 1024;

/// How long shutdown waits for the host-probe worker before detaching it
/// (SEC-133).
const PROBE_JOIN_BOUND: Duration = Duration::from_secs(1);

/// How often an evicted client's connection threads re-check the eviction
/// flag. Both the writer and the reader of a connection run with
/// send/recv timeouts of this length, so eviction tears a wedged client
/// down within about two polls even though a blocked socket call cannot
/// be interrupted from outside.
const EVICTION_POLL: Duration = Duration::from_millis(200);

/// Monotonic client ids, so a disconnecting client's broadcast sender can be
/// removed eagerly rather than waiting for the next broadcast to fail.
static CLIENT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Connected clients' broadcast senders, keyed by their monotonic id.
/// One registered control client: its identity, its bounded queue sender
/// (ARC-011), the eviction flag [`push_to_clients`] sets when the
/// queue overflows — the connection's own threads poll it to tear down
/// (ENH-012) — and the abort handle that unblocks those threads on
/// Windows, where no timeout ever wakes them.
pub(crate) type ClientEntry = (u64, SyncSender<String>, Arc<AtomicBool>, ConnectionAbort);
pub(crate) type Clients = Arc<Mutex<Vec<ClientEntry>>>;

/// Why the accept loop ended — the final save's snapshot semantics hang on
/// the difference between a requested shutdown (any emptiness may be the
/// reboot race) and an exit-when-empty (the emptiness is deliberate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoopExit {
    /// SIGTERM or `kill-server` raised the shutdown flag.
    Requested,
    /// A listener fault ended the loop.
    Fault,
    /// An empty tree — zero sessions, or only dead panes — held past
    /// [`EXIT_EMPTY_GRACE`]: tmux's exit-empty. Clients do not hold it.
    Empty,
}

/// A control-mode multiplexer server listening on a Unix socket.
pub struct MuxServer {
    listener: LocalListener,
    path: PathBuf,
    tree: Arc<Mutex<MuxTree>>,
    clients: Clients,
    /// Per-instance shutdown flag (ARC-016): the accept loop polls it, the
    /// binary's signal handler reaches it through [`Self::shutdown_handle`].
    shutdown: Arc<AtomicBool>,
    /// ENH-039: connections accepted by the tree's pane endpoints (the
    /// opt-in `--pane-endpoints` mode), forwarded by their accept threads
    /// through [`pane_endpoint_channel`]; drained and served on the accept
    /// loop's idle tick. `None` = the mode is off.
    pane_connections: Option<PaneEndpointRx>,
    /// The daemon's applied settings (`reload-config` diffs against and
    /// reports from this copy). `None` when the embedder did not set one —
    /// `reload-config` then answers with an explicit error rather than a
    /// fake reload.
    config: Option<Arc<Mutex<crate::mux::config::EffectiveConfig>>>,
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
        #[cfg(unix)]
        raise_nofile_soft_limit();
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
            pane_connections: None,
            config: None,
        })
    }

    /// [`Self::bind_with_tree`] serving pane endpoints too (ENH-039): the
    /// daemon's `--pane-endpoints` mode wires the factory half of
    /// [`pane_endpoint_channel`] into its pane factory and hands the
    /// server half here. Connections the endpoints accept are answered for
    /// their pane only — the four hook-report methods — never as control
    /// commands.
    pub fn bind_with_tree_and_pane_endpoints(
        path: &Path,
        tree: MuxTree,
        pane_connections: PaneEndpointRx,
    ) -> std::io::Result<Self> {
        let mut server = Self::bind_with_tree(path, tree)?;
        server.pane_connections = Some(pane_connections);
        Ok(server)
    }

    /// Publish the daemon's applied settings for `reload-config`. Call
    /// once after bind, before `run`/`run_persisting` — the accept loop
    /// forwards it into every dispatch context.
    pub fn set_config(&mut self, config: Arc<Mutex<EffectiveConfig>>) {
        self.config = Some(config);
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
        let _exit = self.run_with_state_path(None);
        release_socket_path(&self.path);
    }

    /// [`Self::run`] with the whole state atomically saved to `state_path`
    /// after every mutating dispatch — the mode the `par-mux` daemon runs
    /// in. On a requested shutdown (Task 3.5) a final save captures content
    /// that arrived since the last structural one, so a clean SIGTERM never
    /// loses the last window; a listener fault that ends the loop takes the
    /// same save on its way out, so an accept error never silently discards
    /// unsaved work. An exit-when-empty (below: no clients, and no
    /// sessions or only dead panes) saves with [`SaveOrigin::ShutdownEmpty`]:
    /// an empty tree clears the last-good snapshot, while an all-dead tree
    /// refreshes it so the next start respawns those panes.
    /// Callers resolve the path with
    /// [`crate::mux::persist::state_file_path`].
    pub fn run_persisting(self, state_path: PathBuf) {
        let exit = self.run_with_state_path(Some(state_path.clone()));
        let origin = match exit {
            LoopExit::Empty => SaveOrigin::ShutdownEmpty,
            LoopExit::Fault | LoopExit::Requested => SaveOrigin::Shutdown,
        };
        if let Err(err) = crate::mux::persist::save_off_lock(&self.tree, &state_path, origin) {
            log::error!("par-mux: final state save failed: {err}");
        }
        // The socket path is released only AFTER the final save is on disk.
        // `par-mux --stop`/`--restart` waits for the socket to stop
        // accepting, so an unlink inside the accept-loop teardown made that
        // wait return before this save ran — the next daemon's restore could
        // then read a missing or stale state file (the intermittent
        // "fresh daemon came up empty" handoff bug).
        release_socket_path(&self.path);
    }

    /// A shareable handle to this server's shutdown flag (ARC-016): storing
    /// `true` stops this instance's accept loop on its next tick. One atomic
    /// store, so a signal handler may write it directly. Instances are
    /// independent — a handle cannot stop another server in the same process.
    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }

    fn run_with_state_path(&self, state_path: Option<PathBuf>) -> LoopExit {
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

        // Card 01a0e3f1205371619309073eb5f803d6: the host-probe sweep
        // (disk + git per pane cwd) owns its own thread — its
        // deadline-bounded git invocations must never stall the accept
        // loop's idle poll or any roster query, which reads only what
        // the sweep already wrote.
        let probe_worker = crate::mux::host_probe::spawn_host_probe_worker(
            Arc::clone(&self.tree),
            Arc::clone(&self.shutdown),
        );

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
        let mut last_reap = std::time::Instant::now();
        // When the daemon (persisting only) first observed an empty tree —
        // zero sessions, or only dead panes; clients do not hold it —
        // reset to None the moment a session (or live pane) returns. Held
        // past EXIT_EMPTY_GRACE it ends the loop as [LoopExit::Empty]
        // (when `daemon.exit-empty` is on, the default; off, it never
        // fires).
        let mut empty_since: Option<std::time::Instant> = None;
        let mut exit = LoopExit::Requested;

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                // Tell the clients the daemon is ending deliberately, so
                // they do not have to infer death from a dropped socket.
                broadcast_notification(&self.clients, &TmuxNotification::Exit);
                break;
            }
            match accept_connection(&self.listener) {
                Ok((stream, abort)) => {
                    // BSD/macOS accepted sockets inherit O_NONBLOCK from the
                    // listening socket; the handler loop is written against
                    // blocking reads, so flip the stream back.
                    use interprocess::local_socket::traits::Stream as _;
                    let _ = stream.set_nonblocking(false);
                    let tree = Arc::clone(&self.tree);
                    let clients = Arc::clone(&self.clients);
                    let persist = persist_tx.clone();
                    let shutdown = Arc::clone(&self.shutdown);
                    let config = self.config.clone();
                    std::thread::spawn(move || {
                        handle_client(
                            stream,
                            tree,
                            clients,
                            persist,
                            Some(shutdown),
                            abort,
                            config,
                        )
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    // ENH-039: drain and serve the pane endpoints' accepted
                    // connections. Each gets its own thread so one parked
                    // pane client cannot stall the accept loop; a hook
                    // exchange is one line in, one reply out.
                    if let Some(pane_connections) = &self.pane_connections {
                        while let Ok((stream, abort, pane_id)) = pane_connections.inner.try_recv() {
                            let tree = Arc::clone(&self.tree);
                            let clients = Arc::clone(&self.clients);
                            std::thread::spawn(move || {
                                let _ = abort;
                                serve_pane_connection(stream, pane_id, &tree, &clients);
                            });
                        }
                    }
                    if last_scrape.elapsed() >= SCRAPE_INTERVAL {
                        for notification in crate::mux::scrape::scrape_tick(&self.tree, &engine) {
                            broadcast_notification(&self.clients, &notification);
                        }
                        last_scrape = std::time::Instant::now();
                    }
                    if last_reap.elapsed() >= REAP_INTERVAL {
                        reap_dead_panes(
                            &self.tree,
                            &self.clients,
                            self.config.as_ref(),
                            persist_tx.as_ref(),
                        );
                        last_reap = std::time::Instant::now();
                    }
                    // Exit-when-empty, the persisting daemon only: an
                    // embedded `run()` server serves until stopped, whatever
                    // it holds. Clients do NOT hold an empty daemon — the
                    // last session dying under an attached client exits it
                    // after the grace, and the client learns through the
                    // %exit every shutdown broadcasts (tmux exit-empty
                    // semantics; the manual-pass report). Both locks are
                    // taken and released one at a time — never nested.
                    // `daemon.exit-empty = false` (the live applied config)
                    // skips the check entirely; a server without an
                    // applied config keeps the built-in on default.
                    if state_path.is_some()
                        && self
                            .config
                            .as_ref()
                            .is_none_or(|config| config.lock().exit_empty)
                    {
                        // "Empty" counts a tree whose every pane is dead
                        // too: held panes (remain-on-exit) are for clients
                        // that might come back — with nothing alive
                        // anywhere, the daemon collects itself instead of
                        // lingering on frozen screens.
                        let idle_empty = self.tree.lock().sessions().is_empty()
                            || self.tree.lock().all_panes_dead();
                        empty_since = match (idle_empty, empty_since) {
                            (true, Some(since)) => Some(since),
                            (true, None) => Some(std::time::Instant::now()),
                            (false, _) => None,
                        };
                        if empty_since.is_some_and(|since| since.elapsed() >= EXIT_EMPTY_GRACE) {
                            // Held empty past the grace: exit through the
                            // same %exit + final-save path every shutdown
                            // takes, recorded as Empty so the save's origin
                            // clears the last-good snapshot.
                            exit = LoopExit::Empty;
                            log::info!("par-mux: tree empty; daemon exiting (exit-when-empty)");
                            self.shutdown.store(true, Ordering::Relaxed);
                        }
                    }
                }
                // A signal may land mid-accept; that is not a listener fault.
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                // Transient accept faults back off instead of dying: a full
                // descriptor table (EMFILE/ENFILE — a burst of clients, or
                // a leak elsewhere) or a connection that vanished before
                // acceptance. tmux's server pauses accepting on
                // ENFILE/EMFILE the same way. Held sockets keep working;
                // new clients retry.
                Err(err) if is_transient_accept_fault(&err) => {
                    log::warn!("par-mux: accept backed off ({err}); still serving");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
                Err(err) => {
                    log::error!("par-mux: listener fault ({err}) — saving state and exiting");
                    // A fault ends the server exactly like a shutdown
                    // request, so the flag goes up here too: the persist
                    // worker's channel cannot close while connected clients
                    // hold sender clones (a silent one holds its forever),
                    // and the flag — not the sender count — is what bounds
                    // the fault-exit join below.
                    self.shutdown.store(true, Ordering::Relaxed);
                    exit = LoopExit::Fault;
                    break;
                }
            }
        }

        // Shutdown ordering (ARC-003): join the persist worker BEFORE
        // `run_persisting`'s final synchronous save, so the worker's
        // in-flight write and the final save never touch the same tmp file
        // concurrently. Every loop exit raises the shutdown flag (request,
        // fault, or exit-when-empty) and every exit takes the final save,
        // so the worker is always drained first.
        drop(persist_tx);
        if self.shutdown.load(Ordering::Relaxed) {
            if let Some(worker) = persist_worker {
                let _ = worker.join();
            }
            // At most PROBE_JOIN_BOUND, then detached (SEC-133): the worker
            // normally exits within one poll, but a probe blocked on a
            // wedged filesystem must not hold the daemon's exit.
            if !join_bounded(probe_worker, PROBE_JOIN_BOUND) {
                crate::debug_error!("MUX", "host probe still running at shutdown; detached");
            }
        }
        // The socket file is unlinked by the mode's owner — `run` at once,
        // `run_persisting` only after its final save (see there for why the
        // order is load-bearing for --stop/--restart).
        exit
    }
}

/// Unlink the listener's socket file (on Windows, the marker file) so
/// `ls par-mux-*.sock` lists only live daemons — a `--stop` that leaves
/// the file behind reads as a daemon that is still there. A crash skips
/// this; the next bind reclaims the stale remnant (`prepare_socket_path`).
fn release_socket_path(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

/// Join `handle` if it finishes within `bound`, else drop it (detaching the
/// thread). Returns whether it was joined.
fn join_bounded(handle: JoinHandle<()>, bound: Duration) -> bool {
    let deadline = std::time::Instant::now() + bound;
    while !handle.is_finished() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = handle.join();
    true
}

/// Spawn the persist worker (ARC-003): the one thread that serializes and
/// fsyncs state, so no dispatch ever holds the tree lock across a write.
/// Dispatch captures a [`PersistState`] under the lock and sends it here;
/// the worker coalesces bursts to the newest state before writing.
fn spawn_persist_worker(
    path: PathBuf,
    shutdown: Arc<AtomicBool>,
) -> (Sender<(SaveOrigin, PersistState)>, JoinHandle<()>) {
    let (tx, rx) = channel::<(SaveOrigin, PersistState)>();
    let join = std::thread::spawn(move || {
        persist_worker_loop(rx, &path, &shutdown, write_job);
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
    rx: Receiver<(SaveOrigin, PersistState)>,
    path: &Path,
    shutdown: &AtomicBool,
    mut write: W,
) where
    W: FnMut(SaveOrigin, &PersistState, &Path) -> Result<(), crate::mux::persist::PersistError>,
{
    let mut write_newest = |mut newest: (SaveOrigin, PersistState)| {
        // Coalesce: everything queued behind the newest arrival is strictly
        // newer state; only the last needs writing.
        while let Ok(later) = rx.try_recv() {
            newest = later;
        }
        if let Err(err) = write(newest.0, &newest.1, path) {
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

/// How one bounded fill attempt ended (SEC-127).
#[derive(Debug, PartialEq, Eq)]
enum LineFill {
    /// `buf` ends in `\n`.
    Complete,
    /// `buf` grew past `max`; the caller answers %error and closes.
    Oversize,
    /// EOF; `buf` holds any unterminated final line.
    Eof,
}

/// Append bytes from `reader` to `buf` until a newline, EOF, or the budget
/// trips. Errors (a recv-timeout poll wake above all) propagate with `buf`
/// intact: the bytes stay raw until the line completes, so a wake that
/// splits a multi-byte UTF-8 char loses nothing.
fn fill_line_bounded<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<LineFill> {
    loop {
        let chunk = match reader.fill_buf() {
            Ok(chunk) => chunk,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        if chunk.is_empty() {
            return Ok(LineFill::Eof);
        }
        // max + 1 - len >= 1 while len <= max, and the Oversize return below
        // stops the loop the moment len exceeds max.
        let budget = max + 1 - buf.len();
        let newline = chunk.iter().position(|&b| b == b'\n');
        let take = newline.map_or(chunk.len(), |i| i + 1).min(budget);
        buf.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if buf.len() > max {
            return Ok(LineFill::Oversize);
        }
        if buf.last() == Some(&b'\n') {
            return Ok(LineFill::Complete);
        }
    }
}

/// What [`read_control_line`] produced for one line of a connection.
#[derive(Debug, PartialEq, Eq)]
enum ControlLine {
    /// A complete line (still carrying its `\n` when it had one), or an
    /// unterminated final line read before EOF.
    Line(String),
    /// The line passed the budget; the caller answers %error and closes.
    Oversize,
    /// A line that is not UTF-8, read through its newline so framing
    /// survives; carries its byte length.
    Undecodable(usize),
    /// EOF with nothing pending, a read fault, or eviction: stop serving.
    Closed,
}

/// Read one control line (SEC-127's bounded accumulation). Bytes stay raw
/// across recv-timeout wakes — `read_line` would discard a partial whose
/// tail splits a multi-byte char — so a healthy sender's pause costs
/// nothing and only an evicted connection ends; the budget is checked per
/// chunk, so an unterminated stream trips it without a newline; UTF-8 is
/// decoded once, after the line is complete. An unterminated final line is
/// returned before EOF, matching `Lines`' last-item behavior.
fn read_control_line<R: BufRead>(reader: &mut R, evicted: &AtomicBool) -> ControlLine {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match fill_line_bounded(reader, &mut buf, MAX_CONTROL_LINE_BYTES) {
            Ok(LineFill::Eof) if buf.is_empty() => return ControlLine::Closed,
            Ok(LineFill::Eof) => break,
            Ok(LineFill::Oversize) => return ControlLine::Oversize,
            Ok(LineFill::Complete) => break,
            Err(err) if is_poll_wake(&err) => {
                if evicted.load(Ordering::Relaxed) {
                    return ControlLine::Closed;
                }
            }
            Err(_) => return ControlLine::Closed,
        }
    }
    match String::from_utf8(buf) {
        Ok(line) => ControlLine::Line(line),
        Err(err) => ControlLine::Undecodable(err.as_bytes().len()),
    }
}

/// A connection's one-time join to the broadcast registry. Deferred until
/// the first control command (or error reply), so hook-only connections
/// never receive pushes; the abort handle moves into the registry then.
/// Registration also replays held state (ENH-037): one `%pane-exited` per
/// held pane and one `%layout-change` per zoomed window, queued on the
/// client's own channel ahead of its first command's reply.
struct Registration {
    done: bool,
    /// The `%client-attached` announcement has gone out. It waits for the
    /// connection's first size report (`refresh-client -C`) so one-shot
    /// command clients, which never report one, are neither announced nor
    /// missed.
    announced: bool,
    abort: Option<ConnectionAbort>,
}

impl Registration {
    fn ensure(
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
fn handle_client(
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

/// True when a dispatch reply block ends in `%error` — the daemon's
/// rejection shape ([`emit_block`] with `ok: false`).
fn reply_is_error(reply: &str) -> bool {
    reply
        .lines()
        .last()
        .is_some_and(|l| l.starts_with("%error"))
}

// ---- ENH-039: per-pane hook-only endpoints (the opt-in `--pane-endpoints`
// daemon mode) ----

/// Max pane endpoints one daemon binds. Past the cap a new pane gets NO
/// endpoint: `PAR_MUX_SOCKET` stays unset for it rather than falling back
/// to the full control socket, which would silently restore the
/// least-privilege hole the mode exists to close. The count is the
/// endpoint socket files already beside the control socket at bind time
/// (the startup sweep has removed the stale ones). Documented in
/// docs/MUX.md "Agent Hook Reports".
pub(crate) const MAX_PANE_ENDPOINTS: usize = 256;

/// The file-name prefix of a pane endpoint beside `control_socket`:
/// `<control stem>.pane-<N>.sock`.
fn pane_endpoint_prefix(control_socket: &Path) -> String {
    let name = control_socket
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name.strip_suffix(".sock").unwrap_or(&name);
    format!("{stem}.pane-")
}

/// The socket path of `pane_id`'s endpoint: beside the control socket, in
/// the same guarded runtime directory, so the directory guard and the 0600
/// bind mode apply unchanged.
fn pane_endpoint_path(control_socket: &Path, pane_id: PaneId) -> PathBuf {
    control_socket.with_file_name(format!(
        "{}{pane_id}.sock",
        pane_endpoint_prefix(control_socket)
    ))
}

/// Whether a directory entry name is one of `control_socket`'s pane
/// endpoints (the sweep's candidate test).
fn is_pane_endpoint_name(control_socket: &Path, name: &str) -> bool {
    name.starts_with(&pane_endpoint_prefix(control_socket)) && name.ends_with(".sock")
}

/// The factory half of the pane-endpoint channel: a bound endpoint hands
/// each accepted connection here, and the server's accept loop serves it
/// against the live tree on its next idle tick. A newtype so the public
/// factory field need not name the crate-internal connection tuple.
#[derive(Clone, Debug)]
pub struct PaneEndpointTx {
    inner: std::sync::mpsc::SyncSender<(LocalStream, ConnectionAbort, PaneId)>,
}

/// The server half: drained by [`MuxServer`]'s accept loop.
#[derive(Debug)]
pub struct PaneEndpointRx {
    inner: std::sync::mpsc::Receiver<(LocalStream, ConnectionAbort, PaneId)>,
}

/// The pair a `--pane-endpoints` daemon wires between its pane factory and
/// its server. Bounded: a wedged server backpressures the endpoint accept
/// threads (the kernel accept queue then fills) instead of growing the
/// daemon without bound.
pub fn pane_endpoint_channel() -> (PaneEndpointTx, PaneEndpointRx) {
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    (PaneEndpointTx { inner: tx }, PaneEndpointRx { inner: rx })
}

/// One pane's hook-only endpoint (ENH-039): a socket beside the control
/// socket that accepts ONLY hook reports bound to its pane — a pane process
/// handed this path as `PAR_MUX_SOCKET` can report its agent state but
/// cannot drive other panes or the server. Bound by the pane factory when
/// the daemon runs with `--pane-endpoints`; dropped with the pane, which
/// unlinks the socket file (a crash or SIGKILL leaves the remnant for
/// [`sweep_pane_endpoint_remnants`]).
///
/// The accept thread holds no tree reference: accepted connections are
/// forwarded through the [`PaneEndpointTx`] channel and served by the
/// server's accept loop. A pane owning an endpoint therefore cannot keep
/// the tree — and with it the pane itself — alive; that reference cycle
/// would leak.
pub(crate) struct PaneEndpoint {
    socket_path: PathBuf,
    accept_thread: Option<std::thread::JoinHandle<()>>,
    /// Set on drop: the accept thread polls it between nonblocking accepts
    /// and exits, so the join below is bounded by one poll interval.
    closed: Arc<AtomicBool>,
}

impl PaneEndpoint {
    /// Bind `pane_id`'s endpoint beside `control_socket`.
    ///
    /// Errors past [`MAX_PANE_ENDPOINTS`] live endpoints, when the path
    /// exceeds the platform socket-address limit, or on a bind failure —
    /// the factory answers any of these the same way: the pane's env
    /// contract exports NO socket, never a fallback to the full control
    /// socket.
    pub(crate) fn bind(
        control_socket: &Path,
        pane_id: PaneId,
        sink: PaneEndpointTx,
    ) -> std::io::Result<Self> {
        let socket_path = pane_endpoint_path(control_socket, pane_id);
        let live = control_socket
            .parent()
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        is_pane_endpoint_name(control_socket, &entry.file_name().to_string_lossy())
                    })
                    .count()
            })
            .unwrap_or(0);
        if live >= MAX_PANE_ENDPOINTS {
            return Err(std::io::Error::other(format!(
                "pane endpoint cap reached ({MAX_PANE_ENDPOINTS})"
            )));
        }
        prepare_socket_path(&socket_path)?;
        let listener = bind_local_listener(&socket_path)?;

        // Nonblocking accept on a poll, so Drop can end the thread through
        // `closed` instead of leaving it parked in accept() forever — one
        // thread leaks per spawned pane otherwise.
        use interprocess::local_socket::ListenerNonblockingMode;
        listener
            .set_nonblocking(ListenerNonblockingMode::Accept)
            .expect("the listener was just bound");
        let closed = Arc::new(AtomicBool::new(false));
        let thread_closed = Arc::clone(&closed);
        let thread_sink = sink;
        let accept_thread = std::thread::Builder::new()
            .name(format!("par-mux-pane-endpoint-{pane_id}"))
            .spawn(move || loop {
                match accept_connection(&listener) {
                    Ok((stream, abort)) => {
                        if thread_sink.inner.send((stream, abort, pane_id)).is_err() {
                            break;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        if thread_closed.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    // A signal may land mid-accept; that is not a fault.
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(err) => {
                        log::warn!("par-mux: pane endpoint accept failed: {err}");
                        break;
                    }
                }
            })?;
        Ok(Self {
            socket_path,
            accept_thread: Some(accept_thread),
            closed,
        })
    }

    /// The path to export as the pane's `PAR_MUX_SOCKET`.
    pub(crate) fn socket_path_string(&self) -> String {
        self.socket_path.to_string_lossy().into_owned()
    }
}

impl Drop for PaneEndpoint {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Remove stale pane-endpoint socket remnants beside `control_socket` at
/// daemon startup (ENH-039). [`PaneEndpoint`]'s `Drop` unlinks a live
/// endpoint's socket file, but a crash or SIGKILL does not — without the
/// sweep the remnants accumulate in the runtime directory. Each candidate
/// goes through [`prepare_socket_path`], which reclaims exactly the stale
/// ones (a dead socket file or a stray regular file) and refuses a live
/// one; at startup nothing of this daemon's can be live yet, and no other
/// daemon can own these names (the control socket path is exclusive).
pub fn sweep_pane_endpoint_remnants(control_socket: &Path) {
    let Some(dir) = control_socket.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_pane_endpoint_name(control_socket, &name) {
            let _ = prepare_socket_path(&entry.path());
        }
    }
}

/// Serve one pane-endpoint connection (ENH-039): hook reports for the
/// endpoint's pane only, answered in place; any control-command line is
/// refused with the hook-only error and the connection closed. Reuses the
/// control socket's bounded line reader (SEC-104). No registration, no
/// broadcast-set membership, no eviction — a hook connection is one line
/// in, one reply out.
fn serve_pane_connection(
    stream: LocalStream,
    bound: PaneId,
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Clients,
) {
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    // A pane connection is never in the broadcast set, so nothing ever
    // evicts it; the reader's flag exists for the shared line reader only.
    let never_evicted = AtomicBool::new(false);
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = match read_control_line(&mut reader, &never_evicted) {
            ControlLine::Line(line) => line,
            ControlLine::Closed => break,
            // Over-budget or non-UTF-8 input is refused and the connection
            // closed — a pane endpoint carries one well-formed report per
            // exchange, never a stream of malformed frames.
            ControlLine::Oversize | ControlLine::Undecodable(_) => {
                let _ = write_line(
                    &mut writer,
                    "{\"error\":\"hook-only endpoint\"}",
                    &never_evicted,
                );
                break;
            }
        };
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        if line.trim().is_empty() {
            continue;
        }
        match parse_line(&line) {
            Ok(Line::Hook(report)) => {
                let (reply, broadcast) = crate::mux::hooks::handle_report_for(bound, &report, tree);
                if let Some(notification) = broadcast {
                    broadcast_notification(clients, &notification);
                }
                if write_line(&mut writer, &reply, &never_evicted).is_err() {
                    break;
                }
            }
            // Anything else — a control command or an unparseable line — is
            // what the endpoint exists to refuse. The error is one JSON
            // line, then the connection closes: a client that meant to talk
            // control-mode learns immediately, not after a timeout.
            _ => {
                let _ = write_line(
                    &mut writer,
                    "{\"error\":\"hook-only endpoint\"}",
                    &never_evicted,
                );
                break;
            }
        }
    }
}

/// Commands whose arguments carry user data (typed input, clipboard):
/// logged as name, target and size only (SEC-132).
const PAYLOAD_COMMANDS: &[&str] = &["send-keys", "set-buffer"];

/// A command line reduced for logging. A [`PAYLOAD_COMMANDS`] line keeps
/// only its name, a leading `-t` target and its size, so it is safe to log
/// whether or not it parses; any other line keeps its head and size, cut on
/// a char boundary.
fn summarize_line(line: &str) -> String {
    const HEAD: usize = 120;
    let mut tokens = line.split_whitespace();
    if let Some(name) = tokens.next().filter(|name| PAYLOAD_COMMANDS.contains(name)) {
        // The target is read only from the leading flag, so a `-t` quoted
        // inside a payload can never pull a payload token into the log.
        let target = match (tokens.next(), tokens.next()) {
            (Some("-t"), Some(value)) => format!(" -t {value}"),
            _ => String::new(),
        };
        return format!("{name}{target} [payload redacted, {} bytes]", line.len());
    }
    if line.len() <= HEAD {
        return line.to_string();
    }
    let cut = line
        .char_indices()
        .take_while(|(i, _)| *i < HEAD)
        .map(|(i, _)| i)
        .last()
        .unwrap_or(0);
    format!("{}... ({} bytes total)", &line[..cut], line.len())
}

/// Write one line, tolerating send-timeout wakes: a healthy slow consumer's
/// buffer-full pause retries the remaining bytes, while an evicted client's
/// pause abandons the line — the connection is being torn down.
fn write_line(writer: &mut LocalStream, line: &str, evicted: &AtomicBool) -> std::io::Result<()> {
    let mut buf = line.as_bytes();
    loop {
        match writer.write(buf) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "the client socket wrote zero bytes",
                ));
            }
            Ok(n) => {
                buf = &buf[n..];
                if buf.is_empty() {
                    return writer.flush();
                }
            }
            Err(err) if is_poll_wake(&err) => {
                if evicted.load(Ordering::Relaxed) {
                    return Err(err);
                }
            }
            Err(err) => return Err(err),
        }
    }
}

/// Whether an I/O error is a send/recv-timeout wake rather than a fault —
/// the timeout-driven poll loop's transient, to be retried or examined.
fn is_poll_wake(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Whether an accept error is a transient the accept loop should back off
/// from and keep serving (card 01a0d9b47393): a full descriptor table, or a
/// connection that vanished before acceptance. Everything else is a real
/// listener fault.
#[cfg(unix)]
fn is_transient_accept_fault(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::ConnectionAborted
        || matches!(err.raw_os_error(), Some(libc::EMFILE) | Some(libc::ENFILE))
}

/// Windows named-pipe listeners surface no descriptor-table errors; only
/// the vanished-connection case applies.
#[cfg(not(unix))]
fn is_transient_accept_fault(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::ConnectionAborted
}

/// Raise the RLIMIT_NOFILE soft limit toward the hard limit at daemon start
/// (card 01a0d9b2fd2c).
///
/// The daemon inherits its spawner's limits: under launchd or the Dock on
/// macOS that is a soft limit of 256, and at roughly four descriptors per
/// pane the 60th pane is where new-window starts failing with EMFILE.
/// herdr raises its server to 8192 the same way. An unbounded hard limit
/// (the macOS default) is not infinity to the kernel, so a concrete 8192 is
/// requested in that case instead. Failures are logged and survived — the
/// raise is headroom, not a guarantee.
#[cfg(unix)]
fn raise_nofile_soft_limit() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: valid rlimit out-pointer.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        log::warn!("par-mux: getrlimit(RLIMIT_NOFILE) failed — serving at the inherited limit");
        return;
    }
    let (soft, hard) = (limit.rlim_cur, limit.rlim_max);
    let target = if hard == libc::RLIM_INFINITY {
        8192
    } else {
        hard
    };
    if soft >= target {
        return;
    }
    limit.rlim_cur = target;
    // SAFETY: the limit came from getrlimit with only rlim_cur raised.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } != 0 {
        log::warn!(
            "par-mux: raising the RLIMIT_NOFILE soft limit past {soft} failed — serving at {soft}"
        );
        return;
    }
    log::info!("par-mux: raised the RLIMIT_NOFILE soft limit {soft} -> {target}");
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
    persist: Option<&Sender<(SaveOrigin, PersistState)>>,
    issuer: Option<&SyncSender<String>>,
) -> String {
    match parse_command(line) {
        Ok(command) => {
            let ctx = Ctx {
                tree,
                clients,
                command_number,
                shutdown: None,
                config: None,
                client_id: None,
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
    persist: Option<&Sender<(SaveOrigin, PersistState)>>,
) -> String {
    dispatch_issued(line, command_number, tree, clients, persist, None)
}

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
fn push_to_clients_except(clients: &Clients, skip: u64, line: String) {
    push_to_clients_skipping(clients, Some(skip), line);
}

fn push_to_clients_skipping(clients: &Clients, skip: Option<u64>, line: String) {
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
fn dispatch_contained(
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

/// One auto-remove removal: the window that held the pane, its active pane
/// when the window survived (the `%window-pane-changed` target), and
/// whether the cascade removed a session (the `%sessions-changed` cue).
struct Removal {
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
fn reap_dead_panes(
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
fn render_layout_change(tree: &MuxTree, window_id: WindowId) -> Option<String> {
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
fn held_state_replay_lines(tree: &MuxTree) -> Vec<String> {
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

#[cfg(test)]
mod tests;
