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

// Server seams (ARC-003): line protocol plumbing, the per-client thread,
// broadcast/reply sinks, dead-pane reaping, and the hook-only pane endpoints.
// The accept loop (`MuxServer`) stays here.
mod broadcast;
mod client;
mod endpoint;
mod protocol;
mod reap;
use broadcast::*;
pub(crate) use broadcast::{
    broadcast_layout_change, broadcast_notification, capture_range, pane_output_sink,
    replay_pane_exited_lines,
};
use client::*;
pub use endpoint::sweep_pane_endpoint_remnants;
pub(crate) use endpoint::PaneEndpoint;
use endpoint::*;
use protocol::*;
use reap::*;

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
    /// unsaved work. An exit-when-empty (below: no sessions or only dead
    /// panes, whatever clients are attached) saves with [`SaveOrigin::ShutdownEmpty`]:
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

// ---- ENH-039: per-pane hook-only endpoints (the opt-in `--pane-endpoints`
// daemon mode) ----

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

#[cfg(test)]
mod tests;
