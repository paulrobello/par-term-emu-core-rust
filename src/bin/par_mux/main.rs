//! The par-mux daemon.
//!
//! Owns PTYs and serves the control-mode protocol over a local socket. Runs
//! until killed: clients come and go, panes do not (see par-mux.md D5).

// QA-201: every production `unsafe` block states its invariant.
#![cfg_attr(not(test), warn(clippy::undocumented_unsafe_blocks))]

use clap::Parser;

use par_term_emu_core_rust::mux::config::{
    self, load_canonical, resolve, write_file, EffectiveConfig, Overrides,
};

#[cfg(feature = "attach")]
use par_term_emu_core_rust::mux::attach::AttachMode;

/// The daemon's one config resolution: the file tier re-read per run, the
/// env tier read from this process's env, the flag tier from the parsed
/// CLI. `include_flags` splits the two consumers: `--gen-config` and the
/// actual bind want the flag tier (a flagged socket/state-dir must win);
/// the `reload-config` diff base wants file+env ONLY — startup flags are
/// one-shot overrides the file cannot express, so a flag-spelled socket
/// must not make every later reload report the socket changed. One
/// function so the three cannot drift.
fn effective(cli: &Cli, include_flags: bool) -> EffectiveConfig {
    let file = load_canonical();
    let flag_tier = if include_flags {
        Overrides {
            socket: cli
                .socket
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .or(cli.name.clone()),
            state_dir: cli
                .state_dir
                .as_ref()
                .map(|d| d.to_string_lossy().into_owned()),
            pane_endpoints: cli.pane_endpoints.then_some(true),
            expose_control_socket: cli.expose_control_socket.then_some(true),
            ..Overrides::default()
        }
    } else {
        Overrides::default()
    };
    resolve(
        &file,
        &Overrides {
            env_socket: std::env::var("PAR_MUX_SOCKET")
                .ok()
                .filter(|v| !v.is_empty()),
            ..flag_tier
        },
    )
}

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
    /// Named default socket path (par-mux::default_socket_path). Ignored if
    /// --socket is set; if absent, `$PAR_MUX_SOCKET` decides before the
    /// `default` name does.
    name: Option<String>,

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

    /// Client mode: send one control command to the daemon on this socket,
    /// print its reply body to stdout (one line per reply line), and exit.
    /// A `%error` reply prints to stderr and exits non-zero. Never starts a
    /// daemon. A flag, like --stop, so the positional NAME stays unambiguous.
    #[arg(
        short = 'c',
        long = "cmd",
        value_name = "COMMAND",
        conflicts_with_all = ["stop", "restart", "state_dir"]
    )]
    command: Option<String>,

    /// Give each pane its own hook-only socket endpoint: `PAR_MUX_SOCKET`
    /// in a pane then names a socket that accepts ONLY hook reports for
    /// that pane, so its child processes cannot drive other panes or stop
    /// the server. Default off — the in-pane full-socket fallback is a core
    /// feature; see docs/MUX.md for the trade and the default-flip gate.
    #[arg(long)]
    pane_endpoints: bool,

    /// With --pane-endpoints: also export `PAR_MUX_CONTROL_SOCKET` (the
    /// full control socket) in every pane, so agent-driven pane control
    /// keeps working. A session env of `PAR_MUX_CONTROL=1` grants the same
    /// per session without this flag.
    #[arg(long)]
    expose_control_socket: bool,

    /// Attach a client TUI to the daemon on the resolved socket (the
    /// `attach` cargo feature builds it in). Subcommand, not flag: its
    /// own positional/flag grammar. NOTE: a daemon literally named
    /// "attach" must use --socket — the subcommand name shadows the
    /// positional NAME form for that one name.
    #[cfg(feature = "attach")]
    #[command(subcommand)]
    attach: Option<AttachCommand>,

    /// Write the config file with the CURRENT EFFECTIVE settings (file if
    /// present > env > built-in defaults; startup CLI flags are not
    /// introspectable after parse, so a flagged value that also appears
    /// here wins only where this code path sees the flag — the socket and
    /// the daemon bools). Never overwrites without --force. With no config
    /// file yet, this is how a user starts one.
    #[arg(long)]
    gen_config: bool,

    /// With --gen-config: overwrite an existing config file.
    #[arg(long, requires = "gen_config")]
    force: bool,
}

