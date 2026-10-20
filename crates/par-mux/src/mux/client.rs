//! The control-mode client: connect, auto-spawn, command, and pushed
//! notifications.
//!
//! Command replies and asynchronous notifications use separate paths: `send`
//! consumes `%begin`/`%end` blocks, while [`MuxClient::notifications`] carries
//! everything else, parsed. A reader that conflated the two would deadlock the
//! first time pane output arrived mid-command.

use crate::mux::ipc::{connect_local_stream, default_socket_path, LocalStream};
use crate::mux::server::{fill_line_bounded, is_poll_wake, LineFill};
use interprocess::TryClone as _;
use par_term_emu_core::tmux_control::{TmuxControlParser, TmuxNotification};
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long `send` waits for a reply block before giving up.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-line byte budget for the client reader. A healthy daemon's longest
/// line is a `show-buffer` body under the daemon's own 1 MiB input budget
/// (a `%output` line is at most ~64 KiB), so 4x that never trips; a broken
/// daemon streaming an unterminated line gets the connection closed
/// instead of growing this process without bound.
/// cap: Bytes accumulated from one daemon reply or notification line before the client closes the connection.
const MAX_REPLY_LINE_BYTES: usize = 4 * 1024 * 1024;
/// How long `connect_or_spawn_at` keeps retrying after spawning a daemon.
const SPAWN_CONNECT_DEADLINE: Duration = Duration::from_secs(10);

/// A control-mode client connected (or connectable) to a par-mux daemon.
pub struct MuxClient {
    writer: LocalStream,
    reply_rx: Receiver<Reply>,
    notifications_rx: Receiver<TmuxNotification>,
    /// Set by [`MuxClient::discard_notifications`]: the reader parses
    /// pushed lines but drops them instead of queueing them.
    discard_notifications: Arc<AtomicBool>,
    /// The daemon this client spawned, when it started one. Held so the
    /// spawner can end what it created via [`MuxClient::kill_spawned_daemon`];
    /// `None` when the client attached to an existing server.
    spawned_daemon: Option<std::process::Child>,
    /// Replies owed ahead of the next command's own, in daemon order. The
    /// daemon answers in order and the reply channel is FIFO, so each
    /// arriving block belongs to the front entry: abandoned (timed-out) and
    /// fire-and-forget replies are dropped, async ones are parked in
    /// `completed`. A sync wait first resolves everything ahead of it,
    /// otherwise a late reply would answer the following command.
    owed: VecDeque<Owed>,
    /// Async replies resolved while a sync wait or [`MuxClient::poll_replies`]
    /// was reading, not yet handed to the caller.
    completed: VecDeque<(u64, Reply)>,
    next_token: u64,
}

/// Who a not-yet-arrived reply block belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owed {
    /// A sync command timed out; discard its reply.
    Abandoned,
    /// A fire-and-forget command; discard its reply.
    Forget,
    /// A [`MuxClient::send_nowait`] command; deliver its reply by token.
    Async(u64),
}

impl MuxClient {
    /// Connect to a daemon already listening at `path`.
    pub fn connect(path: &Path) -> io::Result<Self> {
        let stream = connect_local_stream(path)?;
        Self::from_stream(stream)
    }

    /// [`Self::connect`] with a bounded connect: always returns by `deadline`.
    ///
    /// The blocking connect is the unbounded step of a command cycle: a
    /// daemon hung with a full accept backlog blocks every later connect
    /// with no error to retry on. Replies are already bounded by
    /// `REPLY_TIMEOUT` through `send_checked`, so with this the roster
    /// watcher's whole fetch cycle (connect + `list-agents` reply) is
    /// bounded and a hung daemon delays the redial instead of wedging the
    /// thread forever.
    // `pub` (doc-hidden) only for the root crate's streaming roster watcher,
    // its sole caller since par-mux became its own crate (ARC-007 O2).
    #[doc(hidden)]
    pub fn connect_bounded(path: &Path, deadline: Instant) -> io::Result<Self> {
        let stream = super::ipc::connect_local_stream_bounded(path, deadline)?;
        Self::from_stream(stream)
    }

    /// The transparency entry point: connect to the default socket for `name`,
    /// starting a daemon if none is running.
    ///
    /// Before spawning, the pre-0.52 default path is probed (Unix only): a
    /// daemon from before the per-UID socket-directory move still serves it
    /// with every session it owns, and a fresh daemon here would strand it
    /// invisibly. Whatever answers there is attached instead — the client's
    /// version check then surfaces the mismatch — so an upgrade can never
    /// leave two daemons for one name.
    ///
    /// The probe only runs when `XDG_RUNTIME_DIR` is unset: pre-0.52 clients
    /// preferred that directory when it was set, so under XDG the legacy name
    /// was never this user's default and probing it only reaches sockets
    /// someone else planted. The legacy file must also be a socket this user
    /// owns (`legacy_socket_is_trustworthy`) — it sits in the shared temp
    /// dir — and the connect itself verifies the serving daemon's euid.
    pub fn connect_or_spawn(name: &str) -> io::Result<Self> {
        let path = default_socket_path(name);
        #[cfg(unix)]
        let legacy = std::env::var_os("XDG_RUNTIME_DIR")
            .is_none()
            .then(|| crate::mux::ipc::legacy_socket_path(name));
        #[cfg(not(unix))]
        let legacy: Option<PathBuf> = None;
        Self::connect_or_spawn_paths(&path, legacy.as_deref())
    }

    /// [`Self::connect_or_spawn`] with its env-derived paths already resolved:
    /// `legacy` is the pre-0.52 path to probe before spawning (Unix only).
    /// Tests pass paths here instead of mutating the process env (QA-196).
    fn connect_or_spawn_paths(path: &Path, legacy: Option<&Path>) -> io::Result<Self> {
        if let Ok(client) = Self::connect(path) {
            return Ok(client);
        }
        #[cfg(unix)]
        if let Some(legacy) = legacy {
            if crate::mux::ipc::legacy_socket_is_trustworthy(legacy) {
                if let Ok(client) = Self::connect(legacy) {
                    return Ok(client);
                }
            }
        }
        #[cfg(not(unix))]
        let _ = legacy;
        Self::connect_or_spawn_at(path)
    }

