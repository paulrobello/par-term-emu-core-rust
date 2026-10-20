//! Session, workspace, server, and environment command parsers.

use super::*;

/// tmux's value-taking `new-session` flags, so a tmux-shaped sender's
/// `-x 80` is not misread as a start command (QA-219). `-F`/`-f` are NOT
/// here: this parser rejects them loudly (nothing is silently dropped).
pub(super) const NEW_SESSION_VALUE_FLAGS: &[&str] = &["-s", "-e", "-c", "-n", "-t", "-x", "-y"];

/// The flags `new-session` refuses rather than ignores (R3): `-F`
/// (format) and `-f` have no meaning on a create command, and a silent
/// drop once ran the user's session without what they asked for.
pub(super) const NEW_SESSION_REJECT_FLAGS: &[&str] = &["-F", "-f"];

/// Reject the first positional word of a command that takes none (QA-219):
/// `new-window sleep 5` used to succeed and silently drop `sleep 5`.
/// Words are read with [`next_shell_word`], so a quoted value stays one
/// word; a word in `value_flags` also consumes the next word (absent is
/// fine — the flag parsers report that). Any other `-x` word is tolerated
/// as a bare flag, as before. Everything after `--` is positional.
pub(super) fn reject_positionals(a: &Args<'_>, value_flags: &[&str]) -> Result<(), String> {
    let rest = a.line.trim_start().strip_prefix(a.name).unwrap_or_default();
    let mut pos = a.line.len() - rest.len();
    let mut after_dashes = false;
    while let Some((_, end, word)) = next_shell_word(a.line, pos) {
        pos = end;
        if !after_dashes && word == "--" {
            after_dashes = true;
            continue;
        }
        if after_dashes || !(word.len() > 1 && word.starts_with('-')) {
            return Err(format!(
                "{}: unexpected argument {word:?}: a start command is not supported; use respawn-pane",
                a.name
            ));
        }
        if value_flags.contains(&word.as_str()) {
            if let Some((_, value_end, _)) = next_shell_word(a.line, pos) {
                pos = value_end;
            }
        }
    }
    Ok(())
}

