//! The daemon half of `par-mux`: `--stop`, `--restart`, and serving the
//! control socket. [`run_daemon`] walks the phases in order — resolve the
//! config and state path ([`DaemonLaunch::new`]), the stop phase, the
//! unix detach/fork phase, the state lock and nested-daemon guard, then
//! bind ([`DaemonLaunch::bind`]) and serve until shutdown
//! ([`Serving::serve`], which also tears the registry entry down).

use par_mux::mux::config::{self, EffectiveConfig};

use super::{effective, update_cli, Cli};

/// How long --stop/--restart wait for the old daemon to release its socket.
const STOP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Ask the daemon on `path` to shut down (`kill-server`), then wait until
/// the socket stops accepting. `Ok(false)` means nothing was running.
fn stop_daemon(path: &std::path::Path) -> std::io::Result<bool> {
    use std::io::{BufRead, BufReader, Write};
    let stream = match par_mux::mux::connect_local_stream(path) {
        Ok(stream) => stream,
        Err(_) => return Ok(false),
    };
    let mut writer = interprocess::TryClone::try_clone(&stream)?;
    writeln!(writer, "kill-server")?;
    writer.flush()?;
    // Read through the reply block so a refusal surfaces as an error
    // instead of a silent wait for a daemon that will never exit.
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line.starts_with("%error") {
            return Err(std::io::Error::other("the daemon refused kill-server"));
        }
        if line.starts_with("%end") {
            break;
        }
    }
    drop(reader);
    drop(writer);
    let deadline = std::time::Instant::now() + STOP_DEADLINE;
    while par_mux::mux::connect_local_stream(path).is_ok() {
        if std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "the daemon on {} did not exit within {}s",
                    path.display(),
                    STOP_DEADLINE.as_secs()
                ),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(true)
}

/// The inside-a-pane stop guard (card 01a18000): `par-mux --stop` typed
/// inside a pane kills the daemon hosting the caller's own shell — a
/// session died exactly this way. Refuse while the caller sits in a pane
/// whose exported socket is the daemon being stopped, unless the stop was
/// confirmed with `--yes`/`--force`. An explicit `--socket` naming a
/// DIFFERENT daemon is deliberate either way and never refuses; a pane
/// with no socket export has nothing to match against.
pub(super) fn stop_inside_pane_guard(
    inside_pane: bool,
    target_socket: &std::path::Path,
    pane_socket: Option<&str>,
    pane_control_socket: Option<&str>,
    unconfirmed: bool,
) -> Option<String> {
    if !inside_pane || !unconfirmed {
        return None;
    }
    let target =
        std::fs::canonicalize(target_socket).unwrap_or_else(|_| target_socket.to_path_buf());
    for candidate in [pane_socket, pane_control_socket].into_iter().flatten() {
        let candidate_path = std::path::Path::new(candidate);
        let same = std::fs::canonicalize(candidate_path)
            .map(|canonical| canonical == target)
            .unwrap_or_else(|_| candidate == target_socket.to_str().unwrap_or(""));
        if same {
            return Some(
                "refusing to stop the daemon hosting this pane (the stop would kill \
                 this shell and every sibling pane); re-run with --yes or --force to \
                 confirm, or run the stop from outside par-mux"
                    .to_string(),
            );
        }
    }
    None
}

/// One daemon invocation's resolved inputs, shared by every phase.
struct DaemonLaunch {
    cli: Cli,
    path: std::path::PathBuf,
    eff: EffectiveConfig,
    state_dir_pinned: bool,
    state_path: std::path::PathBuf,
}

/// A bound server and what its shutdown teardown needs.
struct Serving {
    server: par_mux::mux::MuxServer,
    path: std::path::PathBuf,
    state_path: std::path::PathBuf,
    registry_base: std::path::PathBuf,
}