/// `par-mux attach [-t TARGET] [--prefix KEY] [NAME | --socket PATH]` — the
/// attach client (feature `attach`). A one-variant enum because clap's
/// `Subcommand` derive only takes enums; the variant name is what spells the
/// subcommand.
#[cfg(feature = "attach")]
#[derive(clap::Subcommand, Debug)]
enum AttachCommand {
    /// Attach a client to a running par-mux daemon; never starts one. A
    /// daemon literally named "attach" must use --socket — the subcommand
    /// name shadows the positional NAME form for that one name.
    Attach(AttachArgs),
}

/// The `par-mux attach` argument set.
#[cfg(feature = "attach")]
#[derive(clap::Args, Debug)]
struct AttachArgs {
    /// Initial target (session/window/pane, tmux-style — resolves like a
    /// send-keys -t target); Phase A's view attaches to it.
    #[arg(short = 't', value_name = "TARGET")]
    target: Option<String>,

    /// Detach prefix key, tmux spelling (e.g. `C-b`); Phase A routes it.
    #[arg(long = "prefix", value_name = "KEY")]
    prefix: Option<String>,

    /// Render pipeline: `passthrough` (the default — pane bytes to the
    /// host terminal verbatim, Phase A) or `render` (the Phase B pane
    /// renderer with the input router: mode-aware key re-encode, mouse
    /// routing, wheel scrollback). Absent = the `[client] mode` from the
    /// config file, else passthrough.
    #[arg(long = "mode", value_name = "MODE")]
    mode: Option<String>,

    /// Named default socket path. A daemon literally named "attach" must
    /// use --socket instead — the subcommand name shadows this form.
    name: Option<String>,

    /// Connect to an explicit socket path instead of the named default.
    #[arg(long, value_name = "PATH")]
    socket: Option<std::path::PathBuf>,
}

#[cfg(feature = "attach")]
impl AttachCommand {
    fn options(&self) -> par_term_emu_core_rust::mux::attach::AttachOptions {
        match self {
            AttachCommand::Attach(args) => par_term_emu_core_rust::mux::attach::AttachOptions {
                socket: args.socket.clone(),
                name: args.name.clone(),
                target: args.target.clone(),
                prefix: args.prefix.clone(),
                reload: None,
                mode: args
                    .mode
                    .as_deref()
                    .map(parse_mode)
                    .unwrap_or(par_term_emu_core_rust::mux::attach::AttachMode::Passthrough),
            },
        }
    }
}

/// The `--mode` spelling: `render` selects the Phase B renderer; anything
/// else (including the default `passthrough`) stays Phase A. An unknown
/// value falls back to passthrough rather than failing the attach — the
/// mode is an enhancement, not a requirement.
#[cfg(feature = "attach")]
fn parse_mode(value: &str) -> par_term_emu_core_rust::mux::attach::AttachMode {
    if value.eq_ignore_ascii_case("render") {
        par_term_emu_core_rust::mux::attach::AttachMode::Render
    } else {
        par_term_emu_core_rust::mux::attach::AttachMode::Passthrough
    }
}

