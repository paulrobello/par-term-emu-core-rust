//! Window command parsers.

use super::*;

/// tmux's value-taking `new-window` flags (QA-219). `-F` and `-e` are not
/// here: the parser rejects them loudly ([`NEW_WINDOW_REJECT_FLAGS`]).
pub(super) const NEW_WINDOW_VALUE_FLAGS: &[&str] = &["-t", "-n", "-c"];

/// The flags `new-window` refuses rather than ignores. `-e` is refused,
/// not implemented: a pane's environment is its SESSION's (`respawn-pane`
/// and restore re-seed it from there), so a per-window variable would
/// vanish on the first respawn.
pub(super) const NEW_WINDOW_REJECT_FLAGS: &[(&str, &str)] = &[
    ("-F", ""),
    (
        "-e",
        " (a pane's environment is its session's: set it with set-environment, or new-session -e)",
    ),
];

pub(super) fn parse_new_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    // The trailing start command (R4): tmux's positional form, the raw
    // text after the flags.
    let (command, a) = a.start_command(NEW_WINDOW_VALUE_FLAGS, NEW_WINDOW_REJECT_FLAGS)?;
    let target = a.any_target_opt("-t", "session")?;
    let name = a.quoted_flag("-n")?;
    let start_dir = a.quoted_flag("-c")?;
    Ok(MuxCommand::NewWindow {
        target,
        name,
        start_dir,
        command,
    })
}

pub(super) fn parse_select_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::SelectWindow {
        window: a.window("-t")?,
    })
}

pub(super) fn parse_kill_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::KillWindow {
        window: a.window("-t")?,
    })
}

/// `rename-window -t <window> [--] <name>`: the name is the rest of the
/// line after the target's value (read as a shell word, so a quoted target
/// is skipped whole), whitespace-split and rejoined with single spaces —
/// rename clients send the raw name, so quoting would become part of it.
/// A leading `--` ends the flags and is not part of the name.
pub(super) fn parse_rename_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    let window = a.window("-t")?;
    let rest = a.line.trim_start().strip_prefix(a.name).unwrap_or_default();
    let mut pos = a.line.len() - rest.len();
    let mut tail = "";
    while let Some((_, end, word)) = next_shell_word(a.line, pos) {
        pos = end;
        if word == "-t" {
            tail =
                next_shell_word(a.line, pos).map_or("", |(_, value_end, _)| &a.line[value_end..]);
            break;
        }
    }
    let mut words = tail.split_whitespace().peekable();
    words.next_if_eq(&"--");
    let name = words.collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("rename-window requires a new name".to_string());
    }
    Ok(MuxCommand::RenameWindow { window, name })
}

pub(super) fn parse_list_windows(a: &Args<'_>) -> Result<MuxCommand, String> {
    // `-t <session>` scopes the listing to one session (tmux's
    // `list-windows -t <session>`); the bare form stays global.
    let a = a.with_value_flags(LIST_WINDOWS_VALUE_FLAGS);
    let session = a.session("-t")?;
    let all = a.has_flag("-a");
    if all && session.is_some() {
        return Err("list-windows: -a and -t are mutually exclusive".to_string());
    }
    reject_positionals(&a, LIST_WINDOWS_VALUE_FLAGS)?;
    Ok(MuxCommand::ListWindows {
        session,
        all,
        format: a.quoted_flag("-F")?,
    })
}

/// tmux's value-taking `list-windows` flags (QA-219's rule). `-F` rides
/// here too — reject_positionals must skip its value while the parser
/// above reads the format out of it.
pub(super) const LIST_WINDOWS_VALUE_FLAGS: &[&str] = &["-t", "-F", "-f"];

pub(super) fn parse_move_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    let source = a.window("-s")?;
    let index = match a.flag("-t") {
        Some(raw) => raw
            .parse()
            .map_err(|_| format!("move-window: -t expects a position, got: {raw}"))?,
        None => return Err("move-window requires -t".to_string()),
    };
    Ok(MuxCommand::MoveWindow { source, index })
}

pub(super) fn parse_swap_windows(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::SwapWindows {
        source: a.window("-s")?,
        target: a.window("-t")?,
    })
}
