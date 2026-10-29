//! `main`'s two halves: [`resolve`] reads the parsed CLI into exactly one
//! [`Mode`] (walking the flag and verb precedence in its historical
//! order, refusals included), and [`run`] hands that mode to its runner
//! behind one match — the binary-side mirror of the daemon library's
//! `dispatch/` table.

use std::process::ExitCode;

use super::daemon::run_daemon;
use super::machine::{run_machine_command, run_machine_relay};
use super::plugin::run_plugin_command;
#[cfg(feature = "attach")]
use super::runners::run_attach;
use super::runners::{
    refuse_reserved_server_name, resolve_cli_socket, run_command, run_gen_config,
    run_install_skill, run_list_servers, run_pane_stream, run_status,
};
use super::update_cli::{self, run_update_command};
#[cfg(feature = "attach")]
use super::AttachCommand;
use super::{
    completion_script, run_agent_extension_action, run_agent_hook_action, run_agent_hooks_status,
    run_integration_status, AgentHookAction, Cli, MachineArgs, PluginArgs, StartArgs, StreamArgs,
    Sub, TargetArgs, UpdateArgs, STDERR_LOG,
};

/// What one invocation does: each variant is one runner.
pub(super) enum Mode {
    /// `--print-schema`.
    PrintSchema,
    /// `--print-completions SHELL` / `completion SHELL`.
    Completions(clap_complete::Shell),
    /// `--machine LABEL`: the whole invocation relays over ssh.
    MachineRelay(String),
    /// `status`, on the resolved socket.
    Status(std::path::PathBuf),
    /// `ls`, on the resolved socket.
    Ls(std::path::PathBuf),
    /// `machine …`.
    Machine(MachineArgs),
    /// `plugin …`.
    Plugin(PluginArgs),
    /// `update …`, on the resolved socket.
    Update(UpdateArgs, std::path::PathBuf),
    /// `observe` (`control: false`) / `control` (`control: true`).
    PaneStream { args: StreamArgs, control: bool },
    /// `attach …`.
    #[cfg(feature = "attach")]
    Attach(AttachCommand),
    /// `--gen-config` / `config`.
    GenConfig,
    /// `--install-skill`.
    InstallSkill,
    /// `--install-agent-hooks` / `--uninstall-agent-hooks`.
    AgentHooks(AgentHookAction),
    /// `--agent-hooks-status`.
    AgentHooksStatus,
    /// `--integration-status`.
    IntegrationStatus,
    /// `--install-agent-extensions` (`true`) / `--uninstall-agent-extensions`.
    AgentExtensions(bool),
    /// `--heal-agent-hooks`.
    HealAgentHooks,
    /// `--check-update` / `--self-update`, on the resolved socket.
    SelfUpdate(std::path::PathBuf),
    /// `--cmd COMMAND`, on the resolved socket.
    Command(String, std::path::PathBuf),
    /// `--list-servers`.
    ListServers,
    /// `--stop`, `--restart`, or serving the resolved socket.
    Daemon(std::path::PathBuf),
}

impl Mode {
    /// Whether the stderr `log` sink goes up before the runner: every mode
    /// past the verb dispatch and the second reserved-name check. The
    /// earlier modes historically returned before the logger existed.
    fn installs_logger(&self) -> bool {
        !matches!(
            self,
            Mode::PrintSchema
                | Mode::Completions(_)
                | Mode::MachineRelay(_)
                | Mode::Status(_)
                | Mode::Ls(_)
                | Mode::Machine(_)
                | Mode::Plugin(_)
                | Mode::Update(..)
                | Mode::PaneStream { .. }
        )
    }
}

/// What the subcommand verb resolved to.
enum Verb {
    /// A verb that names its runner outright.
    Run(Mode),
    /// A verb whose args merged into the flag fields; `verb_drove` marks
    /// an explicit `start` (the bare-NAME collision guard skips it).
    Merged { verb_drove: bool },
    /// `attach`: still subject to the second reserved-name check.
    #[cfg(feature = "attach")]
    Attach(AttachCommand),
}