/// The daemon modes: --stop, --restart, and serving the socket — detached
/// with an auto-attach by default (unix), or in the foreground with
/// --foreground.
pub(super) fn run_daemon(
    cli: Cli,
    path: std::path::PathBuf,
) -> std::io::Result<std::process::ExitCode> {
    // Windows: a pipe-path --socket (\\.\pipe\name) names the same pipe
    // as the bare stem — strip the prefix ONCE here so every consumer
    // downstream (bind, the state-file stem, the panes' PAR_MUX_SOCKET env
    // contract, which the .ps1 hooks pass straight to NamedPipeClientStream)
    // agrees on the bare name.
    #[cfg(windows)]
    let path = par_mux::mux::ipc::strip_pipe_prefix(&path);
    let launch = DaemonLaunch::new(cli, path);
    if launch.cli.stop || launch.cli.restart {
        if let Some(code) = launch.stop_phase()? {
            return Ok(code);
        }
    }

    // Detach policy (unix): fork-to-background is the DEFAULT for both a
    // fresh start and --restart — herdr's launch shape. The forked child
    // serves with no controlling tty and stderr on the daemon log beside
    // the state file, so post-fork startup failures (bind, restore,
    // exit-when-empty) leave evidence instead of vanishing; the parent
    // waits for the socket, then auto-attaches this terminal unless
    // --no-attach was given or stdin is not one (scripted starts stay
    // daemon-only). --foreground keeps this exact process serving — the
    // old default, and the shape every programmatic spawner uses, because
    // a spawner that kills the child it spawned would orphan a detached
    // grandchild it can never name.
    //
    // macOS: hand the daemon start to the installed Par-mux.app wrapper
    // when one exists. TCC's local-network grant attaches to the app
    // bundle, not to a bare binary — a daemon exec'd by the app's
    // launcher inherits the app's grant (the phone's streaming client
    // needs it), while a daemon forked from a terminal binary is blocked.
    // The launcher forwards our argv verbatim and execs the resolved
    // par-mux binary in the app's context; PAR_MUX_NO_APP_WRAPPER=1 opts
    // out, and an exec failure degrades to the bare start below.
    // The whole start path below is unix-gated (fork); the hand-off is
    // additionally macOS-gated and never fires inside a pane — an
    // inside-pane start keeps its deliberate serve-refusal and
    // --restart-exemption semantics byte for byte.
    #[cfg(unix)]
    if !launch.cli.foreground {
        if let Some(code) = launch.detach_phase()? {
            return Ok(code);
        }
    }
    // Claim the state file before any serving half starts, detached or
    // foreground: a second daemon on the same state raced the first's
    // fixed-tmp saves into corruption (the 2026-10-27 crash loop), and
    // bind's live-socket refusal cannot see a daemon that load stalled
    // into unresponsiveness. The claim is taken AFTER the app hand-off
    // (the exec'd launcher must not inherit this process's lock fd — it
    // would collide with the handed-off daemon's own claim), survives
    // the fork (the serving child inherits the open file description,
    // and a flock lives as long as any fd to that description does), and
    // releases on exit, so the --stop/--restart handoff re-acquires
    // inside the retry window.
    let _state_lock = par_mux::mux::persist::acquire_state_lock(&launch.state_path)?;

    // Nested-daemon guard, on the path that actually serves: a daemon
    // started from inside a pane (PAR_MUX_ENV=1) would shadow the outer
    // server's identity for every PTY under it. --cmd returned earlier and
    // --stop/--restart are exempt above, as tmux exempts kill-server; the
    // guard below therefore only bites plain serve mode.
    if !launch.cli.restart {
        if let Some(reason) = par_mux::mux::nested_daemon_refusal() {
            return Err(std::io::Error::other(reason));
        }
    }

    launch.bind()?.serve()
}

