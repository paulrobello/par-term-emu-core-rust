//! Session, window, and pane lifecycle: creation (one-shot and two-phase),
//! respawn, renames, the session environment, and the kill cascades.

use super::{
    kill_detached, MuxSession, MuxTree, MuxWindow, MuxWorkspace, RespawnSpawn, SessionSpawn,
    WindowSpawn,
};
use crate::mux::ids::{PaneId, SessionId, WindowId, WorkspaceId};
use crate::mux::layout::LayoutTree;
use crate::mux::pane::{MuxError, MuxPane, SpawnContext};
use std::collections::BTreeMap;
use std::path::Path;

impl MuxTree {
    /// Create a workspace, making it the daemon's active one — the wire
    /// `new-workspace` (tmux has no workspace concept; this is par-mux's
    /// own level above sessions). The reply-side caller announces the new
    /// id; the session set is untouched.
    pub fn new_workspace(&mut self, name: &str) -> WorkspaceId {
        let id = self.ids.next_workspace();
        self.workspaces.insert(
            id,
            MuxWorkspace {
                id,
                name: name.to_string(),
                sessions: Vec::new(),
                active: 0,
            },
        );
        self.active_workspace = Some(id);
        id
    }

    /// Rename a workspace.
    pub fn rename_workspace(
        &mut self,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> Result<(), MuxError> {
        let workspace = self
            .workspaces
            .get_mut(&workspace_id)
            .ok_or(MuxError::NoSuchWorkspace(workspace_id))?;
        workspace.name = name.to_string();
        Ok(())
    }

    /// Make `workspace_id` the daemon's active workspace. The workspace's
    /// own active-session index is untouched — `select-workspace` resumes
    /// whichever session was active there when last selected.
    pub fn select_workspace(&mut self, workspace_id: WorkspaceId) -> Result<(), MuxError> {
        if !self.workspaces.contains_key(&workspace_id) {
            return Err(MuxError::NoSuchWorkspace(workspace_id));
        }
        self.active_workspace = Some(workspace_id);
        Ok(())
    }

    /// Kill a workspace and every session, window, and pane in it. The Ok
    /// value lists the killed windows, so the caller can emit one
    /// `%window-close` per window before the roster cues — the same line
    /// order `kill-session`'s cascade produces.
    pub fn kill_workspace(&mut self, workspace_id: WorkspaceId) -> Result<Vec<WindowId>, MuxError> {
        let workspace = self
            .workspaces
            .remove(&workspace_id)
            .ok_or(MuxError::NoSuchWorkspace(workspace_id))?;
        let mut killed = Vec::new();
        for session_id in workspace.sessions {
            let Some(session) = self.sessions.remove(&session_id) else {
                continue;
            };
            self.session_workspace.remove(&session_id);
            for window_id in session.windows {
                let Some(window) = self.remove_window(window_id) else {
                    continue;
                };
                for pane_id in window.panes() {
                    if let Some(pane) = self.panes.remove(&pane_id) {
                        kill_detached(pane);
                    }
                }
                killed.push(window_id);
            }
        }
        if self.active_workspace == Some(workspace_id) {
            // Lowest surviving id, not map order, so the fallback is
            // deterministic.
            self.active_workspace = self.workspaces.keys().min().copied();
        }
        Ok(killed)
    }

    /// Create a session, with one window holding one pane — tmux's shape.
    pub fn new_session(&mut self, name: &str, cols: u16, rows: u16) -> Result<SessionId, MuxError> {
        self.new_session_with_env(name, cols, rows, BTreeMap::new())
    }

    /// [`Self::new_session`] with an initial session environment
    /// (`new-session -e`), applied to the first pane as well.
    ///
    /// One-shot form of the two-phase flow for callers that already hold
    /// the tree exclusively (tests, embedders): reserve, spawn, complete.
    pub fn new_session_with_env(
        &mut self,
        name: &str,
        cols: u16,
        rows: u16,
        env: BTreeMap<String, String>,
    ) -> Result<SessionId, MuxError> {
        let plan = self.begin_session(name, cols, rows, &env, None);
        let pane = self.spawn_from(&plan.pane_id, plan.cols, plan.rows, None, &plan.context());
        self.complete_session(plan, pane?)
    }

    /// Phase 1 of `new-session` (ARC-022): reserve the ids under the tree
    /// lock. Infallible — fresh ids depend on no existing state. The session
    /// links to the workspace the explicit `-t` target resolves to, else
    /// the active workspace, else the lazily created default (`main`).
    pub fn begin_session(
        &mut self,
        name: &str,
        cols: u16,
        rows: u16,
        env: &BTreeMap<String, String>,
        workspace: Option<crate::mux::ids::Target<WorkspaceId>>,
    ) -> SessionSpawn {
        let workspace_id = self
            .resolve_new_session_workspace(workspace)
            .unwrap_or_else(|_| self.ensure_default_workspace());
        self.begin_session_at(workspace_id, name, cols, rows, env)
    }

    /// [`Self::begin_session`] with the workspace already resolved — the
    /// dispatcher's form, so an unknown explicit target fails the command
    /// before any id is reserved.
    pub fn begin_session_at(
        &mut self,
        workspace_id: WorkspaceId,
        name: &str,
        cols: u16,
        rows: u16,
        env: &BTreeMap<String, String>,
    ) -> SessionSpawn {
        SessionSpawn {
            session_id: self.ids.next_session(),
            window_id: self.ids.next_window(),
            pane_id: self.ids.next_pane(),
            cols,
            rows,
            name: name.to_string(),
            env: env.clone(),
            workspace_id,
        }
    }

    /// Phase 3 of `new-session`: insert the spawned pane and its window and
    /// session. Infallible by construction (fresh ids), `Err` only for
    /// shape symmetry with the other completions.
    pub fn complete_session(
        &mut self,
        plan: SessionSpawn,
        pane: MuxPane,
    ) -> Result<SessionId, MuxError> {
        let SessionSpawn {
            session_id,
            window_id,
            pane_id,
            cols,
            rows,
            name,
            env,
            workspace_id,
        } = plan;
        self.panes.insert(pane_id, pane);
        self.apply_cell_pixels(pane_id, cols, rows);
        self.sessions.insert(
            session_id,
            MuxSession {
                id: session_id,
                name: name.clone(),
                windows: Vec::new(),
                active: 0,
                env,
            },
        );
        self.insert_window(
            session_id,
            MuxWindow {
                id: window_id,
                name,
                layout: LayoutTree::leaf(pane_id),
                active: pane_id,
                cols,
                rows,
                zoomed: None,
            },
        );
        // The workspace may have been killed while the pane spawned off the
        // lock; a session always belongs to exactly one workspace, so the
        // fallback re-anchors to the lazily created default.
        if !self.link_session(workspace_id, session_id) {
            let fallback = self.ensure_default_workspace();
            self.link_session(fallback, session_id);
        }
        Ok(session_id)
    }

    pub(super) fn spawn_from(
        &self,
        pane_id: &PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        self.factory
            .create_pane(*pane_id, cols, rows, command, context)
    }

    /// Add a window to a session.
    pub fn new_window(
        &mut self,
        session_id: SessionId,
        name: &str,
        cols: u16,
        rows: u16,
    ) -> Result<WindowId, MuxError> {
        self.new_window_with_cwd(session_id, name, cols, rows, None)
    }

    /// [`Self::new_window`] with a start directory for the new pane — the
    /// `new-window -c` path. `None` keeps the factory-wide default; the
    /// caller (dispatch) owns the gone-directory degrade-to-home rule.
    ///
    /// One-shot form of the two-phase flow for callers that already hold
    /// the tree exclusively.
    pub fn new_window_with_cwd(
        &mut self,
        session_id: SessionId,
        name: &str,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
    ) -> Result<WindowId, MuxError> {
        let plan = self.begin_window(session_id, name, cols, rows, cwd)?;
        let pane = self.spawn_from(&plan.pane_id, plan.cols, plan.rows, None, &plan.context());
        self.complete_window(plan, pane?)
    }

    /// Phase 1 of `new-window`: validate the session and reserve the ids.
    pub fn begin_window(
        &mut self,
        session_id: SessionId,
        name: &str,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
    ) -> Result<WindowSpawn, MuxError> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(MuxError::NoSuchSession(session_id))?;
        Ok(WindowSpawn {
            window_id: self.ids.next_window(),
            pane_id: self.ids.next_pane(),
            session_id,
            cols,
            rows,
            name: name.to_string(),
            session_name: session.name.clone(),
            env: session.env.clone(),
            cwd: cwd.map(Path::to_owned),
        })
    }

