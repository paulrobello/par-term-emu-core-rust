//! Named-layout command parsers: save-layout, load-layout, list-layouts,
//! kill-layout. Names are validated daemon-side against the layout store's
//! grammar, so the parsers only split words and flags.

use super::*;

/// The one positional name of a layout verb, with `-f` accepted when
/// `allow_force`. Layout names never contain whitespace, so the
/// whitespace-split tokens are the words.
fn name_and_force(a: &Args<'_>, allow_force: bool) -> Result<(String, bool), String> {
    let mut force = false;
    let mut name = None;
    for word in a.args {
        if allow_force && *word == "-f" {
            force = true;
        } else if word.starts_with('-') && name.is_none() && word.len() > 1 {
            return Err(format!("{}: unknown flag {word}", a.name));
        } else if name.replace((*word).to_string()).is_some() {
            return Err(format!("{} takes exactly one layout name", a.name));
        }
    }
    let name = name.ok_or_else(|| format!("{} requires a layout name", a.name))?;
    Ok((name, force))
}

/// `save-layout <name>`.
pub(super) fn parse_save_layout(a: &Args<'_>) -> Result<MuxCommand, String> {
    let (name, _) = name_and_force(a, false)?;
    Ok(MuxCommand::SaveLayout { name })
}

/// `load-layout [-f] <name>`.
pub(super) fn parse_load_layout(a: &Args<'_>) -> Result<MuxCommand, String> {
    let (name, force) = name_and_force(a, true)?;
    Ok(MuxCommand::LoadLayout { name, force })
}

/// `list-layouts`.
pub(super) fn parse_list_layouts(a: &Args<'_>) -> Result<MuxCommand, String> {
    reject_positionals(a, &[])?;
    Ok(MuxCommand::ListLayouts)
}

/// `kill-layout <name>`.
pub(super) fn parse_kill_layout(a: &Args<'_>) -> Result<MuxCommand, String> {
    let (name, _) = name_and_force(a, false)?;
    Ok(MuxCommand::KillLayout { name })
}