impl DaemonLaunch {
    fn new(cli: Cli, path: std::path::PathBuf) -> Self {
        // Resolved before the stop phase so --restart can report, before the
        // fork, what the fresh daemon will restore — and so the serving half
        // and the peek see the same resolved config. `effective` is pure:
        // flags > env > config-file tiers, no I/O beyond the config read.
        let eff = effective(&cli, true);
        let state_dir_pinned = !eff.state_dir.is_empty();
        let state_path = match eff.state_dir.as_str() {
            "" => par_mux::mux::persist::state_file_path(&path),
            dir => par_mux::mux::persist::state_file_in(std::path::Path::new(dir), &path),
        };
        Self {
            cli,
            path,
            eff,
            state_dir_pinned,
            state_path,
        }
    }

    /// `--stop` / the stop half of `--restart`. `Some` is the finished
    /// exit (`--stop`); `None` falls through to the restart's fresh start.
    fn stop_phase(&self) -> std::io::Result<Option<std::process::ExitCode>> {
        // The suicide guard (card 01a18000): a --stop typed inside a pane
        // kills the daemon hosting the caller's own shell. Refuse while
        // the caller sits in the pane of the daemon being stopped, unless
        // --yes/--force confirmed it. --restart keeps its deliberate
        // exemption (tmux's kill-server shape).
        if self.cli.stop {
            if let Some(reason) = stop_inside_pane_guard(
                std::env::var_os("PAR_MUX_ENV").is_some(),
                &self.path,
                std::env::var("PAR_MUX_SOCKET").ok().as_deref(),
                std::env::var("PAR_MUX_CONTROL_SOCKET").ok().as_deref(),
                !(self.cli.yes || self.cli.force),
            ) {
                return Err(std::io::Error::other(reason));
            }
        }
        if stop_daemon(&self.path)? {
            eprintln!("par-mux: stopped the daemon on {}", self.path.display());
        } else {
            eprintln!("par-mux: no daemon running on {}", self.path.display());
        }
        if self.cli.stop {
            return Ok(Some(std::process::ExitCode::SUCCESS));
        }
        // Pre-fork, on the still-open terminal: what the detached fresh
        // daemon will restore. After the fork its stderr leaves the
        // terminal, so this is the caller's only window on the handoff.
        // Mirrors load_or_quarantine's conclusions (last-good fallback
        // included) without mutating anything.
        match par_mux::mux::persist::peek_state(&self.state_path) {
            par_mux::mux::persist::StatePeek::Populated => {}
            par_mux::mux::persist::StatePeek::Empty => eprintln!(
                "par-mux: previous daemon saved an empty tree at {} — the fresh daemon will \
                 restore nothing and exit (exit-when-empty)",
                self.state_path.display()
            ),
            par_mux::mux::persist::StatePeek::Missing => eprintln!(
                "par-mux: no saved state at {} — the fresh daemon will start empty and exit \
                 (exit-when-empty)",
                self.state_path.display()
            ),
            par_mux::mux::persist::StatePeek::Unreadable => eprintln!(
                "par-mux: saved state {} is unreadable — the fresh daemon will quarantine it \
                 and start fresh (details: {})",
                self.state_path.display(),
                daemon_log_path(&self.state_path, self.state_dir_pinned).display()
            ),
        }
        // --restart falls through to the detach below — the state save the
        // stop just completed is what the fresh daemon restores, and the
        // daemon must outlive the terminal --restart was typed into.
        Ok(None)
    }

