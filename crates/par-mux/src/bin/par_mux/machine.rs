//! The `--machine` relay and the `machine` subcommand handlers: the SSH
//! thin client's two halves. The profiles live in `[machines.<label>]`
//! (see `mux::machine_config`); ssh owns every credential.

use std::process::ExitCode;

use par_mux::mux::config::{config_file_path, MachineProfile};
use par_mux::mux::machine_config::{machines_from, remove_machine, set_machine, ssh_relay_command};

use super::{MachineArgs, MachineCommand};

/// The config file's path for machine edits — resolved the same way every
/// other config read resolves it (the `PAR_MUX_CONFIG` override included).
fn config_path_or_err() -> Result<std::path::PathBuf, String> {
    config_file_path().ok_or_else(|| "no config file location resolves on this machine".to_string())
}

/// `par-mux machine <VERB>`. Errors print to stderr and exit 2; every
/// success is one line on stdout.
pub(super) fn run_machine_command(args: MachineArgs) -> ExitCode {
    match run_machine(args.command) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("par-mux: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_machine(command: MachineCommand) -> Result<String, String> {
    let path = config_path_or_err()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    match command {
        MachineCommand::Add {
            target,
            label,
            session,
        } => {
            let label = label.unwrap_or_else(|| default_label(&target));
            let profile = MachineProfile {
                target: target.clone(),
                session,
                enabled: true,
            };
            let updated = set_machine(&text, &label, &profile)?;
            write_config(&path, &updated)?;
            Ok(format!("machine {label} saved: {target}\n"))
        }
        MachineCommand::List => {
            let machines = machines_from(&text)?;
            if machines.is_empty() {
                return Ok("no saved machines; add one with par-mux machine add <target>\n".into());
            }
            let mut out = String::new();
            for (label, profile) in machines {
                let session = profile.session.as_deref().unwrap_or("-");
                let state = if profile.enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                out.push_str(&format!(
                    "{label}  {session}  {state}  {}\n",
                    profile.target
                ));
            }
            Ok(out)
        }
        MachineCommand::Remove { label } => {
            let updated = remove_machine(&text, &label)?;
            write_config(&path, &updated)?;
            Ok(format!("machine {label} removed\n"))
        }
        MachineCommand::Rename { label, to } => {
            let mut machines = machines_from(&text)?;
            let profile = machines
                .remove(&label)
                .ok_or_else(|| unknown(&label, &machines))?;
            let mut updated = remove_machine(&text, &label)?;
            updated = set_machine(&updated, &to, &profile)?;
            write_config(&path, &updated)?;
            Ok(format!("machine {label} renamed to {to}\n"))
        }
        MachineCommand::Enable { label } => set_enabled(&text, &path, &label, true),
        MachineCommand::Disable { label } => set_enabled(&text, &path, &label, false),
    }
}

fn set_enabled(
    text: &str,
    path: &std::path::Path,
    label: &str,
    enabled: bool,
) -> Result<String, String> {
    let machines = machines_from(text)?;
    if !machines.contains_key(label) {
        return Err(unknown(label, &machines));
    }
    let mut profile = machines.get(label).cloned().unwrap();
    profile.enabled = enabled;
    let updated = set_machine(text, label, &profile)?;
    write_config(path, &updated)?;
    Ok(format!(
        "machine {label} {}\n",
        if enabled { "enabled" } else { "disabled" }
    ))
}

fn write_config(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    let mut temp = path.as_os_str().to_os_string();
    temp.push(format!(".tmp-{}", std::process::id()));
    let temp = std::path::PathBuf::from(temp);
    std::fs::write(&temp, contents).map_err(|err| format!("{}: {err}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|err| format!("{}: {err}", path.display()))
}

fn unknown(label: &str, machines: &std::collections::BTreeMap<String, MachineProfile>) -> String {
    let known: Vec<&str> = machines.keys().map(String::as_str).collect();
    if known.is_empty() {
        format!("no saved machine {label:?} (none are saved yet)")
    } else {
        format!("no saved machine {label:?} (known: {})", known.join(", "))
    }
}

/// The host part of an ssh target — `user@host:port` → `host` — as the
/// default label for `machine add`.
fn default_label(target: &str) -> String {
    // Strip `:port` only when it is exactly one colon plus digits — an
    // IPv6 literal or a malformed target keeps its full spelling.
    let without_port = target.rsplit_once(':').map_or(target, |(host, port)| {
        if !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()) && !host.contains(':') {
            host
        } else {
            target
        }
    });
    let host = without_port
        .rsplit_once('@')
        .map_or(without_port, |(_, host)| host);
    if host.is_empty() {
        target.to_string()
    } else {
        host.to_string()
    }
}

/// The `--machine <LABEL>` relay: re-run this invocation on the machine's
/// host through ssh. The relayed command line is the original argv minus
/// the binary name and the `--machine` pair, so every flag crosses
/// verbatim; `attach` forces `--mode passthrough` (the render TUI needs a
/// local terminal driver; the passthrough client under `ssh -tt` is the
/// thin client). One-shot commands run under `BatchMode=yes`: a scripted
/// relay fails fast instead of hanging on a prompt.
pub(super) fn run_machine_relay(label: &str) -> ExitCode {
    match machine_relay(label) {
        Ok(mut child) => match child.wait() {
            Ok(status) => match status.code() {
                Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
                None => ExitCode::FAILURE,
            },
            Err(err) => {
                eprintln!("par-mux: ssh relay failed: {err}");
                ExitCode::from(2)
            }
        },
        Err(err) => {
            eprintln!("par-mux: {err}");
            ExitCode::from(2)
        }
    }
}

fn machine_relay(label: &str) -> Result<std::process::Child, String> {
    // The local-only verbs have no remote meaning; refuse them up front
    // with the reason instead of a confusing remote error.
    let raw: Vec<String> = std::env::args().skip(1).collect();
    for arg in &raw {
        if arg == "update" {
            return Err("update manages the machine it runs on; ssh in and run it there".into());
        }
        if matches!(
            arg.as_str(),
            "--gen-config"
                | "--install-agent-hooks"
                | "--uninstall-agent-hooks"
                | "--install-agent-extensions"
                | "--uninstall-agent-extensions"
                | "--install-skill"
                | "--agent-hooks-status"
                | "--integration-status"
                | "--heal-agent-hooks"
                | "--foreground"
                | "--restart"
        ) {
            return Err(format!(
                "{arg} manages this machine; it does not relay over --machine"
            ));
        }
    }

    let path = config_path_or_err()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let machines = machines_from(&text)?;
    let profile = machines
        .get(label)
        .ok_or_else(|| unknown(label, &machines))?;

    let mut relayed = Vec::new();
    let mut skip_next = false;
    let mut attach = false;
    for arg in &raw {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--machine" {
            skip_next = true;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--machine=") {
            if rest == label {
                continue;
            }
        }
        if arg == "attach" {
            attach = true;
        }
        relayed.push(arg.clone());
    }
    let mut interactive = false;
    if attach {
        interactive = true;
        // The thin client rides passthrough; an explicit --mode wins.
        let mode_given = relayed
            .iter()
            .any(|arg| arg == "--mode" || arg.starts_with("--mode="));
        if !mode_given {
            relayed.push("--mode".into());
            relayed.push("passthrough".into());
        }
    }

    let command = ssh_relay_command(&profile.target, &relayed, interactive);
    std::process::Command::new(&command[0])
        .args(&command[1..])
        .spawn()
        .map_err(|err| format!("cannot run ssh (is it installed?): {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_default_to_the_target_host() {
        assert_eq!(default_label("paul@10.0.0.4:2222"), "10.0.0.4");
        assert_eq!(default_label("lab"), "lab");
        assert_eq!(default_label("paul@lab"), "lab");
        assert_eq!(default_label("host:port:weird"), "host:port:weird");
    }
}