    /// Phase 3 of `new-window`: insert the pane and window. The session may
    /// have been killed while the pane spawned off the lock; the pane is
    /// killed and the error reported rather than leaking a live PTY.
    pub fn complete_window(
        &mut self,
        plan: WindowSpawn,
        pane: MuxPane,
    ) -> Result<WindowId, MuxError> {
        let WindowSpawn {
            window_id,
            pane_id,
            session_id,
            cols,
            rows,
            name,
            session_name: _,
            env: _,
            cwd: _,
        } = plan;
        if !self.sessions.contains_key(&session_id) {
            kill_detached(pane);
            return Err(MuxError::NoSuchSession(session_id));
        }
        self.panes.insert(pane_id, pane);
        self.apply_cell_pixels(pane_id, cols, rows);
        self.insert_window(
            session_id,
            MuxWindow {
                id: window_id,
                name,
                layout: LayoutTree::leaf(pane_id),
                active: pane_id,
                cols,
                rows,
                zoomed: None,
            },
        );
        Ok(window_id)
    }

    /// Set (`Some`) or remove (`None`) one variable in a session's
    /// environment. Running panes are untouched.
    pub fn set_session_env(
        &mut self,
        session_id: SessionId,
        name: &str,
        value: Option<&str>,
    ) -> Result<(), MuxError> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(MuxError::NoSuchSession(session_id))?;
        match value {
            Some(value) => {
                session.env.insert(name.to_string(), value.to_string());
            }
            None => {
                session.env.remove(name);
            }
        }
        Ok(())
    }

    /// Remove `window_id` from the tree and its session's window list,
    /// closing the session when it was the last window — the one cascade
    /// `kill-pane`, `kill-window`, `break-pane` and `join-pane` share.
    /// Returns the removed session, when the cascade reached it. The
    /// caller guarantees the window exists and every pane it held has
    /// already been re-homed or killed.
    pub(super) fn drop_empty_window(&mut self, window_id: WindowId) -> Option<SessionId> {
        let session_id = self.session_of_window(window_id);
        self.remove_window(window_id);
        let session_id = session_id?;
        let emptied = self
            .sessions
            .get(&session_id)
            .is_some_and(|session| session.windows.is_empty());
        emptied.then(|| {
            self.sessions.remove(&session_id);
            // A session's death also leaves its workspace — and a workspace
            // left with no sessions dies with it (the exit-when-empty
            // contract's workspace-level expression).
            self.unlink_session(session_id);
            session_id
        })
    }

    /// Whether every pane in the tree has been observed dead — the
    /// exit-when-empty daemon's "nothing left to serve" test alongside an
    /// empty tree. A tree holding only dead panes contains frozen screens
    /// (remain-on-exit); with no client connected either, the daemon
    /// collects itself instead of lingering on them. `false` for an empty
    /// tree — the caller tests emptiness separately.
    pub fn all_panes_dead(&self) -> bool {
        !self.panes.is_empty() && self.panes.values().all(|pane| pane.dead())
    }

    /// Phase 1 of `respawn-pane`: resolve the pane, refuse a live one
    /// without `kill`, and snapshot everything the restart needs. The
    /// caller DROPS the tree lock, spawns through the factory, and
    /// re-locks for [`Self::complete_respawn`].
    pub fn begin_respawn(
        &mut self,
        pane: PaneId,
        kill: bool,
        command: Option<String>,
        cwd: Option<&Path>,
    ) -> Result<RespawnSpawn, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        let (session_id, session_name, env) = {
            let session = self
                .session_of_window(window_id)
                .and_then(|id| self.sessions.get(&id))
                .ok_or(MuxError::NoSuchWindow(window_id))?;
            (session.id, session.name.clone(), session.env.clone())
        };
        let (cols, rows, stored_command, stored_cwd, user_title, alive) = {
            let pane = self
                .panes
                .get_mut(&pane)
                .expect("window_of_pane only returns live windows");
            let (cols, rows) = pane.terminal().read().size();
            (
                cols as u16,
                rows as u16,
                pane.spawn_command().map(str::to_string),
                pane.respawn_cwd(),
                pane.user_title().map(str::to_string),
                pane.poll_running(),
            )
        };
        if alive && !kill {
            return Err(MuxError::PaneAlive(pane));
        }
        Ok(RespawnSpawn {
            pane_id: pane,
            window_id,
            session_id,
            session_name,
            env,
            cols,
            rows,
            command: command.or(stored_command),
            // `None` lets the factory's cwd apply, then the spawn's `$HOME`
            // fallback — where a fresh pane lands (SEC-128).
            cwd: cwd.map(Path::to_owned).or(stored_cwd),
            user_title,
        })
    }

    /// Phase 3 of `respawn-pane`: swap the freshly spawned pane in for
    /// the old one — same id, so window and layout are untouched. The
    /// old pane's process is killed off the tree's books; the window may
    /// have been killed while the spawn ran, in which case the new pane
    /// is killed and the error reported.
    pub fn complete_respawn(
        &mut self,
        plan: RespawnSpawn,
        pane: MuxPane,
    ) -> Result<PaneId, MuxError> {
        if self.window_of_pane(plan.pane_id) != Some(plan.window_id) {
            kill_detached(pane);
            return Err(MuxError::NoSuchWindow(plan.window_id));
        }
        let user_title = plan.user_title.clone();
        let old = self
            .panes
            .insert(plan.pane_id, pane)
            .expect("begin_respawn resolved a live pane, and only complete_respawn removes it");
        kill_detached(old);
        if let Some(title) = user_title {
            if let Some(pane) = self.panes.get_mut(&plan.pane_id) {
                pane.set_user_title(&title);
            }
        }
        self.sync_pane_sizes(plan.window_id);
        Ok(plan.pane_id)
    }

    /// Kill a pane, closing its window when it was the last one, and return
    /// the window that held it — resolved BEFORE the kill, because the pane's
    /// window membership is gone afterwards. The dispatcher's
    /// `%layout-change` target; a window closing entirely reports its own
    /// `%window-close` instead. The second element names the session the
    /// cascade removed, when the window's closure emptied it — the caller's
    /// cue to broadcast `%sessions-changed`.
    ///
    /// Cascading matches tmux: a window with no panes and a session with no
    /// windows do not linger. A surviving pane is resized to the extent the
    /// killed pane freed.
    pub fn kill_pane(
        &mut self,
        pane_id: PaneId,
    ) -> Result<(WindowId, Option<SessionId>), MuxError> {
        let affected_window = self
            .window_of_pane(pane_id)
            .ok_or(MuxError::NoSuchPane(pane_id))?;
        let pane = self
            .panes
            .remove(&pane_id)
            .ok_or(MuxError::NoSuchPane(pane_id))?;
        kill_detached(pane);
        let only_pane = self
            .windows
            .get(&affected_window)
            .is_some_and(|window| window.layout == LayoutTree::leaf(pane_id));

        let removed_session = if only_pane {
            // The window's only pane — the window itself closes.
            self.drop_empty_window(affected_window)
        } else {
            // A surviving pane takes the freed extent; the choke point
            // ends any zoom (the killed pane's own included), drops the
            // pane from the index, and re-fits the terminals. The pane is
            // already gone from `panes`, so this cannot fail the kill.
            self.mutate_layout(affected_window, |window| {
                if window.layout.remove_pane(pane_id).is_ok() && window.active == pane_id {
                    // The tree always has a pane left here, so the first
                    // surviving leaf is a reasonable new active pane.
                    // tmux's own choice of successor is more elaborate
                    // (last-focused history).
                    window.active = window.layout.pane_ids()[0];
                }
                Ok(())
            })?;
            None
        };

        Ok((affected_window, removed_session))
    }

    /// Rename a window.
    pub fn rename_window(&mut self, window_id: WindowId, name: &str) -> Result<(), MuxError> {
        let window = self
            .windows
            .get_mut(&window_id)
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        window.name = name.to_string();
        Ok(())
    }

    /// Kill a window and every pane it holds, closing its session when it
    /// was the last window — the same cascade [`Self::kill_pane`] uses. The
    /// Ok value names that removed session, when the cascade reached it, so
    /// the caller can broadcast `%sessions-changed`.
    pub fn kill_window(&mut self, window_id: WindowId) -> Result<Option<SessionId>, MuxError> {
        let pane_ids = self
            .windows
            .get(&window_id)
            .ok_or(MuxError::NoSuchWindow(window_id))?
            .panes();
        for pane_id in pane_ids {
            if let Some(pane) = self.panes.remove(&pane_id) {
                kill_detached(pane);
            }
        }
        Ok(self.drop_empty_window(window_id))
    }

    /// Rename a session. The tree's name is what future pane spawns export
    /// as `PAR_MUX_SESSION`; panes already running keep the name they were
    /// spawned with (the fixed-at-spawn contract, MUX.md).
    pub fn rename_session(&mut self, session_id: SessionId, name: &str) -> Result<(), MuxError> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(MuxError::NoSuchSession(session_id))?;
        session.name = name.to_string();
        Ok(())
    }

    /// Kill a session and every window and pane in it. The Ok value lists
    /// the windows killed, so the caller can emit a `%window-close` per
    /// window before the `%sessions-changed` cue — the same line order
    /// `kill-window`'s cascade produces.
    pub fn kill_session(&mut self, session_id: SessionId) -> Result<Vec<WindowId>, MuxError> {
        let session = self
            .sessions
            .remove(&session_id)
            .ok_or(MuxError::NoSuchSession(session_id))?;
        self.unlink_session(session_id);
        let mut killed = Vec::new();
        for window_id in session.windows {
            // The session is already gone, so this only clears the window
            // and its indexes; no list to unlink from.
            let Some(window) = self.remove_window(window_id) else {
                continue;
            };
            for pane_id in window.panes() {
                if let Some(pane) = self.panes.remove(&pane_id) {
                    kill_detached(pane);
                }
            }
            killed.push(window_id);
        }
        Ok(killed)
    }
}
