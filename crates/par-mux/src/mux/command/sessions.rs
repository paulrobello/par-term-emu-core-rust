//! Session, workspace, server, and environment command parsers.

use super::*;

/// tmux's value-taking `new-session` flags, so a tmux-shaped sender's
/// `-x 80` is not misread as a start command (QA-219).
pub(super) const NEW_SESSION_VALUE_FLAGS: &[&str] =
    &["-s", "-e", "-c", "-n", "-t", "-x", "-y", "-F", "-f"];

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
    reject_positionals(a, NEW_SESSION_VALUE_FLAGS)?;
    Ok(MuxCommand::NewSession {
        name,
        env,
        workspace,
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
/// The words after the flags go through [`shell_split`], so a quoted value
/// may contain spaces; exactly one value word is accepted.
pub(super) fn parse_set_environment(a: &Args<'_>) -> Result<MuxCommand, String> {
    let session = a
        .session("-t")?
        .ok_or_else(|| format!("{} requires -t", a.name))?;
    let unset = a.has_flag("-u");
    let words = shell_split(a.line);
    let mut positional = Vec::new();
    let mut iter = words.iter().skip(1);
    while let Some(word) = iter.next() {
        match word.as_str() {
            "-t" => {
                iter.next();
            }
            "-u" => {}
            _ => positional.push(word.clone()),
        }
    }
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

/// `kill-workspace -t +N`.
pub(super) fn parse_kill_workspace(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::KillWorkspace {
        workspace: a.workspace("-t")?,
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
