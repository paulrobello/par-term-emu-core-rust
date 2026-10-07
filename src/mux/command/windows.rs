//! Window command parsers.

use super::*;

/// tmux's value-taking `new-window` flags (QA-219).
pub(super) const NEW_WINDOW_VALUE_FLAGS: &[&str] = &["-t", "-n", "-c", "-e", "-F"];

pub(super) fn parse_new_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    let target = a.any_target_opt("-t", "session")?;
    let name = a.quoted_flag("-n")?;
    let start_dir = a.quoted_flag("-c")?;
    reject_positionals(a, NEW_WINDOW_VALUE_FLAGS)?;
    Ok(MuxCommand::NewWindow {
        target,
        name,
        start_dir,
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

pub(super) fn parse_rename_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    let window = a.window("-t")?;
    let name = a.trailing_after("-t");
    if name.is_empty() {
        return Err("rename-window requires a new name".to_string());
    }
    Ok(MuxCommand::RenameWindow { window, name })
}

pub(super) fn parse_list_windows(a: &Args<'_>) -> Result<MuxCommand, String> {
    // `-t <session>` scopes the listing to one session (tmux's
    // `list-windows -t <session>`); the bare form stays global.
    let session = a.session("-t")?;
    reject_positionals(a, LIST_WINDOWS_VALUE_FLAGS)?;
    Ok(MuxCommand::ListWindows { session })
}

/// tmux's value-taking `list-windows` flags (QA-219's rule), same shape as
/// `LIST_PANES_VALUE_FLAGS`.
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
