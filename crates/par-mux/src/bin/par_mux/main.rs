//! The par-mux daemon.
//!
//! Owns PTYs and serves the control-mode protocol over a local socket. Runs
//! until killed: clients come and go, panes do not (see par-mux.md D5).

// QA-201: every production `unsafe` block states its invariant.
#![cfg_attr(not(test), warn(clippy::undocumented_unsafe_blocks))]

use clap::Parser;

use par_mux::mux::config::{self, load_canonical, resolve, write_file, EffectiveConfig, Overrides};

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
/// (par_mux::mux::client) spawns this binary with `--socket <path>`; the
/// positional `NAME` form is the equivalent default-path shorthand for manual runs.

#[derive(Parser, Debug)]
#[command(
    name = "par-mux",
    version,
    about = "tmux-control-mode-compatible multiplexer daemon",
    long_about = None
)]
// The mode flags are mutually exclusive: at most one per invocation, none
// meaning "start/serve the daemon".
#[command(group(
    clap::ArgGroup::new("daemon_mode")
        .multiple(false)
        .args([
            "check_update",
            "self_update",
            "stop",
            "restart",
            "command",
            "list_servers",
            "gen_config",
            "install_skill",
            "install_agent_hooks",
            "uninstall_agent_hooks",
            "agent_hooks_status",
            "integration_status",
            "install_agent_extensions",
            "uninstall_agent_extensions",
            "heal_agent_hooks",
            "print_schema",
            "print_completions",
        ])
))]
struct Cli {
    /// Report the latest release without changing the installation.
    #[arg(long, conflicts_with = "self_update")]
    check_update: bool,
    /// Install a checksum-verified release and restart the same daemon.
    #[arg(long)]
    self_update: bool,
    /// Permit replacing a binary inside an .app bundle (breaks signature/TCC).
    #[arg(long, requires = "self_update")]
    force_self_update: bool,
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

    /// Relay this invocation to a saved machine over SSH (the machine's
    /// profile in `[machines.<label>]` names the ssh target; credentials
    /// stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`,
    /// `observe`, `control`, `--stop`) run non-interactively; `attach`
    /// forces passthrough under a pty — the thin client.
    #[arg(long, value_name = "LABEL", global = true)]
    machine: Option<String>,

    /// Stop the daemon serving this socket cleanly (final state save,
    /// `%exit` to clients) and wait for it to exit. Flags rather than
    /// subcommands: the positional NAME would otherwise be ambiguous with a
    /// session named `stop`.
    #[arg(long)]
    stop: bool,

    /// Stop the running daemon (as --stop), then start a fresh one on the
    /// same socket — it restores the tree the stop just saved. Use after
    /// rebuilding par-mux: clients attach to whatever daemon owns the
    /// socket, so an old daemon keeps serving old code until restarted.
    #[arg(long)]
    restart: bool,

    /// Serve the daemon in the foreground of this process instead of the
    /// default daemonize-to-background: the debugging and supervision
    /// escape, and the shape programmatic spawners use (they own and kill
    /// the child they spawned). Not a daemon_mode member on purpose: it
    /// composes with --restart (stop, then serve here).
    #[arg(long, conflicts_with_all = [
        "no_attach", "stop", "command", "list_servers", "gen_config",
        "install_skill", "install_agent_hooks", "uninstall_agent_hooks",
        "agent_hooks_status", "integration_status",
        "install_agent_extensions", "uninstall_agent_extensions",
        "heal_agent_hooks", "print_schema", "print_completions",
    ])]
    foreground: bool,

    /// The app-wrapper hand-off's loop guard: the start already exec'd
    /// through `/Applications/Par-mux.app`'s launcher once, so serving
    /// (detached, after the launcher's own exec) must not hand off again.
    /// Hidden: the serve path sets it itself when re-invoking through the
    /// wrapper; the launcher forwards it verbatim.
    #[arg(long, hide = true)]
    par_mux_in_app_wrapper: bool,

    /// Confirm a `--stop` typed inside a par-mux pane that targets the
    /// hosting daemon: the stop would kill this shell and every sibling
    /// pane, so the unconfirmed form refuses with an explanation. Also
    /// accepted: `--force`.
    #[arg(long)]
    yes: bool,

    /// Daemon-only: daemonize to background as usual but skip the
    /// auto-attach of the invoking terminal (and the isatty fallback that
    /// skips attaching for scripted starts).
    #[arg(long, conflicts_with_all = [
        "foreground", "stop", "command", "list_servers", "gen_config",
        "install_skill", "install_agent_hooks", "uninstall_agent_hooks",
        "agent_hooks_status", "integration_status",
        "install_agent_extensions", "uninstall_agent_extensions",
        "heal_agent_hooks", "print_schema", "print_completions",
    ])]
    no_attach: bool,

    /// Client mode: send one control command to the daemon on this socket,
    /// print its reply body to stdout (one line per reply line), and exit.
    /// A `%error` reply prints to stderr and exits non-zero. Never starts a
    /// daemon. A flag, like --stop, so the positional NAME stays unambiguous.
    #[arg(
        short = 'c',
        long = "cmd",
        value_name = "COMMAND",
        conflicts_with = "state_dir"
    )]
    command: Option<String>,

    /// Client mode: list every live par-mux server this user can reach —
    /// registry entries plus the named-default sockets on disk — one line
    /// per server (`<name-or-path>  sessions=N  stamp`), pruning registry
    /// entries whose probe failed. Never starts a daemon. Honors
    /// `--state-dir`, so tests can point the registry at a fixture; with
    /// `--socket`/NAME, the named/default resolution is skipped for the
    /// target socket (discovery is user-wide, not per-socket).
    #[arg(long)]
    list_servers: bool,

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

    /// The subcommand surface (R1): the verb forms — `par-mux start`,
    /// `stop`, `restart`, `status`, `ls`, `attach`, `config`. Each keeps
    /// the long-standing flag spelling working (par-term's invocations
    /// and muscle memory both stay valid); the subcommands are the
    /// discoverable front door. A subcommand shadows the same-named
    /// NAME-form daemon start (that path needs --socket).
    #[command(subcommand)]
    sub: Option<Sub>,

    /// Write the config file with the CURRENT EFFECTIVE settings (file if
    /// present > env > built-in defaults; startup CLI flags are not
    /// introspectable after parse, so a flagged value that also appears
    /// here wins only where this code path sees the flag — the socket and
    /// the daemon bools). Never overwrites without --force. With no config
    /// file yet, this is how a user starts one.
    #[arg(long)]
    gen_config: bool,

    /// With --gen-config: overwrite an existing config file. With --stop
    /// inside a par-mux pane: confirm the stop (an alias of `--yes`).
    /// Without either it is rejected at dispatch.
    #[arg(long)]
    force: bool,

    /// Install the bundled par-mux agent skill into the Claude Code
    /// skills directory and exit. Never starts or contacts a daemon, so
    /// it runs anywhere, including inside a pane; par-term invokes it
    /// after installing par-mux so the skill matches the shipped binary.
    #[arg(long)]
    install_skill: bool,

    /// With --install-skill: write under this skills root instead of the
    /// Claude Code default (tests, other agent harnesses).
    #[arg(long, value_name = "DIR", requires = "install_skill")]
    skills_dir: Option<std::path::PathBuf>,
    /// Install the agent-state session hooks for claude, codex, grok, muse,
    /// droid, copilot, and cursor (script assets plus the config entries
    /// that run them) and exit.
    /// Never starts or contacts a daemon; par-term invokes it instead of
    /// embedding its own copies of the integrations.
    #[arg(long)]
    install_agent_hooks: bool,

    /// Remove the agent-state session hooks par-mux installed: the config
    /// entries first, then the script assets. Never contacts a daemon.
    #[arg(long)]
    uninstall_agent_hooks: bool,

    /// Print one line per agent — `<agent> installed` or `<agent> absent` —
    /// and exit: the read-only state behind the par-term Settings rows.
    #[arg(long)]
    agent_hooks_status: bool,

    /// Print one line per integration — the agent hooks, the pi/omp
    /// extensions, and the par-mux skill — naming each one's version and
    /// whether the installed asset matches the bundled copy
    /// (`v2`, `v1 → v2 outdated`, `asset missing`, or `absent`), and exit.
    #[arg(long)]
    integration_status: bool,

    /// Install the pi and omp agent-state extensions (a file drop into
    /// each agent extension directory) and exit. Never contacts a daemon.
    #[arg(long)]
    install_agent_extensions: bool,

    /// Remove the pi and omp agent-state extensions par-mux installed.
    #[arg(long)]
    uninstall_agent_extensions: bool,
    /// Rewrite a registered hook script asset that went missing (the
    /// hooks directory has been deleted externally while the agent config
    /// entries survived) without touching any agent config — the startup
    /// self-heal par-term runs. Never contacts a daemon.
    #[arg(long)]
    heal_agent_hooks: bool,

    /// Print the versioned JSON command schema (every control command,
    /// its flags, and its feature tokens) bundled with this binary, and
    /// exit. Never contacts a daemon; `list-commands -J` serves the same
    /// document from a running one.
    #[arg(long)]
    print_schema: bool,

    /// Print the shell-completion script for SHELL (bash, zsh, fish,
    /// powershell, elvish) generated from this CLI, and exit. Never
    /// contacts a daemon.
    // Not `hide = true`: clap_complete 4.6 still emits hidden args, and the
    // flag's own completion (the shell names) is worth offering.
    #[arg(long, value_name = "SHELL")]
    print_completions: Option<clap_complete::Shell>,
}

