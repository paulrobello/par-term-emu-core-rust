//! The par-mux daemon.
//!
//! Owns PTYs and serves the control-mode protocol over a local socket. Runs
//! until killed: clients come and go, panes do not (see par-mux.md D5).

use clap::Parser;

/// par-mux: a tmux-control-mode-compatible multiplexer daemon.
///
/// Binds one control socket and serves it until killed. `MuxClient::connect_or_spawn_at`
/// (par_term_emu_core_rust::mux::client) spawns this binary with `--socket <path>`; the
/// positional `NAME` form is the equivalent default-path shorthand for manual runs.
#[derive(Parser, Debug)]
#[command(
    name = "par-mux",
    version,
    about = "tmux-control-mode-compatible multiplexer daemon",
    long_about = None
)]
struct Cli {
    /// Named default socket path (par-mux::default_socket_path). Ignored if --socket is set.
    #[arg(default_value = "default")]
    name: String,

    /// Bind an explicit socket path instead of the named default.
    #[arg(long, value_name = "PATH")]
    socket: Option<std::path::PathBuf>,

    /// Override the platform state directory (D3.4) that persisted session
    /// trees are written under, instead of the OS-standard state/data dir.
    #[arg(long, value_name = "DIR")]
    state_dir: Option<std::path::PathBuf>,

    /// Stop the daemon serving this socket cleanly (final state save,
    /// `%exit` to clients) and wait for it to exit. Flags rather than
    /// subcommands: the positional NAME would otherwise be ambiguous with a
    /// session named `stop`.
    #[arg(long, conflicts_with = "restart")]
    stop: bool,

    /// Stop the running daemon (as --stop), then start a fresh one on the
    /// same socket — it restores the tree the stop just saved. Use after
    /// rebuilding par-mux: clients attach to whatever daemon owns the
    /// socket, so an old daemon keeps serving old code until restarted.
    #[arg(long)]
    restart: bool,
}

/// How long --stop/--restart wait for the old daemon to release its socket.
const STOP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Ask the daemon on `path` to shut down (`kill-server`), then wait until
/// the socket stops accepting. `Ok(false)` means nothing was running.
fn stop_daemon(path: &std::path::Path) -> std::io::Result<bool> {
    use std::io::{BufRead, BufReader, Write};
    let stream = match par_term_emu_core_rust::mux::connect_local_stream(path) {
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
    while par_term_emu_core_rust::mux::connect_local_stream(path).is_ok() {
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

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();

    // The mux library logs through `log` (ARC-010); without a logger
    // installed those records vanish, so wire the minimal stderr sink up
    // before anything can emit.
    log::set_logger(&STDERR_LOG)
        .map(|_| log::set_max_level(log::LevelFilter::Info))
        .ok();

    // `par-mux <name>` binds that named default path; `par-mux --socket <p>`
    // binds an explicit path (what MuxClient::connect_or_spawn_at spawns).
    let path = match cli.socket {
        Some(p) => p,
        None => par_term_emu_core_rust::mux::default_socket_path(&cli.name),
    };

    if cli.stop || cli.restart {
        if stop_daemon(&path)? {
            eprintln!("par-mux: stopped the daemon on {}", path.display());
        } else {
            eprintln!("par-mux: no daemon running on {}", path.display());
        }
        if cli.stop {
            return Ok(());
        }
        // --restart falls through and serves the socket in this process —
        // the state save the stop just completed is what it restores. Run
        // it detached (e.g. `par-mux --restart NAME &`) to keep a shell.
    }

    // D3.2/D3.3: a corrupt or unknown-version state file is quarantined
    // aside and the daemon starts fresh — unreadable state never blocks
    // startup. A readable state is REBUILT (D3.5): layout and content are
    // restored, and every pane's process is new — the original processes
    // died with the previous server, which is stated rather than papered
    // over.
    let state_path = match cli.state_dir {
        Some(dir) => par_term_emu_core_rust::mux::persist::state_file_in(&dir, &path),
        None => par_term_emu_core_rust::mux::persist::state_file_path(&path),
    };
    let restored = match par_term_emu_core_rust::mux::persist::load_or_quarantine(&state_path) {
        par_term_emu_core_rust::mux::persist::Loaded::Fresh => None,
        par_term_emu_core_rust::mux::persist::Loaded::State(state) => {
            match par_term_emu_core_rust::mux::tree::MuxTree::from_persist_state(
                &state,
                Box::new(par_term_emu_core_rust::mux::pane::ShellPaneFactory {
                    // Restored panes respawn through this factory too, so
                    // they get the same hook env contract as fresh ones.
                    socket_path: Some(path.to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            ) {
                Ok(tree) => Some(tree),
                Err(err) => {
                    log::warn!("par-mux: state restore failed ({err}); starting fresh");
                    None
                }
            }
        }
        par_term_emu_core_rust::mux::persist::Loaded::Quarantined { .. } => None,
    };

    // bind refuses a path a live server already owns, so a racing auto-spawn
    // loses cleanly instead of stealing the socket.
    let server = match restored {
        Some(tree) => par_term_emu_core_rust::mux::MuxServer::bind_with_tree(&path, tree)?,
        None => par_term_emu_core_rust::mux::MuxServer::bind(&path)?,
    };
    log::info!("par-mux listening on {}", path.display());

    // A clean SIGTERM saves on the way out (Task 3.5): the handler requests
    // shutdown with one atomic store (async-signal-safe), the accept loop
    // notices, and run_persisting's final save captures every completed
    // mutation. kill -9 skips all of this and simply loses the last window
    // (D3.3 covers why that is acceptable). The handle is per-instance
    // (ARC-016) and published to the handler via OnceLock.
    #[cfg(unix)]
    {
        SHUTDOWN_HANDLE.set(server.shutdown_handle()).ok();
        install_sigterm_handler()?;
    }

    server.run_persisting(state_path);
    Ok(())
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

/// Install the SIGTERM handler. The accept loop notices the shutdown flag on
/// its own tick, so no SA_RESTART subtleties are involved.
#[cfg(unix)]
fn install_sigterm_handler() -> std::io::Result<()> {
    use nix::sys::signal::{self, SaFlags, SigAction, SigHandler};
    let action = SigAction::new(
        SigHandler::Handler(on_sigterm),
        SaFlags::empty(),
        signal::SigSet::empty(),
    );
    unsafe { signal::sigaction(signal::SIGTERM, &action) }.map_err(std::io::Error::other)?;
    Ok(())
}

/// The daemon's stderr logger (ARC-010): the mux library emits through
/// `log`, and the daemon has no tracing subscriber, so this minimal sink is
/// what makes those records visible. Level-prefixed, info and above.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static STDERR_LOG: StderrLog = StderrLog;