    /// Connect to a daemon at `path`, spawning one when no live server owns it.
    ///
    /// Losing the spawn race is not an error: another client's daemon won the
    /// path, and connecting to the winner is the correct outcome.
    ///
    /// A daemon that cannot be STARTED at all — a missing or unrunnable
    /// `par-mux` binary — fails immediately, naming the paths that were tried.
    /// Only a daemon that really was spawned earns `SPAWN_CONNECT_DEADLINE`:
    /// retrying a socket nothing will ever bind buries the real cause under
    /// ten seconds of generic connect errors.
    pub fn connect_or_spawn_at(path: &Path) -> io::Result<Self> {
        if let Ok(client) = Self::connect(path) {
            return Ok(client);
        }
        // Auto-spawning from inside a pane would start a nested daemon: the
        // spawn inherits this env, PAR_MUX_ENV included, so the daemon's own
        // guard would kill it at startup — surfacing here only as the full
        // ten seconds of connect retries. Refuse up front with the reason
        // instead; connecting to an existing daemon, the branch above, is
        // unaffected.
        if let Some(reason) = super::nested_daemon_refusal() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, reason));
        }
        let candidates = daemon_binary_candidates()?;
        let mut last_err = io::Error::new(
            io::ErrorKind::NotFound,
            "cannot locate the par-mux daemon binary",
        );
        for bin in &candidates {
            match Self::spawn_and_connect(bin, path) {
                Ok(client) => return Ok(client),
                // A candidate that could not even be STARTED (missing,
                // unrunnable) moves the search on to the next one; only
                // when every candidate failed is the error surfaced, naming
                // them all. A spawn that started but never bound its socket
                // already consumed its own deadline inside spawn_and_connect
                // — trying further candidates would multiply the wait.
                Err(err) if err.kind() == io::ErrorKind::NotFound => last_err = err,
                Err(err) => {
                    return Err(io::Error::new(
                        err.kind(),
                        format!(
                            "the par-mux daemon binary at {} started but failed to serve {}: {err}",
                            bin.display(),
                            path.display()
                        ),
                    ));
                }
            }
        }
        Err(io::Error::new(
            last_err.kind(),
            format!(
                "no par-mux daemon binary could be started (tried {}): {} — build it, \
                 place it next to the executable, or install it on PATH",
                candidates
                    .iter()
                    .map(|c| c.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                last_err
            ),
        ))
    }

    /// Start `bin` as the daemon for `socket`, then wait for it to bind.
    ///
    /// Split out of [`Self::connect_or_spawn_at`] so tests can drive the two
    /// outcomes whose costs differ by ten seconds — an unspawnable binary and
    /// a spawned daemon slow to bind — against a chosen binary path, rather
    /// than against whatever the ambient target directory happens to hold.
    fn spawn_and_connect(bin: &Path, socket: &Path) -> io::Result<Self> {
        let daemon = spawn_daemon(bin, socket)?;
        let deadline = Instant::now() + SPAWN_CONNECT_DEADLINE;
        loop {
            match Self::connect(socket) {
                Ok(mut client) => {
                    client.spawned_daemon = Some(daemon);
                    return Ok(client);
                }
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(last) => return Err(last),
            }
        }
    }

    fn from_stream(stream: LocalStream) -> io::Result<Self> {
        let writer = stream.try_clone()?;
        let (reply_tx, reply_rx) = channel::<Reply>();
        let (notification_tx, notifications_rx) = channel::<TmuxNotification>();
        let discard_notifications = Arc::new(AtomicBool::new(false));
        let discard = Arc::clone(&discard_notifications);
        std::thread::spawn(move || reader_loop(stream, reply_tx, notification_tx, &discard));
        Ok(Self {
            writer,
            reply_rx,
            notifications_rx,
            discard_notifications,
            spawned_daemon: None,
            owed: VecDeque::new(),
            completed: VecDeque::new(),
            next_token: 0,
        })
    }

    /// Run one command and return its reply block's body lines.
    ///
    /// A block closed by `%error` still returns its body (the error text) as
    /// `Ok`; use [`Self::send_checked`] to tell the two closings apart.
    pub fn send(&mut self, command: &str) -> io::Result<Vec<String>> {
        self.send_checked(command).map(|reply| reply.body)
    }

    /// Run one command and return its reply block, including whether the
    /// daemon closed it with `%end` (success) or `%error` (failure).
    pub fn send_checked(&mut self, command: &str) -> io::Result<Reply> {
        self.send_checked_timeout(command, REPLY_TIMEOUT)
    }

    /// [`Self::send_checked`] bounded by `timeout` instead of the 10 s
    /// default. A timed-out command's reply is still owed: it is counted
    /// and discarded when it arrives, so later commands keep reading
    /// their own replies.
    pub fn send_checked_timeout(&mut self, command: &str, timeout: Duration) -> io::Result<Reply> {
        self.write_line(command)?;
        let deadline = Instant::now() + timeout;
        // Replies already owed to earlier commands arrive before this one's.
        let mut ahead = self.owed.len();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.reply_rx.recv_timeout(remaining) {
                Ok(reply) if ahead > 0 => {
                    ahead -= 1;
                    self.resolve_front(reply);
                }
                Ok(reply) => return Ok(reply),
                Err(RecvTimeoutError::Timeout) => {
                    self.owed.push_back(Owed::Abandoned);
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("no reply block within {}ms", timeout.as_millis()),
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "server closed the connection",
                    ));
                }
            }
        }
    }

    /// Assign an arrived reply to the front owed entry.
    fn resolve_front(&mut self, reply: Reply) {
        if let Some(Owed::Async(token)) = self.owed.pop_front() {
            self.completed.push_back((token, reply));
        }
    }

    fn write_line(&mut self, command: &str) -> io::Result<()> {
        writeln!(self.writer, "{command}")?;
        self.writer.flush()
    }

    /// Write `command` without waiting; its reply is returned by
    /// [`Self::poll_replies`] under the returned token.
    pub fn send_nowait(&mut self, command: &str) -> io::Result<u64> {
        self.write_line(command)?;
        let token = self.next_token;
        self.next_token += 1;
        self.owed.push_back(Owed::Async(token));
        Ok(token)
    }

    /// Write `command` without waiting; its reply is read and discarded.
    pub fn send_forget(&mut self, command: &str) -> io::Result<()> {
        self.write_line(command)?;
        self.owed.push_back(Owed::Forget);
        Ok(())
    }

    /// Resolve every reply that has already arrived and return the async
    /// ones, in order. Never blocks.
    pub fn poll_replies(&mut self) -> Vec<(u64, Reply)> {
        while let Ok(reply) = self.reply_rx.try_recv() {
            self.resolve_front(reply);
        }
        self.completed.drain(..).collect()
    }

    /// Fire-and-forget and async commands whose reply has not arrived yet.
    pub fn outstanding(&self) -> usize {
        self.owed
            .iter()
            .filter(|owed| !matches!(owed, Owed::Abandoned))
            .count()
    }

    /// Pushed notifications (`%output` and friends), already parsed.
    pub fn notifications(&self) -> &Receiver<TmuxNotification> {
        &self.notifications_rx
    }

    /// Stop queueing pushed notifications: from now on the reader drops
    /// them as they arrive. For a caller that only reads replies (the
    /// `par-mux --cmd` client) — the channel is unbounded, so an unread
    /// queue grows with every `%output` line for as long as a command
    /// such as `wait-output` stays parked. The socket is still read in
    /// full, so the daemon never sees this client back up.
    pub fn discard_notifications(&self) {
        self.discard_notifications.store(true, Ordering::Relaxed);
        while self.notifications_rx.try_recv().is_ok() {}
    }

    /// Terminate the daemon this client spawned, if it started one.
    ///
    /// The tmux model is daemon-outlives-client, so this is deliberately
    /// NOT a `Drop` impl — a disconnecting client must not kill the server
    /// other clients are using. It exists for tests and tooling that own
    /// the daemon they created. A client that attached to an existing
    /// server is a no-op.
    pub fn kill_spawned_daemon(&mut self) -> io::Result<()> {
        match self.spawned_daemon.as_mut() {
            Some(child) => {
                let _ = child.kill();
                child.wait().map(|_| ())
            }
            None => Ok(()),
        }
    }

    /// The pid of the daemon this client auto-spawned, if it started one.
    ///
    /// The exec preserves the pid, so this is the serving daemon's pid for
    /// as long as it lives. Tests assert session/process-group facts about
    /// the spawned daemon through it; `kill_spawned_daemon` remains the way
    /// to end it.
    pub fn spawned_daemon_pid(&self) -> Option<u32> {
        self.spawned_daemon.as_ref().map(|child| child.id())
    }
}

