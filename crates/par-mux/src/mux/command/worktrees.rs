//! Git worktree workspace command parsers. Paths and names are read
//! through the quoting grammar (`Args::quoted_flag`) so they may contain
//! spaces; nothing here touches git — values are validated daemon-side.

use super::*;

/// `worktree-create -c <dir> -b <branch> [-B base] [-p path] [-s name]`.
pub(super) fn parse_worktree_create(a: &Args<'_>) -> Result<MuxCommand, String> {
    // D7 (docs/MUX_DECISIONS.md): the branch is `-b`, as in `git worktree
    // add -b`; `-n` is the NAME flag on `new-workspace`.
    if a.has_flag("-n") {
        return Err("worktree-create: the branch flag is -b <branch> (-n is not accepted)".into());
    }
    reject_positionals(a, &["-c", "-b", "-B", "-p", "-s"])?;
    let repo_dir = a
        .quoted_flag("-c")?
        .ok_or("worktree-create requires -c <repo dir>")?;
    let branch = a
        .quoted_flag("-b")?
        .ok_or("worktree-create requires -b <branch>")?;
    Ok(MuxCommand::WorktreeCreate {
        repo_dir,
        branch,
        base: a.quoted_flag("-B")?,
        path: a.quoted_flag("-p")?,
        name: a.quoted_flag("-s")?,
    })
}

/// `worktree-open -c <checkout> [-s name]`.
pub(super) fn parse_worktree_open(a: &Args<'_>) -> Result<MuxCommand, String> {
    reject_positionals(a, &["-c", "-s"])?;
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
    reject_positionals(a, &["-t"])?;
    Ok(MuxCommand::WorktreeRemove {
        workspace: a.workspace("-t")?,
        force: a.has_flag("-f"),
    })
}

pub(super) fn parse_list_worktrees(a: &Args<'_>) -> Result<MuxCommand, String> {
    reject_positionals(a, &[])?;
    Ok(MuxCommand::ListWorktrees)
}
