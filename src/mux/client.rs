//! The control-mode client: connect, auto-spawn, command, and pushed
//! notifications.
//!
//! Command replies and asynchronous notifications use separate paths: `send`
//! consumes `%begin`/`%end` blocks, while [`MuxClient::notifications`] carries
//! everything else, parsed. A reader that conflated the two would deadlock the
//! first time pane output arrived mid-command.

use crate::mux::ipc::{connect_local_stream, default_socket_path, LocalStream};
use crate::tmux_control::{TmuxControlParser, TmuxNotification};
use interprocess::TryClone as _;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// How long `send` waits for a reply block before giving up.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `connect_or_spawn_at` keeps retrying after spawning a daemon.
const SPAWN_CONNECT_DEADLINE: Duration = Duration::from_secs(10);

/// A control-mode client connected (or connectable) to a par-mux daemon.
pub struct MuxClient {
    writer: LocalStream,
    reply_rx: Receiver<Reply>,
    notifications_rx: Receiver<TmuxNotification>,
    /// The daemon this client spawned, when it started one. Held so the
    /// spawner can end what it created via [`MuxClient::kill_spawned_daemon`];
    /// `None` when the client attached to an existing server.
    spawned_daemon: Option<std::process::Child>,
}

impl MuxClient {
    /// Connect to a daemon already listening at `path`.
    pub fn connect(path: &Path) -> io::Result<Self> {
        let stream = connect_local_stream(path)?;
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
    pub fn connect_or_spawn(name: &str) -> io::Result<Self> {
        let path = default_socket_path(name);
        if let Ok(client) = Self::connect(&path) {
            return Ok(client);
        }
        #[cfg(unix)]
        if let Ok(client) = Self::connect(&crate::mux::ipc::legacy_socket_path(name)) {
            return Ok(client);
        }
        Self::connect_or_spawn_at(&path)
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
        std::thread::spawn(move || reader_loop(stream, reply_tx, notification_tx));
        Ok(Self {
            writer,
            reply_rx,
            notifications_rx,
            spawned_daemon: None,
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
        writeln!(self.writer, "{command}")?;
        self.writer.flush()?;
        match self.reply_rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(reply) => Ok(reply),
            Err(RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("no reply block within {}s", REPLY_TIMEOUT.as_secs()),
            )),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "server closed the connection",
            )),
        }
    }

    /// Pushed notifications (`%output` and friends), already parsed.
    pub fn notifications(&self) -> &Receiver<TmuxNotification> {
        &self.notifications_rx
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
) {
    let reader = BufReader::new(stream);
    let mut parser = TmuxControlParser::new(true);
    // The open reply block: its command number and body so far.
    let mut open: Option<(u32, Vec<String>)> = None;
    for line in reader.lines() {
        let Ok(line) = line else { break };
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
    command
        .arg("--socket")
        .arg(socket)
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

    /// Restores `TMPDIR` and `XDG_RUNTIME_DIR` when dropped, so the env
    /// mutations of the legacy-probe test cannot leak into later tests even
    /// on a panic. The mux suite runs under `--test-threads=1`, which makes
    /// mutating process env safe here.
    struct EnvGuard {
        tmpdir: Option<std::ffi::OsString>,
        xdg: Option<std::ffi::OsString>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.tmpdir.take() {
                Some(v) => std::env::set_var("TMPDIR", v),
                None => std::env::remove_var("TMPDIR"),
            }
            match self.xdg.take() {
                Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
                None => std::env::remove_var("XDG_RUNTIME_DIR"),
            }
        }
    }

    /// A daemon answering on the pre-0.52 default path must be attached, not
    /// silently replaced: 0.52 moved the default socket into a per-UID
    /// directory, and without the probe a fresh daemon would be spawned for
    /// the same name while the old one kept its sessions invisibly.
    #[cfg(unix)]
    #[test]
    fn a_legacy_path_daemon_is_attached_not_replaced_by_a_second_spawn() {
        // Short name: the default path nests two directories deep, and
        // macOS caps a Unix socket path at 104 bytes — a long name would
        // make a regressed run fail on path length instead of on the
        // second spawn it actually performed.
        let name = format!("lp-{}", std::process::id());
        let (dir, legacy) = temp_socket(&format!("par-mux-{name}.sock"));
        // The new default path must land somewhere else inside this temp
        // dir root: point TMPDIR at it and keep XDG_RUNTIME_DIR out of the
        // way so the per-UID fallback applies.
        let guard = EnvGuard {
            tmpdir: std::env::var_os("TMPDIR"),
            xdg: std::env::var_os("XDG_RUNTIME_DIR"),
        };
        std::env::set_var("TMPDIR", dir.path());
        std::env::remove_var("XDG_RUNTIME_DIR");

        // The "legacy daemon": a listener on the pre-0.52 path. MuxClient
        // sends no handshake at connect, so a held accept is a live daemon
        // from the client's point of view.
        let listener = bind_local_listener(&legacy).expect("bind the legacy path");
        let holder = std::thread::spawn(move || {
            let stream = listener.accept().expect("accept the probing client");
            std::thread::sleep(Duration::from_millis(200));
            drop(stream);
        });

        let new_path = default_socket_path(&name);
        let client = MuxClient::connect_or_spawn(&name).expect("attach to the legacy daemon");

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
        drop(guard);
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
