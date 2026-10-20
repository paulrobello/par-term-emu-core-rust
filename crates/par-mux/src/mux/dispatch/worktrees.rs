//! Git worktree workspace handlers: worktree-create, worktree-open,
//! worktree-remove, list-worktrees. git runs with no tree lock held (a
//! checkout of a large repo takes seconds); the tree is re-read after it.

use super::*;
use crate::mux::tree::WorktreeLink;
use crate::mux::worktree;

/// The worktree group's router; `route_command` sends only this group's
/// variants here.
pub(super) fn route_worktree_command(ctx: &Ctx<'_>, command: MuxCommand) -> Outcome {
    match command {
        MuxCommand::WorktreeCreate {
            repo_dir,
            branch,
            base,
            path,
            name,
        } => cmd_worktree_create(ctx, &repo_dir, &branch, base.as_deref(), path, name),
        MuxCommand::WorktreeOpen { checkout, name } => cmd_worktree_open(ctx, &checkout, name),
        MuxCommand::WorktreeRemove { workspace, force } => {
            cmd_worktree_remove(ctx, workspace, force)
        }
        MuxCommand::ListWorktrees => cmd_list_worktrees(ctx),
        other => unreachable!("route_command sent a non-worktree command: {other:?}"),
    }
}

/// A user-supplied directory that must exist.
fn existing_dir(raw: &str) -> Result<PathBuf, String> {
    let dir = worktree::user_path(raw)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {raw}"));
    }
    Ok(dir)
}

/// The last path component, for default workspace names.
fn base_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// `base`, or `base-2`, `base-3`, … — the first spelling no live session
/// and no workspace holds. The workspace's first session takes the same
/// name (the `new-workspace` convention), and session names are unique.
fn unique_name(tree: &MuxTree, base: &str) -> String {
    let base = crate::mux::strip_controls(base).into_owned();
    let taken =
        |n: &str| tree.session_name_taken(n) || tree.workspaces.values().any(|ws| ws.name == n);
    if !taken(&base) {
        return base;
    }
    (2u32..)
        .map(|i| format!("{base}-{i}"))
        .find(|n| !taken(n))
        .expect("an unbounded suffix search ends")
}

/// What [`open_link_workspace`] did.
struct Opened {
    id: WorkspaceId,
    /// The windows a new workspace spawned; empty when one was already
    /// open on the checkout.
    windows: Vec<WindowId>,
}

/// The workspace linked to `link.checkout`, opening one when none is: a
/// `new-workspace` (it becomes the active workspace) whose first session
/// and window start in the checkout, carrying `link`.
fn open_link_workspace(ctx: &Ctx<'_>, name: &str, link: WorktreeLink) -> Result<Opened, String> {
    let checkout = link.checkout.clone();
    let (id, plan, factory) = {
        let mut guard = ctx.tree.lock();
        if let Some(id) = guard.workspace_for_checkout(&checkout) {
            return Ok(Opened {
                id,
                windows: Vec::new(),
            });
        }
        let name = unique_name(&guard, name);
        let id = guard.new_workspace(&name);
        guard
            .set_worktree_link(id, Some(link))
            .map_err(|err| err.to_string())?;
        let plan = guard
            .begin_session_at(id, &name, DEFAULT_COLS, DEFAULT_ROWS, &Default::default())
            .map_err(|err| err.to_string())?
            .with_window_name("1")
            .with_cwd(Some(checkout));
        (id, plan, guard.factory())
    };
    let window = plan.window_id;
    match spawn_and_wire(ctx, &*factory, plan, None, None) {
        Ok(_) => Ok(Opened {
            id,
            windows: vec![window],
        }),
        Err(err) => {
            // A workspace with no session cannot be landed on; do not
            // leave one behind.
            let _ = ctx.tree.lock().kill_workspace(id);
            Err(err.to_string())
        }
    }
}

