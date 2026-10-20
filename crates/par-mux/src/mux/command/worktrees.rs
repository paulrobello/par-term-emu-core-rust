//! Git worktree workspace command parsers. Paths and names are read
//! through the quoting grammar (`Args::quoted_flag`) so they may contain
//! spaces; nothing here touches git — values are validated daemon-side.

use super::*;

/// `worktree-create`'s value-taking flags.
const WORKTREE_CREATE_VALUE_FLAGS: &[&str] = &["-c", "-b", "--base", "-p", "-s"];

/// `worktree-create -c <dir> -b <branch> [--base base] [-p path] [-s name]`.
pub(super) fn parse_worktree_create(a: &Args<'_>) -> Result<MuxCommand, String> {
    let a = &a.with_value_flags(WORKTREE_CREATE_VALUE_FLAGS);
    // D7 (docs/MUX_DECISIONS.md): the branch is `-b`, as in `git worktree
    // add -b`; `-n` is the NAME flag on `new-workspace`. The start point
    // is `--base` (long-only): git's `worktree add -B` means
    // create-or-reset, and a short `-B` here would be read as that
    // (decided 2026-10-20 before the first release; `-B` is refused with
    // a pointer).
    if has_shell_flag(a, "-n", WORKTREE_CREATE_VALUE_FLAGS) {
        return Err("worktree-create: the branch flag is -b <branch> (-n is not accepted)".into());
    }
    if has_shell_flag(a, "-B", WORKTREE_CREATE_VALUE_FLAGS) {
        return Err(
            "worktree-create: the start point is --base <base> (-B is not accepted)".into(),
        );
    }
    reject_positionals(a, WORKTREE_CREATE_VALUE_FLAGS)?;
    let repo_dir = a
        .quoted_flag("-c")?
        .ok_or("worktree-create requires -c <repo dir>")?;
    let branch = a
        .quoted_flag("-b")?
        .ok_or("worktree-create requires -b <branch>")?;
    Ok(MuxCommand::WorktreeCreate {
        repo_dir,
        branch,
        base: a.quoted_flag("--base")?,
        path: a.quoted_flag("-p")?,
        name: a.quoted_flag("-s")?,
    })
}

/// Whether `flag` appears as a flag word: words are read with
/// [`next_shell_word`] and each of `value_flags` consumes its value, so a
/// `-n` inside a quoted value (`-s 'a -n b'`) or AS a value (`-s -n`) is
/// not the flag.
fn has_shell_flag(a: &Args<'_>, flag: &str, value_flags: &[&str]) -> bool {
    let rest = a.line.trim_start().strip_prefix(a.name).unwrap_or_default();
    let mut pos = a.line.len() - rest.len();
    while let Some((_, end, word)) = next_shell_word(a.line, pos) {
        pos = end;
        if word == "--" {
            return false;
        }
        if word == flag {
            return true;
        }
        if value_flags.contains(&word.as_str()) {
            if let Some((_, value_end, _)) = next_shell_word(a.line, pos) {
                pos = value_end;
            }
        }
    }
    false
}

/// `worktree-open -c <checkout> [-s name]`.
pub(super) fn parse_worktree_open(a: &Args<'_>) -> Result<MuxCommand, String> {
    let a = &a.with_value_flags(&["-c", "-s"]);
    reject_positionals(a, a.value_flags)?;
    let checkout = a
        .quoted_flag("-c")?
        .ok_or("worktree-open requires -c <checkout>")?;
    Ok(MuxCommand::WorktreeOpen {
        checkout,
        name: a.quoted_flag("-s")?,
    })
}

/// `worktree-remove -t <workspace> [-f]`.
pub(super) fn parse_worktree_remove(a: &Args<'_>) -> Result<MuxCommand, String> {
    let a = a.with_value_flags(&["-t"]);
    reject_positionals(&a, a.value_flags)?;
    Ok(MuxCommand::WorktreeRemove {
        workspace: a.workspace("-t")?,
        force: a.has_flag("-f"),
    })
}

pub(super) fn parse_list_worktrees(a: &Args<'_>) -> Result<MuxCommand, String> {
    reject_positionals(a, &[])?;
    Ok(MuxCommand::ListWorktrees)
}