/// Resolve the parsed CLI into its [`Mode`]; `Err` is a refusal whose
/// message is already printed (exit code 2).
pub(super) fn resolve(cli: &mut Cli) -> Result<Mode, ExitCode> {
    // `--force` was gen-config-scoped under clap's `requires`; it is now
    // also the --stop-inside-a-pane confirmation, so the old hard pairing
    // moves here: --force means nothing without one of its two users.
    if cli.force && !cli.gen_config && !cli.stop {
        eprintln!(
            "par-mux: --force requires --gen-config (or --stop to confirm a pane-internal stop)"
        );
        return Err(ExitCode::from(2));
    }

    if cli.print_schema {
        return Ok(Mode::PrintSchema);
    }
    if let Some(shell) = cli.print_completions {
        return Ok(Mode::Completions(shell));
    }

    // The machine relay owns the whole invocation before anything local
    // resolves: `par-mux --machine m …` becomes `ssh m -- par-mux …`.
    if let Some(label) = cli.machine.take() {
        return Ok(Mode::MachineRelay(label));
    }

    // The verb surface (R1): translate a subcommand into the flag fields
    // the existing paths read (or a direct return for the client-side
    // verbs), so every handler has exactly one implementation. The merge
    // keeps a top-level flag's value when the subcommand did not speak
    // for it (`par-mux --state-dir X stop` and `par-mux stop --state-dir
    // X` both work — flags may sit before the verb, its own args after).
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
        return Err(ExitCode::from(2));
    }
    let mut verb_drove = false;
    #[cfg(feature = "attach")]
    let mut attach_cmd: Option<AttachCommand> = None;
    if let Some(sub) = cli.sub.take() {
        match merge_verb(cli, sub)? {
            Verb::Run(mode) => return Ok(mode),
            Verb::Merged { verb_drove: drove } => verb_drove = drove,
            #[cfg(feature = "attach")]
            Verb::Attach(attach) => attach_cmd = Some(attach),
        }
    }
    // `par-mux start server` and friends: the verb's own NAME argument
    // merged above, so the reserved check runs again on the merged name.
    if refuse_reserved_server_name(cli.name.as_deref(), cli.socket.as_ref()) {
        return Err(ExitCode::from(2));
    }

    #[cfg(feature = "attach")]
    if let Some(attach) = attach_cmd {
        return Ok(Mode::Attach(attach));
    }
    if let Some(mode) = one_shot_mode(cli) {
        return Ok(mode);
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
    let path = resolve_cli_socket(cli);
    if cli.check_update || cli.self_update {
        return Ok(Mode::SelfUpdate(path));
    }

    if let Some(command) = cli.command.take() {
        return Ok(Mode::Command(command, path));
    }

    if cli.list_servers {
        return Ok(Mode::ListServers);
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
                return Err(std::process::ExitCode::from(2));
            }
        }
    }

    Ok(Mode::Daemon(path))
}

/// The verb surface (R1): merge a subcommand's args into the flag fields
/// the existing paths read, or name its runner outright.
fn merge_verb(cli: &mut Cli, sub: Sub) -> Result<Verb, ExitCode> {
    let mut verb_drove = false;
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
                return Err(std::process::ExitCode::from(2));
            }
            let path = resolve_cli_socket(cli);
            return Ok(Verb::Run(Mode::Status(path)));
        }
        Sub::Ls(args) => {
            let TargetArgs {
                name,
                socket,
                state_dir,
            } = args;
            merge(name, socket, state_dir);
            if refuse_reserved_server_name(cli.name.as_deref(), cli.socket.as_ref()) {
                return Err(std::process::ExitCode::from(2));
            }
            let path = resolve_cli_socket(cli);
            return Ok(Verb::Run(Mode::Ls(path)));
        }
        #[cfg(feature = "attach")]
        Sub::Attach(args) => return Ok(Verb::Attach(AttachCommand::Attach(args))),
        Sub::Machine(args) => return Ok(Verb::Run(Mode::Machine(args))),
        Sub::Plugin(args) => return Ok(Verb::Run(Mode::Plugin(args))),
        Sub::Update(args) => {
            let socket = resolve_cli_socket(cli);
            return Ok(Verb::Run(Mode::Update(args, socket)));
        }
        Sub::Config(args) => {
            cli.gen_config = true;
            cli.force |= args.force;
        }
        Sub::Observe(args) => {
            return Ok(Verb::Run(Mode::PaneStream {
                args,
                control: false,
            }))
        }
        Sub::Control(args) => {
            return Ok(Verb::Run(Mode::PaneStream {
                args,
                control: true,
            }))
        }
        Sub::Completion(args) => return Ok(Verb::Run(Mode::Completions(args.shell))),
    }
    Ok(Verb::Merged { verb_drove })
}