/// Run one control command against the daemon on `path` (client mode).
///
/// Returns the process exit code: 0 on `%end`, 1 on `%error` or when no
/// daemon owns the socket, 2 on a transport failure after connecting.
fn run_command(path: &std::path::Path, command: &str) -> std::process::ExitCode {
    use std::io::Write;
    use std::process::ExitCode;
    let mut client = match par_term_emu_core_rust::mux::MuxClient::connect(path) {
        Ok(client) => client,
        Err(err) => {
            eprintln!("par-mux: no daemon running on {} ({err})", path.display());
            return ExitCode::from(1);
        }
    };
    let reply = match client.send_checked(command) {
        Ok(reply) => reply,
        Err(err) => {
            // A pane endpoint (ENH-039) answers a control command with one
            // {"error":"hook-only endpoint"} JSON line and closes — no
            // %begin/%end block, so the client sees a closed connection.
            // Re-probe raw to map that to the actionable message.
            if pane_endpoint_refusal(path, command) {
                eprintln!(
                    "par-mux: this pane has hook-only access; start the daemon with \
                     --expose-control-socket or pass --socket"
                );
                return ExitCode::from(1);
            }
            // Only the command name: arguments may carry set-environment
            // values, which can be secrets.
            let name = command.split_whitespace().next().unwrap_or_default();
            eprintln!("par-mux: {name} failed: {err}");
            return ExitCode::from(2);
        }
    };
    if !reply.ok {
        eprintln!("par-mux: {}", reply.body.join("\n"));
        return ExitCode::from(1);
    }
    // A closed pipe (`par-mux -c ... | head -1`) ends the output quietly
    // instead of panicking the way println! would.
    let mut out = std::io::stdout().lock();
    for line in &reply.body {
        if writeln!(out, "{line}").is_err() {
            break;
        }
    }
    let _ = out.flush();
    ExitCode::SUCCESS
}

/// Whether `path` is a pane endpoint that just refused `command` (ENH-039):
/// reconnect raw, resend, and look for the endpoint's hook-only error —
/// the one-JSON-line reply a `MuxClient` cannot parse into a reply block.
fn pane_endpoint_refusal(path: &std::path::Path, command: &str) -> bool {
    use interprocess::TryClone as _;
    use std::io::{BufRead, BufReader, Write as _};
    let Ok(stream) = par_term_emu_core_rust::mux::connect_local_stream(path) else {
        return false;
    };
    let Ok(mut writer) = stream.try_clone() else {
        return false;
    };
    if writeln!(writer, "{command}")
        .and_then(|()| writer.flush())
        .is_err()
    {
        return false;
    }
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return false,
            Ok(_) => {}
        }
        if line.contains("hook-only endpoint") {
            return true;
        }
    }
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

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    // The mux library logs through `log` (ARC-010); without a logger
    // installed those records vanish, so wire the minimal stderr sink up
    // before anything can emit.
    log::set_logger(&STDERR_LOG)
        .map(|_| log::set_max_level(log::LevelFilter::Info))
        .ok();

    // par-mux attach: subcommand form — run the attach client and exit
    // with its code. The parsed --mode/--prefix ride above the config
    // file (flags > file); the mode/prefix/reload defaults come from the
    // resolved config; socket resolution lives in
    // AttachOptions::socket_path (same precedence as the daemon/--cmd).
    #[cfg(feature = "attach")]
    if let Some(attach) = cli.attach.as_ref() {
        let mut options = attach.options();
        let eff = effective(&cli, true);
        if options.prefix.is_none() {
            options.prefix = Some(eff.prefix.clone());
        }
        if options.reload.is_none() {
            options.reload = Some(eff.reload.clone());
        }
        if options.mode == AttachMode::Passthrough && eff.mode == "render" {
            // The config file's mode tier: only reachable when the user
            // did not pass --mode (the flag tier wins over the file).
            options.mode = AttachMode::Render;
        }
        return par_term_emu_core_rust::mux::attach::run_with_mode(&options, options.mode);
    }

    // --gen-config: write the effective config and exit. Resolution uses
    // the same function serve mode runs, so the generated file IS what
    // the daemon would resolve (modulo the socket flag tier, which this
    // path does see).
    if cli.gen_config {
        let eff = effective(&cli, true);
        let Some(path) = config::config_file_path() else {
            eprintln!("par-mux: no config directory known (set PAR_MUX_CONFIG or XDG_CONFIG_HOME)");
            return std::process::ExitCode::FAILURE;
        };
        if let Err(err) = write_file(&path, &eff, cli.force) {
            eprintln!("par-mux: {err}");
            return std::process::ExitCode::FAILURE;
        }
        println!("par-mux: wrote {}", path.display());
        return std::process::ExitCode::SUCCESS;
    }

    // `par-mux <name>` binds that named default path; `par-mux --socket <p>`
    // binds an explicit path (what MuxClient::connect_or_spawn_at spawns).
    // With neither given, `$PAR_MUX_SOCKET` — what every pane spawns with —
    // names the daemon to target before the `default` name is fallen back
    // to, so client flags typed inside a pane reach their own daemon, not
    // the unnamed default.
    // Target precedence: an explicit --socket, then the positional NAME,
    // then $PAR_MUX_CONTROL_SOCKET (the full socket, hidden from panes by
    // --pane-endpoints), then $PAR_MUX_SOCKET, then the unnamed default
    // (ENH-039). An empty value counts as unset.
    let path = if cli.socket.is_some() || cli.name.is_some() {
        par_term_emu_core_rust::mux::resolve_socket_path(
            cli.socket.as_deref(),
            cli.name.as_deref(),
            None,
        )
    } else {
        let socket_env = std::env::var_os("PAR_MUX_SOCKET").filter(|v| !v.is_empty());
        let control_env = std::env::var_os("PAR_MUX_CONTROL_SOCKET").filter(|v| !v.is_empty());
        par_term_emu_core_rust::mux::resolve_socket_path(
            None,
            None,
            control_env.or(socket_env).as_deref(),
        )
    };

    if let Some(command) = cli.command.as_deref() {
        return run_command(&path, command);
    }

    match run_daemon(cli, path) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// The daemon modes: --stop, --restart, and serving the socket.