    /// The unix detach: nested refusal, the macOS app hand-off, then the
    /// fork. `None` means this process is the serving child; `Some` is the
    /// parent's exit after its wait-and-attach.
    #[cfg(unix)]
    fn detach_phase(&self) -> std::io::Result<Option<std::process::ExitCode>> {
        // The nested-daemon refusal first, on the terminal rather than
        // buried in the daemon log (--restart stays exempt: it may be
        // typed inside a pane deliberately; the guard below still runs in
        // the serving child for the non-exempt paths).
        let nested = par_mux::mux::nested_daemon_refusal();
        if let (Some(reason), false) = (nested, self.cli.restart) {
            return Err(std::io::Error::other(reason));
        }
        // The wrapper hand-off only for starts the refusal passed: a pane
        // keeps its own semantics (a handed-off daemon would land OUTSIDE
        // the pane's daemon, silently dodging the nested guard).
        #[cfg(target_os = "macos")]
        // Only a real start hands off: with a daemon already serving, the
        // attach below must keep the invoking terminal's stdio, which the
        // launcher's daemon.log redirect would swallow (2026-10-27).
        if !self.cli.par_mux_in_app_wrapper
            && nested.is_none()
            && par_mux::mux::connect_local_stream(&self.path).is_err()
        {
            if let Ok(exe) = std::env::current_exe() {
                let no_wrapper =
                    std::env::var_os("PAR_MUX_NO_APP_WRAPPER").is_some_and(|v| v == "1");
                if let Some(launcher) = app_handoff_launcher(
                    &exe,
                    std::path::Path::new("/Applications/Par-mux.app"),
                    no_wrapper,
                ) {
                    log::info!(
                        "par-mux: handing the daemon start to {}",
                        launcher.display()
                    );
                    let mut cmd = std::process::Command::new(&launcher);
                    // The marker rides first so the handed-off start knows it
                    // is already in the wrapper and never hands off again.
                    cmd.arg("--par-mux-in-app-wrapper");
                    cmd.args(std::env::args_os().skip(1));
                    let err = std::os::unix::process::CommandExt::exec(&mut cmd);
                    log::warn!("par-mux: app hand-off failed ({err}); starting the bare daemon");
                }
            }
        }
        match fork_daemon(Some(&daemon_log_path(
            &self.state_path,
            self.state_dir_pinned,
        )))? {
            ForkOutcome::Child => Ok(None), // this process serves, detached
            ForkOutcome::Parent => run_detached_parent(
                &self.cli,
                &self.path,
                &self.state_path,
                self.state_dir_pinned,
            )
            .map(Some),
        }
    }

