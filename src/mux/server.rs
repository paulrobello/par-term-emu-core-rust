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
use crate::mux::ipc::{
    accept_connection, bind_local_listener, prepare_socket_path, ConnectionAbort, LocalListener,
    LocalStream,
};
use crate::mux::pane::ShellPaneFactory;
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

/// How long a persisting daemon tolerates holding zero sessions and zero
/// clients before exiting — tmux's `exit-empty`, with a grace so the two
/// races it could lose are won instead: a client that disconnects from an
/// emptied daemon and immediately reconnects (or creates a session) resets
/// the clock, and a logout's SIGTERM — which follows pane deaths within
/// moments — beats the grace, so the reboot-race resurrection
/// ([`SaveOrigin::Shutdown`]) stays intact.
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
const CLIENT_QUEUE_DEPTH: usize = 4096;

/// Per-line byte budget for the control-socket read loop (SEC-104). A line
/// is accumulated until its newline, so without a budget a client streaming
/// an unterminated line grows the daemon's memory until the socket closes.
/// Over budget answers one `%error` block and closes the connection — same
/// exposure class as the queue depth above (peer-euid-verified same user),
/// so this bounds accidental growth, not an adversary.
/// cap: Bytes accumulated from one control-socket client line before the daemon closes it.
const MAX_CONTROL_LINE_BYTES: usize = 1024 * 1024;

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
    /// Zero sessions and zero clients, held past [`EXIT_EMPTY_GRACE`] —
    /// tmux's exit-empty.
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
        let _exit = self.run_with_state_path(None);
    }

    /// [`Self::run`] with the whole state atomically saved to `state_path`
    /// after every mutating dispatch — the mode the `par-mux` daemon runs
    /// in. On a requested shutdown (Task 3.5) a final save captures content
    /// that arrived since the last structural one, so a clean SIGTERM never
    /// loses the last window; a listener fault that ends the loop takes the
    /// same save on its way out, so an accept error never silently discards
    /// unsaved work. An exit-when-empty (below) saves with
    /// [`SaveOrigin::ShutdownEmpty`] so the emptiness reads as deliberate.
    /// Callers resolve the path with
    /// [`crate::mux::persist::state_file_path`].
    pub fn run_persisting(self, state_path: PathBuf) {
        let exit = self.run_with_state_path(Some(state_path.clone()));
        let origin = match exit {
            LoopExit::Empty => SaveOrigin::ShutdownEmpty,
            LoopExit::Fault | LoopExit::Requested => SaveOrigin::Shutdown,
        };
        if let Err(err) =
            crate::mux::persist::save_to_with_origin(&self.tree.lock(), &state_path, origin)
        {
            log::error!("par-mux: final state save failed: {err}");
        }
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
        // When the daemon (persisting only) first observed zero sessions AND
        // zero clients — reset to None the moment either returns. Held past
        // EXIT_EMPTY_GRACE it ends the loop as [LoopExit::Empty].
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
                    std::thread::spawn(move || {
                        handle_client(stream, tree, clients, persist, Some(shutdown), abort)
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    if last_scrape.elapsed() >= SCRAPE_INTERVAL {
                        for notification in crate::mux::scrape::scrape_tick(&self.tree, &engine) {
                            broadcast_notification(&self.clients, &notification);
                        }
                        last_scrape = std::time::Instant::now();
                    }
                    if last_reap.elapsed() >= REAP_INTERVAL {
                        reap_dead_panes(&self.tree, &self.clients, persist_tx.as_ref());
                        last_reap = std::time::Instant::now();
                    }
                    // Exit-when-empty, the persisting daemon only: an
                    // embedded `run()` server serves until stopped, whatever
                    // it holds. Both locks are taken and released one at a
                    // time — never nested.
                    if state_path.is_some() {
                        // "Empty" counts a tree whose every pane is dead
                        // too: held panes (remain-on-exit) are for clients
                        // that might come back — with nobody connected and
                        // nothing alive anywhere, the daemon collects
                        // itself instead of lingering on frozen screens.
                        let no_clients = self.clients.lock().is_empty();
                        let idle_empty = no_clients
                            && (self.tree.lock().sessions().is_empty()
                                || self.tree.lock().all_panes_dead());
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
            // Bounded by one 500 ms poll plus an in-flight sweep, itself
            // capped by SWEEP_DEADLINE.
            let _ = probe_worker.join();
        }
        // The socket file dies with the listener (on Windows, the marker
        // file does): remove it so `ls par-mux-*.sock` lists only live
        // daemons — a `--stop` that leaves the file behind reads as a
        // daemon that is still there. A crash skips this; the next bind
        // reclaims the stale remnant (`prepare_socket_path`).
        let _ = std::fs::remove_file(&self.path);
        exit
    }
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
    let mut registered = false;
    // Handed to the registry on first registration — hook connections never
    // register and drop theirs with the frame.
    let mut abort = Some(abort);

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
    'connection: loop {
        // One line, accumulated as raw bytes across recv-timeout wakes: a
        // wake mid-line keeps every byte read so far (`read_line` would
        // discard a partial whose tail splits a multi-byte char), so a
        // healthy sender's pause costs nothing; only an evicted connection
        // breaks out. The budget is checked per chunk, so an unterminated
        // stream trips it without a newline ever arriving (SEC-127), and
        // UTF-8 is decoded once, after the line is complete. An
        // unterminated final line is processed before EOF, matching
        // `Lines`' last-item behavior.
        let mut buf: Vec<u8> = Vec::new();
        let pickup_started = std::time::Instant::now();
        let mut wakes = 0u32;
        loop {
            match fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES) {
                Ok(LineFill::Eof) => {
                    if buf.is_empty() {
                        break 'connection;
                    }
                    break;
                }
                // SEC-104: over-budget accumulation is answered like a
                // malformed command and the connection is closed, whether
                // the line is complete or still unterminated.
                Ok(LineFill::Oversize) => {
                    if !registered {
                        clients.lock().push((
                            client_id,
                            tx.clone(),
                            Arc::clone(&evicted),
                            abort.take().expect("abort is registered once"),
                        ));
                        registered = true;
                    }
                    command_number += 1;
                    let _ = tx.send(emit_block(
                        command_number,
                        "line exceeds 1 MiB budget, closing connection",
                        false,
                    ));
                    break 'connection;
                }
                Ok(LineFill::Complete) => {
                    // Wake-cadence evidence (card 01a0e80db3e870e282af0cf84405043b):
                    // a line that took poll wakes or >1 poll interval to arrive
                    // is the signature of a parked read loop — log it so a
                    // stall's cadence is readable in the debug file.
                    let waited = pickup_started.elapsed();
                    if wakes > 0 || waited > EVICTION_POLL {
                        crate::debug_log!(
                            "MUX",
                            "client {client_id} picked up a {} B line after {:?} and {wakes} poll wakes",
                            buf.len(),
                            waited
                        );
                    }
                    break;
                }
                Err(err) if is_poll_wake(&err) => {
                    wakes += 1;
                    crate::debug_log!(
                        "MUX",
                        "client {client_id} read poll wake #{wakes} ({:?} since line start)",
                        pickup_started.elapsed()
                    );
                    if evicted.load(Ordering::Relaxed) {
                        break 'connection;
                    }
                }
                Err(_) => break 'connection,
            }
        }
        // A non-UTF-8 line (e.g. send-keys -l carrying Latin-1 bytes) was
        // read through its newline, so the stream stays line-framed, but
        // its contents are unusable. Answer it like a parse error — a
        // numbered %error block — instead of dropping the whole client.
        let mut line = match String::from_utf8(buf) {
            Ok(line) => line,
            Err(err) => {
                let undecodable_len = err.as_bytes().len();
                if !registered {
                    clients.lock().push((
                        client_id,
                        tx.clone(),
                        Arc::clone(&evicted),
                        abort.take().expect("abort is registered once"),
                    ));
                    registered = true;
                }
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
                    break 'connection;
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
                    break 'connection;
                }
            }
            Ok(Line::Control(command)) => {
                if !registered {
                    clients.lock().push((
                        client_id,
                        tx.clone(),
                        Arc::clone(&evicted),
                        abort.take().expect("abort is registered once"),
                    ));
                    registered = true;
                }
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
                    break 'connection;
                }
            }
            Err(err) => {
                if !registered {
                    clients.lock().push((
                        client_id,
                        tx.clone(),
                        Arc::clone(&evicted),
                        abort.take().expect("abort is registered once"),
                    ));
                    registered = true;
                }
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
                    break 'connection;
                }
            }
        }
    }
    if registered {
        clients.lock().retain(|(id, _, _, _)| *id != client_id);
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
    clients
        .lock()
        .retain(|(id, tx, evicted, abort)| match tx.try_send(line.clone()) {
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

/// Announce panes whose child exited, and HOLD them (remain-on-exit): the
/// pane, its window, and its frozen screen stay in the tree so
/// `respawn-pane` can restart the process in place — the recovery path a
/// client offers on a crashed program. The PTY reader flips `is_running`
/// on EOF; this pass (the accept loop's idle tick, [`REAP_INTERVAL`])
/// then observes the death ONCE, records the exit code while the child
/// handle can still be asked, and broadcasts `%pane-exited` — the cue a
/// client uses to show "Process exited (code N)" over the frozen screen.
/// No layout changes: the pane keeps its id, window, and geometry.
///
/// Nothing auto-closes anymore. A persisting daemon whose every pane is
/// dead exits once the last client is gone (see [`EXIT_EMPTY_GRACE`] and
/// [`MuxTree::all_panes_dead`]) — held panes are for clients that might
/// come back, and with nobody connected and nothing alive the daemon
/// collects itself.
fn reap_dead_panes(
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Clients,
    persist: Option<&Sender<(SaveOrigin, PersistState)>>,
) {
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
    for &(pane_id, exit_code) in &just_died {
        broadcast_notification(
            clients,
            &TmuxNotification::PaneExited {
                pane_id: pane_id.to_string(),
                exit_code,
            },
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
/// string; the visible-layout copy and raw flags mirror tmux's frame shape.
/// While a pane is zoomed (`resize-pane -Z`) the layout string stays the
/// true (untouched) tree — that is what makes unzoom restore exact — and
/// the zoom shows in the other two fields instead: the visible layout is
/// the zoomed pane alone at full extent, and the raw flags carry `Z`
/// (tmux's zoom flag).
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
        let (visible_layout, raw_flags) = match window.zoomed {
            Some(pane) => (
                // The single-pane form render_node produces for a leaf.
                format!("0000,{}x{},0,0,{}", window.cols, window.rows, pane.0),
                "Z".to_string(),
            ),
            None => (layout.clone(), String::new()),
        };
        emit(&TmuxNotification::LayoutChange {
            window_id: window_id.to_string(),
            window_layout: layout,
            window_visible_layout: visible_layout,
            window_raw_flags: raw_flags,
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

    /// A fresh `TempDir` for a test's socket or state file: the directory
    /// name carries OS-provided randomness, so no other test run can name the
    /// same path (a `process::id()`-derived name repeats once the OS recycles
    /// the pid, and an orphaned listener on it answers as a live server), and
    /// its `Drop` removes everything inside even when the test panics.
    fn temp_dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("par-mux-server-")
            .tempdir()
            .expect("create test temp dir")
    }

    #[test]
    fn reply_is_error_detects_the_error_terminator() {
        assert!(reply_is_error(
            "%begin 1 2 1\ncan't find pane\n%error 1 2 1\n"
        ));
        assert!(!reply_is_error("%begin 1 2 1\n%end 1 2 1\n"));
        // Notifications pushed between commands are not replies.
        assert!(!reply_is_error("%output %1 61"));
    }

    #[test]
    fn summarize_line_keeps_short_lines_and_cut_points_whole() {
        assert_eq!(
            summarize_line("capture-pane -t %1 -S -"),
            "capture-pane -t %1 -S -"
        );
        let long = "capture-pane -t %1 -S - ".to_string() + &"-e ".repeat(200);
        let summary = summarize_line(&long);
        assert!(summary.starts_with("capture-pane -t %1 -S - -e "));
        assert!(summary.contains(&format!("{} bytes total", long.len())));
        // The cut must not split a multi-byte char.
        let multibyte = "é".repeat(200);
        let cut = summarize_line(&multibyte);
        assert!(cut.contains(&format!("{} bytes total", multibyte.len())));
        assert!(cut.ends_with("bytes total)"));
        assert!(cut.is_char_boundary(cut.find("...").expect("ellipsis marker")));
    }

    /// SEC-132: typed input and clipboard content never reach the debug log.
    #[test]
    fn summarize_line_redacts_payload_commands() {
        let literal = summarize_line("send-keys -t %1 -l hunter2");
        assert!(!literal.contains("hunter2"), "payload leaked: {literal}");
        assert!(
            literal.contains("send-keys") && literal.contains("-t %1") && literal.contains("bytes"),
            "name, target and size survive: {literal}"
        );

        let hex = summarize_line("send-keys -t %1 -H 68 75 6e");
        assert!(!hex.contains("68 75"), "hex payload leaked: {hex}");

        let buffer = summarize_line("set-buffer topsecret");
        assert!(!buffer.contains("topsecret"), "buffer leaked: {buffer}");
        assert!(buffer.contains("set-buffer"), "name survives: {buffer}");

        // The target comes only from a leading -t: a -t inside the payload
        // must not smuggle a payload token into the log.
        let smuggled = summarize_line("send-keys -l 'x -t secret' -t %1");
        assert!(!smuggled.contains("secret"), "payload leaked: {smuggled}");

        // Unparseable shapes are redacted too (they are logged at level 1).
        let no_target = summarize_line("  send-keys hunter2");
        assert!(
            !no_target.contains("hunter2"),
            "payload leaked: {no_target}"
        );

        assert_eq!(summarize_line("list-panes"), "list-panes");
    }

    /// A reader that serves a fixed script of chunks and errors, one per
    /// `fill_buf`, so the bounded fill can be driven through poll wakes.
    struct ScriptedReader {
        script: std::collections::VecDeque<std::io::Result<Vec<u8>>>,
        current: Vec<u8>,
        pos: usize,
    }

    impl ScriptedReader {
        fn new(script: Vec<std::io::Result<Vec<u8>>>) -> Self {
            Self {
                script: script.into(),
                current: Vec::new(),
                pos: 0,
            }
        }
    }

    impl std::io::Read for ScriptedReader {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let available = self.fill_buf()?;
            let n = available.len().min(out.len());
            out[..n].copy_from_slice(&available[..n]);
            self.consume(n);
            Ok(n)
        }
    }

    impl BufRead for ScriptedReader {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            if self.pos >= self.current.len() {
                match self.script.pop_front() {
                    Some(Ok(chunk)) => {
                        self.current = chunk;
                        self.pos = 0;
                    }
                    Some(Err(err)) => return Err(err),
                    None => return Ok(&[]),
                }
            }
            Ok(&self.current[self.pos..])
        }

        fn consume(&mut self, amount: usize) {
            self.pos += amount;
        }
    }

    #[test]
    fn fill_line_bounded_trips_the_budget_without_a_newline() {
        let chunks = (0..4).map(|_| Ok(vec![b'x'; 64 * 1024])).collect();
        let mut reader = ScriptedReader::new(chunks);
        let mut buf = Vec::new();
        let fill = fill_line_bounded(&mut reader, &mut buf, 100_000).expect("no I/O error");
        assert_eq!(fill, LineFill::Oversize);
        assert_eq!(buf.len(), 100_001, "nothing past max + 1 is copied");
    }

    #[test]
    fn fill_line_bounded_keeps_a_split_utf8_char_across_a_wake() {
        let mut reader = ScriptedReader::new(vec![
            Ok(b"set-buffer caf\xc3".to_vec()),
            Err(std::io::Error::from(std::io::ErrorKind::TimedOut)),
            Ok(b"\xa9\n".to_vec()),
        ]);
        let mut buf = Vec::new();
        let wake = fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES)
            .expect_err("the wake propagates");
        assert_eq!(wake.kind(), std::io::ErrorKind::TimedOut);
        let fill = fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES)
            .expect("the line completes");
        assert_eq!(fill, LineFill::Complete);
        assert_eq!(
            String::from_utf8(buf).expect("valid UTF-8 once whole"),
            "set-buffer café\n"
        );
    }

    #[test]
    fn fill_line_bounded_reports_eof_with_a_partial() {
        let mut reader = ScriptedReader::new(vec![Ok(b"version".to_vec())]);
        let mut buf = Vec::new();
        let fill =
            fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES).expect("no I/O error");
        assert_eq!(fill, LineFill::Eof);
        assert_eq!(buf, b"version");
    }

    #[test]
    fn fill_line_bounded_stops_at_the_first_newline() {
        let mut reader = ScriptedReader::new(vec![Ok(b"version\nlist-panes\n".to_vec())]);
        let mut buf = Vec::new();
        let fill =
            fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES).expect("no I/O error");
        assert_eq!(fill, LineFill::Complete);
        assert_eq!(buf, b"version\n");
        buf.clear();
        let fill =
            fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES).expect("no I/O error");
        assert_eq!(fill, LineFill::Complete);
        assert_eq!(buf, b"list-panes\n", "the second line stays buffered");
    }

    #[cfg(unix)]
    #[cfg(unix)]
    #[test]
    fn graceful_shutdown_pushes_exit_to_clients_before_closing() {
        use crate::mux::ipc::connect_local_stream;

        let dir = temp_dir();
        let path = dir.path().join("exit.sock");
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
    }

    /// Card 01a0d9b47b26, exit-when-empty: a persisting daemon holding zero
    /// sessions and zero clients exits through the ordinary shutdown path,
    /// and its final save is deliberate — an existing last-good snapshot is
    /// cleared, so the next start is fresh rather than a resurrection.
    #[cfg(unix)]
    #[test]
    fn an_empty_persisting_server_exits_and_clears_the_snapshot() {
        use crate::mux::persist::{load_or_quarantine, Loaded};

        let dir = temp_dir();
        let path = dir.path().join("exit-empty.sock");
        let state_path = dir.path().join("state.json");
        // Content is irrelevant to this test: only the file's removal is
        // asserted (the origin routing is what decides fresh-vs-resurrect;
        // the snapshot-content matrix is persist.rs's suite).
        let lastgood = dir.path().join("state.json.lastgood");
        std::fs::write(&lastgood, b"seed").expect("seed the snapshot");

        let server = MuxServer::bind(&path).expect("bind");
        let (done_tx, done_rx) = channel();
        std::thread::spawn(move || {
            server.run_persisting(state_path);
            let _ = done_tx.send(());
        });

        // 30 s starvation bound, not a timing assertion: unloaded the exit
        // lands within the 300 ms test grace, but the daemon thread and
        // the final save can be starved well past 5 s under full-suite
        // gate load (same class as the generation-test deadlines, 44d4212).
        done_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the empty daemon exits without anyone asking");
        match load_or_quarantine(&dir.path().join("state.json")) {
            Loaded::State(state) => assert!(
                state.sessions.is_empty(),
                "the final save holds the honest empty state"
            ),
            other => panic!("a readable state file loaded as {other:?}"),
        }
        assert!(
            !lastgood.exists(),
            "the empty exit is deliberate — the snapshot is cleared"
        );
    }

    /// The other half of exit-when-empty: a connected client is a reason to
    /// stay. An empty daemon with a registered client outlives the grace,
    /// answers commands, and only exits once the client disconnects.
    #[cfg(unix)]
    #[test]
    fn a_connected_client_keeps_an_empty_persisting_server_alive() {
        use crate::mux::ipc::connect_local_stream;
        use std::io::{BufRead, BufReader, Write};

        let dir = temp_dir();
        let path = dir.path().join("kept-alive.sock");
        let state_path = dir.path().join("state.json");
        let server = MuxServer::bind(&path).expect("bind");
        let (done_tx, done_rx) = channel();
        std::thread::spawn(move || {
            server.run_persisting(state_path);
            let _ = done_tx.send(());
        });

        let stream = connect_local_stream(&path).expect("connect");
        let mut writer = stream.try_clone().expect("clone");
        let round_trip = |writer: &mut LocalStream, command: &str| -> String {
            let mut reader = {
                let clone = writer.try_clone().expect("clone for reading");
                BufReader::new(clone)
            };
            writeln!(writer, "{command}").expect("write");
            writer.flush().expect("flush");
            let mut block = String::new();
            // One full reply block: %begin … %end, in bounded time.
            for _ in 0..16 {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                block.push_str(&line);
                if line.starts_with("%end") || line.starts_with("%error") {
                    break;
                }
            }
            block
        };

        // Registration proof: a completed round trip means the client's
        // sender is in the registry, so the empty-check sees it.
        let reply = round_trip(&mut writer, "list-sessions");
        assert!(reply.contains("%end"), "first reply arrives: {reply}");

        // Outlive the grace (test grace is 300 ms; 900 ms triples it).
        std::thread::sleep(std::time::Duration::from_millis(900));
        let reply = round_trip(&mut writer, "list-sessions");
        assert!(
            reply.contains("%end"),
            "a connected client keeps the empty daemon serving: {reply}"
        );

        // Disconnect: the daemon notices (≤ one idle tick), holds the empty
        // state through the grace, then exits.
        drop(writer);
        drop(stream);
        // 30 s starvation bound, not a timing assertion: after the drop the
        // exit needs one idle tick plus the 300 ms test grace, and that
        // path can be starved past 5 s under full-suite gate load
        // (observed 2026-09-27, 1/2243 under make test-rust; same fix
        // shape as the generation-test deadlines, 44d4212).
        done_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the daemon exits after its last client leaves");
    }

    /// Card 01a0d9b2fd2c: an inherited 256-descriptor soft limit must not
    /// cap the daemon at ~60 panes. The soft limit is lowered to 256 the
    /// way an inherited daemon limit looks, binding a server raises it
    /// toward the hard limit, and 70 sessions open. At roughly four
    /// descriptors per pane, 70 panes need more than 256 descriptors, so
    /// the raise is what lets them all live; without it the spawns die with
    /// EMFILE around the 55th. The guard restores the inherited limit while
    /// the test unwinds, and the panes' own Drop kills their children.
    #[cfg(unix)]
    #[test]
    fn more_than_60_panes_open_under_a_256_descriptor_soft_limit() {
        struct NofileGuard(libc::rlim_t, libc::rlim_t);
        impl Drop for NofileGuard {
            fn drop(&mut self) {
                let limit = libc::rlimit {
                    rlim_cur: self.0,
                    rlim_max: self.1,
                };
                // SAFETY: the values came from getrlimit at test start.
                unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
            }
        }
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: valid rlimit out-pointer.
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
            0,
            "read the inherited limit"
        );
        let _guard = NofileGuard(limit.rlim_cur, limit.rlim_max);
        let inherited_hard = limit.rlim_max;
        limit.rlim_cur = 256;
        // SAFETY: 256 is below the inherited hard limit, and only rlim_cur
        // changes.
        assert_eq!(
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) },
            0,
            "lower the soft limit the way an inherited daemon limit looks"
        );

        let dir = temp_dir();
        let path = dir.path().join("nofile.sock");
        let server = MuxServer::bind(&path).expect("bind raises the soft limit");

        // The raise must have actually happened: 256 is below any sane
        // hard limit, so the soft limit after bind is strictly higher.
        let mut raised = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: valid rlimit out-pointer.
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut raised) },
            0,
            "read the limit after bind"
        );
        assert!(
            raised.rlim_cur > 256 || inherited_hard <= 256,
            "bind raised the soft limit above 256 (now {}, hard {inherited_hard})",
            raised.rlim_cur
        );

        let tree = Arc::clone(&server.tree);
        for pane in 1..=70u32 {
            let session = tree
                .lock()
                .new_session(&format!("nofile-{pane}"), 80, 24)
                .unwrap_or_else(|err| panic!("pane {pane} of 70 opened: {err}"));
            let _ = session;
        }
        assert_eq!(
            tree.lock().sessions.len(),
            70,
            "all 70 panes live under a 256-inherited soft limit"
        );
    }

    /// A listener fault must not silently discard unsaved work (card
    /// 01a0d9b47393: EMFILE killed the daemon with no final save). The fault
    /// is injected by dup2'ing /dev/null over the listener's fd — accept
    /// then fails with a non-transient error (ENOTSOCK), and the listener's
    /// own Drop still closes a valid fd, so the test cannot double-close a
    /// recycled descriptor.
    #[cfg(unix)]
    #[test]
    fn a_listener_fault_still_performs_the_final_save() {
        use crate::mux::persist::{load_or_quarantine, state_file_in, Loaded};
        use std::os::fd::{AsFd as _, AsRawFd as _};

        let dir = temp_dir();
        let path = dir.path().join("fault.sock");
        let state_path = state_file_in(&dir.path().join("state"), &path);
        let server = MuxServer::bind(&path).expect("bind");

        // The fd is captured before the move into the serving thread; the
        // mutation rides the wire first so the accept loop is provably live
        // before anything breaks. The enum wraps the ud-socket listener,
        // whose AsFd is the one fd access interprocess exposes.
        let listener_fd = match &server.listener {
            interprocess::local_socket::Listener::UdSocket(inner) => inner.as_fd().as_raw_fd(),
        };
        let fault_save_path = state_path.clone();
        let serving = std::thread::spawn(move || server.run_persisting(fault_save_path));
        let mut client = crate::mux::MuxClient::connect(&path).expect("client connects");
        client
            .send_checked("new-session -s kept")
            .expect("mutation lands");
        // Disconnect before the fault so this test exercises the plain
        // save-on-fault path; the silent-client variant
        // (`a_listener_fault_exits_even_with_a_silent_connected_client`)
        // covers the join staying bounded with a connected client.
        drop(client);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
        assert!(
            null >= 0,
            "open /dev/null: {}",
            std::io::Error::last_os_error()
        );
        assert_eq!(
            unsafe { libc::dup2(null, listener_fd) },
            listener_fd,
            "replace the listener fd: {}",
            std::io::Error::last_os_error()
        );
        unsafe { libc::close(null) };

        // The accept loop sees a non-socket and breaks; the fault exit must
        // still save. The join returns because the fault ends the loop.
        serving.join().expect("the fault-exit path returns");
        match load_or_quarantine(&state_path) {
            Loaded::State(state) => assert_eq!(
                state.sessions.len(),
                1,
                "the fault-exit save captured the session"
            ),
            Loaded::Fresh | Loaded::Quarantined { .. } => panic!(
                "no state was saved on the listener fault at {}",
                state_path.display()
            ),
        }
    }

    /// A listener fault must not wait on connected clients (card
    /// 01a0da711dfb7cf397fa99ceaba76ae1): every handler thread holds a
    /// persist clone, and a silent one — connected, sends nothing, socket
    /// open — never drops it, so the fault-exit join could only complete by
    /// the worker observing the shutdown flag, not by the channel closing.
    /// The join is bounded: a completion channel fires within the deadline
    /// or the test fails.
    #[cfg(unix)]
    #[test]
    fn a_listener_fault_exits_even_with_a_silent_connected_client() {
        use crate::mux::persist::{load_or_quarantine, state_file_in, Loaded};
        use std::os::fd::{AsFd as _, AsRawFd as _};

        let dir = temp_dir();
        let path = dir.path().join("fault-silent.sock");
        let state_path = state_file_in(&dir.path().join("state"), &path);
        let server = MuxServer::bind(&path).expect("bind");

        let listener_fd = match &server.listener {
            interprocess::local_socket::Listener::UdSocket(inner) => inner.as_fd().as_raw_fd(),
        };
        let fault_save_path = state_path.clone();
        let serving = std::thread::spawn(move || server.run_persisting(fault_save_path));
        let mut client = crate::mux::MuxClient::connect(&path).expect("client connects");
        client
            .send_checked("new-session -s kept")
            .expect("mutation lands");
        // The silent client: connected, registered no command, sends
        // nothing, and stays alive past the fault. Its handler thread must
        // not hold the fault-exit join open.
        let silent = crate::mux::MuxClient::connect(&path).expect("silent client connects");
        std::thread::sleep(std::time::Duration::from_millis(100));

        let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
        assert!(
            null >= 0,
            "open /dev/null: {}",
            std::io::Error::last_os_error()
        );
        assert_eq!(
            unsafe { libc::dup2(null, listener_fd) },
            listener_fd,
            "replace the listener fd: {}",
            std::io::Error::last_os_error()
        );
        unsafe { libc::close(null) };

        let (exited_tx, exited_rx) = std::sync::mpsc::channel();
        let mut serving = Some(serving);
        std::thread::spawn(move || {
            if let Some(handle) = serving.take() {
                let _ = handle.join();
            }
            let _ = exited_tx.send(());
        });
        let deadline = std::time::Duration::from_secs(10);
        assert!(
            exited_rx.recv_timeout(deadline).is_ok(),
            "the fault exit is held open past {deadline:?} — a silent client's \
             persist clone is pinning the persist worker's channel open"
        );
        match load_or_quarantine(&state_path) {
            Loaded::State(state) => assert_eq!(
                state.sessions.len(),
                1,
                "the fault-exit save captured the session"
            ),
            Loaded::Fresh | Loaded::Quarantined { .. } => panic!(
                "no state was saved on the listener fault at {}",
                state_path.display()
            ),
        }
        drop(silent);
    }

    /// The requested-shutdown side of card 01a0da711dfb7cf397fa99ceaba76ae1:
    /// SIGTERM and `kill-server` raise the same flag this test raises, and
    /// the flag — not the persist channel's sender count — must bound the
    /// exit join when a silent client's handler holds a sender clone.
    #[cfg(unix)]
    #[test]
    fn a_requested_shutdown_exits_even_with_a_silent_connected_client() {
        use crate::mux::persist::{load_or_quarantine, state_file_in, Loaded};

        let dir = temp_dir();
        let path = dir.path().join("shutdown-silent.sock");
        let state_path = state_file_in(&dir.path().join("state"), &path);
        let server = MuxServer::bind(&path).expect("bind");
        let shutdown = server.shutdown_handle();
        let save_path = state_path.clone();
        let serving = std::thread::spawn(move || server.run_persisting(save_path));

        let mut client = crate::mux::MuxClient::connect(&path).expect("client connects");
        client
            .send_checked("new-session -s kept")
            .expect("mutation lands");
        // Silent: connected, never sends, outlives the shutdown.
        let silent = crate::mux::MuxClient::connect(&path).expect("silent client connects");
        std::thread::sleep(std::time::Duration::from_millis(100));
        shutdown.store(true, Ordering::Relaxed);

        let (exited_tx, exited_rx) = channel();
        let mut serving = Some(serving);
        std::thread::spawn(move || {
            if let Some(handle) = serving.take() {
                let _ = handle.join();
            }
            let _ = exited_tx.send(());
        });
        let deadline = std::time::Duration::from_secs(10);
        assert!(
            exited_rx.recv_timeout(deadline).is_ok(),
            "the requested-shutdown exit is held open past {deadline:?} — a silent \
             client's persist clone is pinning the persist worker's channel open"
        );
        match load_or_quarantine(&state_path) {
            Loaded::State(state) => assert_eq!(
                state.sessions.len(),
                1,
                "the shutdown save captured the session"
            ),
            Loaded::Fresh | Loaded::Quarantined { .. } => panic!(
                "no state was saved on the requested shutdown at {}",
                state_path.display()
            ),
        }
        drop(silent);
    }

    /// A shutdown removes the socket file: `--stop` reporting success
    /// while the file stays behind makes `ls par-mux-*.sock` useless for
    /// spotting live daemons. SIGTERM and `kill-server` raise the same
    /// flag this test raises.
    #[test]
    fn a_shutdown_removes_the_socket_file() {
        let dir = temp_dir();
        let path = dir.path().join("stop-unlink.sock");
        let server = MuxServer::bind(&path).expect("bind");
        assert!(path.exists(), "the socket file exists while serving");
        let shutdown = server.shutdown_handle();
        let serving = std::thread::spawn(move || server.run());
        shutdown.store(true, Ordering::Relaxed);
        serving.join().expect("server exits cleanly");
        assert!(!path.exists(), "the socket file is removed after shutdown");
    }

    #[cfg(unix)]
    #[test]
    fn bind_replaces_a_stale_socket_file_and_sets_mode_0600() {
        let dir = temp_dir();
        let path = dir.path().join("stale.sock");
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
            context: &crate::mux::pane::SpawnContext<'_>,
        ) -> Result<crate::mux::pane::MuxPane, crate::mux::pane::MuxError> {
            ShellPaneFactory::default().create_pane(id, cols, rows, Some("sleep 30"), context)
        }
    }

    fn quiet_harness() -> (Arc<Mutex<MuxTree>>, Clients) {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(SilentPaneFactory))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        (tree, clients)
    }

    #[test]
    fn pane_info_reports_the_window_and_the_pane_grid_size() {
        let (tree, clients) = quiet_harness();
        dispatch("new-session -s info", 1, &tree, &clients, None);
        dispatch("refresh-client -t %0 -C 100x30", 2, &tree, &clients, None);
        let body = |reply: &str| -> Vec<String> {
            reply
                .lines()
                .filter(|l| !l.starts_with("%begin") && !l.starts_with("%end"))
                .map(str::to_string)
                .collect()
        };
        let reply = dispatch("pane-info -t %0", 3, &tree, &clients, None);
        // The harness pane runs a real shell, so the reply may carry the
        // optional `cmd=` token; the fixed prefix must stay parseable by
        // older clients either way.
        let line = body(&reply).join("");
        assert!(
            line == "%0 @0 100x30" || line.starts_with("%0 @0 100x30 cmd="),
            "pane-info keeps its fixed prefix: {reply}"
        );

        let split = dispatch("split-window -h -t %0", 4, &tree, &clients, None);
        assert!(!split.contains("%error"), "{split}");
        let reply = dispatch("pane-info -t %0", 5, &tree, &clients, None);
        let line = body(&reply).join("");
        assert!(
            line.starts_with("%0 @0 ") && !line.ends_with(" 100x30"),
            "a split re-fits the pane: {reply}"
        );

        let reply = dispatch("pane-info -t %99", 6, &tree, &clients, None);
        assert!(reply.contains("%error"), "{reply}");
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

    /// Card 01a0d9e6f012, criterion 1: a client's cell pixel report rides
    /// `refresh-client -p` through the control protocol, lands daemon-wide,
    /// and reaches the pane terminal's pixel state — what `CSI 14 t`/`16 t`
    /// in the pane answer from. A `-C` grid resize afterwards keeps the
    /// reported cells (every re-fit re-derives the totals).
    #[test]
    fn refresh_client_pixel_report_reaches_the_pane_terminal() {
        let (tree, clients) = quiet_harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let pane = tree.lock().window(window_id).unwrap().panes()[0];

        dispatch(
            &format!("refresh-client -t {pane} -p 10x20"),
            2,
            &tree,
            &clients,
            None,
        );
        {
            let term = tree.lock().pane(pane).unwrap().terminal();
            let term = term.read();
            assert_eq!(
                (term.pixel_width, term.pixel_height),
                (800, 480),
                "an 80x24 pane at 10x20 px cells"
            );
            assert_eq!(
                term.graphics.cell_dimensions,
                (10, 20),
                "image cell-span math uses the reported cell size"
            );
        }

        // A later grid resize re-derives the pixel totals from the kept
        // cell report — the attach-and-resize flow par-term runs.
        dispatch(
            &format!("refresh-client -t {pane} -C 120x40 -p 10x20"),
            3,
            &tree,
            &clients,
            None,
        );
        let term = tree.lock().pane(pane).unwrap().terminal();
        let term = term.read();
        assert_eq!(
            (term.pixel_width, term.pixel_height),
            (1200, 800),
            "a 120x40 grid at the kept 10x20 px cells"
        );
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
        clients.lock().push((
            u64::MAX,
            tx,
            Arc::new(AtomicBool::new(false)),
            ConnectionAbort::none(),
        ));

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
        // The session survives (its first window remains), so the set of
        // sessions did NOT change — the kill above must not have sent
        // %sessions-changed either.
        assert!(
            !lines.iter().any(|l| l.starts_with("%sessions-changed")),
            "a surviving session is not a session-set change: {lines:?}"
        );
    }

    /// Card 01a0d9b47b26: a client must learn its session is gone through a
    /// defined line, not infer it from an empty tab set. tmux's cue is the
    /// argument-less `%sessions-changed`, on both the create and destroy
    /// sides of the session set.
    #[test]
    fn session_set_changes_broadcast_sessions_changed() {
        let (tree, clients) = quiet_harness();

        // An observer client: everything it sees is a broadcast.
        let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
        clients.lock().push((
            u64::MAX,
            tx,
            Arc::new(AtomicBool::new(false)),
            ConnectionAbort::none(),
        ));

        // Create side: new-session lands in the observer's channel.
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let lines = drain_broadcasts(&rx);
        assert!(
            lines.iter().any(|l| l.starts_with("%sessions-changed")),
            "new-session must broadcast %sessions-changed: {lines:?}"
        );

        // Destroy side: kill-window of the session's last window cascades to
        // the session, and the client learns it after the window-close line.
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        dispatch(
            &format!("kill-window -t {window_id}"),
            2,
            &tree,
            &clients,
            None,
        );
        let lines = drain_broadcasts(&rx);
        let close = lines
            .iter()
            .position(|l| l.starts_with("%window-close"))
            .expect("%window-close precedes the session cue");
        let changed = lines
            .iter()
            .position(|l| l.starts_with("%sessions-changed"))
            .expect("the emptied session must broadcast %sessions-changed");
        assert!(
            close < changed,
            "%window-close names the window first, %sessions-changed follows: {lines:?}"
        );
    }

    /// The kill-pane cascade sends the same cue: a session emptied through
    /// its last pane is still a session-set change.
    #[test]
    fn a_kill_pane_that_empties_the_session_broadcasts_sessions_changed() {
        let (tree, clients) = quiet_harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

        let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
        clients.lock().push((
            u64::MAX,
            tx,
            Arc::new(AtomicBool::new(false)),
            ConnectionAbort::none(),
        ));
        dispatch(&format!("kill-pane -t {pane_id}"), 2, &tree, &clients, None);
        let lines = drain_broadcasts(&rx);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("%window-close") && l.contains(&window_id.to_string())),
            "the emptied window closes first: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.starts_with("%sessions-changed")),
            "kill-pane's cascade to the session must broadcast %sessions-changed: {lines:?}"
        );
        assert!(tree.lock().session(session_id).is_none());
    }

    /// rename-session and kill-session (card 01a0ea74ec2e): the rename
    /// broadcasts %session-renamed with the new name; the kill closes every
    /// window in the session before the %sessions-changed cue.
    #[test]
    fn rename_and_kill_session_broadcast_their_notifications() {
        let (tree, clients) = quiet_harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        dispatch(
            &format!("new-window -t {session_id} -n logs"),
            2,
            &tree,
            &clients,
            None,
        );
        let windows = tree.lock().session(session_id).unwrap().windows.clone();

        // An observer client registered before the mutations: everything it
        // sees is a broadcast.
        let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
        clients.lock().push((
            u64::MAX,
            tx,
            Arc::new(AtomicBool::new(false)),
            ConnectionAbort::none(),
        ));

        dispatch(
            &format!("rename-session -t {session_id} 'Renamed Main'"),
            3,
            &tree,
            &clients,
            None,
        );
        let lines = drain_broadcasts(&rx);
        assert!(
            lines.iter().any(|l| l.starts_with("%session-renamed")
                && l.contains(&session_id.to_string())
                && l.contains("Renamed Main")),
            "rename-session must broadcast %session-renamed with the new name: {lines:?}"
        );
        assert_eq!(
            tree.lock().session(session_id).unwrap().name,
            "Renamed Main",
            "the tree keeps the new name for future spawns"
        );

        dispatch(
            &format!("kill-session -t {session_id}"),
            4,
            &tree,
            &clients,
            None,
        );
        let lines = drain_broadcasts(&rx);
        for window in &windows {
            assert!(
                lines
                    .iter()
                    .any(|l| l.starts_with("%window-close") && l.contains(&window.to_string())),
                "kill-session must broadcast %window-close for {window}: {lines:?}"
            );
        }
        let close = lines
            .iter()
            .position(|l| l.starts_with("%window-close"))
            .expect("the killed windows close first");
        let changed = lines
            .iter()
            .position(|l| l.starts_with("%sessions-changed"))
            .expect("kill-session must broadcast %sessions-changed");
        assert!(
            close < changed,
            "%window-close lines precede %sessions-changed: {lines:?}"
        );
        assert!(
            tree.lock().session(session_id).is_none(),
            "the session is gone from the tree"
        );
        assert!(
            tree.lock().sessions().is_empty(),
            "kill-session leaves no orphaned windows or sessions behind"
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
        clients.lock().push((
            u64::MAX,
            tx,
            Arc::new(AtomicBool::new(false)),
            ConnectionAbort::none(),
        ));

        let reply = dispatch(
            &format!("split-window -t {pane_id} -h -p 25"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "split-window succeeds: {reply}");

        // The pane behind new-session is an interactive shell (harness uses
        // ShellPaneFactory, not SilentPaneFactory), so its banner can
        // broadcast a %output before the %layout-change lands — poll for the
        // layout change instead of asserting the FIRST notification is it
        // (single-shot recv raced cmd.exe's banner on Windows, run 36051903299).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut notification = String::from("<no notification>");
        loop {
            match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
                Ok(msg) if msg.contains("%layout-change") => {
                    notification = msg;
                    break;
                }
                Ok(msg) => notification = msg,
                Err(_) => break,
            }
        }
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
        clients.lock().push((
            u64::MAX,
            tx,
            Arc::new(AtomicBool::new(false)),
            ConnectionAbort::none(),
        ));

        let reply = dispatch(
            &format!("resize-pane -t {first} -x 25"),
            3,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "absolute resize succeeds: {reply}");

        // A resized pane's shell gets SIGWINCH and (bash under ConPTY)
        // emits a DECXCPR query whose %output can beat the %layout-change
        // to this observer — poll for the layout change instead of
        // asserting the FIRST notification is it (QA-143; same shape the
        // split-window test at the banner race hit, run 36051903299).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut notification = String::from("<no notification>");
        loop {
            match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
                Ok(msg) if msg.contains("%layout-change") => {
                    notification = msg;
                    break;
                }
                Ok(msg) => notification = msg,
                Err(_) => break,
            }
        }
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
        clients.lock().push((
            u64::MAX,
            tx,
            Arc::new(AtomicBool::new(false)),
            ConnectionAbort::none(),
        ));

        let reply = dispatch(
            &format!("refresh-client -t {pane_id} -C 120x40"),
            2,
            &tree,
            &clients,
            None,
        );
        assert!(reply.contains("%end"), "-C report succeeds: {reply}");

        // Same drain as the resize test: the re-fitted pane's shell can
        // race a %output past the %layout-change after its SIGWINCH.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut notification = String::from("<no notification>");
        loop {
            match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
                Ok(msg) if msg.contains("%layout-change") => {
                    notification = msg;
                    break;
                }
                Ok(msg) => notification = msg,
                Err(_) => break,
            }
        }
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
    fn capture_pane_e_returns_sgr_rows_and_plain_stays_plain() {
        let (tree, clients) = quiet_harness();
        dispatch("new-session -s main", 1, &tree, &clients, None);
        let session_id = tree.lock().sessions()[0];
        let window_id = tree.lock().session(session_id).unwrap().windows[0];
        let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

        // Styled content at a known position: a bold, blue-background tag
        // on row 2 and plain text on row 4.
        let payload = b"\x1b[2;3H\x1b[1;44mTAG\x1b[0m\x1b[4;1Hplain";
        tree.lock()
            .pane_mut(pane_id)
            .unwrap()
            .terminal()
            .write()
            .process(payload);

        let plain = dispatch(
            &format!("capture-pane -t {pane_id} -p"),
            2,
            &tree,
            &clients,
            None,
        );
        let escaped = dispatch(
            &format!("capture-pane -t {pane_id} -p -e"),
            3,
            &tree,
            &clients,
            None,
        );
        assert!(plain.contains("%end"), "plain capture succeeds: {plain}");
        assert!(escaped.contains("%end"), "-e capture succeeds: {escaped}");

        // Without -e the reply stays the pre--e capture: the pane's
        // logical lines, no ESC byte.
        let expected = tree
            .lock()
            .pane(pane_id)
            .unwrap()
            .terminal()
            .read()
            .content();
        assert!(
            plain.contains("TAG") && plain.contains("plain"),
            "plain capture carries the text: {plain}"
        );
        assert!(
            plain.contains(&expected),
            "plain capture is content(): {plain:?} vs {expected:?}"
        );
        assert!(
            !plain.contains('\x1b'),
            "plain capture carries no ESC byte: {plain:?}"
        );

        // With -e the styled row carries its SGR run inline (reset, fg,
        // bg — push_sgr_style's fixed order) and a reset before the
        // line break; the unstyled row stays plain text.
        assert!(
            escaped.contains("\x1b[0;37;44") && escaped.contains("TAG\x1b[0m\n"),
            "-e capture carries the styled run inline, reset before the \
             line break: {escaped:?}"
        );
        assert!(
            escaped.contains("plain\n"),
            "-e capture keeps the unstyled row plain: {escaped:?}"
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
    fn set_buffer_quoted_payload_round_trips() {
        let (tree, clients) = harness();
        let set = dispatch(
            "set-buffer 'it'\\''s \"doubly\" quoted'",
            1,
            &tree,
            &clients,
            None,
        );
        assert!(set.contains("%end"), "quoted set-buffer succeeds: {set}");

        let show = dispatch("show-buffer", 2, &tree, &clients, None);
        assert!(
            show.contains("it's \"doubly\" quoted"),
            "quotes stripped, not stored: {show}"
        );
    }

    #[test]
    fn set_buffer_hex_payload_is_byte_exact() {
        let (tree, clients) = harness();
        // "a\nb\n" — the trailing newline is exactly what the reply-block
        // format cannot express, so assert on the STORED content through
        // the tree rather than show-buffer.
        let set = dispatch("set-buffer -H 61 0a 62 0a", 1, &tree, &clients, None);
        assert!(set.contains("%end"), "hex set-buffer succeeds: {set}");

        let stored = tree.lock().get_buffer("default").map(str::to_string);
        assert_eq!(stored.as_deref(), Some("a\nb\n"), "stored byte-exact");
    }

    #[test]
    fn set_buffer_hex_payload_rejects_garbage() {
        let (tree, clients) = harness();
        let bad = dispatch("set-buffer -H zz", 1, &tree, &clients, None);
        assert!(bad.contains("%error"), "invalid hex is an error: {bad}");
        let none = dispatch("set-buffer -H", 1, &tree, &clients, None);
        assert!(
            none.contains("%error"),
            "missing payload is an error: {none}"
        );
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
        let dir = temp_dir();
        let target = dir.path().join("state.json");

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
    }

    /// A burst of queued states must coalesce: fewer writes than states,
    /// and the last state written is the newest one sent.
    #[test]
    fn persist_worker_coalesces_a_burst_to_the_newest_state() {
        let (tx, rx) = channel::<(SaveOrigin, PersistState)>();
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
                |_origin: SaveOrigin, state: &PersistState, _path| {
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
            tx.send((SaveOrigin::Command, state))
                .expect("worker owns the receiver");
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
        let stalled_flag = Arc::new(AtomicBool::new(false));
        clients.lock().push((
            1,
            stalled_tx,
            Arc::clone(&stalled_flag),
            ConnectionAbort::none(),
        ));
        // The sibling: drains as lines arrive, like a healthy reader thread.
        let (sibling_tx, sibling_rx) = sync_channel::<String>(CLIENT_QUEUE_DEPTH);
        clients.lock().push((
            2,
            sibling_tx,
            Arc::new(AtomicBool::new(false)),
            ConnectionAbort::none(),
        ));

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
        assert!(
            stalled_flag.load(Ordering::Relaxed),
            "eviction raised the flag the connection threads tear down on"
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

    /// Card 01a0d9b47dae7751a6c7e5a4a900be6e: eviction is by queue depth
    /// only — intended (ARC-011's memory bound; see MUX.md's broadcast
    /// eviction paragraph), and deliberately unlike tmux, which evicts a
    /// control client by output age (300 s) while buffering without bound.
    /// This pins the consequence of the depth policy: a client draining
    /// continuously but slower than the producer — healthy, just slow — is
    /// evicted once its backlog passes the cap, losing the queued tail. An
    /// age-based policy replaces this test with a drains-slowly-survives
    /// one.
    #[test]
    fn a_slow_draining_client_is_evicted_once_its_backlog_passes_the_cap() {
        let clients: Clients = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = sync_channel::<String>(CLIENT_QUEUE_DEPTH);
        let flag = Arc::new(AtomicBool::new(false));
        clients
            .lock()
            .push((7, tx, Arc::clone(&flag), ConnectionAbort::none()));

        // Drains exactly one line per two pushes — continuously, but at
        // half the producer's rate, so the backlog grows ~0.5 lines per
        // push and passes the cap mid-burst.
        let burst = CLIENT_QUEUE_DEPTH * 3;
        let mut drained = 0;
        for n in 0..burst {
            push_to_clients(&clients, format!("line-{n}"));
            if n % 2 == 0 && rx.try_recv().is_ok() {
                drained += 1;
            }
        }
        // After eviction the sender is gone; the queue's remaining backlog
        // still drains, so `drained` settles just past the eviction point.
        while rx.try_recv().is_ok() {
            drained += 1;
        }

        assert!(
            flag.load(Ordering::Relaxed),
            "the slow drainer was evicted once its backlog passed the cap"
        );
        assert!(
            clients.lock().is_empty(),
            "the evicted client is no longer registered"
        );
        assert!(
            drained > CLIENT_QUEUE_DEPTH && drained < burst,
            "drained {drained} of {burst} — continuous but half-rate, evicted mid-burst"
        );
    }

    /// ENH-012: eviction must CLOSE the evicted client's connection, not
    /// just stop queueing to it. Dropping the queue sender alone leaves the
    /// client's writer thread blocked in a full socket buffer and its
    /// `handle_client` thread parked in `lines()`, so the queued lines stay
    /// pinned and the client never learns it was evicted (measured live:
    /// daemon RSS held ~70 MiB and the stalled socket never EOF'd while the
    /// flood continued; the moment it started reading, the writer resumed
    /// feeding it the retained queue).
    #[test]
    fn an_evicted_clients_connection_closes() {
        use crate::mux::ipc::connect_local_stream;
        use std::io::{Read, Write};

        // A raw listener, not a MuxServer, so the shutdown-unlink never
        // runs for it — keep the socket inside a temp dir the harness
        // removes rather than littering the real $TMPDIR.
        let dir = temp_dir();
        let socket_path = dir.path().join("evict-close.sock");
        let listener = bind_local_listener(&socket_path).expect("binds the test listener");

        let clients: Clients = Arc::new(Mutex::new(Vec::new()));
        let registry = Arc::clone(&clients);
        let (tree, _) = harness();
        std::thread::spawn(
            move || match crate::mux::ipc::accept_connection(&listener) {
                Ok((stream, abort)) => handle_client(stream, tree, registry, None, None, abort),
                Err(err) => panic!("accept_connection failed: {err}"),
            },
        );

        let mut client = connect_local_stream(&socket_path).expect("connects");
        client
            .write_all(b"list-sessions\n")
            .expect("sends a command");

        // The first control command is what registers the client.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while clients.lock().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "client never registered"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        // Overflow the queue: the CLIENT_QUEUE_DEPTH-th push fills it, the
        // next one evicts. Lines are ~1 KiB so the socket's own buffer
        // absorbs only a handful on macOS (measured 8 KiB both directions)
        // — but a larger kernel socket buffer (ubuntu CI runners) lets the
        // connection's writer drain past any fixed headroom, so push UNTIL
        // the eviction lands rather than a fixed count, bounded by a
        // deadline.
        let filler = "x".repeat(1000);
        let evicted_by = std::time::Instant::now() + Duration::from_secs(10);
        let mut n = 0usize;
        while !clients.lock().is_empty() {
            assert!(
                std::time::Instant::now() < evicted_by,
                "client never evicted after {n} flood lines"
            );
            push_to_clients(&clients, format!("flood-{n:05}-{filler}"));
            n += 1;
        }

        // The evicted client observes the connection closing: drain the
        // socket's residue, then EOF must arrive within the deadline. On
        // Unix the eviction poll (ENH-012) wakes this connection's
        // writer/reader on send/recv timeouts; on Windows eviction aborts
        // their blocked pipe I/O through the registry entry's
        // ConnectionAbort, and every server-side handle drops — including
        // the abort's and the re-canceller's — which is the EOF itself.
        {
            let (eof_tx, eof_rx) = channel::<()>();
            std::thread::spawn(move || {
                let mut byte = [0u8; 1];
                loop {
                    match client.read(&mut byte) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => continue,
                    }
                }
                let _ = eof_tx.send(());
            });
            assert!(
                eof_rx.recv_timeout(Duration::from_secs(5)).is_ok(),
                "the evicted client's socket closed within 5 s of eviction"
            );
        }
        let _ = std::fs::remove_file(&socket_path);
    }

    /// QA-113: a panicking dispatcher is contained — the issuer gets an
    /// error block, and the same "connection" answers the next command.
    #[test]
    fn a_panicking_command_yields_an_error_block_and_the_next_command_survives() {
        let (tree, clients) = harness();

        crate::mux::dispatch::PANIC_ON_COMMAND.store(true, Ordering::Relaxed);
        let ctx = Ctx {
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
        };
        let poisoned = parse_command("list-sessions").expect("parses");
        let reply = dispatch_contained(poisoned, &ctx, None, None);
        assert!(
            reply.contains("%error") && reply.contains("internal error"),
            "a panicked command answers with an error block: {reply}"
        );
        crate::mux::dispatch::PANIC_ON_COMMAND.store(false, Ordering::Relaxed);

        // The tree lock unwound free; the next command on the same
        // connection dispatches normally.
        let ctx = Ctx {
            tree: &tree,
            clients: &clients,
            command_number: 2,
            shutdown: None,
        };
        let healthy = parse_command("list-sessions").expect("parses");
        let reply = dispatch_contained(healthy, &ctx, None, None);
        assert!(
            reply.contains("%end"),
            "the connection survives the panic: {reply}"
        );
    }

    /// A kill-pane that empties a window must BROADCAST %window-close:
    /// `tree.kill_pane` closes the window (and an emptied session), but
    /// the dispatch only queued a layout push for the window — which
    /// resolves to nothing once the window is gone — so clients kept a
    /// tab for a window that no longer existed.
    #[cfg(unix)]
    #[test]
    fn killing_a_windows_last_pane_broadcasts_window_close() {
        let dir = temp_dir();
        let path = dir.path().join("lastpane.sock");
        let server = MuxServer::bind(&path).expect("bind");
        std::thread::spawn(move || server.run());

        let mut client = crate::mux::MuxClient::connect(&path).expect("connect");
        client.send("new-session -s t").expect("new-session");
        client.send("kill-pane -t %0").expect("kill-pane");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match client
                .notifications()
                .recv_timeout(std::time::Duration::from_millis(100))
            {
                Ok(crate::tmux_control::TmuxNotification::WindowClose { window_id }) => {
                    assert_eq!(window_id, "@0");
                    return;
                }
                Ok(_) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "%window-close never arrived after kill-pane emptied the window"
                    );
                }
                Err(_) => panic!("notification channel died before %window-close"),
            }
        }
    }
}