/// One command's reply block: its body lines, and whether the daemon closed
/// it with `%end` (`ok`) or `%error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The lines between `%begin` and the closing line.
    pub body: Vec<String>,
    /// `true` for `%end`, `false` for `%error`.
    pub ok: bool,
}

/// Split one connection's byte stream into reply blocks and notifications.
fn reader_loop(
    stream: LocalStream,
    reply_tx: Sender<Reply>,
    notification_tx: Sender<TmuxNotification>,
    discard: &AtomicBool,
) {
    let mut reader = BufReader::new(stream);
    let mut parser = TmuxControlParser::new(true);
    // The open reply block: its command number and body so far.
    let mut open: Option<(u32, Vec<String>)> = None;
    loop {
        let line = match read_daemon_line(&mut reader, MAX_REPLY_LINE_BYTES) {
            DaemonLine::Line(line) => line,
            DaemonLine::Oversize => {
                log::warn!(
                    "par-mux client: daemon line exceeded {MAX_REPLY_LINE_BYTES} bytes; \
                     closing the connection"
                );
                break;
            }
            // EOF, a read fault, or a non-UTF-8 line: the same clean break
            // `lines()` produced, so a pending command reports
            // ConnectionAborted on the dropped reply channel.
            DaemonLine::Closed => break,
        };
        match framing(&line) {
            // A begin opens a block only when none is open — the daemon
            // writes each block atomically, so a begin inside an open
            // block is content that happens to look like framing.
            Some((Framing::Begin, number)) if open.is_none() => {
                open = Some((number, Vec::new()));
            }
            // A close ends the block only when it carries the SAME command
            // number as the begin that opened it (tmux's own rule). Reply
            // bodies are written raw — a pane that printed %end/%begin
            // lines rides them into capture-pane/show-buffer bodies, and a
            // mismatched close must stay content or every later command
            // would answer with the previous one's leftover block.
            Some((kind @ (Framing::End | Framing::Error), number)) if matches!(open, Some((open_number, _)) if open_number == number) =>
            {
                let (_, body) = open.take().expect("the guard matched an open block");
                let reply = Reply {
                    body,
                    ok: kind == Framing::End,
                };
                if reply_tx.send(reply).is_err() {
                    break;
                }
            }
            _ => {
                if let Some((_, body)) = open.as_mut() {
                    body.push(line);
                } else {
                    // lines() strips the terminator; the parser only emits
                    // complete lines, so hand it back the newline it
                    // buffers on.
                    let mut framed = line.clone().into_bytes();
                    framed.push(b'\n');
                    let mut forwarded = true;
                    for notification in parser.parse(&framed) {
                        if discard.load(Ordering::Relaxed) {
                            continue;
                        }
                        if notification_tx.send(notification).is_err() {
                            forwarded = false;
                        }
                    }
                    if !forwarded {
                        break;
                    }
                }
            }
        }
    }
}