    /// Restore or build the tree, bind the socket, publish the config,
    /// and register in the discovery registry.
    fn bind(self) -> std::io::Result<Serving> {
        let Self {
            cli,
            path,
            eff,
            state_path,
            ..
        } = self;
        // D3.2/D3.3: a corrupt or unknown-version state file is quarantined
        // aside and the daemon starts fresh — unreadable state never blocks
        // startup. A readable state is REBUILT (D3.5): layout and content are
        // restored, and every pane's process is new — the original processes
        // died with the previous server, which is stated rather than papered
        // over.
        // ENH-039: --pane-endpoints wires the pane-endpoint channel between the
        // factory (which binds each pane's hook-only socket) and the server
        // (which serves the connections the endpoints accept).
        let (pane_endpoint_tx, pane_endpoint_rx) = if eff.pane_endpoints {
            let (tx, rx) = par_mux::mux::server::pane_endpoint_channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        // One factory serves both fresh and restored trees, so every pane gets
        // the same env contract. PAR_MUX_BIN is this executable: only the binary
        // knows it — in library code current_exe() names the embedding process.
        let factory = || par_mux::mux::pane::ShellPaneFactory {
            socket_path: Some(path.to_string_lossy().into_owned()),
            bin_path: std::env::current_exe()
                .ok()
                .map(|exe| exe.to_string_lossy().into_owned()),
            pane_endpoint_tx: pane_endpoint_tx.clone(),
            expose_control_socket: eff.expose_control_socket,
            ..Default::default()
        };
        let restored = match par_mux::mux::persist::load_or_quarantine(&state_path) {
            par_mux::mux::persist::Loaded::Fresh => None,
            par_mux::mux::persist::Loaded::State(state) => {
                match par_mux::mux::tree::MuxTree::from_persist_state_with_policy(
                    &state,
                    Box::new(factory()),
                    par_mux::mux::config::daemon_remain_on_exit(),
                    &par_mux::mux::config::daemon_agent_append(),
                    &par_mux::mux::config::daemon_agent_resume_prompt(),
                ) {
                    Ok(tree) => Some(tree),
                    Err(err) => {
                        log::warn!("par-mux: state restore failed ({err}); starting fresh");
                        None
                    }
                }
            }
            par_mux::mux::persist::Loaded::Quarantined { .. } => None,
        };

        // bind refuses a path a live server already owns, so a racing auto-spawn
        // loses cleanly instead of stealing the socket.
        let tree =
            restored.unwrap_or_else(|| par_mux::mux::tree::MuxTree::new(Box::new(factory())));
        // Restart loudness: a fresh daemon that restored nothing used to
        // exit-when-empty without a trace anywhere. The pre-fork peek (above)
        // reported the save side on the terminal; this records the restore
        // side — including a populated save that failed to rebuild — in the
        // daemon log the fork points stderr at.
        if cli.restart && tree.sessions().is_empty() {
            log::warn!("par-mux: restored tree is empty; daemon exiting (exit-when-empty)");
        }
        // Committed upload files persist for pane apps by design, so spool
        // files orphaned by a dead daemon linger until OS temp cleanup — which
        // is not guaranteed to ever run. Anything older than 24 h cannot be an
        // in-flight upload of a concurrently running daemon (the age guard is
        // the concurrency guard), so the startup sweep deletes it. Fail-open:
        // a sweep error logs and daemon start continues.
        let swept = par_mux::mux::uploads::sweep_stale_at_startup();
        if swept > 0 {
            log::info!("par-mux: startup sweep deleted {swept} stale upload spool file(s)");
        }
        if eff.pane_endpoints {
            // Reclaim crash leftovers before this daemon binds anything, so the
            // endpoint cap counts only live endpoints (ENH-039).
            par_mux::mux::server::sweep_pane_endpoint_remnants(&path);
        }
        let mut server = match pane_endpoint_rx {
            Some(rx) => {
                par_mux::mux::MuxServer::bind_with_tree_and_pane_endpoints(&path, tree, rx)?
            }
            None => par_mux::mux::MuxServer::bind_with_tree(&path, tree)?,
        };
        // Publish the applied settings: `reload-config` diffs its re-read
        // against this copy (restart-required vs unchanged, per setting).
        // The copy carries the file it was resolved from so the reload
        // re-reads THAT path, not whatever the env resolves to later.
        let mut eff = eff;
        eff.config_path = config::config_file_path();
        server.set_config(std::sync::Arc::new(parking_lot::Mutex::new(eff.clone())));
        log::info!("par-mux listening on {}", path.display());

        // Discovery registry: publish this daemon under the same state base
        // its state file uses, so `--list-servers` finds it. Best-effort on
        // both ends — a registry failure must not stop serving, and a failed
        // unregister must not fail a clean shutdown.
        let registry_base = match eff.state_dir.as_str() {
            "" => par_mux::mux::discovery::default_registry_base(),
            dir => std::path::PathBuf::from(dir),
        };
        if let Err(err) = par_mux::mux::discovery::register(&registry_base, &path) {
            log::warn!("par-mux: could not register in the server registry: {err}");
        }
        Ok(Serving {
            server,
            path,
            state_path,
            registry_base,
        })
    }
}

impl Serving {
    /// Serve until shutdown, then the teardown (registry unregister).
    fn serve(self) -> std::io::Result<std::process::ExitCode> {
        let Self {
            server,
            path,
            state_path,
            registry_base,
        } = self;
        // A clean SIGTERM saves on the way out (Task 3.5): the handler requests
        // shutdown with one atomic store (async-signal-safe), the accept loop
        // notices, and run_persisting's final save captures every completed
        // mutation. Terminal-generated signals that would stop a daemon whose
        // panes outlive any one terminal are ignored instead (tmux does the
        // same on its server). kill -9 skips all of this and simply loses the
        // last window (D3.3 covers why that is acceptable). The handle is
        // per-instance (ARC-016) and published to the handler via OnceLock.
        #[cfg(unix)]
        {
            SHUTDOWN_HANDLE.set(server.shutdown_handle()).ok();
            install_signal_handlers()?;
        }

        update_cli::start_auto_update(&path);
        server.run_persisting(state_path);
        // Registry entry out only AFTER the serve loop returns (the final save
        // has landed): a list-servers racing the shutdown must not see the
        // entry vanish while the socket can still answer. Best-effort — a
        // failed unlink must not fail a clean shutdown.
        if let Err(err) = par_mux::mux::discovery::unregister(&registry_base, &path) {
            log::warn!("par-mux: could not remove the registry entry: {err}");
        }
        Ok(std::process::ExitCode::SUCCESS)
    }
}

/// The outcome of [`fork_daemon`] for the caller.
#[cfg(unix)]
enum ForkOutcome {
    /// This is the original process; the detached child serves on its own.
    Parent,
    /// This is the detached child; it serves.
    Child,
}

/// Fork the serving daemon off the terminal it was typed into: the child
/// calls `setsid` and serves with stdin/stdout on `/dev/null` and stderr
/// on `stderr_log` (tmux's `daemon(1,0)` shape, with stderr kept); the
/// PARENT RETURNS — it waits for the socket and auto-attaches (the
/// herdr launch shape), or exits daemon-only under --no-attach. Unfixed,
/// `par-mux --restart NAME &` stayed in the shell's job, so closing that
/// terminal SIGHUP-killed the daemon and every pane with no save.
///
/// Must run while the process is still single-threaded (fork discipline);
/// everything before serve mode — argument parsing, the `--stop` half of
/// `--restart` — qualifies.
///
/// stderr goes to `stderr_log` (the daemon log beside the state file)
/// rather than `/dev/null`: after the fork nothing else carries the fresh
/// daemon's startup failures (bind, restore, exit-when-empty) anywhere the
/// caller can find them, and the terminal it was typed into may close at
/// any moment, so a terminal-attached stderr would risk EIO on every log
/// line once it does. A log that cannot be opened falls back to /dev/null.
#[cfg(unix)]
fn fork_daemon(stderr_log: Option<&std::path::Path>) -> std::io::Result<ForkOutcome> {
    use nix::unistd::{fork, setsid, ForkResult};
    use std::io::Write as _;
    use std::os::fd::AsRawFd as _;

    // Flush the stop-phase report lines before the fork duplicates buffers.
    let _ = std::io::stderr().flush();
    // SAFETY: the caller runs this before serve mode spawns any thread (see
    // the doc comment), so the child inherits a single-threaded process and
    // may call non-async-signal-safe functions.
    match unsafe { fork() }.map_err(std::io::Error::from)? {
        ForkResult::Parent { .. } => Ok(ForkOutcome::Parent),
        ForkResult::Child => {
            // The daemon now leads its own session, no controlling tty.
            // stdin/stdout land on /dev/null; stderr goes to the daemon log
            // (see the doc comment).
            setsid().map_err(std::io::Error::from)?;
            let null = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/null")?;
            let fd = null.as_raw_fd();
            for target in [0, 1] {
                // libc rather than nix::unistd: nix 0.31 no longer ships dup2.
                // SAFETY: `fd` is the open /dev/null handle `null` owns for
                // this whole loop, and 0-1 are the standard descriptors.
                if unsafe { libc::dup2(fd, target) } == -1 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            // stderr: the daemon log when it can be opened (append, so a
            // restart appends to the previous daemon's record), else
            // /dev/null as before. The log's parent (the state dir) may not
            // exist yet — create it rather than silently losing the log.
            if let Some(parent) = stderr_log.and_then(std::path::Path::parent) {
                let _ = std::fs::create_dir_all(parent);
            }
            let log = stderr_log.and_then(|log_path| {
                let mut options = std::fs::OpenOptions::new();
                options.append(true).create(true);
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                options.open(log_path).ok()
            });
            match log {
                Some(file) => {
                    let log_fd = file.as_raw_fd();
                    // SAFETY: `log_fd` is the open log file `file` owns for
                    // this whole statement, and 2 is stderr.
                    if unsafe { libc::dup2(log_fd, 2) } == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                None => {
                    // SAFETY: `fd` is the open /dev/null handle `null`
                    // still owns, and 2 is stderr.
                    if unsafe { libc::dup2(fd, 2) } == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
            }
            Ok(ForkOutcome::Child)
        }
    }
}

/// The parent half of a detached start: wait (bounded) for the child's
/// socket to accept, then either exit daemon-only (--no-attach, or stdin
/// is not a terminal — scripted starts never attach) or exec the attach
/// client in this process, herdr's launch-or-attach. A socket that never
/// accepts is an error naming the daemon log, because after the fork the
/// child's failures are only visible there.
#[cfg(unix)]
/// The installed Par-mux.app wrapper's launcher, when a daemon start
/// should hand off to it: the launcher exists, the running binary is not
/// itself inside that bundle (a wrapper must never hand off to itself),
/// and the operator has not opted out with
/// `PAR_MUX_NO_APP_WRAPPER=1`. `exe` and `app_root` are injectable so the
/// tests can build both sides; the serve path passes `current_exe()` and
/// the fixed `/Applications/Par-mux.app`.
#[cfg(target_os = "macos")]
pub(super) fn app_handoff_launcher(
    exe: &std::path::Path,
    app_root: &std::path::Path,
    no_wrapper: bool,
) -> Option<std::path::PathBuf> {
    if no_wrapper {
        return None;
    }
    let app_root = app_root.canonicalize().ok()?;
    let launcher = app_root.join("Contents/MacOS/Par-mux");
    if !launcher.is_file() {
        return None;
    }
    let exe = exe.canonicalize().ok()?;
    if exe.starts_with(&app_root) {
        return None;
    }
    Some(launcher)
}

#[cfg(unix)]
fn run_detached_parent(
    cli: &Cli,
    path: &std::path::Path,
    state_path: &std::path::Path,
    state_dir_pinned: bool,
) -> std::io::Result<std::process::ExitCode> {
    const READY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
    let deadline = std::time::Instant::now() + READY_DEADLINE;
    let mut ready = false;
    while std::time::Instant::now() < deadline {
        if par_mux::mux::connect_local_stream(path).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !ready {
        // --restart keeps its fire-and-forget contract: a serve failure
        // after the fork is the daemon log's news (the contract
        // cli_restart_surfaces_a_fresh_daemon_startup_failure pins), and a
        // healthy restart still lands in the attach below. A plain start
        // is the caller's primary intent, so it fails loudly instead.
        let note = format!(
            "the daemon did not become ready on {} within 10s; see {}",
            path.display(),
            daemon_log_path(state_path, state_dir_pinned).display()
        );
        if cli.restart {
            eprintln!("par-mux: {note}");
            return Ok(std::process::ExitCode::SUCCESS);
        }
        return Err(std::io::Error::other(note));
    }
    eprintln!("par-mux: daemon running on {}", path.display());
    if cli.no_attach {
        return Ok(std::process::ExitCode::SUCCESS);
    }
    // SAFETY: isatty takes no arguments and cannot fail.
    #[cfg(unix)]
    if unsafe { libc::isatty(0) } != 1 {
        return Ok(std::process::ExitCode::SUCCESS);
    }
    #[cfg(all(unix, feature = "attach"))]
    {
        use std::os::unix::process::CommandExt as _;
        let exe = std::env::current_exe()?;
        // exec(2) replaces this process on success; the returned error is
        // only the exec failure itself.
        let err = std::process::Command::new(exe)
            .arg("attach")
            .arg("--socket")
            .arg(path)
            .exec();
        Err(err)
    }
    #[cfg(not(all(unix, feature = "attach")))]
    {
        eprintln!(
            "par-mux: daemon running on {} (auto-attach unavailable here)",
            path.display()
        );
        Ok(std::process::ExitCode::SUCCESS)
    }
}

/// The detached daemon's stderr log. The default instance (no pinned
/// state dir, state file `par-mux-default.state.json`) logs to the file the
/// Par-mux.app launcher and launchd write, `~/Library/Logs/par-mux/daemon.log`
/// on macOS (`daemon.log` beside the state file elsewhere), so a `--restart`
/// self-relaunch keeps appending to its predecessor's log instead of forking
/// stderr to a sibling file nobody tails. Every other instance, and any
/// explicit state dir, logs beside its state file as `<state>.log`, so test
/// and side daemons never touch the default log. After daemonize the
/// terminal the command was typed into may be gone, so a `--restart` whose
/// serving half fails leaves its reason here. On Windows restart serves
/// in-process and keeps the caller's stderr, so the log is only written on
/// Unix.
fn daemon_log_path(state_path: &std::path::Path, state_dir_pinned: bool) -> std::path::PathBuf {
    let default_instance = !state_dir_pinned
        && state_path.file_name().and_then(|n| n.to_str()) == Some("par-mux-default.state.json");
    if default_instance {
        #[cfg(target_os = "macos")]
        if let Some(home) = dirs::home_dir() {
            return home.join("Library/Logs/par-mux/daemon.log");
        }
        if let Some(dir) = state_path.parent() {
            return dir.join("daemon.log");
        }
    }
    let mut name = state_path.as_os_str().to_os_string();
    name.push(".log");
    std::path::PathBuf::from(name)
}

/// The running server's per-instance shutdown flag (ARC-016), published for
/// the signal handler. `OnceLock::get` is async-signal-safe enough for this
/// use: it is only read after `main` set it, and the store it performs is
/// one atomic write.
#[cfg(unix)]
static SHUTDOWN_HANDLE: std::sync::OnceLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
    std::sync::OnceLock::new();

/// Minimal async-signal-safe handler: request shutdown and return.
#[cfg(unix)]
extern "C" fn on_sigterm(_signum: i32) {
    if let Some(flag) = SHUTDOWN_HANDLE.get() {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Install the daemon's signal dispositions. SIGTERM requests the clean
/// shutdown (the accept loop notices the shutdown flag on its own tick, so
/// no SA_RESTART subtleties are involved); the terminal-generated set is
/// ignored, because a daemon whose clients and panes outlive any one
/// terminal must survive that terminal's hangup and stray Ctrl-C/Ctrl-Z —
/// tmux installs the same ignore set on its server. SIGHUP: closing
/// terminal; SIGINT/SIGQUIT: Ctrl-C / Ctrl-\; SIGTSTP: Ctrl-Z; SIGPIPE: a
/// client socket closing mid-write.
#[cfg(unix)]
fn install_signal_handlers() -> std::io::Result<()> {
    use nix::sys::signal::{self, SaFlags, SigAction, SigHandler};
    let shutdown = SigAction::new(
        SigHandler::Handler(on_sigterm),
        SaFlags::empty(),
        signal::SigSet::empty(),
    );
    // SAFETY: `on_sigterm` is async-signal-safe: it only reads an
    // initialized `OnceLock` and performs one atomic store.
    unsafe { signal::sigaction(signal::SIGTERM, &shutdown) }.map_err(std::io::Error::other)?;
    let ignore = SigAction::new(
        SigHandler::SigIgn,
        SaFlags::empty(),
        signal::SigSet::empty(),
    );
    for terminal_signal in [
        signal::SIGHUP,
        signal::SIGINT,
        signal::SIGQUIT,
        signal::SIGPIPE,
        signal::SIGTSTP,
    ] {
        // SAFETY: SIG_IGN runs no handler code, so no signal-safety
        // requirement applies.
        unsafe { signal::sigaction(terminal_signal, &ignore) }.map_err(std::io::Error::other)?;
    }
    Ok(())
}