/// The no-socket, no-daemon one-shot flags, in their precedence order.
fn one_shot_mode(cli: &Cli) -> Option<Mode> {
    if cli.gen_config {
        return Some(Mode::GenConfig);
    }
    if cli.install_skill {
        return Some(Mode::InstallSkill);
    }
    // The agent-state integration flags: install/uninstall/status for the
    // claude/codex/grok/muse/droid/copilot/cursor session hooks and the pi/omp extensions this
    // binary owns. No socket, no daemon, so they run anywhere.
    if cli.install_agent_hooks {
        return Some(Mode::AgentHooks(AgentHookAction::Install));
    }
    if cli.uninstall_agent_hooks {
        return Some(Mode::AgentHooks(AgentHookAction::Uninstall));
    }
    if cli.agent_hooks_status {
        return Some(Mode::AgentHooksStatus);
    }

    if cli.integration_status {
        return Some(Mode::IntegrationStatus);
    }
    if cli.install_agent_extensions {
        return Some(Mode::AgentExtensions(true));
    }
    if cli.heal_agent_hooks {
        return Some(Mode::HealAgentHooks);
    }
    if cli.uninstall_agent_extensions {
        return Some(Mode::AgentExtensions(false));
    }
    None
}

/// Run the resolved mode: the one dispatch match.
pub(super) fn run(mut cli: Cli, mode: Mode) -> ExitCode {
    if mode.installs_logger() {
        // The mux library logs through `log` (ARC-010); without a logger
        // installed those records vanish, so wire the minimal stderr sink up
        // before anything can emit.
        log::set_logger(&STDERR_LOG)
            .map(|_| log::set_max_level(log::LevelFilter::Debug))
            .ok();
    }
    match mode {
        Mode::PrintSchema => {
            print!("{}", par_mux::mux::command::COMMAND_SCHEMA_JSON);
            ExitCode::SUCCESS
        }
        Mode::Completions(shell) => {
            print!("{}", completion_script(shell));
            ExitCode::SUCCESS
        }
        Mode::MachineRelay(label) => run_machine_relay(&label),
        Mode::Status(path) => run_status(&path),
        Mode::Ls(path) => run_command(&path, "list-sessions"),
        Mode::Machine(args) => run_machine_command(args),
        Mode::Plugin(args) => run_plugin_command(args),
        Mode::Update(args, socket) => run_update_command(args, socket),
        Mode::PaneStream { args, control } => run_pane_stream(&mut cli, args, control),
        #[cfg(feature = "attach")]
        Mode::Attach(attach) => run_attach(&cli, &attach),
        Mode::GenConfig => run_gen_config(&cli),
        Mode::InstallSkill => run_install_skill(&cli),
        Mode::AgentHooks(action) => run_agent_hook_action(action),
        Mode::AgentHooksStatus => run_agent_hooks_status(),
        Mode::IntegrationStatus => run_integration_status(),
        Mode::AgentExtensions(install) => run_agent_extension_action(install),
        Mode::HealAgentHooks => {
            par_mux::mux::integrations::heal_hook_assets();
            ExitCode::SUCCESS
        }
        Mode::SelfUpdate(path) => {
            update_cli::run_self_update(cli.check_update, cli.force_self_update, &path)
        }
        Mode::Command(command, path) => run_command(&path, &command),
        Mode::ListServers => run_list_servers(&cli),
        Mode::Daemon(path) => match run_daemon(cli, path) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("Error: {err}");
                ExitCode::FAILURE
            }
        },
    }
}