/// Open (or find) the repo workspace for `main`, then the workspace for
/// `checkout` — the same one when `checkout` is the main checkout.
/// Returns the checkout's workspace and every window spawned on the way.
fn open_grouped(
    ctx: &Ctx<'_>,
    main: &Path,
    checkout: &Path,
    name: Option<String>,
) -> Result<Opened, String> {
    let repo_name = base_name(main);
    let repo_link = WorktreeLink {
        repo: main.to_path_buf(),
        checkout: main.to_path_buf(),
        branch: worktree::current_branch(main),
    };
    let repo_ws_name = if checkout == main {
        name.clone().unwrap_or_else(|| repo_name.clone())
    } else {
        repo_name.clone()
    };
    let parent = open_link_workspace(ctx, &repo_ws_name, repo_link)?;
    if checkout == main {
        return Ok(parent);
    }
    let branch = worktree::current_branch(checkout);
    let label = branch.clone().unwrap_or_else(|| base_name(checkout));
    let link = WorktreeLink {
        repo: main.to_path_buf(),
        checkout: checkout.to_path_buf(),
        branch,
    };
    let name = name.unwrap_or_else(|| format!("{repo_name}/{label}"));
    let mut opened = open_link_workspace(ctx, &name, link)?;
    let mut windows = parent.windows;
    windows.append(&mut opened.windows);
    opened.windows = windows;
    Ok(opened)
}

/// The reply for an open: a newly opened workspace announces itself like
/// `new-workspace`; an already open one is focused like
/// `select-workspace`. The body is the workspace id either way.
fn opened_outcome(ctx: &Ctx<'_>, opened: Opened) -> Outcome {
    let body = opened.id.to_string();
    let mut result = Outcome::ok(ctx, &body);
    if !opened.windows.is_empty() {
        result = result
            .notifying(TmuxNotification::WorkspacesChanged)
            .notifying(TmuxNotification::SessionsChanged);
        for window in &opened.windows {
            result = result.notifying(window_add_notification(ctx.tree, *window));
        }
    }
    // Opening the repo workspace on the way makes IT active; the target
    // (or an already open one) is what the command focuses.
    if ctx.tree.lock().active_workspace() != Some(opened.id) {
        let selected = cmd_select_workspace(ctx, Target::Id(opened.id));
        if !selected.succeeded {
            return selected;
        }
        result.notifications.extend(selected.notifications);
        result.layout_changed.extend(selected.layout_changed);
    } else if opened.windows.is_empty() {
        result = result.notifying(TmuxNotification::WorkspacesChanged);
    }
    result
}

/// `worktree-create`: validate, `git worktree add`, then open the
/// worktree's workspace grouped under the repo's.
fn cmd_worktree_create(
    ctx: &Ctx<'_>,
    repo_dir: &str,
    branch: &str,
    base: Option<&str>,
    path: Option<String>,
    name: Option<String>,
) -> Outcome {
    let prepared = (|| {
        let dir = existing_dir(repo_dir)?;
        let main = worktree::main_checkout(&dir)?;
        worktree::validate_branch(&main, branch)?;
        let checkout = match path {
            Some(raw) => worktree::user_path(&raw)?,
            None => worktree::default_checkout_path(&main, branch),
        };
        if checkout.exists() {
            return Err(format!(
                "checkout path already exists: {}",
                checkout.display()
            ));
        }
        worktree::add(&main, &checkout, branch, base.unwrap_or("HEAD"))?;
        Ok((main, worktree::canonical(&checkout)))
    })();
    let (main, checkout) = match prepared {
        Ok(paths) => paths,
        Err(err) => return Outcome::err(ctx, &format!("worktree-create: {err}")),
    };
    match open_grouped(ctx, &main, &checkout, name) {
        Ok(opened) => opened_outcome(ctx, opened),
        Err(err) => Outcome::err(
            ctx,
            &format!(
                "worktree-create: checkout created at {} but its workspace did not open: {err}",
                checkout.display()
            ),
        ),
    }
}

/// `worktree-open`: focus the workspace open on the checkout, or open one
/// grouped under its repo's workspace.
fn cmd_worktree_open(ctx: &Ctx<'_>, checkout: &str, name: Option<String>) -> Outcome {
    let resolved = (|| {
        let dir = existing_dir(checkout)?;
        let top = worktree::toplevel(&dir)?;
        let main = worktree::main_checkout(&top)?;
        Ok::<_, String>((main, top))
    })();
    let (main, top) = match resolved {
        Ok(paths) => paths,
        Err(err) => return Outcome::err(ctx, &format!("worktree-open: {err}")),
    };
    match open_grouped(ctx, &main, &top, name) {
        Ok(opened) => opened_outcome(ctx, opened),
        Err(err) => Outcome::err(ctx, &format!("worktree-open: {err}")),
    }
}