/// One line from the daemon, as `reader_loop` consumes it.
#[derive(Debug, PartialEq, Eq)]
enum DaemonLine {
    /// A line with its `\n` (and a preceding `\r`) stripped, like `Lines`;
    /// also an unterminated final line read before EOF.
    Line(String),
    /// EOF with nothing pending, a read fault, or a non-UTF-8 line: the
    /// same clean break `lines()` produced, so a pending command reports
    /// ConnectionAborted on the dropped reply channel.
    Closed,
    /// The line passed `max` bytes.
    Oversize,
}

/// One bounded daemon line. `fill_line_bounded` keeps the bytes raw across
/// read wakes, so a healthy sender's pause never loses a partial line, and
/// an unterminated stream trips the budget without a newline — the
/// property `Lines` lacks (it grows one `String` without limit).
fn read_daemon_line<R: BufRead>(reader: &mut R, max: usize) -> DaemonLine {
    let mut buf = Vec::new();
    loop {
        match fill_line_bounded(reader, &mut buf, max) {
            Ok(LineFill::Complete) => break,
            Ok(LineFill::Eof) if buf.is_empty() => return DaemonLine::Closed,
            Ok(LineFill::Eof) => break,
            Ok(LineFill::Oversize) => return DaemonLine::Oversize,
            // No recv timeout is set on the client stream today; if one is
            // ever added, a wake must keep `buf` (Lines would drop it).
            Err(err) if is_poll_wake(&err) => continue,
            Err(_) => return DaemonLine::Closed,
        }
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    String::from_utf8(buf).map_or(DaemonLine::Closed, DaemonLine::Line)
}

/// Which framing line a connection line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    Begin,
    End,
    Error,
}

/// Parse a `%begin`/`%end`/`%error` line into its kind and command number
/// (`%begin <time> <number> <flags>`). `None` when the line is not framing
/// or is malformed — the first whitespace field must be exactly the
/// keyword, and the number must parse, so body content that merely starts
/// with `%end` does not qualify.
fn framing(line: &str) -> Option<(Framing, u32)> {
    let mut fields = line.split_whitespace();
    let kind = match fields.next()? {
        "%begin" => Framing::Begin,
        "%end" => Framing::End,
        "%error" => Framing::Error,
        _ => return None,
    };
    let _timestamp = fields.next()?;
    let number = fields.next()?.parse().ok()?;
    Some((kind, number))
}

/// Where the par-mux daemon binary is looked up, in order: next to our own
/// executable first, then on `PATH` — the bundled daemon (shipped beside the
/// par-term binary in releases) wins over whatever an older install left on
/// `PATH`, while a `cargo install par-term-emu-core-rust --bin par-mux`-style
/// standalone install stays reachable when no sibling exists.
///
/// Returns a list; the caller tries each in order and the combined error
/// names every path tried, so the resolution order stays visible at the
/// failure where it mattered.
fn daemon_binary_candidates() -> io::Result<Vec<PathBuf>> {
    #[cfg(unix)]
    let file_name = "par-mux";
    #[cfg(windows)]
    let file_name = "par-mux.exe";

    let mut candidates = vec![exe_sibling_daemon(file_name)?];
    // PATH lookup second. `which`-style search via the PATH env var: skip
    // empty segments (embedded `::`), and only propose entries that exist,
    // so the spawn error names real candidates rather than every PATH miss.
    if let Some(path_var) = std::env::var_os("PATH") {
        candidates.extend(path_daemon_candidates(&path_var, file_name));
    }
    Ok(candidates)
}

/// The exe-relative half of [`daemon_binary_candidates`]: the daemon bin
/// sitting next to our own executable.
fn exe_sibling_daemon(file_name: &str) -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let Some(mut dir) = exe.parent().map(Path::to_path_buf) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "cannot locate the par-mux daemon binary: {} has no parent directory",
                exe.display()
            ),
        ));
    };
    // Integration-test binaries live in <target>/<profile>/deps while bins
    // sit in <target>/<profile> — walk out of deps to find the sibling bin.
    if dir.file_name() == Some(std::ffi::OsStr::new("deps")) {
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        }
    }
    Ok(dir.join(file_name))
}

/// The PATH half of [`daemon_binary_candidates`]: every PATH entry that
/// actually holds the daemon binary. Split out as a pure function of the
/// PATH value so tests can drive it with a synthetic PATH instead of
/// mutating the process environment.
fn path_daemon_candidates(path_var: &std::ffi::OsStr, file_name: &str) -> Vec<PathBuf> {
    std::env::split_paths(path_var)
        .filter(|entry| !entry.as_os_str().is_empty())
        .map(|entry| entry.join(file_name))
        .filter(|candidate| candidate.is_file())
        .collect()
}