/// The completion script for `shell`, generated from the live clap surface
/// so it cannot drift from what `Cli` parses.
fn completion_script(shell: clap_complete::Shell) -> String {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let mut out = Vec::new();
    clap_complete::generate(shell, &mut cmd, "par-mux", &mut out);
    String::from_utf8(out).expect("clap_complete emits UTF-8")
}

/// `par-mux attach [-t TARGET] [--prefix KEY] [NAME | --socket PATH]` — the
/// attach client (feature `attach`). A one-variant enum because clap's
/// `Subcommand` derive only takes enums; the variant name is what spells the
/// subcommand.
#[cfg(feature = "attach")]
#[derive(clap::Subcommand, Debug)]
enum AttachCommand {
    /// Attach a client to a running par-mux daemon; never starts one.
    /// Attaching by NAME resolves normally, even for a daemon literally
    /// named "attach"; only STARTING one of that name needs --socket,
    /// where the subcommand shadows the NAME form.
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

    /// Render pipeline: `render` (the default — the Phase B pane renderer
    /// with the input router: mode-aware key re-encode, mouse routing,
    /// wheel scrollback) or `passthrough` (pane bytes to the host terminal
    /// verbatim, Phase A). Absent = the `[client] mode` from the config
    /// file, else render.
    #[arg(long = "mode", value_name = "MODE")]
    mode: Option<String>,

    /// Named default socket path. Only STARTING a daemon named "attach"
    /// needs --socket (the subcommand shadows the NAME form there);
    /// attaching by this NAME form works, including for "attach".
    name: Option<String>,

    /// Connect to an explicit socket path instead of the named default.
    #[arg(long, value_name = "PATH")]
    socket: Option<std::path::PathBuf>,
}

#[cfg(feature = "attach")]
impl AttachCommand {
    fn options(&self) -> Result<par_mux::mux::attach::AttachOptions, String> {
        match self {
            AttachCommand::Attach(args) => {
                let mode = args.mode.as_deref().map(parse_mode).transpose()?;
                Ok(par_mux::mux::attach::AttachOptions {
                    socket: args.socket.clone(),
                    name: args.name.clone(),
                    target: args.target.clone(),
                    prefix: args.prefix.clone(),
                    reload: None,
                    mode,
                })
            }
        }
    }

    /// The `--socket` path as typed, when the user gave one — the
    /// multi-server ambiguity guard must not override an explicit target.
    fn explicit_socket(&self) -> Option<&std::path::Path> {
        match self {
            AttachCommand::Attach(args) => args.socket.as_deref(),
        }
    }

    /// The positional NAME as typed, when the user gave one — same guard.
    fn explicit_name(&self) -> Option<&str> {
        match self {
            AttachCommand::Attach(args) => args.name.as_deref(),
        }
    }
}

/// The `--mode` spelling: `render` (the default) selects the Phase B
/// renderer, `passthrough` the Phase A byte pump. Anything else is a
/// spelling error and fails the attach loudly — a silent fallback ran the
/// user's session in the wrong mode (the manual-pass `--mode rendered`
/// report).
#[cfg(feature = "attach")]
fn parse_mode(value: &str) -> Result<par_mux::mux::attach::AttachMode, String> {
    if value.eq_ignore_ascii_case("render") {
        Ok(par_mux::mux::attach::AttachMode::Render)
    } else if value.eq_ignore_ascii_case("passthrough") {
        Ok(par_mux::mux::attach::AttachMode::Passthrough)
    } else {
        Err(format!(
            "unknown --mode {value:?} — valid modes: render, passthrough"
        ))
    }
}

/// The verb surface (R1): one enum, one variant per subcommand. `Attach`
/// reuses [`AttachArgs`] verbatim; the daemon verbs carry the same
/// targeting trio (`[NAME]`, `--socket`, `--state-dir`) the flag forms
/// read.
#[derive(clap::Subcommand, Debug)]
enum Sub {
    /// Start the daemon (the default action): detached, auto-attaching
    /// this terminal unless --no-attach. Same as bare `par-mux [NAME]`.
    Start(StartArgs),
    /// Stop the daemon serving the resolved socket cleanly. Same as
    /// `--stop`.
    Stop(TargetArgs),
    /// Stop, then start a fresh daemon that restores the saved tree.
    /// Same as `--restart`.
    Restart(TargetArgs),
    /// One-shot orientation: the daemon's build stamp, its workspaces
    /// and sessions, the displayed session, and the pane count — the
    /// orient step in one invocation (R2).
    Status(TargetArgs),
    /// List the resolved daemon's sessions (`list-sessions`, tmux's
    /// `ls`).
    Ls(TargetArgs),
    /// Attach a client to a running daemon; never starts one. The
    /// feature-gated interactive client.
    #[cfg(feature = "attach")]
    Attach(AttachArgs),
    /// Write the config file with the current effective settings. Same
    /// as `--gen-config` (add `--force` to overwrite).
    Config(ConfigArgs),
    /// Stream one pane's raw output to stdout, read-only (`observe-pane`):
    /// the screen seed, then live PTY bytes until the pane closes. Any
    /// number of observers may watch the same pane.
    Observe(StreamArgs),
    /// Drive one pane (`control-pane`): stream its output to stdout like
    /// `observe`, forward stdin bytes to it until EOF (no raw mode — for
    /// pipes and scripts; interactive use is `attach -t %N`), and report
    /// this terminal's size (polled) or `--size` as a viewer of the
    /// pane's window.
    Control(StreamArgs),
    /// Print the shell-completion script for SHELL, herdr's spelling of
    /// `--print-completions`. Never contacts a daemon.
    Completion(CompletionArgs),
    /// Manage saved machines (`[machines.<label>]` in the config): SSH
    /// thin-client profiles for `--machine`. Never contacts a daemon.
    Machine(MachineArgs),
    /// Manage plugins: manifests under the state dir's `plugins/` whose
    /// hooks run on daemon events. Install previews every command; nothing
    /// runs until `--trust`. Never contacts a daemon.
    Plugin(PluginArgs),
    /// Check for a newer release and install it (self-update): resolve the
    /// channel, download the platform archive, verify the binary, replace
    /// this one, restart the daemon (its sessions ride the stop-and-restore
    /// handoff).
    Update(UpdateArgs),
}

/// `par-mux update` — the self-update arguments.
#[derive(clap::Args, Debug)]
struct UpdateArgs {
    /// The release channel: `stable` (latest release, the default) or
    /// `preview` (newest prerelease).
    #[arg(long)]
    channel: Option<par_mux::mux::update::Channel>,
    /// Report what would install, touch nothing.
    #[arg(long)]
    check: bool,
}

/// `par-mux plugin <VERB>` — the plugin lifecycle.
#[derive(clap::Subcommand, Debug)]
enum PluginCommand {
    /// Resolve a plugin source (a directory or git URL) and PREVIEW it:
    /// the source, its identity, and every command an event would run.
    /// Installs only with `--trust`.
    Install {
        /// A plugin directory or a git URL (depth-1 clone; the manifest at
        /// the root or one level down).
        source: String,
        /// Trust the previewed commands and install.
        #[arg(long)]
        trust: bool,
    },
    /// One line per installed plugin: name, version, hook count, source.
    List,
    /// Remove a plugin (its hooks stop after the daemon reloads).
    Remove { name: String },
    /// Re-fetch a plugin from its recorded source (the preview and
    /// `--trust` apply again).
    Update {
        name: String,
        #[arg(long)]
        trust: bool,
    },
}

/// `par-mux plugin <VERB> [...]` — the verb's arguments.
#[derive(clap::Args, Debug)]
struct PluginArgs {
    #[command(subcommand)]
    command: PluginCommand,
}

/// `par-mux machine <VERB>` — the saved-machine manager.
#[derive(clap::Subcommand, Debug)]
enum MachineCommand {
    /// Save `[machines.<LABEL>]` (or replace it): the ssh target, an
    /// optional default remote session, enabled by default. Nothing is
    /// contacted — the first attach relays whenever ssh is ready.
    Add {
        /// The ssh target as openssh spells it (`user@host`, a config
        /// alias, `host:port`).
        target: String,
        /// The machine's label (defaults to the target's host part).
        #[arg(long)]
        label: Option<String>,
        /// The remote session `--machine <LABEL> attach` lands on.
        #[arg(long)]
        session: Option<String>,
    },
    /// One line per saved machine: `label  target  [session]  enabled|disabled`.
    List,
    /// Remove a machine's profile (remote sessions keep running).
    Remove { label: String },
    /// Rename a machine's label.
    Rename {
        label: String,
        #[arg(long)]
        to: String,
    },
    /// Mark a machine enabled.
    Enable { label: String },
    /// Mark a machine disabled (listed, never auto-attached).
    Disable { label: String },
}

