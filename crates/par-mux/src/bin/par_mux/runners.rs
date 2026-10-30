//! The client-side runners `main` dispatches to: every invocation that
//! talks to (or merely names) a daemon without serving one — `--cmd`,
//! `status`, `--list-servers`, `observe`/`control`, `attach`, and the
//! no-socket one-shots `--gen-config`/`--install-skill` — plus the socket
//! resolution and reserved-name refusal they share.

use par_mux::mux::config::{self, write_file};

#[cfg(feature = "attach")]
use super::AttachCommand;
use super::{effective, Cli, StreamArgs};

/// `par-mux observe|control`: resolve the socket like every client verb,
/// then hand the stream to the library runner.
pub(super) fn run_pane_stream(
    cli: &mut Cli,
    args: StreamArgs,
    control: bool,
) -> std::process::ExitCode {
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
/// daemon owns the socket, 2 on an empty command or a transport failure.
pub(super) fn run_command(path: &std::path::Path, command: &str) -> std::process::ExitCode {
    use std::io::Write;
    use std::process::ExitCode;
    if command.trim().is_empty() {
        eprintln!("par-mux: empty command");
        return ExitCode::from(2);
    }
    if command.trim() == "help" {
        let mut out = std::io::stdout().lock();
        for name in par_mux::mux::command::command_names() {
            if writeln!(out, "{name}").is_err() {
                break;
            }
        }
        return ExitCode::SUCCESS;
    }
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
pub(super) fn run_list_servers(cli: &Cli) -> std::process::ExitCode {
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

/// The reserved daemon name "server" (`par-mux server stop` reads as
/// "stop the server" but addresses a daemon literally named "server" on
/// `par-mux-server.sock` — Paul, 2026-10-18). Returns true (after
/// printing the explanation) when the invocation must be refused; an
/// explicit `--socket` reads as deliberate and is allowed, and attach
/// keeps resolving any NAME normally.
pub(super) fn refuse_reserved_server_name(
    name: Option<&str>,
    socket: Option<&std::path::PathBuf>,
) -> bool {
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
pub(super) fn resolve_cli_socket(cli: &Cli) -> std::path::PathBuf {
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
pub(super) fn run_status(path: &std::path::Path) -> std::process::ExitCode {
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

/// par-mux attach: subcommand form — run the attach client and exit
/// with its code. The parsed --mode/--prefix ride above the config
/// file (flags > file); the mode/prefix/reload defaults come from the
/// resolved config; socket resolution lives in
/// AttachOptions::socket_path (same precedence as the daemon/--cmd).
#[cfg(feature = "attach")]
pub(super) fn run_attach(cli: &Cli, attach: &AttachCommand) -> std::process::ExitCode {
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
    let eff = effective(cli, true);
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
        let base = discovery_base(cli);
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
    par_mux::mux::attach::run_with_mode(&options, mode)
}

/// --gen-config: write the effective config and exit. Resolution uses
/// the same function serve mode runs, so the generated file IS what
/// the daemon would resolve (modulo the socket flag tier, which this
/// path does see).
pub(super) fn run_gen_config(cli: &Cli) -> std::process::ExitCode {
    let eff = effective(cli, true);
    let Some(path) = config::config_file_path() else {
        eprintln!("par-mux: no config directory known (set PAR_MUX_CONFIG or XDG_CONFIG_HOME)");
        return std::process::ExitCode::FAILURE;
    };
    if let Err(err) = write_file(&path, &eff, cli.force) {
        eprintln!("par-mux: {err}");
        return std::process::ExitCode::FAILURE;
    }
    println!("par-mux: wrote {}", path.display());
    std::process::ExitCode::SUCCESS
}

/// --install-skill: write the embedded agent skill and exit — no
/// socket, no daemon, so it also runs inside a pane.
pub(super) fn run_install_skill(cli: &Cli) -> std::process::ExitCode {
    let result = match cli.skills_dir.as_deref() {
        Some(dir) => par_mux::mux::skill_installer::install_skill_into(dir),
        None => par_mux::mux::skill_installer::install_skill(),
    };
    match result {
        Ok(path) => {
            println!("par-mux: installed agent skill to {}", path.display());
            std::process::ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("par-mux: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}