/// Start `bin` as a par-mux daemon owning `socket`.
///
/// A spawn failure is returned, not swallowed: it means no process will ever
/// bind `socket`, so the caller has to fail now rather than spend
/// `SPAWN_CONNECT_DEADLINE` waiting on a daemon that was never started. The
/// error names the path tried and keeps the OS error kind, so a missing
/// binary stays distinguishable from one that is present but unexecutable.
///
/// The child handle is returned so the spawner can end the daemon later —
/// dropping it (as an earlier version did) orphans a live process, one leak
/// per spawn.
fn spawn_daemon(bin: &Path, socket: &Path) -> io::Result<std::process::Child> {
    let mut command = std::process::Command::new(bin);
    // --foreground: this spawner owns the child's lifecycle (it is killed
    // through the held Child), so the spawned process must BE the daemon —
    // the default daemonize-to-background would orphan a grandchild this
    // handle can never name.
    command
        .arg("--socket")
        .arg(socket)
        .arg("--foreground")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Detach into a fresh session (herdr's pre_exec setsid; tmux daemonizes
    // its server for the same reason): a daemon left in the spawner's
    // process group dies with that terminal's SIGHUP/SIGINT — taking every
    // pane, unsaved — and Ctrl-Z stops it. After setsid it leads its own
    // session with no controlling tty, so terminal-generated signals can
    // never reach it. Between fork and exec the child's pid cannot equal any
    // existing pgid, so the setsid cannot fail with EPERM.
    // SAFETY: the pre_exec hook runs in the forked child before exec, where
    // only async-signal-safe work is sound. It calls setsid(2) (listed as
    // async-signal-safe) and, on failure, builds an OS-error io::Error, which
    // only reads errno and allocates nothing; it touches no lock or heap.
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "cannot start the par-mux daemon binary at {}: {err} — build it \
                     or place it next to the executable",
                bin.display()
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::ipc::bind_local_listener;
    // Windows serves its wrapper listener, whose accept is inherent.
    #[cfg(unix)]
    use interprocess::local_socket::traits::Listener as _;

    /// How long the slow-bind daemon waits before binding its socket. Long
    /// enough that a caller which did not retry would miss it, short enough to
    /// stay far below `SPAWN_CONNECT_DEADLINE`.
    const SLOW_BIND_DELAY: Duration = Duration::from_millis(300);
    /// The bar the fast-fail path must clear. The bug was that an unspawnable
    /// binary cost the full `SPAWN_CONNECT_DEADLINE` (10s); anything near a
    /// second means the retry loop was entered anyway.
    const FAST_FAIL_BUDGET: Duration = Duration::from_millis(1000);

    /// A socket path that no other test run can ever name, cleaned up even
    /// when the test panics.
    ///
    /// A `process::id()`-derived path in the shared temp dir repeats once the
    /// OS recycles that pid, so a remnant of an earlier run collides with a
    /// later one, and a trailing `remove_file` never runs on a panic — see
    /// `mux::ipc`'s `TempSocket`, which this mirrors. The guard is returned
    /// alongside the path (not just held internally) so callers can keep it
    /// alive for as long as the socket must exist — a binder thread spawned
    /// after this call still needs the directory to be there when it binds.
    fn temp_socket(tag: &str) -> (tempfile::TempDir, PathBuf) {
        // Short prefix and tag: a slow-bind test binds a real socket here,
        // and macOS caps a Unix socket path at 104 bytes.
        let dir = tempfile::Builder::new()
            .prefix("par-mux-cli-")
            .tempdir()
            .expect("create temp dir for socket");
        let path = dir.path().join(tag);
        (dir, path)
    }

    /// A binary path guaranteed not to exist, so `spawn` must fail.
    ///
    /// The parent directory is absent too: a bare missing file next to the
    /// test binary would start existing the moment a full `cargo test` built
    /// the real `par-mux`, and the test would silently stop testing anything.
    /// Built inside a `TempDir` (removed on drop) rather than a
    /// `process::id()`-derived name in the shared temp dir, so a leftover
    /// from an earlier run can never collide with this one.
    fn missing_binary(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("absent").join("par-mux")
    }

    /// A binary that exists and spawns cleanly: this test process itself.
    ///
    /// libtest rejects `--socket` and exits immediately, which is exactly the
    /// stub wanted here — a real child process that never binds the socket,
    /// leaving the bind to the listener the test raises on its own schedule.
    fn spawnable_stub() -> PathBuf {
        std::env::current_exe().expect("the test binary's own path")
    }

    #[test]
    fn an_unspawnable_daemon_binary_fails_fast_and_names_the_path() {
        let (dir, socket) = temp_socket("nospawn");
        let bin = missing_binary(&dir);

        let started = Instant::now();
        let err = MuxClient::spawn_and_connect(&bin, &socket)
            .err()
            .expect("a daemon binary that cannot be started is an error");
        let elapsed = started.elapsed();

        assert!(
            elapsed < FAST_FAIL_BUDGET,
            "a daemon binary that cannot be started must fail immediately, not \
             after the {}s spawn-connect deadline — nothing will ever bind the \
             socket, so every retry is waste. Took {elapsed:?}",
            SPAWN_CONNECT_DEADLINE.as_secs()
        );
        let message = err.to_string();
        assert!(
            message.contains(&bin.display().to_string()),
            "the error must name the binary path that was tried, or the caller \
             cannot tell a missing daemon from an unreachable socket: {message}"
        );
        assert_eq!(
            err.kind(),
            io::ErrorKind::NotFound,
            "an absent binary keeps its OS error kind, so a missing daemon \
             stays distinguishable from one present but unexecutable: {message}"
        );
    }

    #[test]
    fn a_spawned_daemon_slow_to_bind_still_gets_the_retry_deadline() {
        let (dir, socket) = temp_socket("slowbind");
        let bind_at = socket.clone();

        // The daemon that binds late. The retry loop must outlast this.
        let binder = std::thread::spawn(move || {
            std::thread::sleep(SLOW_BIND_DELAY);
            let listener = bind_local_listener(&bind_at).expect("late bind");
            // One accept: the client only has to reach a live socket, and
            // holding the stream briefly keeps its reader thread attached.
            let stream = listener.accept().expect("accept the retrying client");
            std::thread::sleep(Duration::from_millis(100));
            drop(stream);
        });

        let started = Instant::now();
        let client = MuxClient::spawn_and_connect(&spawnable_stub(), &socket);
        let elapsed = started.elapsed();

        assert!(
            client.is_ok(),
            "a daemon that really was spawned must keep the full {}s deadline \
             to bind — fast-failing this case would break every slow start: {:?}",
            SPAWN_CONNECT_DEADLINE.as_secs(),
            client.err()
        );
        assert!(
            elapsed >= SLOW_BIND_DELAY,
            "the connect can only have succeeded by retrying past the bind \
             delay — {elapsed:?} is too fast to have waited for it"
        );
        assert!(
            elapsed < SPAWN_CONNECT_DEADLINE,
            "the retry loop must still be bounded: {elapsed:?}"
        );

        drop(client);
        binder.join().expect("binder thread");
        drop(dir);
    }

    /// A daemon answering on the pre-0.52 default path must be attached, not
    /// silently replaced: 0.52 moved the default socket into a per-UID
    /// directory, and without the probe a fresh daemon would be spawned for
    /// the same name while the old one kept its sessions invisibly.
    ///
    /// Drives the path-injected seam: the env-derived paths `connect_or_spawn`
    /// computes are passed in, so no process env is mutated (QA-196).
    #[cfg(unix)]
    #[test]
    fn a_legacy_path_daemon_is_attached_not_replaced_by_a_second_spawn() {
        let (dir, legacy) = temp_socket("legacy.sock");
        // Where the post-0.52 default would live: absent, so the connect
        // there fails and the legacy probe runs.
        let new_path = dir.path().join("new").join("default.sock");

        // The "legacy daemon": a listener on the pre-0.52 path. MuxClient
        // sends no handshake at connect, so a held accept is a live daemon
        // from the client's point of view.
        let listener = bind_local_listener(&legacy).expect("bind the legacy path");
        let holder = std::thread::spawn(move || {
            let stream = listener.accept().expect("accept the probing client");
            std::thread::sleep(Duration::from_millis(200));
            drop(stream);
        });

        let client = MuxClient::connect_or_spawn_paths(&new_path, Some(&legacy))
            .expect("attach to the legacy daemon");

        assert!(
            client.spawned_daemon.is_none(),
            "a daemon answered on the legacy path — spawning a second one for \
             the same name strands the old daemon and its sessions"
        );
        assert!(
            !new_path.exists(),
            "no daemon may bind the new default path while the legacy one is \
             alive: {}",
            new_path.display()
        );

        drop(client);
        holder.join().expect("holder thread");
        drop(dir);
    }

    #[test]
    fn the_daemon_binary_resolves_next_to_the_current_executable_first() {
        let candidates = daemon_binary_candidates().expect("the current exe has a directory");
        let bin = candidates
            .first()
            .expect("the exe-relative candidate is always present");
        let name = bin
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("a binary file name");
        assert!(
            name.starts_with("par-mux"),
            "the first daemon-binary candidate is par-mux: {}",
            bin.display()
        );
        // Test binaries live in <target>/<profile>/deps while the daemon is
        // the sibling bin one level up, so `deps` must have been walked out of.
        assert_ne!(
            bin.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new("deps")),
            "the deps walk-out did not happen: {}",
            bin.display()
        );
    }

    /// The PATH fallback half of the resolution order, driven through the
    /// pure [`path_daemon_candidates`]: a PATH entry holding par-mux
    /// contributes that candidate, and a PATH with no par-mux anywhere
    /// contributes nothing.
    #[test]
    fn a_daemon_on_path_is_a_candidate_and_a_path_without_one_adds_none() {
        let (dir, _socket) = temp_socket("pathlookup");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("create the PATH entry");
        #[cfg(unix)]
        let daemon = bin_dir.join("par-mux");
        #[cfg(windows)]
        let daemon = bin_dir.join("par-mux.exe");
        std::fs::write(&daemon, b"").expect("write the daemon stub");

        // A PATH with only the stub's directory in it yields exactly the
        // stub, as its own entry — nothing more, nothing less.
        let path_with = std::env::join_paths([&bin_dir]).expect("join a single PATH entry");
        let with = path_daemon_candidates(&path_with, file_name());
        assert_eq!(
            with,
            vec![daemon.clone()],
            "a PATH entry holding par-mux contributes exactly that candidate"
        );

        // A PATH pointing at a directory with no par-mux in it yields none.
        let empty = dir.path().join("no-bin-here");
        std::fs::create_dir_all(&empty).expect("create the empty PATH entry");
        let path_without = std::env::join_paths([&empty]).expect("join a single PATH entry");
        let without = path_daemon_candidates(&path_without, file_name());
        assert!(
            without.is_empty(),
            "a PATH entry without par-mux must not contribute candidates: {:?}",
            without
        );
    }

    /// A daemon already serving the primary path is attached through both
    /// public entry points without spawning anything, and ending "the
    /// spawned daemon" on such a client is a no-op, never a kill of a
    /// server other clients share.
    #[cfg(unix)]
    #[test]
    fn a_live_daemon_is_attached_without_spawning() {
        let (dir, socket) = temp_socket("live.sock");
        let listener = bind_local_listener(&socket).expect("bind");
        let holder = std::thread::spawn(move || {
            let a = listener.accept().expect("first client");
            let b = listener.accept().expect("second client");
            std::thread::sleep(Duration::from_millis(200));
            drop((a, b));
        });

        let mut via_paths =
            MuxClient::connect_or_spawn_paths(&socket, None).expect("attach via paths");
        let mut via_at = MuxClient::connect_or_spawn_at(&socket).expect("attach via at");
        for client in [&mut via_paths, &mut via_at] {
            assert_eq!(client.spawned_daemon_pid(), None);
            client
                .kill_spawned_daemon()
                .expect("no spawned daemon: a no-op");
        }
        drop((via_paths, via_at));
        holder.join().expect("holder thread");
        drop(dir);
    }

    /// Connecting where nothing listens is an error, not a hang or a
    /// silent spawn.
    #[test]
    fn connect_to_an_absent_socket_errors() {
        let (_dir, socket) = temp_socket("absent.sock");
        assert!(MuxClient::connect(&socket).is_err());
    }

    /// Drives the reader loop against a scripted server: a notification
    /// sent before the reply reaches `notifications()` parsed; framing
    /// lines carrying another command's number stay body content; a block
    /// closed by `%error` comes back `ok: false` with its body; and a
    /// server that closes the connection mid-command surfaces as
    /// `ConnectionAborted`, not a hang.
    #[cfg(unix)]
    #[test]
    fn replies_and_notifications_are_split_by_the_reader() {
        use std::io::{BufRead as _, Write as _};
        let (dir, socket) = temp_socket("script.sock");
        let listener = bind_local_listener(&socket).expect("bind");
        let server = std::thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            let mut writer = stream.try_clone().expect("clone");
            let mut lines = std::io::BufReader::new(stream).lines();

            assert_eq!(lines.next().unwrap().unwrap(), "capture-pane -t %1");
            writer
                .write_all(
                    b"%output %1 hi\\012\n\
                      %begin 1 5 1\n\
                      row one\n\
                      %end 1 4 1\n\
                      %begin 1 9 1\n\
                      %end 1 5 1\n",
                )
                .unwrap();

            assert_eq!(lines.next().unwrap().unwrap(), "kill-pane -t %9");
            writer
                .write_all(b"%begin 2 6 1\nno such pane: %9\n%error 2 6 1\n")
                .unwrap();

            assert_eq!(lines.next().unwrap().unwrap(), "list-panes");
            // Close without answering.
        });

        let mut client = MuxClient::connect(&socket).expect("connect");
        let reply = client.send_checked("capture-pane -t %1").expect("reply");
        assert_eq!(
            reply,
            Reply {
                body: vec![
                    "row one".to_string(),
                    "%end 1 4 1".to_string(),
                    "%begin 1 9 1".to_string(),
                ],
                ok: true,
            },
            "mismatched framing numbers are content"
        );
        assert_eq!(
            client
                .notifications()
                .recv_timeout(Duration::from_secs(5))
                .expect("the pre-reply notification"),
            TmuxNotification::Output {
                pane_id: "%1".to_string(),
                data: b"hi\n".to_vec(),
            }
        );

        let reply = client.send_checked("kill-pane -t %9").expect("reply");
        assert_eq!(
            reply,
            Reply {
                body: vec!["no such pane: %9".to_string()],
                ok: false,
            }
        );

        let err = client.send("list-panes").expect_err("server hung up");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted, "{err}");
        server.join().expect("server thread");
        drop(dir);
    }

    /// A reply-only client under a stream of pushed output: with
    /// `discard_notifications` none of the thousands of `%output` lines
    /// sent ahead of the reply is queued (the reader handles every line
    /// before the block that follows them), while a default client queues
    /// them all — the control that proves the count measures the channel.
    #[cfg(unix)]
    #[test]
    fn discarded_notifications_are_never_queued() {
        use std::io::{BufRead as _, Write as _};
        const PUSHED: usize = 5000;
        let (dir, socket) = temp_socket("discard.sock");
        let listener = bind_local_listener(&socket).expect("bind");
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let stream = listener.accept().expect("accept");
                let mut writer = stream.try_clone().expect("clone");
                let mut lines = std::io::BufReader::new(stream).lines();
                assert_eq!(lines.next().unwrap().unwrap(), "wait-output -t %1 -m X");
                for n in 0..PUSHED {
                    writeln!(writer, "%output %1 line-{n}\\012").unwrap();
                }
                writer.write_all(b"%begin 1 1 1\nX\n%end 1 1 1\n").unwrap();
            }
        });

        let mut discarding = MuxClient::connect(&socket).expect("connect");
        discarding.discard_notifications();
        let reply = discarding
            .send_checked("wait-output -t %1 -m X")
            .expect("reply");
        assert!(reply.ok);
        assert_eq!(
            discarding.notifications().try_iter().count(),
            0,
            "a discarding client queues no pushed output"
        );

        let mut keeping = MuxClient::connect(&socket).expect("connect");
        let reply = keeping
            .send_checked("wait-output -t %1 -m X")
            .expect("reply");
        assert!(reply.ok);
        assert_eq!(keeping.notifications().try_iter().count(), PUSHED);

        server.join().expect("server thread");
        drop(dir);
    }

    /// A daemon that answers each line, in order, with a block whose body is
    /// the command itself, sleeping first when the command is in `delays`.
    #[cfg(unix)]
    fn echo_daemon(
        tag: &str,
        delays: Vec<(&'static str, Duration)>,
    ) -> (tempfile::TempDir, MuxClient, std::thread::JoinHandle<()>) {
        use std::io::{BufRead as _, Write as _};
        let (dir, socket) = temp_socket(tag);
        let listener = bind_local_listener(&socket).expect("bind");
        let server = std::thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            let mut writer = stream.try_clone().expect("clone");
            let mut number = 0u32;
            for line in std::io::BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if let Some((_, delay)) = delays.iter().find(|(c, _)| *c == line) {
                    std::thread::sleep(*delay);
                }
                number += 1;
                let block = format!("%begin 1 {number} 1\n{line}\n%end 1 {number} 1\n");
                if writer.write_all(block.as_bytes()).is_err() {
                    break;
                }
            }
        });
        let client = MuxClient::connect(&socket).expect("connect");
        (dir, client, server)
    }

    #[cfg(unix)]
    fn poll_until(client: &mut MuxClient, want: usize) -> Vec<(u64, Reply)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.len() < want {
            assert!(
                Instant::now() < deadline,
                "only {} of {want} replies",
                got.len()
            );
            got.extend(client.poll_replies());
            std::thread::sleep(Duration::from_millis(2));
        }
        got
    }

    #[cfg(unix)]
    #[test]
    fn async_reply_never_answers_a_later_sync_command() {
        let (_dir, mut client, _server) = echo_daemon("owed-a.sock", vec![]);
        let token = client.send_nowait("A").expect("write A");
        let reply = client.send_checked("B").expect("B");
        assert_eq!(reply.body, vec!["B".to_string()]);
        let got = client.poll_replies();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, token);
        assert_eq!(got[0].1.body, vec!["A".to_string()]);
        assert_eq!(client.outstanding(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_timed_out_wait_does_not_steal_a_later_async_or_sync_reply() {
        let (_dir, mut client, _server) =
            echo_daemon("owed-b.sock", vec![("slow", Duration::from_millis(300))]);
        let err = client
            .send_checked_timeout("slow", Duration::from_millis(40))
            .expect_err("times out");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        let token = client.send_nowait("C").expect("write C");
        let reply = client.send_checked("D").expect("D");
        assert_eq!(
            reply.body,
            vec!["D".to_string()],
            "the abandoned block is dropped"
        );
        let got = client.poll_replies();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, token);
        assert_eq!(got[0].1.body, vec!["C".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn async_replies_are_delivered_in_order() {
        let (_dir, mut client, _server) = echo_daemon("owed-c.sock", vec![]);
        let tokens: Vec<u64> = ["one", "two", "three"]
            .iter()
            .map(|c| client.send_nowait(c).expect("write"))
            .collect();
        let got = poll_until(&mut client, 3);
        let bodies: Vec<&str> = got.iter().map(|(_, r)| r.body[0].as_str()).collect();
        assert_eq!(bodies, ["one", "two", "three"]);
        assert_eq!(got.iter().map(|(t, _)| *t).collect::<Vec<_>>(), tokens);
    }

    #[cfg(unix)]
    #[test]
    fn forgotten_replies_are_dropped_and_drain_the_outstanding_count() {
        let (_dir, mut client, _server) = echo_daemon("owed-d.sock", vec![]);
        client.send_forget("x").expect("write x");
        client.send_forget("y").expect("write y");
        assert_eq!(client.outstanding(), 2);
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.outstanding() > 0 {
            assert!(Instant::now() < deadline, "forget replies never drained");
            assert!(
                client.poll_replies().is_empty(),
                "forget replies are never returned"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// `read_daemon_line` strips `\n` and one preceding `\r` like `Lines`,
    /// and reads an unterminated final line before EOF.
    #[test]
    fn read_daemon_line_strips_lf_and_crlf() {
        let mut reader = io::Cursor::new(b"a\nb\r\nc".to_vec());
        assert_eq!(
            read_daemon_line(&mut reader, 16),
            DaemonLine::Line("a".into())
        );
        assert_eq!(
            read_daemon_line(&mut reader, 16),
            DaemonLine::Line("b".into())
        );
        assert_eq!(
            read_daemon_line(&mut reader, 16),
            DaemonLine::Line("c".into())
        );
        assert_eq!(read_daemon_line(&mut reader, 16), DaemonLine::Closed);
    }

    /// The budget trips mid-line without a newline — the property `Lines`
    /// lacks: it would grow one `String` forever.
    #[test]
    fn read_daemon_line_trips_the_budget_without_a_newline() {
        let mut reader = io::Cursor::new(vec![b'x'; 300]);
        assert_eq!(read_daemon_line(&mut reader, 100), DaemonLine::Oversize);
    }

    /// Non-UTF-8 closes the connection, matching `Lines`' InvalidData break.
    #[test]
    fn read_daemon_line_closes_on_non_utf8() {
        let mut reader = io::Cursor::new(b"\xff\n".to_vec());
        assert_eq!(read_daemon_line(&mut reader, 16), DaemonLine::Closed);
    }

    /// An unterminated daemon line past the budget ends the reader while
    /// the socket stays open: the pending `send_checked` reports
    /// `ConnectionAborted` from the dropped reply channel, not EOF.
    #[cfg(unix)]
    #[test]
    fn an_unterminated_oversize_daemon_line_aborts_the_pending_command() {
        use std::io::Write as _;
        let (dir, socket) = temp_socket("oversize.sock");
        let listener = bind_local_listener(&socket).expect("bind");
        let server = std::thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            let mut writer = stream.try_clone().expect("clone");
            let mut lines = std::io::BufReader::new(stream).lines();

            assert_eq!(lines.next().unwrap().unwrap(), "show-buffer");
            writer.write_all(b"%begin 1 5 1\n").unwrap();
            let chunk = [b'x'; 64 * 1024];
            let mut remaining = MAX_REPLY_LINE_BYTES + 1;
            while remaining > 0 {
                let take = remaining.min(chunk.len());
                writer.write_all(&chunk[..take]).unwrap();
                remaining -= take;
            }
            writer.flush().unwrap();
            // Hold the socket: a line the client never finishes reading
            // keeps the budget — not EOF — as the only way out. Blocks in
            // `next()` until the client drops its side.
            let _ = lines.next();
        });

        let mut client = MuxClient::connect(&socket).expect("connect");
        let err = client.send_checked("show-buffer").expect_err("budget trip");
        assert_eq!(
            err.kind(),
            io::ErrorKind::ConnectionAborted,
            "the budget trip must surface as the dropped reply channel, not a hang: {err}"
        );
        drop(client);
        server.join().expect("server thread");
        drop(dir);
    }

    /// The platform daemon file name, shared by the tests above.
    fn file_name() -> &'static str {
        #[cfg(unix)]
        {
            "par-mux"
        }
        #[cfg(windows)]
        {
            "par-mux.exe"
        }
    }

    /// The framing parser accepts the daemon's `%begin/%end/%error <time>
    /// <number> <flags>` shape and rejects everything else — a bare
    /// keyword, a missing or non-numeric number, and content that merely
    /// starts with a keyword are all body, not framing.
    #[test]
    fn framing_parses_only_well_formed_numbered_lines() {
        assert_eq!(framing("%begin 1780000000 4 1"), Some((Framing::Begin, 4)));
        assert_eq!(framing("%end 1780000000 4 1"), Some((Framing::End, 4)));
        assert_eq!(framing("%error 0 12 1"), Some((Framing::Error, 12)));
        assert_eq!(framing("%end"), None, "no fields at all");
        assert_eq!(framing("%end 1780000000"), None, "number missing");
        assert_eq!(framing("%end 1780000000 x 1"), None, "number not numeric");
        assert_eq!(
            framing("%endless 1780000000 4 1"),
            None,
            "keyword is a prefix of a longer word, not a field"
        );
        assert_eq!(
            framing("%output %0 1"),
            None,
            "a notification is not framing"
        );
    }
}