/// `par-mux machine <VERB> [...]` — the verb's arguments.
#[derive(clap::Args, Debug)]
struct MachineArgs {
    #[command(subcommand)]
    command: MachineCommand,
}

/// `par-mux completion <SHELL>` — the completion verb.
#[derive(clap::Args, Debug)]
struct CompletionArgs {
    /// The shell to print for (bash, zsh, fish, powershell, elvish).
    shell: clap_complete::Shell,
}

/// `par-mux observe|control -t PANE` — the raw pane stream verbs.
#[derive(clap::Args, Debug)]
struct StreamArgs {
    /// The pane (`%N` or a pane title).
    #[arg(short = 't', value_name = "PANE")]
    target: String,
    /// Skip the screen seed: stdout carries only output that follows.
    #[arg(long)]
    no_seed: bool,
    /// `control` only: report this fixed size (`WxH`) instead of following
    /// the terminal's.
    #[arg(long, value_name = "WxH")]
    size: Option<String>,
    #[command(flatten)]
    at: TargetArgs,
}

/// `par-mux start` — the daemon-start flags, subcommand spelling.
#[derive(clap::Args, Debug)]
struct StartArgs {
    /// Named default socket path.
    name: Option<String>,
    /// Bind an explicit socket path instead of the named default.
    #[arg(long, value_name = "PATH")]
    socket: Option<std::path::PathBuf>,
    /// Override the platform state directory.
    #[arg(long, value_name = "DIR")]
    state_dir: Option<std::path::PathBuf>,
    /// Serve in this process instead of detaching.
    #[arg(long)]
    foreground: bool,
    /// Skip the auto-attach of the invoking terminal.
    #[arg(long)]
    no_attach: bool,
    /// Give each pane its own hook-only socket endpoint.
    #[arg(long)]
    pane_endpoints: bool,
    /// With --pane-endpoints: also export the full control socket.
    #[arg(long)]
    expose_control_socket: bool,
}

/// The targeting trio every client-side verb reads.
#[derive(clap::Args, Debug)]
struct TargetArgs {
    /// Named default socket path.
    name: Option<String>,
    /// Connect to an explicit socket path instead of the named default.
    #[arg(long, value_name = "PATH")]
    socket: Option<std::path::PathBuf>,
    /// Override the platform state directory (the registry base for
    /// discovery-flavored reads).
    #[arg(long, value_name = "DIR")]
    state_dir: Option<std::path::PathBuf>,
}

/// `par-mux config` — the gen-config spelling.
#[derive(clap::Args, Debug)]
struct ConfigArgs {
    /// Overwrite an existing config file.
    #[arg(long)]
    force: bool,
}

/// `par-mux observe|control`: resolve the socket like every client verb,
/// then hand the stream to the library runner.
fn run_pane_stream(cli: &mut Cli, args: StreamArgs, control: bool) -> std::process::ExitCode {
    let StreamArgs {
        target,
        no_seed,
        size,
        at,
    } = args;
    cli.name = at.name.or(cli.name.take());
    cli.socket = at.socket.or(cli.socket.take());
    cli.state_dir = at.state_dir.or(cli.state_dir.take());
    if refuse_reserved_server_name(cli.name.as_deref(), cli.socket.as_ref()) {
        return std::process::ExitCode::from(2);
    }
    let size = match size.as_deref().map(parse_size).transpose() {
        Ok(size) => size,
        Err(message) => {
            eprintln!("par-mux: {message}");
            return std::process::ExitCode::from(2);
        }
    };
    let path = resolve_cli_socket(cli);
    let options = par_mux::mux::pane_stream::StreamOptions {
        control,
        size,
        no_seed,
    };
    let code = par_mux::mux::pane_stream::run(&path, &target, &options);
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(2))
}

/// `--size WxH`, both halves positive.
fn parse_size(raw: &str) -> Result<(u16, u16), String> {
    raw.split_once('x')
        .and_then(|(c, r)| Some((c.parse::<u16>().ok()?, r.parse::<u16>().ok()?)))
        .filter(|(c, r)| *c > 0 && *r > 0)
        .ok_or_else(|| format!("--size expects WxH, got {raw:?}"))
}