/// `worktree-remove`: refuse a running agent or a dirty checkout unless
/// forced, `git worktree remove` the checkout, then close the workspace.
/// git runs first, so a refusal or failure leaves the workspace intact.
fn cmd_worktree_remove(ctx: &Ctx<'_>, workspace: Target<WorkspaceId>, force: bool) -> Outcome {
    let checked = {
        let guard = ctx.tree.lock();
        (|| {
            let id = guard
                .resolve_workspace_target(workspace)
                .map_err(|err| err.to_string())?;
            let link = guard
                .workspace(id)
                .and_then(|ws| ws.worktree.clone())
                .filter(WorktreeLink::is_linked)
                .ok_or_else(|| {
                    format!("workspace {id} is not a linked worktree workspace (close a repo workspace with kill-workspace)")
                })?;
            if !force && guard.workspace_has_agent_claim(id) {
                return Err(format!(
                    "workspace {id} has a pane with an agent claim; use -f to remove anyway"
                ));
            }
            Ok((id, link))
        })()
    };
    let (id, link) = match checked {
        Ok(found) => found,
        Err(err) => return Outcome::err(ctx, &format!("worktree-remove: {err}")),
    };
    if !force && worktree::is_dirty(&link.checkout).unwrap_or(false) {
        return Outcome::err(
            ctx,
            &format!(
                "worktree-remove: {} has modified or untracked files; use -f to remove anyway",
                link.checkout.display()
            ),
        );
    }
    let (removed, mut killed) = remove_checkout(ctx, id, &link, force);
    if let Err(err) = removed {
        let outcome = Outcome::err(ctx, &format!("worktree-remove: {err}"));
        return if killed.is_empty() {
            outcome
        } else {
            notify_kill_cascade(outcome, &killed, true)
        };
    }
    // The workspace closing on its own meanwhile is not a failure: the
    // checkout is gone either way.
    if let Ok(more) = ctx.tree.lock().kill_workspace(id) {
        killed.extend(more);
    }
    notify_kill_cascade(Outcome::ok(ctx, ""), &killed, true)
}

/// `git worktree remove` the link's checkout; the windows it had to close
/// on the way (Windows only) ride along so their cascade is announced.
#[cfg(not(windows))]
fn remove_checkout(
    _ctx: &Ctx<'_>,
    _id: WorkspaceId,
    link: &WorktreeLink,
    force: bool,
) -> (Result<(), String>, Vec<WindowId>) {
    (
        worktree::remove(&link.repo, &link.checkout, force),
        Vec::new(),
    )
}

/// Windows cannot delete a directory a live process has as its cwd, so a
/// first refusal there closes the workspace's panes and retries while
/// they exit.
#[cfg(windows)]
fn remove_checkout(
    ctx: &Ctx<'_>,
    id: WorkspaceId,
    link: &WorktreeLink,
    force: bool,
) -> (Result<(), String>, Vec<WindowId>) {
    let first = worktree::remove(&link.repo, &link.checkout, force);
    if first.is_ok() {
        return (first, Vec::new());
    }
    let killed = ctx.tree.lock().kill_workspace(id).unwrap_or_default();
    let mut last = first;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(150));
        last = worktree::remove(&link.repo, &link.checkout, force);
        if last.is_ok() {
            break;
        }
    }
    (last, killed)
}

/// `list-worktrees`: one `+N <kind> <parent> <branch> <checkout>` row per
/// worktree-backed workspace, id order. `kind` is `repo` or `worktree`;
/// `parent` is the open repo workspace a worktree is grouped under (`-`
/// for a repo row or when none is open); `branch` is `-` for a detached
/// HEAD. The checkout path is last because it may contain spaces.
fn cmd_list_worktrees(ctx: &Ctx<'_>) -> Outcome {
    let rows: Vec<String> = worktree_rows(&ctx.tree.lock())
        .iter()
        .map(ToString::to_string)
        .collect();
    Outcome::ok(ctx, &rows.join("\n"))
}

/// The `list-worktrees` rows, id order. Shared with `client-snapshot`'s
/// `worktree` rows, so the side panel groups by the same facts.
pub(super) fn worktree_rows(guard: &MuxTree) -> Vec<crate::mux::ipc::WorktreeRow> {
    let mut ids = guard.workspaces();
    ids.sort();
    ids.iter()
        .filter_map(|id| guard.workspace(*id))
        .filter_map(|ws| {
            let link = ws.worktree.as_ref()?;
            let parent = if link.is_linked() {
                guard.repo_workspace(&link.repo).map(|p| p.to_string())
            } else {
                None
            };
            Some(crate::mux::ipc::WorktreeRow {
                id: ws.id.to_string(),
                linked: link.is_linked(),
                parent,
                branch: link.branch.clone(),
                checkout: link.checkout.display().to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