pub(super) fn parse_new_session(a: &Args<'_>) -> Result<MuxCommand, String> {
    let (command, a) = a.start_command(NEW_SESSION_VALUE_FLAGS, NEW_SESSION_REJECT_FLAGS)?;
    let env = a
        .quoted_values("-e")
        .into_iter()
        .map(|assignment| {
            let (name, value) = assignment
                .split_once('=')
                .ok_or_else(|| format!("new-session: -e expects NAME=VALUE, got: {assignment}"))?;
            validate_env_name(name, a.name)?;
            Ok((name.to_string(), value.to_string()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let name = a.quoted_flag("-s")?;
    let workspace = a.workspace_opt("-t")?;
    // `-n`/`-c`/`-x`/`-y` gain their tmux meanings here (R3/R4): the
    // first window's name, the first pane's start directory, and the
    // initial grid extents — parity with `new-window`'s flag set.
    let window_name = a.quoted_flag("-n")?;
    let start_dir = a.quoted_flag("-c")?;
    let cols = a.size("-x")?;
    let rows = a.size("-y")?;
    Ok(MuxCommand::NewSession {
        name,
        env,
        workspace,
        window_name,
        start_dir,
        cols,
        rows,
        command,
    })
}

/// `has-session -t <session>`: the bare existence probe.
pub(super) fn parse_has_session(a: &Args<'_>) -> Result<MuxCommand, String> {
    let session = a
        .session("-t")?
        .ok_or_else(|| format!("{} requires -t", a.name))?;
    reject_positionals(a, &["-t"])?;
    Ok(MuxCommand::HasSession { session })
}

/// `show-options`: dump the applied daemon settings. No flags.
pub(super) fn parse_show_options(a: &Args<'_>) -> Result<MuxCommand, String> {
    reject_positionals(a, &[])?;
    Ok(MuxCommand::ShowOptions)
}

/// `set-option <key> <value>`: exactly two positional words.
pub(super) fn parse_set_option(a: &Args<'_>) -> Result<MuxCommand, String> {
    let words = shell_split(a.line);
    let mut positional = Vec::new();
    let mut iter = words.iter().skip(1);
    while let Some(word) = iter.next() {
        match word.as_str() {
            "-t" => {
                iter.next();
            }
            _ => positional.push(word.clone()),
        }
    }
    let [key, value] = positional.as_slice() else {
        return Err(format!("{} requires <key> <value>", a.name));
    };
    Ok(MuxCommand::SetOption {
        key: key.clone(),
        value: value.clone(),
    })
}

/// Reject a variable name no environment can hold: empty, or containing
/// `=` or NUL.
pub(super) fn validate_env_name(name: &str, command: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('=') || name.contains('\0') {
        return Err(format!(
            "{command}: invalid environment variable name: {name:?}"
        ));
    }
    Ok(())
}

/// `set-environment -t $N NAME VALUE` or `set-environment -t $N -u NAME`.
/// Flags lead ([`split_leading_flags`]), so a VALUE that is literally `-u`
/// or `-t` stays the value. The words after the flags go through
/// [`shell_split`], so a quoted value may contain spaces; exactly one value
/// word is accepted.
pub(super) fn parse_set_environment(a: &Args<'_>) -> Result<MuxCommand, String> {
    use LeadingFlag::{Bare, Valued};
    let lead = split_leading_flags(a.line, a.name, &[Valued("-t"), Bare("-u")])?;
    let raw = match lead.value("-t") {
        None => return Err(format!("{} requires -t", a.name)),
        Some("") => return Err(format!("{}: -t requires a non-empty name", a.name)),
        Some(raw) => raw,
    };
    let session = Target::parse(raw).map_err(|_| format!("invalid session target: {raw}"))?;
    let unset = lead.has("-u");
    let positional = shell_split(lead.tail);
    let (name, value) = match (unset, positional.as_slice()) {
        (true, [name]) => (name.clone(), None),
        (false, [name, value]) => (name.clone(), Some(value.clone())),
        (true, _) => return Err(format!("{}: -u takes exactly one NAME", a.name)),
        (false, _) => return Err(format!("{} requires NAME VALUE", a.name)),
    };
    validate_env_name(&name, a.name)?;
    if value.as_deref().is_some_and(|v| v.contains('\0')) {
        return Err(format!("{}: value contains NUL", a.name));
    }
    Ok(MuxCommand::SetEnvironment {
        session,
        name,
        value,
    })
}

pub(super) fn parse_list_sessions(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ListSessions {
        workspace: a.workspace_opt("-t")?,
        format: a.quoted_flag("-F")?,
    })
}

/// `new-workspace [-n name]` — create and select a workspace.
pub(super) fn parse_new_workspace(a: &Args<'_>) -> Result<MuxCommand, String> {
    let name = a.quoted_flag("-n")?;
    reject_positionals(a, &["-n"])?;
    Ok(MuxCommand::NewWorkspace { name })
}

pub(super) fn parse_list_workspaces(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ListWorkspaces)
}

pub(super) fn parse_select_workspace(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::SelectWorkspace {
        workspace: a.workspace("-t")?,
    })
}

/// `switch-client [-t <session|window|pane>]` — move the displayed session
/// (tmux's client-session switch; par-mux has one shared display). Bare,
/// it is the read-only query for the displayed session.
pub(super) fn parse_switch_client(a: &Args<'_>) -> Result<MuxCommand, String> {
    // Only `-t` exists: tmux's other switch-client flags (-n/-p/-l/…) must
    // fail loudly, not silently degrade into the read-only query.
    let rest = a.line.trim_start().strip_prefix(a.name).unwrap_or_default();
    let mut pos = a.line.len() - rest.len();
    while let Some((_, end, word)) = next_shell_word(a.line, pos) {
        pos = end;
        if word != "-t" {
            return Err(format!("{}: unexpected argument {word:?}", a.name));
        }
        if let Some((_, value_end, _)) = next_shell_word(a.line, pos) {
            pos = value_end;
        }
    }
    Ok(MuxCommand::SwitchClient {
        target: a.any_target_opt("-t", "session")?,
    })
}

/// `rename-workspace -t +N <name>` — the same single-name grammar
/// `rename-session` uses (shell_split, exactly one name word).
pub(super) fn parse_rename_workspace(a: &Args<'_>) -> Result<MuxCommand, String> {
    let workspace = a.workspace("-t")?;
    let words = shell_split(a.line);
    let mut positional = Vec::new();
    let mut iter = words.iter().skip(1);
    while let Some(word) = iter.next() {
        match word.as_str() {
            "-t" => {
                iter.next();
            }
            _ => positional.push(word.clone()),
        }
    }
    match positional.len() {
        0 => Err("rename-workspace requires a new name".to_string()),
        1 => Ok(MuxCommand::RenameWorkspace {
            workspace,
            name: positional.remove(0),
        }),
        _ => Err("rename-workspace takes exactly one name".to_string()),
    }
}

/// `set-workspace-respawn -t +N [on|off|toggle]` — at most one value
/// word; none is the query form.
pub(super) fn parse_set_workspace_respawn(a: &Args<'_>) -> Result<MuxCommand, String> {
    let workspace = a.workspace("-t")?;
    let words = shell_split(a.line);
    let mut positional = Vec::new();
    let mut iter = words.iter().skip(1);
    while let Some(word) = iter.next() {
        match word.as_str() {
            "-t" => {
                iter.next();
            }
            _ => positional.push(word.clone()),
        }
    }
    let change = match positional.as_slice() {
        [] => None,
        [value] => Some(match value.as_str() {
            "on" => RespawnChange::On,
            "off" => RespawnChange::Off,
            "toggle" => RespawnChange::Toggle,
            other => {
                return Err(format!(
                    "set-workspace-respawn: expected on, off, or toggle, got {other:?}"
                ))
            }
        }),
        _ => return Err("set-workspace-respawn takes at most one value".to_string()),
    };
    Ok(MuxCommand::SetWorkspaceRespawn { workspace, change })
}

/// `kill-workspace -t +N [-g]`.
pub(super) fn parse_kill_workspace(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::KillWorkspace {
        workspace: a.workspace("-t")?,
        group: a.has_flag("-g"),
    })
}