/// Run one control command against the daemon on `path` (client mode).
///
/// Returns the process exit code: 0 on `%end`, 1 on `%error` or when no
/// daemon owns the socket, 2 on a transport failure after connecting.
fn run_command(path: &std::path::Path, command: &str) -> std::process::ExitCode {
    use std::io::Write;
    use std::process::ExitCode;
    let mut client = match par_mux::mux::MuxClient::connect(path) {
        Ok(client) => client,
        Err(err) => {
            eprintln!(
                "par-mux: no daemon running on {} ({err}) — start one with `par-mux` (or \
                 `par-mux --socket {}` for this socket)",
                path.display(),
                path.display()
            );
            return ExitCode::from(1);
        }
    };
    client.discard_notifications();
    let sent = match par_mux::mux::command::reply_timeout(command) {
        Some(budget) => client.send_checked_timeout(command, budget),
        None => client.send_checked(command),
    };
    let reply = match sent {
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

/// `par-mux --list-servers`: enumerate and probe every known server under
/// the resolved state base, print one line per LIVE server, and prune the
/// dead ones SILENTLY — the enumeration owns cleanup, and the per-entry
/// "pruned a dead entry" notes were noise in every caller's output (Paul,
/// 2026-10-17). Registry entries whose probe failed are pruned as a side
/// effect. A `--socket`/NAME argument does
/// not narrow the list — discovery is user-wide, the flag pair is accepted
/// only so `par-mux --list-servers` reads uniformly with the other
/// client-mode flags.
fn run_list_servers(cli: &Cli) -> std::process::ExitCode {
    use std::io::Write as _;
    let base = discovery_base(cli);
    let found = par_mux::mux::discovery::enumerate(&base);
    let mut out = std::io::stdout().lock();
    let default_socket = par_mux::mux::resolve_socket_path(None, None, None);
    for server in &found.live {
        let row = par_mux::mux::discovery::describe_server(server, Some(&default_socket));
        if writeln!(out, "{row}").is_err() {
            break;
        }
    }
    let _ = out.flush();
    if found.live.is_empty() {
        eprintln!("par-mux: no running par-mux servers");
    }
    std::process::ExitCode::SUCCESS
}

/// The registry base this invocation reads and writes: `--state-dir`'s
/// value when given (tests), else the platform state base every
/// default-`--state-dir` daemon registers under.
fn discovery_base(cli: &Cli) -> std::path::PathBuf {
    match cli.state_dir.as_deref() {
        Some(dir) => dir.to_path_buf(),
        None => par_mux::mux::discovery::default_registry_base(),
    }
}

/// Whether `path` is a pane endpoint that just refused `command` (ENH-039):
/// reconnect raw, resend, and look for the endpoint's hook-only error —
/// the one-JSON-line reply a `MuxClient` cannot parse into a reply block.
fn pane_endpoint_refusal(path: &std::path::Path, command: &str) -> bool {
    use interprocess::TryClone as _;
    use std::io::{BufRead, BufReader, Write as _};
    let Ok(stream) = par_mux::mux::connect_local_stream(path) else {
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

/// One integration arm: its name and the action that yields the stdout detail.
type Arm = fn() -> std::io::Result<String>;

/// Which one-shot agent-hook action the flag runs.
enum AgentHookAction {
    Install,
    Uninstall,
}

/// The bulk flag's arm table: the four bespoke installers, then every
/// [`par_mux::mux::integrations::CONFIG_HOOK_ARMS`] entry. `Install`
/// arms carry the stdout detail, `Uninstall` arms the one-word outcome.
fn agent_hook_arms(action: &AgentHookAction) -> [(&'static str, Arm); 12] {
    use par_mux::mux::integrations as integ;
    match action {
        AgentHookAction::Install => [
            ("claude", || {
                let r = integ::install_claude_hook()?;
                Ok(format!(
                    "{} (settings {})",
                    r.hook_path.display(),
                    if r.settings_changed {
                        "updated"
                    } else {
                        "already present"
                    }
                ))
            }),
            ("codex", || {
                let r = integ::install_codex_hook()?;
                Ok(format!(
                    "{} + {}",
                    r.hook_path.display(),
                    r.hooks_path.display()
                ))
            }),
            ("grok", || {
                let r = integ::install_grok_hook()?;
                Ok(format!(
                    "{} + {}",
                    r.hook_path.display(),
                    r.config_path.display()
                ))
            }),
            ("muse", || {
                let r = integ::install_muse_hook()?;
                Ok(format!(
                    "{} (settings {})",
                    r.hook_path.display(),
                    if r.settings_changed {
                        "updated"
                    } else {
                        "already present"
                    }
                ))
            }),
            ("droid", || config_hook_install(&integ::DROID)),
            ("copilot", || config_hook_install(&integ::COPILOT)),
            ("cursor", || config_hook_install(&integ::CURSOR)),
            ("qwen", || config_hook_install(&integ::QWEN)),
            ("qodercli", || config_hook_install(&integ::QODERCLI)),
            ("devin", || config_hook_install(&integ::DEVIN)),
            ("kimi", || config_hook_install(&integ::KIMI)),
            ("mastracode", || config_hook_install(&integ::MASTRACODE)),
        ],
        AgentHookAction::Uninstall => [
            ("claude", || {
                let r = integ::uninstall_claude_hook()?;
                Ok(arm_summary(r.settings_changed || r.hook_removed))
            }),
            ("codex", || {
                let r = integ::uninstall_codex_hook()?;
                Ok(arm_summary(r.hooks_changed || r.hook_removed))
            }),
            ("grok", || {
                let r = integ::uninstall_grok_hook()?;
                Ok(arm_summary(r.hook_removed || r.config_removed))
            }),
            ("muse", || {
                let r = integ::uninstall_muse_hook()?;
                Ok(arm_summary(r.settings_changed || r.hook_removed))
            }),
            ("droid", || config_hook_uninstall(&integ::DROID)),
            ("copilot", || config_hook_uninstall(&integ::COPILOT)),
            ("cursor", || config_hook_uninstall(&integ::CURSOR)),
            ("qwen", || config_hook_uninstall(&integ::QWEN)),
            ("qodercli", || config_hook_uninstall(&integ::QODERCLI)),
            ("devin", || config_hook_uninstall(&integ::DEVIN)),
            ("kimi", || config_hook_uninstall(&integ::KIMI)),
            ("mastracode", || config_hook_uninstall(&integ::MASTRACODE)),
        ],
    }
}

/// Install or uninstall every agent session hook, one stdout line per
/// arm, exiting non-zero when any arm failed. Each arm runs even when an
/// earlier one fails — a partial install is visible in full, not hidden
/// by the first error.
fn run_agent_hook_action(action: AgentHookAction) -> std::process::ExitCode {
    run_arms(&agent_hook_arms(&action))
}

/// The install line detail for a droid/copilot/cursor arm. An agent whose
/// config directory is absent is skipped, not failed: these arms joined a
/// bulk flag whose exit status callers (par-term) treat as all-or-nothing.
fn config_hook_install(arm: &par_mux::mux::integrations::ConfigHookArm) -> std::io::Result<String> {
    if !arm.agent_present() {
        return Ok(format!(
            "skipped ({} not installed: no config directory)",
            arm.agent
        ));
    }
    let r = arm.install()?;
    Ok(format!(
        "{} (config {} {})",
        r.hook_path.display(),
        r.config_path.display(),
        if r.config_changed {
            "updated"
        } else {
            "already present"
        }
    ))
}

/// The uninstall line detail for a droid/copilot/cursor arm.
fn config_hook_uninstall(
    arm: &par_mux::mux::integrations::ConfigHookArm,
) -> std::io::Result<String> {
    let r = arm.uninstall()?;
    Ok(arm_summary(r.config_changed || r.hook_removed))
}

/// One arm's one-word outcome for the uninstall lines.
fn arm_summary(removed_any: bool) -> String {
    if removed_any {
        "removed".to_string()
    } else {
        "not installed".to_string()
    }
}

/// Print one `par-mux: <arm> …` line per (name, action) pair, stdout for
/// successes and stderr for failures; FAILURE when any arm failed.
fn run_arms(arms: &[(&str, Arm)]) -> std::process::ExitCode {
    let mut failed = false;
    for (name, action) in arms {
        match action() {
            Ok(detail) => println!("par-mux: {name} hook: {detail}"),
            Err(err) => {
                failed = true;
                eprintln!("par-mux: {name} hook failed: {err}")
            }
        }
    }
    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

/// One status line's value half: the `installed|absent` token the
/// par-term Settings rows parse, then the asset's version currency.
fn integration_status_line(
    installed: bool,
    asset: &par_mux::mux::integrations::AssetState,
) -> String {
    let base = if installed { "installed" } else { "absent" };
    match asset.currency {
        par_mux::mux::integrations::AssetCurrency::Absent if !installed => base.to_string(),
        _ => format!("{base} {}", asset.display(installed)),
    }
}

/// `--agent-hooks-status`: one line per agent hook — `<agent> installed`
/// or `<agent> absent`, then the asset's version currency when there is
/// an asset to grade (`v2`, `v1 → v2 outdated`, `asset missing`).
fn run_agent_hooks_status() -> std::process::ExitCode {
    for status in par_mux::mux::integrations::hook_status() {
        println!(
            "{} {}",
            status.agent,
            integration_status_line(status.installed, &status.asset)
        );
    }
    std::process::ExitCode::SUCCESS
}

/// `--integration-status`: the same grammar across EVERY bundled
/// integration asset — the agent hooks, the pi/omp extensions, and the
/// par-mux skill — so a script (or a human) can see not-installed and
/// out-of-date in one place.
fn run_integration_status() -> std::process::ExitCode {
    for status in par_mux::mux::integrations::hook_status() {
        println!(
            "hook {} {}",
            status.agent,
            integration_status_line(status.installed, &status.asset)
        );
    }
    for status in par_mux::mux::integrations::extension_status() {
        println!(
            "extension {} {}",
            status.agent,
            integration_status_line(status.installed, &status.asset)
        );
    }
    match par_mux::mux::skill_installer::skill_status() {
        Ok(status) => println!(
            "skill par-mux {}",
            integration_status_line(status.installed, &status.asset)
        ),
        Err(err) => println!("skill par-mux unknown ({err})"),
    }
    std::process::ExitCode::SUCCESS
}

/// `--(un)install-agent-extensions`: drop or remove the pi and omp
/// agent-state extension files, one line per agent.
fn run_agent_extension_action(install: bool) -> std::process::ExitCode {
    use par_mux::mux::integrations as integ;
    let arms: [(&str, Arm); 2] = if install {
        [
            ("pi", || {
                integ::install_pi_extension().map(|p| p.display().to_string())
            }),
            ("omp", || {
                integ::install_omp_extension().map(|p| p.display().to_string())
            }),
        ]
    } else {
        [
            ("pi", || {
                integ::uninstall_pi_extension().map(|r| arm_summary(r.removed))
            }),
            ("omp", || {
                integ::uninstall_omp_extension().map(|r| arm_summary(r.removed))
            }),
        ]
    };
    let mut failed = false;
    for (name, action) in arms {
        match action() {
            Ok(detail) => println!("par-mux: {name} extension: {detail}"),
            Err(err) => {
                failed = true;
                eprintln!("par-mux: {name} extension failed: {err}")
            }
        }
    }
    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

mod machine;
mod plugin;
mod update_cli;

use machine::{run_machine_command, run_machine_relay};
use plugin::run_plugin_command;
use update_cli::run_update_command;

fn main() -> std::process::ExitCode {
    let mut cli = Cli::parse();

    // `--force` was gen-config-scoped under clap's `requires`; it is now
    // also the --stop-inside-a-pane confirmation, so the old hard pairing
    // moves here: --force means nothing without one of its two users.
    if cli.force && !cli.gen_config && !cli.stop {
        eprintln!(
            "par-mux: --force requires --gen-config (or --stop to confirm a pane-internal stop)"
        );
        return std::process::ExitCode::from(2);
    }

    if cli.print_schema {
        print!("{}", par_mux::mux::command::COMMAND_SCHEMA_JSON);
        return std::process::ExitCode::SUCCESS;
    }
    if let Some(shell) = cli.print_completions {
        print!("{}", completion_script(shell));
        return std::process::ExitCode::SUCCESS;
    }

    // The machine relay owns the whole invocation before anything local
    // resolves: `par-mux --machine m …` becomes `ssh m -- par-mux …`.
    if let Some(label) = cli.machine.take() {
        return run_machine_relay(&label);
    }

    // The verb surface (R1): translate a subcommand into the flag fields
    // the existing paths read (or a direct return for the client-side
    // verbs), so every handler has exactly one implementation. The merge
    // keeps a top-level flag's value when the subcommand did not speak
    // for it (`par-mux --state-dir X stop` and `par-mux stop --state-dir
    // X` both work — flags may sit before the verb, its own args after).
    #[cfg(feature = "attach")]
    let mut attach_cmd: Option<AttachCommand> = None;
    // Whether a subcommand verb drove this invocation: the collision
    // guard below only fires on the bare NAME form, where intent is
    // ambiguous (`par-mux ls`), never on an explicit `par-mux start ls`.
    // The reserved name "server" (Paul, 2026-10-18): `par-mux server stop`
    // reads as "stop the server" but clap takes "server" as the daemon
    // NAME, silently narrowing to a daemon literally named "server"
    // (socket par-mux-server.sock). Refused in every name position, before
    // the verb dispatch (status/ls return inside it) and again after it
    // (`par-mux start server` merges the name below); only an explicit
    // --socket reads as deliberate. Checked before the subcommand block
    // AND after it — see refuse_reserved_server_name's call sites.
    if refuse_reserved_server_name(cli.name.as_deref(), cli.socket.as_ref()) {
        return std::process::ExitCode::from(2);
    }
    let mut verb_drove = false;
    if let Some(sub) = cli.sub.take() {
        let mut merge = |name: Option<String>,
                         socket: Option<std::path::PathBuf>,
                         state_dir: Option<std::path::PathBuf>| {
            cli.name = name.or(cli.name.take());
            cli.socket = socket.or(cli.socket.take());
            cli.state_dir = state_dir.or(cli.state_dir.take());
        };
        match sub {
            Sub::Start(args) => {
                verb_drove = true;
                let StartArgs {
                    name,
                    socket,
                    state_dir,
                    foreground,
                    no_attach,
                    pane_endpoints,
                    expose_control_socket,
                } = args;
                merge(name, socket, state_dir);
                cli.foreground |= foreground;
                cli.no_attach |= no_attach;
                cli.pane_endpoints |= pane_endpoints;
                cli.expose_control_socket |= expose_control_socket;
            }
            Sub::Stop(args) => {
                let TargetArgs {
                    name,
                    socket,
                    state_dir,
                } = args;
                merge(name, socket, state_dir);
                cli.stop = true;
            }
            Sub::Restart(args) => {
                let TargetArgs {
                    name,
                    socket,
                    state_dir,
                } = args;
                merge(name, socket, state_dir);
                cli.restart = true;
            }
            Sub::Status(args) => {
                let TargetArgs {
                    name,
                    socket,
                    state_dir,
                } = args;
                merge(name, socket, state_dir);
                if refuse_reserved_server_name(cli.name.as_deref(), cli.socket.as_ref()) {
                    return std::process::ExitCode::from(2);
                }
                let path = resolve_cli_socket(&cli);
                return run_status(&path);
            }
            Sub::Ls(args) => {
                let TargetArgs {
                    name,
                    socket,
                    state_dir,
                } = args;
                merge(name, socket, state_dir);
                if refuse_reserved_server_name(cli.name.as_deref(), cli.socket.as_ref()) {
                    return std::process::ExitCode::from(2);
                }
                let path = resolve_cli_socket(&cli);
                return run_command(&path, "list-sessions");
            }
            #[cfg(feature = "attach")]
            Sub::Attach(args) => attach_cmd = Some(AttachCommand::Attach(args)),
            Sub::Machine(args) => return run_machine_command(args),
            Sub::Plugin(args) => return run_plugin_command(args),
            Sub::Update(args) => {
                let socket = resolve_cli_socket(&cli);
                return run_update_command(args, socket);
            }
            Sub::Config(args) => {
                cli.gen_config = true;
                cli.force |= args.force;
            }
            Sub::Observe(args) => return run_pane_stream(&mut cli, args, false),
            Sub::Control(args) => return run_pane_stream(&mut cli, args, true),
            Sub::Completion(args) => {
                print!("{}", completion_script(args.shell));
                return std::process::ExitCode::SUCCESS;
            }
        }
    }
    // `par-mux start server` and friends: the verb's own NAME argument
    // merged above, so the reserved check runs again on the merged name.
    if refuse_reserved_server_name(cli.name.as_deref(), cli.socket.as_ref()) {
        return std::process::ExitCode::from(2);
    }

    // The mux library logs through `log` (ARC-010); without a logger
    // installed those records vanish, so wire the minimal stderr sink up
    // before anything can emit.
    log::set_logger(&STDERR_LOG)
        .map(|_| log::set_max_level(log::LevelFilter::Debug))
        .ok();

    // par-mux attach: subcommand form — run the attach client and exit
    // with its code. The parsed --mode/--prefix ride above the config
    // file (flags > file); the mode/prefix/reload defaults come from the
    // resolved config; socket resolution lives in
    // AttachOptions::socket_path (same precedence as the daemon/--cmd).
    #[cfg(feature = "attach")]
    if let Some(attach) = attach_cmd.as_ref() {
        // The client caps the facade at Error: its stderr IS the rendered
        // screen, and the emulator's per-sequence diagnostics (a chatty
        // TUI emits hundreds) would otherwise flood the display.
        log::set_max_level(log::LevelFilter::Error);
        let mut options = match attach.options() {
            Ok(options) => options,
            Err(message) => {
                // An unknown --mode spelling: fail loudly instead of
                // silently running the wrong mode (the manual-pass
                // `--mode rendered` report).
                use std::io::Write as _;
                let _ = writeln!(std::io::stderr().lock(), "par-mux: {message}");
                return std::process::ExitCode::from(2);
            }
        };
        let eff = effective(&cli, true);
        if options.prefix.is_none() {
            options.prefix = Some(eff.prefix.clone());
        }
        if options.reload.is_none() {
            options.reload = Some(eff.reload.clone());
        }
        let mode = match options.mode {
            Some(mode) => mode,
            None => {
                // The flag did not speak: the config file's mode tier (the
                // resolution defaults it to render). An unknown FILE value
                // falls back to the default with a warning — the file is
                // not a spelling error the user just typed.
                match eff.mode.to_ascii_lowercase().as_str() {
                    "render" => par_mux::mux::attach::AttachMode::Render,
                    "passthrough" => par_mux::mux::attach::AttachMode::Passthrough,
                    other => {
                        use std::io::Write as _;
                        let _ = writeln!(
                            std::io::stderr().lock(),
                            "par-mux: unknown [client] mode {other:?} — using render"
                        );
                        par_mux::mux::attach::AttachMode::Render
                    }
                }
            }
        };

        // Multi-server ambiguity guard: with NEITHER an explicit socket
        // NOR a name on the command line, enumerate under the resolved
        // registry base first. Multiple live servers: print the same
        // list `--list-servers` prints plus a hint, and refuse to pick
        // silently. Exactly one: attach there even when the default
        // resolution would name a different (dead) socket — "attach"
        // with one server running means that server. Zero or an
        // explicit target: unchanged behavior (the connect reports no
        // daemon on the resolved path).
        let explicit_target = attach
            .explicit_socket()
            .map(std::path::Path::to_path_buf)
            .or_else(|| {
                attach
                    .explicit_name()
                    .map(|name| par_mux::mux::resolve_socket_path(None, Some(name), None))
            });
        if explicit_target.is_none() {
            let base = discovery_base(&cli);
            let found = par_mux::mux::discovery::enumerate(&base);
            if found.live.len() > 1 {
                use std::io::Write as _;
                let mut err = std::io::stderr().lock();
                let _ = writeln!(
                    err,
                    "par-mux: multiple par-mux servers are running — attach to one with \
                     `par-mux attach <name|path>` (append `-t <session>` when it holds \
                     several sessions):"
                );
                let default_socket = par_mux::mux::resolve_socket_path(None, None, None);
                for server in &found.live {
                    let _ = writeln!(
                        err,
                        "  {}",
                        par_mux::mux::discovery::describe_server(server, Some(&default_socket))
                    );
                }
                return std::process::ExitCode::from(1);
            }
            if let [single] = found.live.as_slice() {
                // Pre-resolve the socket so the attach client never needs
                // its own resolution: AttachOptions carries the explicit
                // path from here on.
                options.socket = Some(single.socket.clone());
            }
        }
        return par_mux::mux::attach::run_with_mode(&options, mode);
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

    // --install-skill: write the embedded agent skill and exit — no
    // socket, no daemon, so it also runs inside a pane.
    if cli.install_skill {
        let result = match cli.skills_dir.as_deref() {
            Some(dir) => par_mux::mux::skill_installer::install_skill_into(dir),
            None => par_mux::mux::skill_installer::install_skill(),
        };
        return match result {
            Ok(path) => {
                println!("par-mux: installed agent skill to {}", path.display());
                std::process::ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("par-mux: {err}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    // The agent-state integration flags: install/uninstall/status for the
    // claude/codex/grok/muse/droid/copilot/cursor session hooks and the pi/omp extensions this
    // binary owns. No socket, no daemon, so they run anywhere.
    if cli.install_agent_hooks {
        return run_agent_hook_action(AgentHookAction::Install);
    }
    if cli.uninstall_agent_hooks {
        return run_agent_hook_action(AgentHookAction::Uninstall);
    }
    if cli.agent_hooks_status {
        return run_agent_hooks_status();
    }

    if cli.integration_status {
        return run_integration_status();
    }
    if cli.install_agent_extensions {
        return run_agent_extension_action(true);
    }
    if cli.heal_agent_hooks {
        par_mux::mux::integrations::heal_hook_assets();
        return std::process::ExitCode::SUCCESS;
    }
    if cli.uninstall_agent_extensions {
        return run_agent_extension_action(false);
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
    let path = resolve_cli_socket(&cli);
    if cli.check_update || cli.self_update {
        return update_cli::run_self_update(cli.check_update, cli.force_self_update, &path);
    }

    if let Some(command) = cli.command.as_deref() {
        return run_command(&path, command);
    }

    if cli.list_servers {
        return run_list_servers(&cli);
    }

    // The bare-NAME collision guard (R1): starting a daemon whose NAME
    // spells a command word or a subcommand verb is almost certainly a
    // typo'd invocation that would otherwise silently start a stray
    // daemon (`par-mux ls` used to boot one named "ls"). An explicit
    // --socket alongside the name reads as deliberate and is allowed.
    if !cli.stop && !cli.restart && !verb_drove && cli.socket.is_none() {
        if let Some(name) = cli.name.as_deref() {
            if par_mux::mux::command::is_command_name(name)
                || matches!(
                    name,
                    "start" | "stop" | "restart" | "status" | "ls" | "attach" | "config"
                )
            {
                eprintln!(
                    "par-mux: {name:?} is a command word, not a daemon name — did you mean \
                     `par-mux --cmd {name} …` or a subcommand? To start a daemon literally \
                     named {name:?}, pass --socket"
                );
                return std::process::ExitCode::from(2);
            }
        }
    }

    match run_daemon(cli, path) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// The reserved daemon name "server" (`par-mux server stop` reads as
/// "stop the server" but addresses a daemon literally named "server" on
/// `par-mux-server.sock` — Paul, 2026-10-18). Returns true (after
/// printing the explanation) when the invocation must be refused; an
/// explicit `--socket` reads as deliberate and is allowed, and attach
/// keeps resolving any NAME normally.
fn refuse_reserved_server_name(name: Option<&str>, socket: Option<&std::path::PathBuf>) -> bool {
    if socket.is_some() || name != Some("server") {
        return false;
    }
    eprintln!(
        "par-mux: \"server\" is reserved — `par-mux server …` addresses a daemon literally \
         named \"server\" (socket par-mux-server.sock), not \"the server\". The default \
         daemon takes no name: `par-mux stop`, `par-mux --stop`, `par-mux status`. To run \
         or address one named \"server\", pass --socket"
    );
    true
}

/// The socket-path resolution every client-side verb and the daemon
/// start share (the block main() used to inline): `--socket`, then NAME,
/// then `$PAR_MUX_CONTROL_SOCKET`/`$PAR_MUX_SOCKET`, then the unnamed
/// default.
fn resolve_cli_socket(cli: &Cli) -> std::path::PathBuf {
    if cli.socket.is_some() || cli.name.is_some() {
        par_mux::mux::resolve_socket_path(cli.socket.as_deref(), cli.name.as_deref(), None)
    } else {
        let socket_env = std::env::var_os("PAR_MUX_SOCKET").filter(|v| !v.is_empty());
        let control_env = std::env::var_os("PAR_MUX_CONTROL_SOCKET").filter(|v| !v.is_empty());
        par_mux::mux::resolve_socket_path(None, None, control_env.or(socket_env).as_deref())
    }
}

/// `par-mux status` (R2): the one-shot orientation — build stamp,
/// workspaces, sessions, the displayed session, and the pane count, one
/// plain line each. Client-side composition over the existing queries:
/// no new wire surface, and a daemon too old to answer one of them
/// degrades that section to `?` instead of failing the whole reply.
fn run_status(path: &std::path::Path) -> std::process::ExitCode {
    let mut client = match par_mux::mux::MuxClient::connect(path) {
        Ok(client) => client,
        Err(err) => {
            eprintln!(
                "par-mux: no daemon running on {} ({err}) — start one with `par-mux` (or \
                 `par-mux --socket {}` for this socket)",
                path.display(),
                path.display()
            );
            return std::process::ExitCode::from(1);
        }
    };
    // Best-effort sections: each query's failure degrades to `?` so one
    // missing section never hides the rest.
    let section = |client: &mut par_mux::mux::MuxClient, command: &str| -> Vec<String> {
        client
            .send(command)
            .unwrap_or_else(|_| vec!["?".to_string()])
    };
    let stamp = section(&mut client, "version")
        .into_iter()
        .next()
        .unwrap_or_else(|| "?".to_string());
    println!("par-mux {}  {}", path.display(), stamp);
    println!("workspaces:");
    for line in section(&mut client, "list-workspaces") {
        println!("  {line}");
    }
    println!("sessions:");
    for line in section(&mut client, "list-sessions") {
        println!("  {line}");
    }
    let displayed = section(&mut client, "switch-client")
        .into_iter()
        .next()
        .unwrap_or_else(|| "?".to_string());
    println!("displayed: {displayed}");
    let panes = section(&mut client, "list-panes")
        .into_iter()
        .filter(|l| l.starts_with('%'))
        .count();
    println!("panes: {panes}");
    std::process::ExitCode::SUCCESS
}

/// The daemon modes: --stop, --restart, and serving the socket — detached
/// with an auto-attach by default (unix), or in the foreground with
/// --foreground.
/// The inside-a-pane stop guard (card 01a18000): `par-mux --stop` typed
/// inside a pane kills the daemon hosting the caller's own shell — a
/// session died exactly this way. Refuse while the caller sits in a pane
/// whose exported socket is the daemon being stopped, unless the stop was
/// confirmed with `--yes`/`--force`. An explicit `--socket` naming a
/// DIFFERENT daemon is deliberate either way and never refuses; a pane
/// with no socket export has nothing to match against.
fn stop_inside_pane_guard(
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

fn run_daemon(cli: Cli, path: std::path::PathBuf) -> std::io::Result<std::process::ExitCode> {
    // Windows: a pipe-path --socket (\\.\pipe\name) names the same pipe
    // as the bare stem — strip the prefix ONCE here so every consumer
    // downstream (bind, the state-file stem, the panes' PAR_MUX_SOCKET env
    // contract, which the .ps1 hooks pass straight to NamedPipeClientStream)
    // agrees on the bare name.
    #[cfg(windows)]
    let path = par_mux::mux::ipc::strip_pipe_prefix(&path);
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
    if cli.stop || cli.restart {
        // The suicide guard (card 01a18000): a --stop typed inside a pane
        // kills the daemon hosting the caller's own shell. Refuse while
        // the caller sits in the pane of the daemon being stopped, unless
        // --yes/--force confirmed it. --restart keeps its deliberate
        // exemption (tmux's kill-server shape).
        if cli.stop {
            if let Some(reason) = stop_inside_pane_guard(
                std::env::var_os("PAR_MUX_ENV").is_some(),
                &path,
                std::env::var("PAR_MUX_SOCKET").ok().as_deref(),
                std::env::var("PAR_MUX_CONTROL_SOCKET").ok().as_deref(),
                !(cli.yes || cli.force),
            ) {
                return Err(std::io::Error::other(reason));
            }
        }
        if stop_daemon(&path)? {
            eprintln!("par-mux: stopped the daemon on {}", path.display());
        } else {
            eprintln!("par-mux: no daemon running on {}", path.display());
        }
        if cli.stop {
            return Ok(std::process::ExitCode::SUCCESS);
        }
        // Pre-fork, on the still-open terminal: what the detached fresh
        // daemon will restore. After the fork its stderr leaves the
        // terminal, so this is the caller's only window on the handoff.
        // Mirrors load_or_quarantine's conclusions (last-good fallback
        // included) without mutating anything.
        match par_mux::mux::persist::peek_state(&state_path) {
            par_mux::mux::persist::StatePeek::Populated => {}
            par_mux::mux::persist::StatePeek::Empty => eprintln!(
                "par-mux: previous daemon saved an empty tree at {} — the fresh daemon will \
                 restore nothing and exit (exit-when-empty)",
                state_path.display()
            ),
            par_mux::mux::persist::StatePeek::Missing => eprintln!(
                "par-mux: no saved state at {} — the fresh daemon will start empty and exit \
                 (exit-when-empty)",
                state_path.display()
            ),
            par_mux::mux::persist::StatePeek::Unreadable => eprintln!(
                "par-mux: saved state {} is unreadable — the fresh daemon will quarantine it \
                 and start fresh (details: {})",
                state_path.display(),
                daemon_log_path(&state_path, state_dir_pinned).display()
            ),
        }
        // --restart falls through to the detach below — the state save the
        // stop just completed is what the fresh daemon restores, and the
        // daemon must outlive the terminal --restart was typed into.
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
    if !cli.foreground {
        // The nested-daemon refusal first, on the terminal rather than
        // buried in the daemon log (--restart stays exempt: it may be
        // typed inside a pane deliberately; the guard below still runs in
        // the serving child for the non-exempt paths).
        let nested = par_mux::mux::nested_daemon_refusal();
        if let (Some(reason), false) = (nested, cli.restart) {
            return Err(std::io::Error::other(reason));
        }
        // The wrapper hand-off only for starts the refusal passed: a pane
        // keeps its own semantics (a handed-off daemon would land OUTSIDE
        // the pane's daemon, silently dodging the nested guard).
        #[cfg(target_os = "macos")]
        // Only a real start hands off: with a daemon already serving, the
        // attach below must keep the invoking terminal's stdio, which the
        // launcher's daemon.log redirect would swallow (2026-10-27).
        if !cli.par_mux_in_app_wrapper
            && nested.is_none()
            && par_mux::mux::connect_local_stream(&path).is_err()
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
        match fork_daemon(Some(&daemon_log_path(&state_path, state_dir_pinned)))? {
            ForkOutcome::Child => {} // this process serves, detached
            ForkOutcome::Parent => {
                return run_detached_parent(&cli, &path, &state_path, state_dir_pinned);
            }
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
    let _state_lock = par_mux::mux::persist::acquire_state_lock(&state_path)?;

    // Nested-daemon guard, on the path that actually serves: a daemon
    // started from inside a pane (PAR_MUX_ENV=1) would shadow the outer
    // server's identity for every PTY under it. --cmd returned earlier and
    // --stop/--restart are exempt above, as tmux exempts kill-server; the
    // guard below therefore only bites plain serve mode.
    if !cli.restart {
        if let Some(reason) = par_mux::mux::nested_daemon_refusal() {
            return Err(std::io::Error::other(reason));
        }
    }

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
    let tree = restored.unwrap_or_else(|| par_mux::mux::tree::MuxTree::new(Box::new(factory())));
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
        Some(rx) => par_mux::mux::MuxServer::bind_with_tree_and_pane_endpoints(&path, tree, rx)?,
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
fn app_handoff_launcher(
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

/// The daemon's stderr logger (ARC-010): the mux library emits through
/// `log`, and the daemon has no tracing subscriber, so this minimal sink is
/// what makes those records visible. Level-prefixed, info and above, plus
/// the hook-report debug trail (the restart-roster forensics lines); other
/// debug records stay off because the emulator's diagnostics are chatty.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
            || (metadata.level() == log::Level::Debug
                && metadata.target().starts_with("par_mux::mux::hooks"))
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static STDERR_LOG: StderrLog = StderrLog;

#[cfg(test)]
mod mode_group_tests {
    use super::*;

    /// The app hand-off decision (card 01a17f9f15): a bare binary outside
    /// the bundle hands off to the installed wrapper's launcher; the
    /// bundle's own binary, a missing launcher, and the
    /// PAR_MUX_NO_APP_WRAPPER opt-out never do.
    /// The inside-a-pane stop guard (card 01a18000): the refusal fires
    /// only for an inside-pane caller, an unconfirmed stop, and a socket
    /// that matches one of the pane's exports; the confirmation flags,
    /// an outside caller, and a different socket all pass.
    #[test]
    fn stop_inside_pane_guard_branches() {
        let target = std::path::Path::new("/tmp/par-mux-host.sock");
        let host = Some("/tmp/par-mux-host.sock");
        // Endpoint mode exports the host socket as PAR_MUX_CONTROL_SOCKET
        // and the pane's own hook socket as PAR_MUX_SOCKET — either
        // naming the target is a match.
        let control = Some("/tmp/par-mux-host.sock");
        let other = Some("/tmp/other.sock");
        assert!(stop_inside_pane_guard(true, target, host, control, true)
            .expect("unconfirmed match refuses")
            .contains("refusing to stop the daemon hosting this pane"));
        // Either export matching is enough (endpoint mode exports both).
        assert!(stop_inside_pane_guard(true, target, None, control, true).is_some());
        // The confirmations and the outside caller pass through.
        assert!(stop_inside_pane_guard(true, target, host, control, false).is_none());
        assert!(stop_inside_pane_guard(false, target, host, control, true).is_none());
        // A different daemon's socket is a deliberate stop.
        assert!(stop_inside_pane_guard(true, target, other, None, true).is_none());
        // A pane with no socket export has nothing to match.
        assert!(stop_inside_pane_guard(true, target, None, None, true).is_none());
        // A stale host export (the daemon is gone) does not match a live
        // target: canonicalize fails, the string compare disagrees.
        assert!(
            stop_inside_pane_guard(true, target, Some("/tmp/stale-host.sock"), None, true)
                .is_none()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn app_handoff_decision() {
        let dir = tempfile::Builder::new()
            .prefix("par-mux-handoff-")
            .tempdir()
            .expect("tempdir");
        let launcher = dir.path().join("Par-mux.app/Contents/MacOS/Par-mux");
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::write(&launcher, b"").unwrap();

        let bare_exe = dir.path().join("cargo-bin/par-mux");
        std::fs::create_dir_all(bare_exe.parent().unwrap()).unwrap();
        std::fs::write(&bare_exe, b"").unwrap();
        assert_eq!(
            app_handoff_launcher(&bare_exe, &dir.path().join("Par-mux.app"), false),
            Some(launcher.canonicalize().unwrap()),
            "a bare binary hands off to the installed launcher"
        );

        let in_app_exe = dir.path().join("Par-mux.app/Contents/MacOS/other-tool");
        std::fs::write(&in_app_exe, b"").unwrap();
        assert_eq!(
            app_handoff_launcher(&in_app_exe, &dir.path().join("Par-mux.app"), false),
            None,
            "the bundle's own binary never hands off"
        );

        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            app_handoff_launcher(&bare_exe, empty.path(), false),
            None,
            "a missing launcher degrades to the bare start"
        );

        assert_eq!(
            app_handoff_launcher(&bare_exe, &dir.path().join("Par-mux.app"), true),
            None,
            "the opt-out wins"
        );
    }

    #[test]
    fn update_flags_require_explicit_bundle_force() {
        assert!(
            Cli::try_parse_from(["par-mux", "--check-update"])
                .unwrap()
                .check_update
        );
        assert!(
            Cli::try_parse_from(["par-mux", "--self-update"])
                .unwrap()
                .self_update
        );
        assert!(
            Cli::try_parse_from(["par-mux", "--self-update", "--force-self-update"])
                .unwrap()
                .force_self_update
        );
        assert!(Cli::try_parse_from(["par-mux", "--force-self-update"]).is_err());
        assert!(Cli::try_parse_from(["par-mux", "--check-update", "--self-update"]).is_err());
    }
    use clap::{CommandFactory, Parser};

    /// The bulk --install-agent-hooks/--uninstall-agent-hooks flag covers
    /// every library CONFIG_HOOK_ARMS entry: the docs promise the full
    /// set, so a new config-hook arm shipping without its bulk line is a
    /// gap (observed 2026-10-21: five arms were bulk-skipped).
    #[test]
    fn bulk_arm_covers_config_hook_arms() {
        for action in [AgentHookAction::Install, AgentHookAction::Uninstall] {
            let names: Vec<_> = agent_hook_arms(&action)
                .iter()
                .map(|(name, _)| *name)
                .collect();
            let expected: Vec<_> = ["claude", "codex", "grok", "muse"]
                .into_iter()
                .chain(
                    par_mux::mux::integrations::CONFIG_HOOK_ARMS
                        .iter()
                        .map(|arm| arm.agent),
                )
                .collect();
            assert_eq!(
                names, expected,
                "the bulk arms must cover every library arm"
            );
        }
    }

    #[test]
    fn cli_debug_asserts() {
        Cli::command().debug_assert();
    }

    #[test]
    fn integration_status_line_keeps_the_installed_absent_token() {
        use par_mux::mux::integrations::{AssetCurrency, AssetState};
        let absent = AssetState {
            currency: AssetCurrency::Absent,
            installed_version: None,
            bundled_version: Some("2".to_string()),
        };
        assert_eq!(integration_status_line(false, &absent), "absent");
        assert_eq!(
            integration_status_line(true, &absent),
            "installed asset missing (bundled v2)"
        );
        let current = AssetState {
            currency: AssetCurrency::Current,
            installed_version: Some("2".to_string()),
            bundled_version: Some("2".to_string()),
        };
        assert_eq!(integration_status_line(true, &current), "installed v2");
        let outdated = AssetState {
            currency: AssetCurrency::Outdated,
            installed_version: Some("1".to_string()),
            bundled_version: Some("2".to_string()),
        };
        assert_eq!(
            integration_status_line(true, &outdated),
            "installed v1 \u{2192} v2 outdated"
        );
    }

    #[test]
    fn foreground_and_no_attach_conflict_with_the_one_shots_but_compose_with_restart() {
        // The daemon_mode group stays exclusive among itself; --foreground
        // and --no-attach are modifiers of the serve path, so each may pair
        // with --restart but with nothing one-shot.
        for one_shot in [
            "--stop",
            "--cmd=list-sessions",
            "--list-servers",
            "--install-agent-hooks",
            "--agent-hooks-status",
            "--print-completions=zsh",
        ] {
            for modifier in ["--foreground", "--no-attach"] {
                let err = Cli::try_parse_from(["par-mux", modifier, one_shot])
                    .expect_err(&format!("{modifier} {one_shot} must conflict"));
                assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
            }
        }
        assert!(Cli::try_parse_from(["par-mux", "--foreground"]).is_ok());
        assert!(Cli::try_parse_from(["par-mux", "--no-attach"]).is_ok());
        assert!(Cli::try_parse_from(["par-mux", "--foreground", "--restart"]).is_ok());
        assert!(Cli::try_parse_from(["par-mux", "--no-attach", "--restart"]).is_ok());
        let err = Cli::try_parse_from(["par-mux", "--foreground", "--no-attach"])
            .expect_err("foreground and no_attach are opposites");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn mode_flags_are_mutually_exclusive() {
        let flags = [
            "--stop",
            "--restart",
            "--cmd=list-sessions",
            "--list-servers",
            "--gen-config",
            "--install-skill",
            "--install-agent-hooks",
            "--uninstall-agent-hooks",
            "--agent-hooks-status",
            "--integration-status",
            "--install-agent-extensions",
            "--uninstall-agent-extensions",
            "--heal-agent-hooks",
            "--print-schema",
            "--print-completions=bash",
        ];
        for (i, a) in flags.iter().enumerate() {
            assert!(
                Cli::try_parse_from(["par-mux", a]).is_ok(),
                "{a} alone parses"
            );
            for b in &flags[i + 1..] {
                let err = Cli::try_parse_from(["par-mux", a, b])
                    .expect_err(&format!("{a} {b} must conflict"));
                assert_eq!(
                    err.kind(),
                    clap::error::ErrorKind::ArgumentConflict,
                    "{a} {b}"
                );
            }
        }
    }

    #[test]
    fn cmd_still_conflicts_with_state_dir() {
        let err = Cli::try_parse_from(["par-mux", "--cmd=x", "--state-dir", "/tmp"])
            .expect_err("conflict");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

#[cfg(all(test, feature = "attach"))]
mod attach_cli_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn attach_parses_name_form() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "work"]).expect("parse");
        let Sub::Attach(attach) = cli.sub.expect("subcommand present") else {
            unreachable!("only the attach verb reaches the attach tests")
        };
        assert_eq!(attach.name.as_deref(), Some("work"));
        assert!(attach.socket.is_none());
        assert!(attach.target.is_none());
        assert!(attach.prefix.is_none());
    }

    #[test]
    fn attach_target_flag_is_short_t_only() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "-t", "%0"]).expect("parse");
        let Sub::Attach(attach) = cli.sub.expect("subcommand present") else {
            unreachable!("only the attach verb reaches the attach tests")
        };
        assert_eq!(attach.target.as_deref(), Some("%0"));
        assert!(attach.prefix.is_none());
    }

    /// `--mode` absent leaves the mode to the config tier (render by
    /// default); `render`/`passthrough` parse and anything else fails
    /// loudly.
    #[test]
    fn attach_mode_flag_parses_and_validates() {
        use par_mux::mux::attach::AttachMode;
        let cli = Cli::try_parse_from(["par-mux", "attach"]).expect("parse");
        let Sub::Attach(attach) = cli.sub.expect("subcommand present") else {
            unreachable!("only the attach verb reaches the attach tests")
        };
        assert!(
            attach.mode.is_none(),
            "no --mode flag: the config tier decides (render by default)"
        );

        let cli = Cli::try_parse_from(["par-mux", "attach", "--mode", "render"]).expect("parse");
        let Sub::Attach(attach) = cli.sub.expect("subcommand present") else {
            unreachable!("only the attach verb reaches the attach tests")
        };
        assert_eq!(
            parse_mode(attach.mode.as_deref().unwrap_or("")),
            Ok(AttachMode::Render)
        );
        let cli =
            Cli::try_parse_from(["par-mux", "attach", "--mode", "passthrough"]).expect("parse");
        let Sub::Attach(attach) = cli.sub.expect("subcommand present") else {
            unreachable!("only the attach verb reaches the attach tests")
        };
        assert_eq!(
            parse_mode(attach.mode.as_deref().unwrap_or("")),
            Ok(AttachMode::Passthrough)
        );

        // An unknown value fails loudly — a silent fallback ran the user's
        // session in the wrong mode (the manual-pass `--mode rendered`
        // report).
        assert!(parse_mode("rendered").is_err());
        assert!(parse_mode("wat").is_err());
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
        let Sub::Attach(attach) = cli.sub.expect("subcommand present") else {
            unreachable!("only the attach verb reaches the attach tests")
        };
        assert_eq!(attach.prefix.as_deref(), Some("C-b"));
        assert_eq!(attach.socket, Some(std::path::PathBuf::from("/tmp/x.sock")));
        assert!(attach.target.is_none());
    }

    #[test]
    fn attach_target_and_prefix_together() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "work", "-t", "$0", "--prefix", "C-b"])
            .expect("parse");
        let Sub::Attach(attach) = cli.sub.expect("subcommand present") else {
            unreachable!("only the attach verb reaches the attach tests")
        };
        assert_eq!(attach.name.as_deref(), Some("work"));
        assert_eq!(attach.target.as_deref(), Some("$0"));
        assert_eq!(attach.prefix.as_deref(), Some("C-b"));
    }

    #[test]
    fn attach_long_prefix_flag() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "--prefix", "C-a"]).expect("parse");
        let Sub::Attach(attach) = cli.sub.expect("subcommand present") else {
            unreachable!("only the attach verb reaches the attach tests")
        };
        assert_eq!(attach.prefix.as_deref(), Some("C-a"));
        assert!(attach.target.is_none());
    }

    #[test]
    fn attach_socket_wins_over_name() {
        let cli = Cli::try_parse_from(["par-mux", "attach", "work", "--socket", "/tmp/y.sock"])
            .expect("parse");
        let Sub::Attach(attach) = cli.sub.expect("subcommand present") else {
            unreachable!("only the attach verb reaches the attach tests")
        };
        assert_eq!(attach.socket, Some(std::path::PathBuf::from("/tmp/y.sock")));
        assert_eq!(attach.name.as_deref(), Some("work"));
    }

    #[test]
    fn daemon_mode_still_parses_without_subcommand() {
        // The flag-based daemon form must keep parsing unchanged.
        let cli = Cli::try_parse_from(["par-mux", "--socket", "/tmp/d.sock"]).expect("parse");
        assert!(cli.sub.is_none());
        assert_eq!(cli.socket, Some(std::path::PathBuf::from("/tmp/d.sock")));
    }
}

// The tracked scripts are generated from the full release surface
// (`mux-bin,attach`): without `attach` the `attach` verb is absent and the
// scripts legitimately differ.
#[cfg(all(test, feature = "attach"))]
mod completion_tests {
    use super::completion_script;
    use clap_complete::{Generator, Shell};

    const SHELLS: [Shell; 5] = [
        Shell::Bash,
        Shell::Zsh,
        Shell::Fish,
        Shell::PowerShell,
        Shell::Elvish,
    ];

    #[test]
    fn completion_scripts_cover_the_cli_surface() {
        for shell in [Shell::Bash, Shell::Zsh] {
            let script = completion_script(shell);
            for word in [
                "--print-schema",
                "--socket",
                "--print-completions",
                "powershell",
                "observe",
                "attach",
                "status",
            ] {
                assert!(script.contains(word), "{shell} script lacks {word}");
            }
        }
    }

    #[test]
    fn the_completion_subcommand_spelling_matches_the_flag() {
        // The verb form parses to the same shells the flag takes —
        // herdr's spelling of --print-completions.
        use clap::Parser as _;
        for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
            let cli = super::Cli::try_parse_from(["par-mux", "completion", shell])
                .unwrap_or_else(|_| panic!("completion {shell} parses"));
            match cli.sub {
                Some(super::Sub::Completion(args)) => {
                    assert_eq!(args.shell.to_string(), shell);
                }
                other => panic!("expected the completion verb, got {other:?}"),
            }
        }
    }

    #[test]
    fn fish_completion_script_passes_fish_n() {
        // `fish -n` is the syntax gate the bash/zsh shells get from their
        // own -n runs in the Makefile-adjacent flows; here it guards the
        // generated fish script directly.
        let script = completion_script(Shell::Fish);
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("par-mux.fish");
        std::fs::write(&path, &script).expect("write");
        let have_fish = std::process::Command::new("fish")
            .arg("-n")
            .arg(&path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success());
        match have_fish {
            Ok(true) => {}
            // No fish on the host: the script is still clap_complete's
            // own output (covered by the surface test); skip rather than
            // fail on a host without the shell.
            Ok(false) if which_fish_missing() => {}
            Ok(false) => panic!("fish -n rejected the completion script"),
            Err(_) => {}
        }
    }

    fn which_fish_missing() -> bool {
        std::process::Command::new("which")
            .arg("fish")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| !s.success())
    }

    #[test]
    fn committed_completion_scripts_match_the_clap_surface() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("completions");
        for shell in SHELLS {
            let path = dir.join(shell.file_name("par-mux"));
            let committed = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            assert!(
                committed == completion_script(shell),
                "{} is stale against the clap surface — run `make completions`",
                path.display()
            );
        }
    }
}