fn run_daemon(cli: Cli, path: std::path::PathBuf) -> std::io::Result<()> {
    if cli.stop || cli.restart {
        if stop_daemon(&path)? {
            eprintln!("par-mux: stopped the daemon on {}", path.display());
        } else {
            eprintln!("par-mux: no daemon running on {}", path.display());
        }
        if cli.stop {
            return Ok(());
        }
        // --restart falls through and serves the socket from a detached
        // process — the state save the stop just completed is what it
        // restores, and the daemon must outlive the terminal --restart was
        // typed into (fork + setsid + stdio to /dev/null, tmux's
        // daemon(1,0) shape), so the invocation returns immediately and no
        // `&` is needed.
        #[cfg(unix)]
        daemonize()?;
    }

    // Nested-daemon guard, on the path that actually serves: a daemon
    // started from inside a pane (PAR_MUX_ENV=1) would shadow the outer
    // server's identity for every PTY under it. --cmd returned earlier and
    // --stop/--restart are exempt above, as tmux exempts kill-server; the
    // guard below therefore only bites plain serve mode.
    if !cli.restart {
        if let Some(reason) = par_term_emu_core_rust::mux::nested_daemon_refusal() {
            return Err(std::io::Error::other(reason));
        }
    }

    // D3.2/D3.3: a corrupt or unknown-version state file is quarantined
    // aside and the daemon starts fresh — unreadable state never blocks
    // startup. A readable state is REBUILT (D3.5): layout and content are
    // restored, and every pane's process is new — the original processes
    // died with the previous server, which is stated rather than papered
    // over. The config's `[daemon] state-dir` tier joins the flag tier
    // here: `effective()` already resolved flags > env-less > file, and
    // an empty value means the OS default.
    let eff = effective(&cli, true);
    let state_path = match eff.state_dir.as_str() {
        "" => par_term_emu_core_rust::mux::persist::state_file_path(&path),
        dir => {
            par_term_emu_core_rust::mux::persist::state_file_in(std::path::Path::new(dir), &path)
        }
    };
    // ENH-039: --pane-endpoints wires the pane-endpoint channel between the
    // factory (which binds each pane's hook-only socket) and the server
    // (which serves the connections the endpoints accept).
    let (pane_endpoint_tx, pane_endpoint_rx) = if eff.pane_endpoints {
        let (tx, rx) = par_term_emu_core_rust::mux::server::pane_endpoint_channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    // One factory serves both fresh and restored trees, so every pane gets
    // the same env contract. PAR_MUX_BIN is this executable: only the binary
    // knows it — in library code current_exe() names the embedding process.
    let factory = || par_term_emu_core_rust::mux::pane::ShellPaneFactory {
        socket_path: Some(path.to_string_lossy().into_owned()),
        bin_path: std::env::current_exe()
            .ok()
            .map(|exe| exe.to_string_lossy().into_owned()),
        pane_endpoint_tx: pane_endpoint_tx.clone(),
        expose_control_socket: eff.expose_control_socket,
        ..Default::default()
    };
    let restored = match par_term_emu_core_rust::mux::persist::load_or_quarantine(&state_path) {
        par_term_emu_core_rust::mux::persist::Loaded::Fresh => None,
        par_term_emu_core_rust::mux::persist::Loaded::State(state) => {
            match par_term_emu_core_rust::mux::tree::MuxTree::from_persist_state(
                &state,
                Box::new(factory()),
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
    let tree = restored
        .unwrap_or_else(|| par_term_emu_core_rust::mux::tree::MuxTree::new(Box::new(factory())));
    if eff.pane_endpoints {
        // Reclaim crash leftovers before this daemon binds anything, so the
        // endpoint cap counts only live endpoints (ENH-039).
        par_term_emu_core_rust::mux::server::sweep_pane_endpoint_remnants(&path);
    }
    let mut server = match pane_endpoint_rx {
        Some(rx) => par_term_emu_core_rust::mux::MuxServer::bind_with_tree_and_pane_endpoints(
            &path, tree, rx,
        )?,
        None => par_term_emu_core_rust::mux::MuxServer::bind_with_tree(&path, tree)?,
    };
    // Publish the applied settings: `reload-config` diffs its re-read
    // against this copy (restart-required vs unchanged, per setting).
    server.set_config(std::sync::Arc::new(parking_lot::Mutex::new(eff)));
    log::info!("par-mux listening on {}", path.display());

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

    server.run_persisting(state_path);
    Ok(())
}

/// Detach the serving `--restart` process from the terminal it was typed
/// into: fork, the parent reports success and exits, the child calls
/// `setsid` and moves stdio to `/dev/null` before serving (tmux's
/// `daemon(1,0)` shape). Unfixed, `par-mux --restart NAME &` stayed in the
/// shell's job, so closing that terminal SIGHUP-killed the daemon and every
/// pane with no save.
///
/// Must run while the process is still single-threaded (fork discipline);
/// everything before serve mode — argument parsing, the `--stop` half of
/// `--restart` — qualifies.
#[cfg(unix)]
fn daemonize() -> std::io::Result<()> {
    use nix::unistd::{fork, setsid, ForkResult};
    use std::io::Write as _;
    use std::os::fd::AsRawFd as _;

    // Flush the stop-phase report lines before the fork duplicates buffers.
    let _ = std::io::stderr().flush();
    // SAFETY: the caller runs this before serve mode spawns any thread (see
    // the doc comment), so the child inherits a single-threaded process and
    // may call non-async-signal-safe functions.
    match unsafe { fork() }.map_err(std::io::Error::from)? {
        ForkResult::Parent { .. } => std::process::exit(0),
        ForkResult::Child => {
            // The daemon now leads its own session, no controlling tty. The
            // old terminal may close at any moment; nothing the daemon
            // prints is load-bearing (the state file is the durable
            // record), so stdio lands on /dev/null as daemon() specifies.
            setsid().map_err(std::io::Error::from)?;
            let null = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/null")?;
            let fd = null.as_raw_fd();
            for target in [0, 1, 2] {
                // libc rather than nix::unistd: nix 0.31 no longer ships dup2.
                // SAFETY: `fd` is the open /dev/null handle `null` owns for
                // this whole loop, and 0-2 are the standard descriptors.
                if unsafe { libc::dup2(fd, target) } == -1 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }
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

#[cfg(all(test, feature = "attach"))]
mod attach_cli_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn attach_parses_name_form() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "work"]).expect("parse");
        let attach = cli.attach.expect("subcommand present");
        let AttachCommand::Attach(attach) = attach;
        assert_eq!(attach.name.as_deref(), Some("work"));
        assert!(attach.socket.is_none());
        assert!(attach.target.is_none());
        assert!(attach.prefix.is_none());
    }

    #[test]
    fn attach_target_flag_is_short_t_only() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "-t", "%0"]).expect("parse");
        let attach = cli.attach.expect("subcommand present");
        let AttachCommand::Attach(attach) = attach;
        assert_eq!(attach.target.as_deref(), Some("%0"));
        assert!(attach.prefix.is_none());
    }

    /// `--mode` defaults to passthrough; the `render` spelling selects the
    /// Phase B renderer and an unknown value falls back to passthrough.
    #[test]
    fn attach_mode_defaults_to_passthrough_and_parses_render() {
        use par_term_emu_core_rust::mux::attach::AttachMode;
        let cli = Cli::try_parse_from(["par-mux", "attach"]).expect("parse");
        let AttachCommand::Attach(attach) = cli.attach.expect("subcommand present");
        assert_eq!(
            parse_mode(attach.mode.as_deref().unwrap_or("")),
            AttachMode::Passthrough
        );

        let cli = Cli::try_parse_from(["par-mux", "attach", "--mode", "render"]).expect("parse");
        let AttachCommand::Attach(attach) = cli.attach.expect("subcommand present");
        assert_eq!(
            parse_mode(attach.mode.as_deref().unwrap_or("")),
            AttachMode::Render
        );

        // An unknown value is a passthrough attach, not a parse error.
        let cli = Cli::try_parse_from(["par-mux", "attach", "--mode", "wat"]).expect("parse");
        let AttachCommand::Attach(attach) = cli.attach.expect("subcommand present");
        assert_eq!(
            parse_mode(attach.mode.as_deref().unwrap_or("")),
            AttachMode::Passthrough
        );
    }

    #[test]
    fn attach_parses_socket_and_prefix() {
        let cli = Cli::try_parse_from([
            "par-mux",
            "attach",
            "--prefix",
            "C-b",
            "--socket",
            "/tmp/x.sock",
        ])
        .expect("parse");
        let attach = cli.attach.expect("subcommand present");
        let AttachCommand::Attach(attach) = attach;
        assert_eq!(attach.prefix.as_deref(), Some("C-b"));
        assert_eq!(attach.socket, Some(std::path::PathBuf::from("/tmp/x.sock")));
        assert!(attach.target.is_none());
    }

    #[test]
    fn attach_target_and_prefix_together() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "work", "-t", "$0", "--prefix", "C-b"])
            .expect("parse");
        let attach = cli.attach.expect("subcommand present");
        let AttachCommand::Attach(attach) = attach;
        assert_eq!(attach.name.as_deref(), Some("work"));
        assert_eq!(attach.target.as_deref(), Some("$0"));
        assert_eq!(attach.prefix.as_deref(), Some("C-b"));
    }

    #[test]
    fn attach_long_prefix_flag() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "--prefix", "C-a"]).expect("parse");
        let attach = cli.attach.expect("subcommand present");
        let AttachCommand::Attach(attach) = attach;
        assert_eq!(attach.prefix.as_deref(), Some("C-a"));
        assert!(attach.target.is_none());
    }

    #[test]
    fn attach_socket_wins_over_name() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "work", "--socket", "/tmp/y.sock"])
            .expect("parse");
        let attach = cli.attach.expect("subcommand present");
        let AttachCommand::Attach(attach) = attach;
        assert_eq!(attach.socket, Some(std::path::PathBuf::from("/tmp/y.sock")));
        assert_eq!(attach.name.as_deref(), Some("work"));
    }

    #[test]
    fn daemon_mode_still_parses_without_subcommand() {
        // The flag-based daemon form must keep parsing unchanged.
        let cli = Cli::try_parse_from(["par-mux", "--socket", "/tmp/d.sock"]).expect("parse");
        assert!(cli.attach.is_none());
        assert_eq!(cli.socket, Some(std::path::PathBuf::from("/tmp/d.sock")));
    }
}