pub(super) fn parse_kill_server(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::KillServer)
}

/// `rename-session -t $N <name>`. The name goes through [`shell_split`],
/// so a quoted name may contain spaces; exactly one name word is accepted
/// (the same grammar `set-environment` uses for its value).
pub(super) fn parse_rename_session(a: &Args<'_>) -> Result<MuxCommand, String> {
    let session = a
        .session("-t")?
        .ok_or_else(|| format!("{} requires -t", a.name))?;
    let words = shell_split(a.line);
    let mut positional = Vec::new();
    let mut iter = words.iter().skip(1);
    while let Some(word) = iter.next() {
        match word.as_str() {
            "-t" => {
                iter.next();
            }
            _ => positional.push(word.clone()),
        }
    }
    match positional.len() {
        0 => Err("rename-session requires a new name".to_string()),
        1 => Ok(MuxCommand::RenameSession {
            session,
            name: positional.remove(0),
        }),
        _ => Err("rename-session takes exactly one name".to_string()),
    }
}

/// `kill-session -t $N`.
pub(super) fn parse_kill_session(a: &Args<'_>) -> Result<MuxCommand, String> {
    let session = a
        .session("-t")?
        .ok_or_else(|| format!("{} requires -t", a.name))?;
    Ok(MuxCommand::KillSession { session })
}
