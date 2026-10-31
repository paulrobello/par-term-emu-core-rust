//! The par-mux daemon.
//!
//! Owns PTYs and serves the control-mode protocol over a local socket. Runs
//! until killed: clients come and go, panes do not (see par-mux.md D5).

// QA-201: every production `unsafe` block states its invariant.
#![cfg_attr(not(test), warn(clippy::undocumented_unsafe_blocks))]

use clap::Parser;

use par_mux::mux::config::{load_canonical, resolve, EffectiveConfig, Overrides};

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

    /// Export OpenTelemetry metrics over OTLP (default off; zero OTLP calls
    /// when off — kanban 01a18d2606). Daemon-only: a --cmd client or attach
    /// invocation ignores these flags.
    #[arg(long, default_value = "false", env = "PAR_MUX_OTEL_ENABLED")]
    otel_enabled: bool,

    /// OTLP collector endpoint for the metrics exporter
    #[arg(
        long,
        default_value = "http://127.0.0.1:4317",
        env = "PAR_MUX_OTEL_ENDPOINT"
    )]
    otel_endpoint: String,

    /// OTLP exporter transport: `grpc` (default) or `http` (protobuf POSTed
    /// to the full endpoint URL — the transport CF-Access-fronted collectors
    /// like par-obs's obs-ingest expose)
    #[arg(long, default_value = "grpc", env = "PAR_MUX_OTEL_PROTOCOL")]
    otel_protocol: String,

    /// Newline-separated `Key: Value` headers attached to HTTP OTLP exports
    /// (e.g. CF-Access service-token pairs). Parsed values are never logged.
    #[arg(long, default_value = "", env = "PAR_MUX_OTEL_HEADERS")]
    otel_headers: String,

    /// OTel service.name resource attribute for exported metrics
    #[arg(long, default_value = "par-mux", env = "PAR_MUX_OTEL_SERVICE_NAME")]
    otel_service_name: String,

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
                let mod_note = match &r.mod_path {
                    Some(path) => format!(
                        " + mod {} ({})",
                        path.display(),
                        if r.mod_settings_changed {
                            "settings updated"
                        } else {
                            "already present"
                        }
                    ),
                    None => String::new(),
                };
                Ok(format!(
                    "{} (settings {}){}",
                    r.hook_path.display(),
                    if r.settings_changed {
                        "updated"
                    } else {
                        "already present"
                    },
                    mod_note
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
                Ok(arm_summary(
                    r.settings_changed || r.hook_removed || r.mod_removed,
                ))
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

mod daemon;
mod dispatch;
mod machine;
mod plugin;
mod runners;
mod update_cli;

/// Parse, resolve the invocation into its one mode, run that mode.
fn main() -> std::process::ExitCode {
    let mut cli = Cli::parse();
    match dispatch::resolve(&mut cli) {
        Ok(mode) => dispatch::run(cli, mode),
        Err(code) => code,
    }
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
    #[cfg(target_os = "macos")]
    use crate::daemon::app_handoff_launcher;
    use crate::daemon::stop_inside_pane_guard;

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

    /// The OTLP flags default off and parse on the daemon Cli (kanban
    /// 01a18d2606); the env tier (`PAR_MUX_OTEL_*`) rides clap's `env =`
    /// attributes, so only the flag tier is pinned here.
    #[test]
    fn otel_flags_default_off_and_parse() {
        let cli = Cli::try_parse_from(["par-mux"]).unwrap();
        assert!(!cli.otel_enabled);
        assert_eq!(cli.otel_endpoint, "http://127.0.0.1:4317");
        assert_eq!(cli.otel_protocol, "grpc");
        assert_eq!(cli.otel_headers, "");
        assert_eq!(cli.otel_service_name, "par-mux");
        let cli = Cli::try_parse_from(["par-mux", "--otel-enabled"]).unwrap();
        assert!(cli.otel_enabled);
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
