//! Worktree grouping: which workspaces are git-worktree-backed and which
//! repo group each belongs to. Grouping is DERIVED from the link's repo
//! key (the repo's canonical main-checkout path), never stored as a parent
//! workspace id: a group survives its repo workspace's death (the linked
//! members just have no open parent until the next `worktree-create` /
//! `worktree-open` for that repo opens one), and ids are never reused, so
//! no stale parent pointer can exist.

use super::{MuxTree, WorktreeLink};
use crate::mux::ids::WorkspaceId;
use crate::mux::pane::MuxError;
use std::path::Path;

impl MuxTree {
    /// Set or clear a workspace's worktree link.
    pub fn set_worktree_link(
        &mut self,
        workspace_id: WorkspaceId,
        link: Option<WorktreeLink>,
    ) -> Result<(), MuxError> {
        self.workspaces
            .get_mut(&workspace_id)
            .ok_or(MuxError::NoSuchWorkspace(workspace_id))?
            .worktree = link;
        Ok(())
    }

    /// The open repo workspace for `repo` (a main-checkout key) — the
    /// lowest id when several claim it.
    pub fn repo_workspace(&self, repo: &Path) -> Option<WorkspaceId> {
        self.workspaces
            .values()
            .filter(|ws| {
                ws.worktree
                    .as_ref()
                    .is_some_and(|link| !link.is_linked() && link.repo == repo)
            })
            .map(|ws| ws.id)
            .min()
    }

    /// The workspace linked to `checkout`, if one is open — lowest id.
    pub fn workspace_for_checkout(&self, checkout: &Path) -> Option<WorkspaceId> {
        self.workspaces
            .values()
            .filter(|ws| {
                ws.worktree
                    .as_ref()
                    .is_some_and(|link| link.checkout == checkout)
            })
            .map(|ws| ws.id)
            .min()
    }

    /// The linked-worktree workspaces grouped under `workspace_id` when it
    /// is a repo workspace, in id order; empty for any other workspace.
    pub fn linked_worktrees_of(&self, workspace_id: WorkspaceId) -> Vec<WorkspaceId> {
        let Some(repo) = self
            .workspaces
            .get(&workspace_id)
            .and_then(|ws| ws.worktree.as_ref())
            .filter(|link| !link.is_linked())
            .map(|link| link.repo.clone())
        else {
            return Vec::new();
        };
        let mut ids: Vec<WorkspaceId> = self
            .workspaces
            .values()
            .filter(|ws| {
                ws.worktree
                    .as_ref()
                    .is_some_and(|link| link.is_linked() && link.repo == repo)
            })
            .map(|ws| ws.id)
            .collect();
        ids.sort();
        ids
    }

    /// `kill-workspace`'s tree half with the group rule: a repo workspace
    /// with open linked worktree workspaces is refused
    /// ([`MuxError::GroupCloseRequired`]) unless `group`, which kills the
    /// linked members too (repo workspace first). Any other workspace —
    /// `-g` or not — is killed alone. Returns the killed windows in kill
    /// order. Checkouts on disk are untouched.
    pub fn kill_workspace_grouped(
        &mut self,
        workspace_id: WorkspaceId,
        group: bool,
    ) -> Result<Vec<crate::mux::ids::WindowId>, MuxError> {
        if !self.workspaces.contains_key(&workspace_id) {
            return Err(MuxError::NoSuchWorkspace(workspace_id));
        }
        let linked = self.linked_worktrees_of(workspace_id);
        if !linked.is_empty() && !group {
            return Err(MuxError::GroupCloseRequired(workspace_id, linked));
        }
        let mut killed = self.kill_workspace(workspace_id)?;
        for member in linked {
            killed.extend(self.kill_workspace(member)?);
        }
        Ok(killed)
    }

    /// Whether any pane in the workspace carries an agent claim — exactly
    /// the panes `list-agents` lists (an `agent` plus any `agent_state`).
    /// An `idle` or `done` agent is still a live process holding the
    /// checkout, so every state counts.
    pub fn workspace_has_agent_claim(&self, workspace_id: WorkspaceId) -> bool {
        let Some(workspace) = self.workspaces.get(&workspace_id) else {
            return false;
        };
        workspace
            .sessions
            .iter()
            .filter_map(|s| self.sessions.get(s))
            .flat_map(|s| s.windows.iter())
            .filter_map(|w| self.windows.get(w))
            .flat_map(|w| w.panes())
            .filter_map(|p| self.panes.get(&p))
            .any(|pane| {
                let meta = pane.metadata();
                meta.contains_key("agent") && meta.contains_key("agent_state")
            })
    }
}
