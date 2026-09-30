//! The session/window/pane tree: the server's single source of truth.

use crate::color::Color;
use crate::mux::ids::{IdAllocator, PaneId, SessionId, Target, WindowId};
use crate::mux::layout::{LayoutTree, ResizeDirection, SplitDirection};
use crate::mux::pane::{MuxError, MuxPane, PaneFactory, SpawnContext};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Kill a pane the tree has already removed, off the tree lock: killing is
/// signal-then-reap with a bounded wait (portable-pty polls its SIGHUP
/// grace for ~200 ms before SIGKILL, and the reap adds a short wait after),
/// and running that under the tree mutex would stall every command for the
/// duration. The thread always exits — SIGKILL cannot be trapped — and the
/// pane it owns is dropped reaped.
fn kill_detached(mut pane: MuxPane) {
    // Stop forwarding before the SIGHUP, on this thread: the pane's id may
    // already belong to a replacement (respawn), and the reader can deliver
    // bytes before the kill thread first runs (ARC-089).
    pane.detach_output();
    std::thread::spawn(move || {
        let _ = pane.kill();
    });
}

/// One window: an interior split structure (Task 2.1) with one pane active.
///
/// `active` is a pane id, not an index — [`LayoutTree`] has no stable linear
/// order for a mutation (`split_pane`/`swap_pane`) to preserve, unlike the
/// flat `Vec<PaneId>` this replaced.
#[derive(Debug)]
pub struct MuxWindow {
    /// This window's identifier.
    pub id: WindowId,
    /// Display name.
    pub name: String,
    /// The window's interior pane structure.
    pub layout: LayoutTree,
    /// The currently active pane.
    pub active: PaneId,
    /// The window's width in columns — the extent the layout renders into
    /// and the base a resize delta converts against.
    pub cols: u16,
    /// The window's height in rows.
    pub rows: u16,
    /// `resize-pane -Z`: the pane currently zoomed to the full window
    /// grid, hiding the others. The layout tree is never touched while
    /// zoomed, so unzooming restores the exact prior geometry — zoom lives
    /// only in pane sizes. Every layout mutation goes through
    /// `MuxTree::mutate_layout`, which ends the zoom. Not persisted: a
    /// restored window starts unzoomed (tmux's behavior).
    pub zoomed: Option<PaneId>,
}

impl MuxWindow {
    /// Every pane id in this window's layout, in tree order.
    pub fn panes(&self) -> Vec<PaneId> {
        self.layout.pane_ids()
    }
}

/// One session: an ordered set of windows with one of them active.
#[derive(Debug)]
pub struct MuxSession {
    /// This session's identifier.
    pub id: SessionId,
    /// Display name.
    pub name: String,
    /// Windows in order.
    pub windows: Vec<WindowId>,
    /// Index into `windows` of the active window.
    pub active: usize,
    /// The session environment (`set-environment`, `new-session -e`),
    /// applied on top of the daemon's environment for every pane spawned
    /// into this session after it is set.
    pub env: BTreeMap<String, String>,
}

/// A pane spawn reserved under the tree lock but not yet run — the first
/// phase of two-phase spawning (ARC-022). `begin_*` reserves ids and
/// geometry; the caller then DROPS the tree lock, spawns through
/// [`MuxTree::factory`], and re-locks to `complete_*`, which inserts the
/// pane or — when the tree moved underneath the reservation — kills the
/// spawned pane and reports the error. Discarding a plan (spawn failed)
/// leaves only an id gap, the same residue a failed one-shot spawn leaves.
pub struct SessionSpawn {
    /// The reserved ids, visible to the caller for the factory call.
    pub session_id: SessionId,
    pub window_id: WindowId,
    pub pane_id: PaneId,
    pub cols: u16,
    pub rows: u16,
    name: String,
    env: BTreeMap<String, String>,
}

/// A pane restart reserved under the tree lock but not yet run —
/// `respawn-pane`'s phase 1, the same two-phase shape session, window,
/// and split spawns use (ARC-022).
pub struct RespawnSpawn {
    /// The pane being restarted — the replacement keeps this id, so the
    /// window and layout never change.
    pub pane_id: PaneId,
    /// The pane's window, re-validated at insert.
    pub window_id: WindowId,
    /// The owning session's identity, re-exported to the new process.
    pub session_id: SessionId,
    session_name: String,
    env: BTreeMap<String, String>,
    /// The pane's current grid size — the fresh terminal starts here,
    /// not at a construction default.
    pub cols: u16,
    pub rows: u16,
    /// The command to run: the explicit override, else the pane's stored
    /// spawn command.
    pub command: Option<String>,
    /// The restart's working directory: the explicit override, else
    /// [`MuxPane::respawn_cwd`]; `None` defers to the factory's cwd.
    pub cwd: Option<std::path::PathBuf>,
    /// The old pane's user title, carried to the replacement.
    user_title: Option<String>,
}

impl RespawnSpawn {
    /// The factory-facing context — the same fields a fresh spawn passes.
    pub fn context(&self) -> SpawnContext<'_> {
        SpawnContext {
            session: Some((self.session_id, &self.session_name)),
            window: Some(self.window_id),
            env: Some(&self.env),
            cwd: self.cwd.as_deref(),
            output: None,
        }
    }
}

impl SessionSpawn {
    /// The factory-facing context — the same fields the one-shot path passes.
    pub fn context(&self) -> SpawnContext<'_> {
        SpawnContext {
            session: Some((self.session_id, &self.name)),
            window: Some(self.window_id),
            env: Some(&self.env),
            cwd: None,
            output: None,
        }
    }
}

/// [`SessionSpawn`] for `new-window`: a second window in an existing
/// session, so completion depends on that session surviving the spawn.
pub struct WindowSpawn {
    pub window_id: WindowId,
    pub pane_id: PaneId,
    session_id: SessionId,
    pub cols: u16,
    pub rows: u16,
    name: String,
    session_name: String,
    env: BTreeMap<String, String>,
    cwd: Option<PathBuf>,
}

impl WindowSpawn {
    /// The factory-facing context — the same fields the one-shot path passes.
    pub fn context(&self) -> SpawnContext<'_> {
        SpawnContext {
            session: Some((self.session_id, &self.session_name)),
            window: Some(self.window_id),
            env: Some(&self.env),
            cwd: self.cwd.as_deref(),
            output: None,
        }
    }
}

/// [`SessionSpawn`] for `split-window`: completion depends on the target
/// pane still being a live leaf of the same window after the spawn.
pub struct SplitSpawn {
    pub pane_id: PaneId,
    window_id: WindowId,
    target: PaneId,
    direction: SplitDirection,
    new_share: f32,
    /// `-b`: the new pane takes `first` (left/top) of the new split
    /// instead of `second` (right/bottom).
    before: bool,
    pub cols: u16,
    pub rows: u16,
    session: Option<(SessionId, String)>,
    env: Option<BTreeMap<String, String>>,
    cwd: Option<PathBuf>,
}

impl SplitSpawn {
    /// The factory-facing context — the same fields the one-shot path passes.
    pub fn context(&self) -> SpawnContext<'_> {
        SpawnContext {
            session: self.session.as_ref().map(|(id, name)| (*id, name.as_str())),
            window: Some(self.window_id),
            env: self.env.as_ref(),
            cwd: self.cwd.as_deref(),
            output: None,
        }
    }
}

/// A reserved spawn the dispatcher runs through one spawn-and-wire helper
/// (ARC-103): the four `begin_*` plans share phases 2 and 3, so they share
/// this seam instead of four copies of the wiring.
pub(crate) trait SpawnPlan {
    /// What a successful completion returns.
    type Done;
    /// The reserved pane id.
    fn pane_id(&self) -> PaneId;
    /// The pane's initial grid size.
    fn size(&self) -> (u16, u16);
    /// The factory-facing context.
    fn spawn_context(&self) -> SpawnContext<'_>;
    /// Phase 3: insert the spawned pane, or kill it and report why not.
    fn complete(self, tree: &mut MuxTree, pane: MuxPane) -> Result<Self::Done, MuxError>;
}

impl SpawnPlan for SessionSpawn {
    type Done = SessionId;
    fn pane_id(&self) -> PaneId {
        self.pane_id
    }
    fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
    fn spawn_context(&self) -> SpawnContext<'_> {
        SessionSpawn::context(self)
    }
    fn complete(self, tree: &mut MuxTree, pane: MuxPane) -> Result<SessionId, MuxError> {
        tree.complete_session(self, pane)
    }
}

impl SpawnPlan for WindowSpawn {
    type Done = WindowId;
    fn pane_id(&self) -> PaneId {
        self.pane_id
    }
    fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
    fn spawn_context(&self) -> SpawnContext<'_> {
        WindowSpawn::context(self)
    }
    fn complete(self, tree: &mut MuxTree, pane: MuxPane) -> Result<WindowId, MuxError> {
        tree.complete_window(self, pane)
    }
}

impl SpawnPlan for SplitSpawn {
    type Done = (PaneId, WindowId);
    fn pane_id(&self) -> PaneId {
        self.pane_id
    }
    fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
    fn spawn_context(&self) -> SpawnContext<'_> {
        SplitSpawn::context(self)
    }
    fn complete(self, tree: &mut MuxTree, pane: MuxPane) -> Result<(PaneId, WindowId), MuxError> {
        tree.complete_split(self, pane)
    }
}

impl SpawnPlan for RespawnSpawn {
    type Done = PaneId;
    fn pane_id(&self) -> PaneId {
        self.pane_id
    }
    fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
    fn spawn_context(&self) -> SpawnContext<'_> {
        RespawnSpawn::context(self)
    }
    fn complete(self, tree: &mut MuxTree, pane: MuxPane) -> Result<PaneId, MuxError> {
        tree.complete_respawn(self, pane)
    }
}

/// The server's whole state: every session, window, and pane.
///
/// Flat maps keyed by id rather than a nested ownership tree, because panes are
/// addressed directly by the protocol (`%3`) far more often than they are
/// walked to from a session.
pub struct MuxTree {
    // pub(crate): the persistence conversions (mux::persist) read and rebuild
    // the tree wholesale; the field set IS the save format's source.
    pub(crate) sessions: HashMap<SessionId, MuxSession>,
    pub(crate) windows: HashMap<WindowId, MuxWindow>,
    pub(crate) panes: HashMap<PaneId, MuxPane>,
    pub(crate) ids: IdAllocator,
    factory: Arc<dyn PaneFactory>,
    /// Named paste buffers (`set-buffer`/`show-buffer`). A single value per
    /// name, not tmux's numbered stack — the Phase 2 non-goal in par-mux.md D3.
    pub(crate) buffers: HashMap<String, String>,
    /// The client's per-cell pixel size (`refresh-client -p`), the one
    /// renderer metric every pane shares. Latest report wins, the same
    /// policy the grid-size report (`-C`) uses — par-mux has no other
    /// client-metrics input. `None` until a client reports, so panes keep
    /// the 10×20 construction default. Never persisted: a reconnecting
    /// client re-reports on attach.
    pub(crate) client_cell_pixels: Option<(u16, u16)>,
    /// The client's theme colors (`set-client-colors`), held daemon-wide
    /// for the same reason the cell pixels are: the daemon's terminals
    /// answer OSC 10/11 queries programs make, and the honest answer is
    /// what the client actually renders, not the core's built-in theme.
    /// Each half is independently optional. Never persisted — re-reported
    /// on attach.
    pub(crate) client_fg: Option<Color>,
    pub(crate) client_bg: Option<Color>,
    /// Reverse index pane → window (ARC-096), so [`Self::window_of_pane`]
    /// is a map lookup instead of a scan of every window's layout under
    /// the tree lock. Maintained where a window's pane set changes: in
    /// [`Self::mutate_layout`] for layout edits, and in
    /// [`Self::insert_window`] / [`Self::remove_window`] for window
    /// lifetimes. Nothing else writes it.
    pane_window: HashMap<PaneId, WindowId>,
    /// Reverse index window → session (ARC-096), for
    /// [`Self::session_of_window`]. Maintained only by
    /// [`Self::link_window`] and [`Self::unlink_window`], the only code
    /// that changes which session a window belongs to. Reordering a
    /// session's windows keeps membership and needs no update.
    window_session: HashMap<WindowId, SessionId>,
}

impl MuxTree {
    /// Create an empty tree that builds panes with `factory` (seam S1).
    pub fn new(factory: Box<dyn PaneFactory>) -> Self {
        Self {
            sessions: HashMap::new(),
            windows: HashMap::new(),
            panes: HashMap::new(),
            ids: IdAllocator::new(),
            factory: Arc::from(factory),
            buffers: HashMap::new(),
            client_cell_pixels: None,
            client_fg: None,
            client_bg: None,
            pane_window: HashMap::new(),
            window_session: HashMap::new(),
        }
    }

    /// Append `window` to `session_id`'s window list and index it and its
    /// panes — every window creation (new-session, new-window, break-pane,
    /// restore) goes through here. The session must exist.
    pub(crate) fn insert_window(&mut self, session_id: SessionId, window: MuxWindow) {
        let window_id = window.id;
        for pane in window.panes() {
            self.pane_window.insert(pane, window_id);
        }
        self.windows.insert(window_id, window);
        self.link_window(session_id, window_id);
    }

    /// Remove `window_id` from the tree, its session's window list and the
    /// indexes, returning it. A pane the window held stays indexed only if
    /// it already moved to another window (break-pane re-homes before the
    /// source closes). The caller disposes of the returned window's panes.
    fn remove_window(&mut self, window_id: WindowId) -> Option<MuxWindow> {
        let window = self.windows.remove(&window_id);
        if let Some(window) = &window {
            for pane in window.panes() {
                if self.pane_window.get(&pane) == Some(&window_id) {
                    self.pane_window.remove(&pane);
                }
            }
        }
        self.unlink_window(window_id);
        window
    }

    /// Append `window_id` to `session_id`'s window list.
    fn link_window(&mut self, session_id: SessionId, window_id: WindowId) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.windows.push(window_id);
            self.window_session.insert(window_id, session_id);
        }
    }

    /// Take `window_id` out of its session's window list, clamping the
    /// session's active index to the shortened list.
    fn unlink_window(&mut self, window_id: WindowId) {
        let Some(session_id) = self.window_session.remove(&window_id) else {
            return;
        };
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        if let Some(pos) = session.windows.iter().position(|w| *w == window_id) {
            session.windows.remove(pos);
        }
        if session.active >= session.windows.len() && !session.windows.is_empty() {
            session.active = session.windows.len() - 1;
        }
    }

    /// Brute-force the two reverse indexes from the layouts and window
    /// lists and assert they match the maintained ones (ARC-096). O(n) —
    /// tests only.
    #[cfg(test)]
    pub(crate) fn assert_indexes_consistent(&self) {
        let mut pane_window = HashMap::new();
        for (window_id, window) in &self.windows {
            for pane in window.panes() {
                pane_window.insert(pane, *window_id);
            }
        }
        let mut window_session = HashMap::new();
        for (session_id, session) in &self.sessions {
            for window_id in &session.windows {
                window_session.insert(*window_id, *session_id);
            }
        }
        assert_eq!(self.pane_window, pane_window, "pane → window index drifted");
        assert_eq!(
            self.window_session, window_session,
            "window → session index drifted"
        );
    }

    /// Store `content` under `name`, overwriting any existing value.
    pub fn set_buffer(&mut self, name: &str, content: String) {
        self.buffers.insert(name.to_string(), content);
    }

    /// Look up a named buffer's content.
    pub fn get_buffer(&self, name: &str) -> Option<&str> {
        self.buffers.get(name).map(String::as_str)
    }

    /// Every session id currently live.
    pub fn sessions(&self) -> Vec<SessionId> {
        self.sessions.keys().copied().collect()
    }

    /// Look up a session.
    pub fn session(&self, id: SessionId) -> Option<&MuxSession> {
        self.sessions.get(&id)
    }

    /// Look up a window.
    pub fn window(&self, id: WindowId) -> Option<&MuxWindow> {
        self.windows.get(&id)
    }

    /// Look up a pane.
    pub fn pane(&self, id: PaneId) -> Option<&MuxPane> {
        self.panes.get(&id)
    }

    /// Look up a pane mutably.
    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut MuxPane> {
        self.panes.get_mut(&id)
    }

    /// Resolve a pane target: typed `%N` ids pass through untouched (the
    /// caller's pane lookup reports unknown ids as before), any other
    /// value matches the pane's sticky user title exactly — the OSC 0/2
    /// program title never matches, it changes with the running program
    /// and would make name targets flaky.
    ///
    /// A title held by more than one pane is an error listing the
    /// candidate ids, never a silent pick.
    pub fn resolve_pane_target(&self, target: Target<PaneId>) -> Result<PaneId, MuxError> {
        let name = match target {
            Target::Id(id) => return Ok(id),
            Target::Name(name) => name,
        };
        match match_name(
            self.panes
                .iter()
                .filter_map(|(id, pane)| (pane.user_title() == Some(name.as_str())).then_some(*id)),
        ) {
            Match::None => Err(MuxError::NoSuchPaneNamed(name)),
            Match::One(id) => Ok(id),
            Match::Many(ids) => Err(MuxError::AmbiguousPaneTarget(name, ids)),
        }
    }

    /// Resolve a window target: typed `@N` ids pass through; a name
    /// matches window names exactly, across every session (no command
    /// pairs a window target with a session scope today). Ambiguous names
    /// error with the candidates.
    pub fn resolve_window_target(&self, target: Target<WindowId>) -> Result<WindowId, MuxError> {
        let name = match target {
            Target::Id(id) => return Ok(id),
            Target::Name(name) => name,
        };
        match match_name(
            self.windows
                .iter()
                .filter_map(|(id, window)| (window.name == name).then_some(*id)),
        ) {
            Match::None => Err(MuxError::NoSuchWindowNamed(name)),
            Match::One(id) => Ok(id),
            Match::Many(ids) => Err(MuxError::AmbiguousWindowTarget(name, ids)),
        }
    }

    /// Resolve a session target: typed `$N` ids pass through; a name
    /// matches session names exactly. Ambiguous names error with the
    /// candidates.
    pub fn resolve_session_target(&self, target: Target<SessionId>) -> Result<SessionId, MuxError> {
        let name = match target {
            Target::Id(id) => return Ok(id),
            Target::Name(name) => name,
        };
        match match_name(
            self.sessions
                .iter()
                .filter_map(|(id, session)| (session.name == name).then_some(*id)),
        ) {
            Match::None => Err(MuxError::NoSuchSessionNamed(name)),
            Match::One(id) => Ok(id),
            Match::Many(ids) => Err(MuxError::AmbiguousSessionTarget(name, ids)),
        }
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
        let plan = self.begin_session(name, cols, rows, &env);
        let pane = self.spawn_from(&plan.pane_id, plan.cols, plan.rows, None, &plan.context());
        self.complete_session(plan, pane?)
    }

    /// Phase 1 of `new-session` (ARC-022): reserve the ids under the tree
    /// lock. Infallible — fresh ids depend on no existing state.
    pub fn begin_session(
        &mut self,
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
        Ok(session_id)
    }

    /// The factory this tree spawns panes with (seam S1), handed out so the
    /// dispatcher can run phase 2 — the spawn itself — OFF the tree lock.
    pub fn factory(&self) -> Arc<dyn PaneFactory> {
        self.factory.clone()
    }

    fn spawn_from(
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

    /// Split `target`'s area in two and spawn a new pane in the other half.
    ///
    /// tmux's `split-window`: the new pane divides the target's extent along
    /// `direction`, receiving `new_share` of it (the `-p` percentage), and
    /// becomes the window's active pane. Afterwards every pane terminal in
    /// the window is resized to the new geometry — the layout is the source
    /// of truth for pane extents, and the terminals follow it.
    pub fn split_pane(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
        command: Option<&str>,
    ) -> Result<PaneId, MuxError> {
        self.split_pane_in_window(target, direction, new_share, command, None)
            .map(|(pane_id, _)| pane_id)
    }

    /// [`Self::split_pane`] also reporting the window the new pane landed
    /// in — the dispatcher's form, so its `%layout-change` broadcast names
    /// the window without re-deriving it after the fact. `cwd` is the
    /// `split-window -c` start directory; `None` keeps the factory-wide
    /// default and the caller owns the gone-directory degrade.
    ///
    /// One-shot form of the two-phase flow for callers that already hold
    /// the tree exclusively.
    pub fn split_pane_in_window(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
        command: Option<&str>,
        cwd: Option<&Path>,
    ) -> Result<(PaneId, WindowId), MuxError> {
        let plan = self.begin_split(target, direction, new_share, cwd, false)?;
        let pane = self.spawn_from(
            &plan.pane_id,
            plan.cols,
            plan.rows,
            command,
            &plan.context(),
        );
        self.complete_split(plan, pane?)
    }

    /// Phase 1 of `split-window`: resolve the target's window, snapshot its
    /// geometry and session environment, reserve the pane id.
    pub fn begin_split(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
        cwd: Option<&Path>,
        before: bool,
    ) -> Result<SplitSpawn, MuxError> {
        let window_id = self
            .window_of_pane(target)
            .ok_or(MuxError::NoSuchPane(target))?;
        let (cols, rows) = {
            let window = self.windows.get(&window_id).expect("just found");
            (window.cols, window.rows)
        };
        let pane_id = self.ids.next_pane();
        let session = self
            .session_of_window(window_id)
            .and_then(|id| self.sessions.get(&id));
        Ok(SplitSpawn {
            pane_id,
            window_id,
            target,
            direction,
            new_share,
            before,
            cols,
            rows,
            session: session.map(|s| (s.id, s.name.clone())),
            env: session.map(|s| s.env.clone()),
            cwd: cwd.map(Path::to_owned),
        })
    }

    /// Phase 3 of `split-window`: insert the pane and re-shape the layout.
    /// The target pane (or its window) may be gone after the unlocked
    /// spawn; the pane is killed and the error reported rather than leaking
    /// a live PTY.
    pub fn complete_split(
        &mut self,
        plan: SplitSpawn,
        pane: MuxPane,
    ) -> Result<(PaneId, WindowId), MuxError> {
        let SplitSpawn {
            pane_id,
            window_id,
            target,
            direction,
            new_share,
            before,
            cols: _,
            rows: _,
            session: _,
            env: _,
            cwd: _,
        } = plan;
        // `LayoutTree::split_pane`'s ratio is the fraction kept by `first`
        // (the target), while the command speaks in the NEW pane's share.
        // `-b` instead puts the NEW pane in `first`: split at the new
        // pane's share and swap the two leaves, which lands the new pane
        // left/above with exactly its `-p` share.
        // The target (or its window) may be gone: validate before the
        // insert so a failure leaves the tree untouched.
        if self.window_of_pane(target) != Some(window_id) {
            kill_detached(pane);
            return Err(MuxError::NoSuchPane(target));
        }
        self.panes.insert(pane_id, pane);
        // Through the choke point: it ends the zoom that would hide the
        // new pane, indexes the new pane (ARC-096), and re-fits every
        // terminal to the new geometry.
        self.mutate_layout(window_id, |window| {
            let placed = if before {
                window
                    .layout
                    .split_pane(target, pane_id, direction, new_share)
                    .and_then(|_| window.layout.swap_pane(target, pane_id))
            } else {
                window
                    .layout
                    .split_pane(target, pane_id, direction, 1.0 - new_share)
            };
            placed.expect("the target is a leaf of this window, checked above");
            window.active = pane_id;
            Ok(())
        })?;
        Ok((pane_id, window_id))
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

    /// The session whose window list holds `window`, if any. A map lookup
    /// (ARC-096).
    pub fn session_of_window(&self, window: WindowId) -> Option<SessionId> {
        self.window_session.get(&window).copied()
    }

    /// The window whose layout holds `pane`, if any. A map lookup
    /// (ARC-096).
    pub fn window_of_pane(&self, pane: PaneId) -> Option<WindowId> {
        self.pane_window.get(&pane).copied()
    }

    /// Make `pane` its window's active pane, returning the window — the
    /// dispatcher's `%layout-change` target.
    pub fn select_pane(&mut self, pane: PaneId) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        let mut unzoomed = false;
        {
            let window = self
                .windows
                .get_mut(&window_id)
                .expect("window_of_pane only returns live windows");
            window.active = pane;
            // Selecting another pane reveals the layout — tmux unzooms on
            // the switch; selecting the zoomed pane itself keeps the zoom.
            if window.zoomed.is_some_and(|z| z != pane) {
                window.zoomed = None;
                unzoomed = true;
            }
        }
        if unzoomed {
            self.sync_pane_sizes(window_id);
        }
        Ok(window_id)
    }

    /// Swap two panes' positions within their window, returning it — the
    /// dispatcher's `%layout-change` target.
    ///
    /// tmux's `swap-pane` exchanges panes inside one window; panes in
    /// different windows have no shared split structure to trade places in.
    /// Both terminals are resized to their traded geometry.
    pub fn swap_panes(&mut self, target: PaneId, source: PaneId) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(target)
            .ok_or(MuxError::NoSuchPane(target))?;
        self.mutate_layout(window_id, |window| {
            if !window.layout.pane_ids().contains(&source) {
                return Err(MuxError::PanesInDifferentWindows(target, source));
            }
            window
                .layout
                .swap_pane(target, source)
                .map_err(|_| MuxError::PanesInDifferentWindows(target, source))?;
            Ok(window_id)
        })
    }

    /// Toggle `pane`'s zoom (`resize-pane -Z`): zooming resizes its
    /// terminal and PTY to the full window grid; unzooming re-fits every
    /// pane to the layout, which the zoom never edited — the exact prior
    /// geometry. Zooming a different pane moves the zoom to it. The Ok
    /// payload is the pane's window — the dispatcher's `%layout-change`
    /// target.
    pub fn zoom_pane(&mut self, pane: PaneId) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        {
            let window = self
                .windows
                .get_mut(&window_id)
                .expect("window_of_pane only returns live windows");
            window.zoomed = if window.zoomed == Some(pane) {
                None
            } else {
                Some(pane)
            };
        }
        self.sync_pane_sizes(window_id);
        Ok(window_id)
    }

    /// Remove `window_id` from the tree and its session's window list,
    /// closing the session when it was the last window — the one cascade
    /// `kill-pane`, `kill-window`, `break-pane` and `join-pane` share.
    /// Returns the removed session, when the cascade reached it. The
    /// caller guarantees the window exists and every pane it held has
    /// already been re-homed or killed.
    fn drop_empty_window(&mut self, window_id: WindowId) -> Option<SessionId> {
        let session_id = self.session_of_window(window_id);
        self.remove_window(window_id);
        let session_id = session_id?;
        let emptied = self
            .sessions
            .get(&session_id)
            .is_some_and(|session| session.windows.is_empty());
        emptied.then(|| {
            self.sessions.remove(&session_id);
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

    /// `break-pane -s %N`: move `pane` out of its window into a new
    /// window appended to the same session, which becomes the session's
    /// active window (tmux's behavior). The pane's process, terminal,
    /// and title all move with it — the move is pure layout surgery, so
    /// nothing is re-spawned. The new window inherits the source
    /// window's grid size, and the pane's terminal is re-fitted to that
    /// full extent. The source window closes when the pane was its last
    /// one; the session cannot die that way — the new window is
    /// inserted before the source is dropped, so the cascade in
    /// [`Self::drop_empty_window`] never finds an empty session. The Ok
    /// payload is `(new window, source window, whether the source
    /// window closed)`.
    pub fn break_pane(
        &mut self,
        pane: PaneId,
        name: &str,
    ) -> Result<(WindowId, WindowId, bool), MuxError> {
        let source_window = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        let session_id = self
            .session_of_window(source_window)
            .ok_or(MuxError::NoSuchWindow(source_window))?;
        let (cols, rows) = {
            let window = self
                .windows
                .get(&source_window)
                .expect("window_of_pane only returns live windows");
            (window.cols, window.rows)
        };
        let only_pane = self
            .windows
            .get(&source_window)
            .expect("window_of_pane only returns live windows")
            .layout
            == LayoutTree::leaf(pane);

        // The new window exists before the pane leaves the source, so an
        // only-pane break never leaves the session windowless. Inserting
        // it re-homes the pane's index entry first; the source's removal
        // below then leaves that entry alone.
        let window_id = self.ids.next_window();
        self.insert_window(
            session_id,
            MuxWindow {
                id: window_id,
                name: name.to_string(),
                layout: LayoutTree::leaf(pane),
                active: pane,
                cols,
                rows,
                zoomed: None,
            },
        );
        {
            let session = self
                .sessions
                .get_mut(&session_id)
                .expect("checked directly above");
            session.active = session.windows.len() - 1;
        }

        let source_closed = if only_pane {
            self.drop_empty_window(source_window);
            true
        } else {
            self.mutate_layout(source_window, |window| {
                window.layout.remove_pane(pane).expect("not the only pane");
                if window.active == pane {
                    window.active = window.layout.pane_ids()[0];
                }
                Ok(())
            })?;
            false
        };
        // A fresh, unzoomed window around the pane — a fit, not a mutation.
        self.sync_pane_sizes(window_id);
        Ok((window_id, source_window, source_closed))
    }

    /// `join-pane -s %N -t %M`: move `source` next to `target` — into
    /// `target`'s window when the panes live in different ones, the
    /// `split-window` arrangement rule (`-h` beside, `-v`/default
    /// below) and `-p` share applied to the moved pane. The moved pane
    /// becomes the destination window's active pane, like a split's new
    /// pane does. The source window closes when the pane was its last
    /// one — and that cascade CAN reach the session (the destination
    /// belongs to whichever window `target` lives in, so nothing
    /// backstops the source's session). The Ok payload is
    /// `(destination window, source window, whether the source window
    /// closed, the session removed by the cascade)`.
    pub fn join_pane(
        &mut self,
        source: PaneId,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
    ) -> Result<(WindowId, WindowId, bool, Option<SessionId>), MuxError> {
        if source == target {
            return Err(MuxError::SamePane(source));
        }
        let source_window = self
            .window_of_pane(source)
            .ok_or(MuxError::NoSuchPane(source))?;
        let target_window = self
            .window_of_pane(target)
            .ok_or(MuxError::NoSuchPane(target))?;
        let only_pane = self
            .windows
            .get(&source_window)
            .expect("window_of_pane only returns live windows")
            .layout
            == LayoutTree::leaf(source);
        let remove_source = |window: &mut MuxWindow| {
            window
                .layout
                .remove_pane(source)
                .expect("not the only pane");
            if window.active == source {
                window.active = window.layout.pane_ids()[0];
            }
        };
        let place_source = |window: &mut MuxWindow| {
            window
                .layout
                .split_pane(target, source, direction, new_share)
                .expect("target pane exists in its own window");
            window.active = source;
        };

        if source_window == target_window {
            if only_pane {
                // The target lives in the same single-pane window, so it
                // is the source itself — rejected above. Unreachable.
                return Err(MuxError::SamePane(source));
            }
            // One mutation, so no pane re-fits to the transient layout
            // between the removal and the re-split.
            self.mutate_layout(target_window, |window| {
                remove_source(window);
                place_source(window);
                Ok(())
            })?;
            return Ok((target_window, source_window, false, None));
        }

        let mut removed_session = None;
        if only_pane {
            removed_session = self.drop_empty_window(source_window);
        } else {
            self.mutate_layout(source_window, |window| {
                remove_source(window);
                Ok(())
            })?;
        }
        self.mutate_layout(target_window, |window| {
            place_source(window);
            Ok(())
        })?;
        Ok((target_window, source_window, only_pane, removed_session))
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

    /// `move-window -s @N -t <index>`: move `window_id` to `index` in
    /// its session's window list, clamping out-of-range positions to
    /// the ends. The active window is tracked by identity, not
    /// position — the window that was active stays active after the
    /// list moves.
    pub fn move_window(&mut self, window_id: WindowId, index: usize) -> Result<(), MuxError> {
        let session = self
            .session_of_window(window_id)
            .and_then(|id| self.sessions.get_mut(&id))
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        let active_window = session.windows.get(session.active).copied();
        let pos = session
            .windows
            .iter()
            .position(|w| *w == window_id)
            .expect("the index names this window's session");
        session.windows.remove(pos);
        let insert_at = index.min(session.windows.len());
        session.windows.insert(insert_at, window_id);
        if let Some(active) = active_window {
            session.active = session
                .windows
                .iter()
                .position(|w| *w == active)
                .expect("the active window is in the list");
        }
        Ok(())
    }

    /// `swap-window -s @A -t @B`: exchange the two windows' positions in
    /// their session's window list. Same session only — a swap across
    /// sessions changes window ownership, a different operation than a
    /// reorder. The active window is tracked by identity, not position.
    pub fn swap_windows(&mut self, a: WindowId, b: WindowId) -> Result<(), MuxError> {
        if a == b {
            return Ok(());
        }
        let session_id = match (self.session_of_window(a), self.session_of_window(b)) {
            (Some(sa), Some(sb)) if sa == sb => sa,
            _ => return Err(MuxError::WindowsInDifferentSessions(a, b)),
        };
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(MuxError::WindowsInDifferentSessions(a, b))?;
        let active_window = session.windows.get(session.active).copied();
        let pos_a = session
            .windows
            .iter()
            .position(|w| *w == a)
            .expect("the index names this window's session");
        let pos_b = session
            .windows
            .iter()
            .position(|w| *w == b)
            .expect("the index names this window's session");
        session.windows.swap(pos_a, pos_b);
        if let Some(active) = active_window {
            session.active = session
                .windows
                .iter()
                .position(|w| *w == active)
                .expect("the active window is in the list");
        }
        Ok(())
    }

    /// Grow or shrink `pane` by `cells` toward `direction` (tmux's
    /// `-L`/`-R`/`-U`/`-D`), adjusting the ratio of the split it borders —
    /// from either side of it.
    ///
    /// Only a split of the matching orientation can absorb the adjustment:
    /// `-L`/`-R` move a side-by-side divider, `-U`/`-D` a stacked one. A
    /// pane with no such bordering split — a lone pane, or one whose only
    /// bordering split is the other orientation — is an error, not a no-op.
    /// Pane terminals are resized to the new geometry. The Ok payload is the
    /// pane's window — the dispatcher's `%layout-change` target.
    pub fn resize_pane(
        &mut self,
        pane: PaneId,
        direction: ResizeDirection,
        cells: u32,
    ) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        self.mutate_layout(window_id, |window| {
            let Some((split_direction, ratio, target_is_first)) =
                window.layout.bordering_split(pane)
            else {
                return Err(MuxError::PaneNotResizable(pane));
            };
            let axis_matches = matches!(
                (direction, split_direction),
                (
                    ResizeDirection::Left | ResizeDirection::Right,
                    SplitDirection::Vertical
                ) | (
                    ResizeDirection::Up | ResizeDirection::Down,
                    SplitDirection::Horizontal
                )
            );
            if !axis_matches {
                return Err(MuxError::PaneNotResizable(pane));
            }
            let extent = match split_direction {
                SplitDirection::Vertical => window.cols as f32,
                SplitDirection::Horizontal => window.rows as f32,
            };
            let sign = match direction {
                ResizeDirection::Right | ResizeDirection::Down => 1.0,
                ResizeDirection::Left | ResizeDirection::Up => -1.0,
            };
            // The pane's own share of the split grows by the adjustment,
            // whichever side of the divider it sits on; the setter converts
            // back to the split's first-perspective ratio.
            let current_share = if target_is_first { ratio } else { 1.0 - ratio };
            let new_share = current_share + sign * (cells as f32) / extent;
            window
                .layout
                .set_bordering_share(pane, new_share)
                .expect("bordering_split found the split set_bordering_share adjusts");
            Ok(window_id)
        })
    }

    /// Set `pane`'s absolute width and/or height (tmux's `resize-pane -x`/
    /// `-y`), moving the pane's innermost enclosing split of the matching
    /// orientation — the renderer-driven form par-term sends.
    ///
    /// Either bound may be `None` (only the given axis is set). A pane with
    /// no enclosing split along a requested axis — one that already spans
    /// the window there — is an error, not a no-op: there is no divider to
    /// move. All-or-nothing: when either requested axis fails, neither is
    /// applied. Pane terminals are resized to the new geometry. The Ok
    /// payload is the pane's window — the dispatcher's `%layout-change`
    /// target.
    pub fn resize_pane_absolute(
        &mut self,
        pane: PaneId,
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        self.mutate_layout(window_id, |window| {
            let mut next = window.layout.clone();
            let bounds = [
                (cols, SplitDirection::Vertical, window.cols as usize),
                (rows, SplitDirection::Horizontal, window.rows as usize),
            ];
            for (bound, axis, extent) in bounds {
                let Some(cells) = bound else { continue };
                match next
                    .set_leaf_extent(pane, axis, cells, extent)
                    .map_err(|_| MuxError::NoSuchPane(pane))?
                {
                    crate::mux::layout::ExtentOutcome::Adjusted => {}
                    crate::mux::layout::ExtentOutcome::SpansAxis => {
                        return Err(MuxError::PaneNotResizable(pane));
                    }
                }
            }
            window.layout = next;
            Ok(window_id)
        })
    }

    /// Set a window's extent and re-fit every pane terminal to the
    /// re-divided geometry.
    ///
    /// The landing point of the window-size policy (par-mux.md Phase 4
    /// T4.C): `refresh-client -C WxH` carries a client's renderer size, and
    /// the latest such report wins — par-mux has no other client-size
    /// input, so "latest-attached-client" and "latest `-C`" are the same
    /// rule here.
    pub fn resize_window(
        &mut self,
        window_id: WindowId,
        cols: u16,
        rows: u16,
    ) -> Result<(), MuxError> {
        let window = self
            .windows
            .get_mut(&window_id)
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        window.cols = cols;
        window.rows = rows;
        self.sync_pane_sizes(window_id);
        Ok(())
    }

    /// Record the client's per-cell pixel size and re-fit every pane to it.
    ///
    /// `refresh-client -p WxH`'s landing point: the cell size is the one
    /// renderer metric every pane shares (grid extents differ per pane, the
    /// font does not), so it is held daemon-wide and every pane's terminal,
    /// PTY `TIOCGWINSZ`, and image cell-span math re-derive from it — see
    /// [`MuxPane::resize_with_cell_pixels`]. Latest report wins, matching
    /// the `-C` grid-size policy.
    pub fn set_client_cell_pixels(&mut self, cell_w: u16, cell_h: u16) {
        self.client_cell_pixels = Some((cell_w, cell_h));
        let window_ids: Vec<WindowId> = self.windows.keys().copied().collect();
        for window_id in window_ids {
            self.sync_pane_sizes(window_id);
        }
    }

    /// Record the client's theme colors and apply them to every pane
    /// terminal — the OSC 10/11 answer path. Each half is set only when
    /// reported; panes created later inherit at insert. Never persisted.
    pub fn set_client_colors(&mut self, fg: Option<Color>, bg: Option<Color>) {
        if fg.is_some() {
            self.client_fg = fg;
        }
        if bg.is_some() {
            self.client_bg = bg;
        }
        let fg = self.client_fg;
        let bg = self.client_bg;
        for pane in self.panes.values_mut() {
            pane.with_terminal_mut(|term| {
                if let Some(fg) = fg {
                    term.set_default_fg(fg);
                }
                if let Some(bg) = bg {
                    term.set_default_bg(bg);
                }
            });
        }
    }

    /// Apply the recorded cell pixel size to one just-inserted pane — the
    /// creation paths that build the window around the pane and never
    /// re-fit through [`Self::sync_pane_sizes`]. A no-op until a client
    /// has reported, so fresh daemons spawn with the construction default.
    fn apply_cell_pixels(&mut self, pane_id: PaneId, cols: u16, rows: u16) {
        if let Some((cell_w, cell_h)) = self.client_cell_pixels {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                if let Err(err) = pane.resize_with_cell_pixels(cols, rows, cell_w, cell_h) {
                    if !pane.dead() {
                        log::warn!(
                            "par-mux: resize of pane {pane_id} to {cols}x{rows} failed: {err}"
                        );
                    }
                }
            }
        }
        let (fg, bg) = (self.client_fg, self.client_bg);
        if fg.is_some() || bg.is_some() {
            if let Some(pane) = self.panes.get(&pane_id) {
                pane.with_terminal_mut(|term| {
                    if let Some(fg) = fg {
                        term.set_default_fg(fg);
                    }
                    if let Some(bg) = bg {
                        term.set_default_bg(bg);
                    }
                });
            }
        }
    }

    /// The single entry for layout-shape mutations (ARC-090): run `f` on
    /// the window, then end any zoom and re-fit every pane terminal. On
    /// `Err` the window must be untouched, so the zoom and sizes stay as
    /// they were — `f` validates before it edits.
    ///
    /// `f` holds `&mut MuxWindow` borrowed from the tree, so it cannot call
    /// other tree methods; resolve lookups such as `window_of_pane` first.
    ///
    /// The pane → window index is maintained here (ARC-096): panes the
    /// edit added map to this window, and panes it removed are dropped
    /// unless they already map elsewhere (a move re-homes the pane in its
    /// destination first).
    fn mutate_layout<R>(
        &mut self,
        window_id: WindowId,
        f: impl FnOnce(&mut MuxWindow) -> Result<R, MuxError>,
    ) -> Result<R, MuxError> {
        let window = self
            .windows
            .get_mut(&window_id)
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        let before = window.panes();
        let out = f(window)?;
        let after = window.panes();
        self.reindex_window_panes(window_id, &before, &after);
        self.end_layout_mutation(window_id);
        #[cfg(test)]
        self.assert_indexes_consistent();
        Ok(out)
    }

    /// Bring the pane → window index in line with one window's pane set
    /// changing from `before` to `after`.
    fn reindex_window_panes(&mut self, window_id: WindowId, before: &[PaneId], after: &[PaneId]) {
        for pane in before {
            if !after.contains(pane) && self.pane_window.get(pane) == Some(&window_id) {
                self.pane_window.remove(pane);
            }
        }
        for pane in after {
            self.pane_window.insert(*pane, window_id);
        }
    }

    /// The post-step of [`Self::mutate_layout`] alone, for a mutation
    /// already applied outside a closure (the kill cascade's removal).
    fn end_layout_mutation(&mut self, window_id: WindowId) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.zoomed = None;
        }
        self.sync_pane_sizes(window_id);
    }

    /// Resize every pane terminal (and PTY) in `window_id` to the window's
    /// current layout geometry — the step every extent-affecting mutation
    /// ends with, so the terminals `capture-pane` reads and clients render
    /// agree with the layout string the server broadcasts.
    ///
    /// A pane whose PTY cannot be resized (its child is gone) does not fail
    /// the structural mutation around it — the same best-effort treatment
    /// [`Self::kill_pane`] gives pane teardown. The terminal itself, which
    /// is what capture and rendering read, always resizes.
    pub(crate) fn sync_pane_sizes(&mut self, window_id: WindowId) {
        let Some(window) = self.windows.get(&window_id) else {
            return;
        };
        let zoomed = window.zoomed;
        let (window_cols, window_rows) = (window.cols as usize, window.rows as usize);
        let geometry = window
            .layout
            .geometry(0, 0, window.cols as usize, window.rows as usize);
        for pane_geometry in geometry {
            if let Some(pane) = self.panes.get_mut(&pane_geometry.pane) {
                // A reported cell pixel size rides every re-fit, so grid
                // changes keep XTWINOPS/TIOCGWINSZ/image-span math correct
                // instead of reverting to the construction default. A
                // zoomed pane takes the full window grid instead of its
                // layout cell.
                let (width, height) = if zoomed == Some(pane_geometry.pane) {
                    (window_cols, window_rows)
                } else {
                    (pane_geometry.width, pane_geometry.height)
                };
                let resized = match self.client_cell_pixels {
                    Some((cell_w, cell_h)) => {
                        pane.resize_with_cell_pixels(width as u16, height as u16, cell_w, cell_h)
                    }
                    None => pane.resize(width as u16, height as u16),
                };
                // Best-effort (see above), but visible. A held-dead pane's
                // PTY resize outcome is irrelevant, so it stays quiet.
                if let Err(err) = resized {
                    if !pane.dead() {
                        log::warn!(
                            "par-mux: resize of pane {} to {width}x{height} failed: {err}",
                            pane_geometry.pane
                        );
                    }
                }
            }
        }
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

    /// Make `window_id` its session's active window.
    ///
    /// The session is derived from the window itself — a client targets
    /// `@window_id` directly, the way `select-pane -t %pane_id` already does.
    pub fn select_window(&mut self, window_id: WindowId) -> Result<(), MuxError> {
        if !self.windows.contains_key(&window_id) {
            return Err(MuxError::NoSuchWindow(window_id));
        }
        let session = self
            .session_of_window(window_id)
            .and_then(|id| self.sessions.get_mut(&id))
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        let index = session
            .windows
            .iter()
            .position(|w| *w == window_id)
            .expect("the index names this window's session");
        session.active = index;
        Ok(())
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

/// The outcome of matching a name against the tree: the resolvers above
/// turn each case into its id-pass-through, not-found, or ambiguity error.
enum Match<I> {
    None,
    One(I),
    Many(Vec<I>),
}

/// Reduce a name's candidate ids to zero/one/many, sorted so an ambiguity
/// error lists candidates in id order regardless of map iteration order.
fn match_name<I: Ord + Copy>(candidates: impl Iterator<Item = I>) -> Match<I> {
    let mut ids: Vec<I> = candidates.collect();
    match ids.len() {
        0 => Match::None,
        1 => Match::One(ids.remove(0)),
        _ => {
            ids.sort_unstable();
            Match::Many(ids)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::pane::test_support::ContextRecordingFactory;
    use crate::mux::pane::ShellPaneFactory;

    fn tree() -> MuxTree {
        MuxTree::new(Box::new(ShellPaneFactory::default()))
    }

    fn recording_tree() -> (MuxTree, ContextRecordingFactory) {
        let factory = ContextRecordingFactory::default();
        (MuxTree::new(Box::new(factory.clone())), factory)
    }

    #[test]
    fn session_env_reaches_panes_spawned_after_it_is_set_only() {
        let (mut tree, factory) = recording_tree();
        let initial = BTreeMap::from([("A".to_string(), "1".to_string())]);
        let session = tree.new_session_with_env("work", 80, 24, initial).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];
        assert_eq!(
            factory.spawn_of(first).env.get("A").map(String::as_str),
            Some("1")
        );

        tree.set_session_env(session, "B", Some("2")).unwrap();
        tree.set_session_env(session, "A", None).unwrap();
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        let later = factory.spawn_of(second).env;
        assert_eq!(later.get("B").map(String::as_str), Some("2"));
        assert!(
            !later.contains_key("A"),
            "an unset var is gone for new panes"
        );
        assert_eq!(
            factory.spawn_of(first).env.get("B"),
            None,
            "the earlier pane's spawn is not rewritten"
        );
        let third_window = tree.new_window(session, "w2", 80, 24).unwrap();
        let third = tree.window(third_window).unwrap().panes()[0];
        assert_eq!(factory.spawn_of(third).env, later);

        assert!(tree.set_session_env(SessionId(99), "X", Some("y")).is_err());
    }

    #[test]
    fn new_session_spawns_with_its_session_and_window_identity() {
        let (mut tree, factory) = recording_tree();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let pane = tree.window(window).unwrap().panes()[0];
        let spawn = factory.spawn_of(pane);
        assert_eq!(spawn.session, Some((session, "work".to_string())));
        assert_eq!(spawn.window, Some(window));
    }

    #[test]
    fn new_window_spawns_with_its_session_and_new_window_identity() {
        let (mut tree, factory) = recording_tree();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.new_window(session, "second", 80, 24).unwrap();
        let pane = tree.window(window).unwrap().panes()[0];
        let spawn = factory.spawn_of(pane);
        assert_eq!(spawn.session, Some((session, "work".to_string())));
        assert_eq!(spawn.window, Some(window));
    }

    /// Card 01a0d9e6f012, criterion 2: an OSC 11 query inside a pane
    /// answers with the CLIENT's theme background — what the client
    /// actually renders — once `set-client-colors` has reported it, and a
    /// pane created later inherits the same answer.
    #[test]
    fn client_colors_answer_osc_queries_in_existing_and_later_panes() {
        let mut tree = tree();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];

        fn osc_11_reply(tree: &MuxTree, pane: PaneId) -> String {
            let pane = tree.pane(pane).unwrap();
            let terminal = pane.terminal();
            let mut term = terminal.write();
            term.process(b"\x1b]11;?\x1b\\");
            String::from_utf8(term.drain_responses()).unwrap()
        }

        let before = osc_11_reply(&tree, first);
        assert!(
            !before.contains("2e2e3e1e1e") && !before.contains("1e1e/2e2e"),
            "pre-report: the core theme answers, not the client's"
        );

        tree.set_client_colors(None, Some(Color::Rgb(0x1e, 0x1e, 0x2e)));
        let after = osc_11_reply(&tree, first);
        assert!(
            after.contains("rgb:1e1e/1e1e/2e2e"),
            "OSC 11 answers with the client bg: {after}"
        );

        let later_window = tree.new_window(session, "w2", 80, 24).unwrap();
        let later = tree.window(later_window).unwrap().panes()[0];
        let inherited = osc_11_reply(&tree, later);
        assert!(
            inherited.contains("rgb:1e1e/1e1e/2e2e"),
            "a pane created after the report inherits it: {inherited}"
        );
    }

    /// Card 01a0d9e6f012: the client's cell pixel size is daemon-wide state
    /// that reaches every pane — existing ones re-fit through the sync path,
    /// panes created later inherit it at insert — and the pane terminal's
    /// pixel state (XTWINOPS 14 t's answer) and graphics cell dimensions
    /// (image cell-span math) both derive from it.
    #[test]
    fn client_cell_pixels_reach_existing_and_later_panes() {
        let mut tree = tree();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];

        {
            let term = tree.pane(first).unwrap().terminal();
            let term = term.read();
            assert_eq!(
                (term.pixel_width, term.pixel_height),
                (800, 480),
                "pre-report: the 10x20 construction default (80x24 grid)"
            );
        }

        tree.set_client_cell_pixels(12, 24);
        {
            let term = tree.pane(first).unwrap().terminal();
            let term = term.read();
            assert_eq!(
                (term.pixel_width, term.pixel_height),
                (960, 576),
                "an existing pane re-fits: 80x24 cells at 12x24 px"
            );
            assert_eq!(
                term.graphics.cell_dimensions,
                (12, 24),
                "image cell-span math uses the client's cell size, not the (1,2) default"
            );
        }

        // A pane created after the report inherits it at insert — the
        // creation paths that never run sync_pane_sizes.
        let later_window = tree.new_window(session, "w2", 40, 10).unwrap();
        let later = tree.window(later_window).unwrap().panes()[0];
        {
            let term = tree.pane(later).unwrap().terminal();
            let term = term.read();
            assert_eq!(
                (term.pixel_width, term.pixel_height),
                (480, 240),
                "a later pane derives its totals from its own 40x10 grid"
            );
        }
    }

    #[test]
    fn split_pane_spawns_with_the_target_windows_identity() {
        let (mut tree, factory) = recording_tree();
        tree.new_session("other", 80, 24).unwrap();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.new_window(session, "second", 80, 24).unwrap();
        let target = tree.window(window).unwrap().panes()[0];
        let pane = tree
            .split_pane(target, SplitDirection::Horizontal, 0.5, None)
            .unwrap();
        let spawn = factory.spawn_of(pane);
        assert_eq!(spawn.session, Some((session, "work".to_string())));
        assert_eq!(spawn.window, Some(window));
    }

    /// `split-window -c` / `new-window -c`: the start directory reaches the
    /// factory's spawn context on both dispatcher forms (the plain wrappers
    /// keep the factory-wide default).
    #[test]
    fn a_start_directory_reaches_the_spawn_on_split_and_new_window() {
        let (mut tree, factory) = recording_tree();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let target = tree.window(window).unwrap().panes()[0];
        let dir = tempfile::tempdir().unwrap();
        let (split, _) = tree
            .split_pane_in_window(
                target,
                SplitDirection::Horizontal,
                0.5,
                None,
                Some(dir.path()),
            )
            .unwrap();
        assert_eq!(
            factory.spawn_of(split).cwd.as_deref(),
            Some(dir.path()),
            "the split pane spawns in the -c directory"
        );

        let second = tree
            .new_window_with_cwd(session, "second", 80, 24, Some(dir.path()))
            .unwrap();
        let pane = tree.window(second).unwrap().panes()[0];
        assert_eq!(
            factory.spawn_of(pane).cwd.as_deref(),
            Some(dir.path()),
            "the new window's pane spawns in the -c directory"
        );
    }

    #[test]
    fn new_session_creates_a_window_and_a_pane() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).expect("session creates");

        let session = tree.session(session_id).expect("session exists");
        assert_eq!(session.name, "main");
        assert_eq!(
            session.windows.len(),
            1,
            "a new session has exactly one window"
        );

        let window_id = session.windows[0];
        let window = tree.window(window_id).expect("window exists");
        assert_eq!(window.panes().len(), 1, "a new window has exactly one pane");

        let pane_id = window.panes()[0];
        assert!(tree.pane(pane_id).is_some(), "the pane is in the tree");
    }

    #[test]
    fn ids_are_unique_across_sessions() {
        let mut tree = tree();
        let a = tree.new_session("a", 80, 24).unwrap();
        let b = tree.new_session("b", 80, 24).unwrap();
        assert_ne!(a, b);
        assert_eq!(tree.sessions().len(), 2);
    }

    #[test]
    fn splitting_a_pane_joins_the_window_and_takes_its_requested_share() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];

        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.25, None)
            .expect("split creates");
        let window = tree.window(window_id).unwrap();
        assert_eq!(window.panes().len(), 2);
        assert!(window.panes().contains(&second));

        // -p 25 semantics: the NEW pane gets a quarter of the 80-column
        // extent, the target keeps the rest.
        let geo = window
            .layout
            .geometry(0, 0, window.cols as usize, window.rows as usize);
        let width_of = |pane| {
            geo.iter()
                .find(|g| g.pane == pane)
                .unwrap_or_else(|| panic!("pane {pane} in geometry"))
                .width
        };
        assert_eq!(width_of(first), 60);
        assert_eq!(width_of(second), 20);
        assert_eq!(
            window.active, second,
            "tmux's split-window makes the new pane active"
        );
    }

    #[test]
    fn killing_a_pane_removes_it_from_its_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let extra = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.kill_pane(extra).expect("kill succeeds");
        assert!(tree.pane(extra).is_none(), "pane is gone from the tree");
        assert_eq!(tree.window(window_id).unwrap().panes().len(), 1);
    }

    /// A pane whose child ignores SIGHUP must not survive its kill as a
    /// zombie, and the kill must not hold the tree lock through the ~200 ms
    /// SIGHUP-grace poll portable-pty runs before SIGKILL (card
    /// 01a0d9b4789c79219e7720d61729544c).
    #[cfg(unix)]
    #[test]
    fn killing_a_hup_ignoring_pane_reaps_it_and_returns_promptly() {
        let mut tree = tree();
        let session_id = tree.new_session("zombies", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        // The loop keeps the SHELL itself as the live process: `sh -c` (and
        // zsh) exec-optimizes a trailing simple command, which would turn
        // the pane's process into `sleep` — a fresh binary that never ran
        // the trap and dies to the first SIGHUP. The echo is a readiness
        // marker: kill must not race the shell's own startup (a SIGHUP
        // delivered before the trap line runs kills the shell outright).
        let doomed = tree
            .split_pane(
                first,
                SplitDirection::Vertical,
                0.5,
                Some("trap '' HUP; echo PANEMUX-TRAP-SET; while true; do sleep 57; done"),
            )
            .unwrap();
        let pid = tree
            .pane(doomed)
            .expect("doomed pane exists")
            .child_pid()
            .expect("child pid");
        let ready = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let screen = tree
                .pane(doomed)
                .expect("doomed pane exists")
                .terminal()
                .read()
                .content();
            if screen.contains("PANEMUX-TRAP-SET") {
                break;
            }
            assert!(
                std::time::Instant::now() < ready,
                "trap marker never reached the pane screen: {screen}"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        let started = std::time::Instant::now();
        tree.kill_pane(doomed).expect("kill succeeds");
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(150),
            "kill_pane held the tree lock through the SIGHUP grace poll: {elapsed:?}"
        );

        // A reaped child vanishes from the process table; a zombie keeps
        // answering signal 0 until someone waits for it. The detached kill
        // needs a moment, so poll to a deadline instead of asserting at
        // once — the assertion is that it EVER goes away, promptly.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let gone = unsafe { libc::kill(pid as libc::pid_t, 0) != 0 };
            if gone {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child {pid} still in the process table after kill — unreaped zombie"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    #[test]
    fn killing_the_last_pane_closes_its_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let pane_id = tree.window(window_id).unwrap().panes()[0];

        tree.kill_pane(pane_id).expect("kill succeeds");
        assert!(
            tree.window(window_id).is_none(),
            "a window with no panes does not survive — matches tmux"
        );
    }

    #[test]
    fn killing_the_last_pane_of_the_only_window_removes_the_session() {
        // The second half of the tmux cascade: window closes, and a session
        // with no windows does not linger either.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let pane_id = tree.window(window_id).unwrap().panes()[0];

        tree.kill_pane(pane_id).expect("kill succeeds");
        assert!(tree.window(window_id).is_none());
        assert!(
            tree.session(session_id).is_none(),
            "a session with no windows does not survive — matches tmux"
        );
    }

    #[test]
    fn kill_results_name_the_session_the_cascade_removed() {
        // The %sessions-changed cue: both entry points report the removed
        // session, and report None when the session survives.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let second = tree.new_window(session_id, "logs", 80, 24).unwrap();
        let pane_id = tree.window(second).unwrap().panes()[0];

        let (_, removed) = tree.kill_pane(pane_id).expect("kill succeeds");
        assert_eq!(removed, None, "the session survives its non-last window");
        let (_, removed) = tree
            .kill_pane(tree.window(window_id).unwrap().panes()[0])
            .expect("kill succeeds");
        assert_eq!(removed, Some(session_id), "the cascade names the session");

        let session_id = tree.new_session("next", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        assert_eq!(
            tree.kill_window(window_id).expect("kill succeeds"),
            Some(session_id),
            "kill-window names the session it emptied"
        );
    }

    #[test]
    fn killing_an_unknown_pane_is_an_error_not_a_panic() {
        let mut tree = tree();
        let result = tree.kill_pane(PaneId(999));
        assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
    }

    #[test]
    fn new_window_adds_a_window_to_the_session() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();

        let window_id = tree
            .new_window(session_id, "logs", 80, 24)
            .expect("window creates");
        let session = tree.session(session_id).unwrap();
        assert_eq!(session.windows.len(), 2);
        assert!(session.windows.contains(&window_id));
        assert_eq!(tree.window(window_id).unwrap().name, "logs");
    }

    #[test]
    fn new_window_rejects_an_unknown_session() {
        let mut tree = tree();
        let result = tree.new_window(SessionId(999), "logs", 80, 24);
        assert!(matches!(result, Err(MuxError::NoSuchSession(_))));
    }

    #[test]
    fn split_pane_rejects_an_unknown_pane() {
        let mut tree = tree();
        let result = tree.split_pane(PaneId(999), SplitDirection::Vertical, 0.5, None);
        assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
    }

    #[test]
    fn select_pane_changes_the_active_pane() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        assert_eq!(tree.window(window_id).unwrap().active, second);

        tree.select_pane(first).expect("select succeeds");
        assert_eq!(tree.window(window_id).unwrap().active, first);
    }

    #[test]
    fn select_pane_rejects_an_unknown_pane() {
        let mut tree = tree();
        let result = tree.select_pane(PaneId(999));
        assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
    }

    #[test]
    fn swap_panes_exchanges_positions_within_a_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.swap_panes(first, second).expect("swap succeeds");
        assert_eq!(
            tree.window(window_id).unwrap().panes(),
            vec![second, first],
            "the panes traded tree positions"
        );
    }

    #[test]
    fn swap_panes_across_windows_is_an_error() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let first_window = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(first_window).unwrap().panes()[0];
        let second_window = tree.new_window(session_id, "logs", 80, 24).unwrap();
        let outsider = tree.window(second_window).unwrap().panes()[0];

        let result = tree.swap_panes(first, outsider);
        assert!(matches!(
            result,
            Err(MuxError::PanesInDifferentWindows(a, b)) if a == first && b == outsider
        ));
    }

    #[test]
    fn resize_pane_grows_the_bordering_split_by_cells() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        tree.split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        // -R 10 on a 0.5 ratio over 80 columns: 0.5 + 10/80 = 0.625.
        tree.resize_pane(first, ResizeDirection::Right, 10)
            .expect("resize succeeds");
        let window = tree.window(window_id).unwrap();
        let geo = window
            .layout
            .geometry(0, 0, window.cols as usize, window.rows as usize);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(width_of(first), 50);
        assert_eq!(geo.iter().find(|g| g.pane != first).unwrap().width, 30);
    }

    #[test]
    fn resize_pane_on_the_wrong_axis_is_an_error() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        // A stacked (Horizontal) split: -R moves a side-by-side divider.
        tree.split_pane(first, SplitDirection::Horizontal, 0.5, None)
            .unwrap();

        let result = tree.resize_pane(first, ResizeDirection::Right, 5);
        assert!(matches!(result, Err(MuxError::PaneNotResizable(_))));
    }

    #[test]
    fn resize_pane_on_a_lone_pane_is_an_error() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];

        let result = tree.resize_pane(first, ResizeDirection::Right, 5);
        assert!(matches!(result, Err(MuxError::PaneNotResizable(_))));
    }

    #[test]
    fn split_pane_resizes_both_terminals_to_the_layout_geometry() {
        // The Phase 4 T4.C fidelity contract: the layout tree is the source
        // of truth for pane extents, and the terminals (with their PTYs)
        // follow it. Before T4.C the new pane spawned at the window's full
        // size and the target kept its old size, so capture-pane and client
        // rendering disagreed with the layout string.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];

        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        let size_of = |pane| {
            tree.pane(pane)
                .expect("pane in tree")
                .terminal()
                .read()
                .size()
        };
        assert_eq!(size_of(first), (40, 24), "the target shrank to its half");
        assert_eq!(
            size_of(second),
            (40, 24),
            "the new pane spawned at its geometry, not the window's full size"
        );
    }

    #[test]
    fn resize_pane_relative_syncs_pane_terminals() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.resize_pane(first, ResizeDirection::Right, 10).unwrap();

        let size_of = |pane| tree.pane(pane).unwrap().terminal().read().size();
        assert_eq!(size_of(first), (50, 24));
        assert_eq!(size_of(second), (30, 24));
    }

    #[test]
    fn resize_pane_relative_works_for_the_second_pane_too() {
        // A pane on either side of its bordering split can grow; before
        // T4.C only the split's `first` child was resizable.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        // -R on the right-hand pane: it grows by taking from its sibling.
        tree.resize_pane(second, ResizeDirection::Right, 10)
            .unwrap();

        let geo = tree
            .window(window_id)
            .unwrap()
            .layout
            .geometry(0, 0, 80, 24);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(width_of(second), 50);
        assert_eq!(width_of(first), 30);
    }

    #[test]
    fn resize_pane_absolute_sets_exact_dimensions() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        tree.split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.resize_pane_absolute(first, Some(25), None).unwrap();

        let geo = tree
            .window(window_id)
            .unwrap()
            .layout
            .geometry(0, 0, 80, 24);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(width_of(first), 25);
        assert_eq!(
            width_of(tree.window(window_id).unwrap().panes()[1]),
            55,
            "the sibling absorbs the difference"
        );
        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (25, 24),
            "the terminal follows the absolute size"
        );
    }

    #[test]
    fn resize_pane_absolute_through_a_cross_orientation_ancestor() {
        // Split(V){0, Split(H){1,2}}: pane 1's width is set by the OUTER
        // vertical divider — its direct parent is horizontal, so a naive
        // direct-parent lookup would wrongly call it unresizable.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        let third = tree
            .split_pane(second, SplitDirection::Horizontal, 0.5, None)
            .unwrap();

        tree.resize_pane_absolute(second, Some(20), None).unwrap();

        let geo = tree
            .window(window_id)
            .unwrap()
            .layout
            .geometry(0, 0, 80, 24);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(width_of(second), 20);
        assert_eq!(width_of(third), 20, "the stacked sibling shares the width");
        assert_eq!(width_of(first), 60);
    }

    #[test]
    fn resize_pane_absolute_on_a_spanning_pane_is_an_error() {
        // A lone pane spans both axes; there is no divider to move.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];

        let result = tree.resize_pane_absolute(first, Some(40), None);
        assert!(matches!(result, Err(MuxError::PaneNotResizable(_))));
    }

    /// `resize-pane -Z`: two panes 40x24 each; zooming takes the full
    /// window grid while the hidden pane keeps its size, and unzooming
    /// restores the exact prior extent — the zoom never edits the layout.
    #[test]
    fn zoom_toggles_full_grid_and_restores_exactly() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (40, 24));

        tree.zoom_pane(first).unwrap();
        assert_eq!(tree.window(window_id).unwrap().zoomed, Some(first));
        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (80, 24),
            "the zoomed pane takes the full window grid"
        );
        assert_eq!(
            tree.pane(second).unwrap().terminal().read().size(),
            (40, 24),
            "the hidden pane keeps its size"
        );

        tree.zoom_pane(first).unwrap();
        assert_eq!(tree.window(window_id).unwrap().zoomed, None);
        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (40, 24),
            "unzoom restores the exact prior extent"
        );
    }

    /// Zooming a different pane moves the zoom to it (tmux semantics).
    #[test]
    fn zoom_moves_to_another_pane() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.zoom_pane(first).unwrap();
        tree.zoom_pane(second).unwrap();

        assert_eq!(tree.window(window_id).unwrap().zoomed, Some(second));
        assert_eq!(
            tree.pane(second).unwrap().terminal().read().size(),
            (80, 24)
        );
        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (40, 24),
            "the previous zoom target returns to its layout cell"
        );
    }

    /// A window resize while zoomed re-fits the zoomed pane to the NEW
    /// full grid.
    #[test]
    fn window_resize_while_zoomed_follows_the_new_grid() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        tree.split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.zoom_pane(first).unwrap();
        tree.resize_window(window_id, 100, 30).unwrap();

        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (100, 30),
            "the zoom tracks the window's new extent"
        );
    }

    /// Every layout mutation ends the zoom: split, kill, swap, and
    /// select-pane to another pane unzoom first; selecting the zoomed
    /// pane itself keeps it (tmux's rule).
    #[test]
    fn layout_mutations_unzoom_the_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        // split-window: the new pane must be visible.
        tree.zoom_pane(first).unwrap();
        let third = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        assert_eq!(tree.window(window_id).unwrap().zoomed, None);
        let geo = tree
            .window(window_id)
            .unwrap()
            .layout
            .geometry(0, 0, 80, 24);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (width_of(first), 24),
            "the split target returns to its layout cell"
        );

        // kill-pane of a non-zoomed pane still changes the layout.
        tree.zoom_pane(first).unwrap();
        tree.kill_pane(third).unwrap();
        assert_eq!(tree.window(window_id).unwrap().zoomed, None);

        // kill-pane of the zoomed pane itself.
        tree.zoom_pane(second).unwrap();
        tree.kill_pane(second).unwrap();
        assert_eq!(tree.window(window_id).unwrap().zoomed, None);

        // swap-pane: the traded geometry no longer matches the zoom.
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        tree.zoom_pane(first).unwrap();
        tree.swap_panes(first, second).unwrap();
        assert_eq!(tree.window(window_id).unwrap().zoomed, None);

        // select-pane to another pane reveals the layout; to the zoomed
        // pane itself keeps the zoom.
        tree.zoom_pane(first).unwrap();
        tree.select_pane(second).unwrap();
        assert_eq!(tree.window(window_id).unwrap().zoomed, None);
        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (40, 24),
            "unzoom on select restores the pane's layout cell"
        );
        tree.zoom_pane(first).unwrap();
        tree.select_pane(first).unwrap();
        assert_eq!(tree.window(window_id).unwrap().zoomed, Some(first));
    }

    /// Two 40-wide panes side by side in an 80x24 window, `first` zoomed.
    fn zoomed_split() -> (MuxTree, WindowId, PaneId, PaneId) {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        tree.zoom_pane(first).unwrap();
        (tree, window_id, first, second)
    }

    /// ARC-090 audit probe: `resize-pane -x` while zoomed must unzoom
    /// first (tmux's `server_unzoom_window`), not silently rewrite the
    /// hidden layout so the next unzoom lands on 79/1.
    #[test]
    fn absolute_resize_while_zoomed_unzooms_first() {
        let (mut tree, window_id, first, second) = zoomed_split();

        tree.resize_pane_absolute(first, Some(79), None).unwrap();

        assert_eq!(tree.window(window_id).unwrap().zoomed, None);
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (79, 24));
        assert_eq!(tree.pane(second).unwrap().terminal().read().size(), (1, 24));
    }

    #[test]
    fn relative_resize_while_zoomed_unzooms_first() {
        let (mut tree, window_id, first, second) = zoomed_split();

        tree.resize_pane(first, ResizeDirection::Right, 5).unwrap();

        assert_eq!(tree.window(window_id).unwrap().zoomed, None);
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (45, 24));
        assert_eq!(
            tree.pane(second).unwrap().terminal().read().size(),
            (35, 24)
        );
    }

    /// A failed command changes nothing: a wrong-axis resize keeps the
    /// zoom and the zoomed pane's full-grid size.
    #[test]
    fn a_rejected_resize_keeps_the_zoom() {
        let (mut tree, window_id, first, _second) = zoomed_split();

        let relative = tree.resize_pane(first, ResizeDirection::Up, 1);
        assert!(matches!(relative, Err(MuxError::PaneNotResizable(_))));
        let absolute = tree.resize_pane_absolute(first, None, Some(10));
        assert!(matches!(absolute, Err(MuxError::PaneNotResizable(_))));

        assert_eq!(tree.window(window_id).unwrap().zoomed, Some(first));
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (80, 24));
    }

    /// `-x 60 -y 10` where the pane spans the vertical axis: the y half
    /// fails, so the x half must not be left applied (all-or-nothing).
    #[test]
    fn a_half_failing_absolute_resize_applies_neither_axis() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        let before = tree.window(window_id).unwrap().layout.clone();

        let result = tree.resize_pane_absolute(first, Some(60), Some(10));

        assert!(matches!(result, Err(MuxError::PaneNotResizable(_))));
        assert_eq!(tree.window(window_id).unwrap().layout, before);
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (40, 24));
        assert_eq!(
            tree.pane(second).unwrap().terminal().read().size(),
            (40, 24)
        );
    }

    #[test]
    fn zoom_rejects_an_unknown_pane() {
        let mut tree = tree();
        let result = tree.zoom_pane(PaneId(9999));
        assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
    }

    /// `break-pane` + `join-pane` round trip: breaking a pane out of a
    /// two-pane window gives it a new full-grid window (the session's
    /// active one) while the survivor re-fits; joining it back beside
    /// the survivor restores the two-pane layout and closes the
    /// one-pane window it leaves behind.
    #[test]
    fn break_then_join_restores_two_panes() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let first_window = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(first_window).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (40, 24));

        let (new_window, source, source_closed) = tree.break_pane(first, "broken").unwrap();
        assert_eq!(source, first_window);
        assert!(!source_closed, "the source window keeps its other pane");
        assert_eq!(
            tree.session(session_id).unwrap().windows,
            vec![first_window, new_window],
            "the new window is appended to the session"
        );
        assert_eq!(tree.session(session_id).unwrap().active, 1);
        assert_eq!(tree.window(new_window).unwrap().name, "broken");
        assert_eq!(tree.window(new_window).unwrap().panes(), vec![first]);
        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (80, 24),
            "the broken pane takes the new window's full grid"
        );
        assert_eq!(
            tree.pane(second).unwrap().terminal().read().size(),
            (80, 24),
            "the survivor grows into the freed extent"
        );

        let (dest, src, closed, removed) = tree
            .join_pane(first, second, SplitDirection::Vertical, 0.5)
            .unwrap();
        assert_eq!(dest, first_window);
        assert_eq!(src, new_window);
        assert!(
            closed,
            "the break's one-pane window closed when its pane left"
        );
        assert_eq!(removed, None, "the session kept the destination window");
        // The split machinery puts the target first and the moved pane
        // second, exactly like a fresh split of the survivor.
        assert_eq!(
            tree.window(first_window).unwrap().panes(),
            vec![second, first]
        );
        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (40, 24),
            "the rejoined pane returns to its half"
        );
        assert_eq!(
            tree.session(session_id).unwrap().windows,
            vec![first_window],
            "the closed window left the session list"
        );
    }

    /// Breaking a window's only pane moves the window instead of killing
    /// the session — the new window exists before the source drops.
    #[test]
    fn breaking_the_only_pane_closes_the_source_not_the_session() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let first_window = tree.session(session_id).unwrap().windows[0];
        let only = tree.window(first_window).unwrap().panes()[0];

        let (new_window, source, source_closed) = tree.break_pane(only, "solo").unwrap();
        assert!(source_closed, "the emptied source window closed");
        assert_eq!(source, first_window);
        assert!(tree.window(first_window).is_none());
        assert!(tree.session(session_id).is_some(), "the session survives");
        assert_eq!(
            tree.session(session_id).unwrap().windows,
            vec![new_window],
            "the pane's new window replaced the source in the list"
        );
        assert_eq!(tree.window(new_window).unwrap().panes(), vec![only]);
    }

    /// Joining the last pane out of a session's only window closes that
    /// session — the destination belongs to the target's window, so
    /// nothing backstops the source's session (the mirror image of
    /// break-pane's guarantee).
    #[test]
    fn joining_the_last_pane_out_of_the_only_window_closes_its_session() {
        let mut tree = tree();
        let donor = tree.new_session("donor", 80, 24).unwrap();
        let donor_window = tree.session(donor).unwrap().windows[0];
        let mover = tree.window(donor_window).unwrap().panes()[0];
        let keeper = tree.new_session("keeper", 80, 24).unwrap();
        let keeper_window = tree.session(keeper).unwrap().windows[0];
        let anchor = tree.window(keeper_window).unwrap().panes()[0];

        let (dest, src, closed, removed) = tree
            .join_pane(mover, anchor, SplitDirection::Horizontal, 0.5)
            .unwrap();
        assert_eq!(dest, keeper_window);
        assert_eq!(src, donor_window);
        assert!(closed);
        assert_eq!(removed, Some(donor), "the donor session closed");
        assert!(tree.session(donor).is_none());
        assert!(tree.window(donor_window).is_none());
        // The moved pane landed next to the anchor, below it.
        assert_eq!(
            tree.window(keeper_window).unwrap().panes(),
            vec![anchor, mover]
        );
    }

    /// `join-pane` rejects a pane onto itself and unknown panes, and a
    /// same-window join is a within-window move, not an error.
    #[test]
    fn join_pane_rejects_self_and_unknown_but_allows_same_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        assert!(matches!(
            tree.join_pane(first, first, SplitDirection::Vertical, 0.5),
            Err(MuxError::SamePane(_first))
        ));
        assert!(matches!(
            tree.join_pane(PaneId(9999), first, SplitDirection::Vertical, 0.5),
            Err(MuxError::NoSuchPane(_))
        ));

        // Same window: first moves below second (target first, moved
        // pane second), and the window keeps both panes.
        let (dest, src, closed, removed) = tree
            .join_pane(first, second, SplitDirection::Horizontal, 0.5)
            .unwrap();
        assert_eq!((dest, src), (window_id, window_id));
        assert!(!closed);
        assert_eq!(removed, None);
        assert_eq!(tree.window(window_id).unwrap().panes(), vec![second, first]);
    }

    /// Break and join are layout mutations — a zoomed window unzooms.
    #[test]
    fn break_and_join_end_a_zoom() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let first_window = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(first_window).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        let other_window = tree.new_window(session_id, "other", 80, 24).unwrap();
        let other = tree.window(other_window).unwrap().panes()[0];

        tree.zoom_pane(second).unwrap();
        let (new_window, _, _) = tree.break_pane(second, "zoomed").unwrap();
        assert_eq!(tree.window(new_window).unwrap().zoomed, None);

        tree.zoom_pane(other).unwrap();
        tree.join_pane(other, first, SplitDirection::Vertical, 0.5)
            .unwrap();
        assert_eq!(tree.window(first_window).unwrap().zoomed, None);

        // A same-window join is one mutation: it ends the zoom and leaves
        // every pane fitted to the final layout.
        tree.zoom_pane(first).unwrap();
        tree.join_pane(first, other, SplitDirection::Horizontal, 0.5)
            .unwrap();
        let window = tree.window(first_window).unwrap();
        assert_eq!(window.zoomed, None);
        for geometry in window.layout.geometry(0, 0, 80, 24) {
            assert_eq!(
                tree.pane(geometry.pane).unwrap().terminal().read().size(),
                (geometry.width, geometry.height),
                "{} fits its layout cell",
                geometry.pane
            );
        }
    }

    /// ARC-096: the pane → window and window → session indexes match a
    /// brute-force scan after every structural command.
    #[test]
    fn reverse_indexes_track_every_structural_command() {
        let mut tree = tree();
        let check = |tree: &MuxTree| tree.assert_indexes_consistent();

        let s0 = tree.new_session("a", 80, 24).unwrap();
        check(&tree);
        let w0 = tree.session(s0).unwrap().windows[0];
        let p0 = tree.window(w0).unwrap().panes()[0];
        let p1 = tree
            .split_pane(p0, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        check(&tree);
        let p2 = tree
            .split_pane(p1, SplitDirection::Horizontal, 0.5, None)
            .unwrap();
        check(&tree);
        assert_eq!(tree.window_of_pane(p2), Some(w0));

        tree.swap_panes(p0, p2).unwrap();
        check(&tree);

        let w1 = tree.new_window(s0, "b", 80, 24).unwrap();
        check(&tree);
        assert_eq!(tree.session_of_window(w1), Some(s0));

        // break-pane: p2 moves to a fresh window in the same session.
        let (w2, _, closed) = tree.break_pane(p2, "broken").unwrap();
        check(&tree);
        assert!(!closed);
        assert_eq!(tree.window_of_pane(p2), Some(w2));

        // join-pane across windows; w2 empties and closes.
        let (dest, _, closed, _) = tree
            .join_pane(p2, p0, SplitDirection::Vertical, 0.5)
            .unwrap();
        check(&tree);
        assert!(closed);
        assert_eq!(dest, w0);
        assert_eq!(tree.window_of_pane(p2), Some(w0));
        assert_eq!(tree.session_of_window(w2), None);

        tree.move_window(w1, 0).unwrap();
        check(&tree);
        tree.swap_windows(w0, w1).unwrap();
        check(&tree);

        // respawn swaps the pane in place under the same id.
        let factory = tree.factory();
        let plan = tree.begin_respawn(p1, true, None, None).unwrap();
        let respawned = factory
            .create_pane(plan.pane_id, plan.cols, plan.rows, None, &plan.context())
            .unwrap();
        tree.complete_respawn(plan, respawned).unwrap();
        check(&tree);
        assert_eq!(tree.window_of_pane(p1), Some(w0));

        // kill-pane of a non-last pane, then of a window's last pane.
        tree.kill_pane(p2).unwrap();
        check(&tree);
        assert_eq!(tree.window_of_pane(p2), None);
        let w1_pane = tree.window(w1).unwrap().panes()[0];
        let (_, removed) = tree.kill_pane(w1_pane).unwrap();
        check(&tree);
        assert_eq!(removed, None, "w0 keeps the session alive");
        assert_eq!(tree.session_of_window(w1), None);

        let s1 = tree.new_session("c", 80, 24).unwrap();
        let w3 = tree.new_window(s1, "d", 80, 24).unwrap();
        check(&tree);
        assert_eq!(tree.kill_window(w3).unwrap(), None);
        check(&tree);
        tree.kill_session(s1).unwrap();
        check(&tree);
        assert!(tree.session(s1).is_none());

        // Last window of the last session: the cascade clears everything.
        assert_eq!(tree.kill_window(w0).unwrap(), Some(s0));
        check(&tree);
        assert!(tree.pane_window.is_empty() && tree.window_session.is_empty());
    }

    /// `move-window` reorders the session's window list, clamps
    /// out-of-range positions, and keeps the active window active by
    /// identity rather than index.
    #[test]
    fn move_window_reorders_and_keeps_the_active_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let w0 = tree.session(session_id).unwrap().windows[0];
        let w1 = tree.new_window(session_id, "one", 80, 24).unwrap();
        let w2 = tree.new_window(session_id, "two", 80, 24).unwrap();
        assert_eq!(tree.session(session_id).unwrap().windows, vec![w0, w1, w2]);
        assert_eq!(tree.session(session_id).unwrap().active, 0);

        tree.move_window(w2, 0).unwrap();
        assert_eq!(tree.session(session_id).unwrap().windows, vec![w2, w0, w1]);
        assert_eq!(
            tree.session(session_id).unwrap().windows[tree.session(session_id).unwrap().active],
            w0,
            "the active window stayed active through the move"
        );

        // Out-of-range clamps to the end.
        tree.move_window(w0, 99).unwrap();
        assert_eq!(tree.session(session_id).unwrap().windows, vec![w2, w1, w0]);
        assert_eq!(
            tree.session(session_id).unwrap().windows[tree.session(session_id).unwrap().active],
            w0
        );

        assert!(matches!(
            tree.move_window(WindowId(9999), 0),
            Err(MuxError::NoSuchWindow(_))
        ));
    }

    /// `swap-window` exchanges two windows' positions in their shared
    /// session and refuses windows in different sessions.
    #[test]
    fn swap_windows_exchanges_positions_within_a_session() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let w0 = tree.session(session_id).unwrap().windows[0];
        let w1 = tree.new_window(session_id, "one", 80, 24).unwrap();
        let w2 = tree.new_window(session_id, "two", 80, 24).unwrap();

        tree.swap_windows(w0, w2).unwrap();
        assert_eq!(tree.session(session_id).unwrap().windows, vec![w2, w1, w0]);
        // A self-swap is a no-op, not an error.
        tree.swap_windows(w1, w1).unwrap();
        assert_eq!(tree.session(session_id).unwrap().windows, vec![w2, w1, w0]);

        let other_session = tree.new_session("other", 80, 24).unwrap();
        let other_window = tree.session(other_session).unwrap().windows[0];
        assert!(matches!(
            tree.swap_windows(w0, other_window),
            Err(MuxError::WindowsInDifferentSessions(_, _))
        ));
    }

    /// Window order is session state — a persist round trip keeps the
    /// reordered list (the restore path par-mux's restart runs).
    #[test]
    fn window_order_survives_a_persist_round_trip() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let w0 = tree.session(session_id).unwrap().windows[0];
        let w1 = tree.new_window(session_id, "one", 80, 24).unwrap();
        let w2 = tree.new_window(session_id, "two", 80, 24).unwrap();
        tree.move_window(w2, 0).unwrap();
        tree.swap_windows(w1, w0).unwrap();
        let order = tree.session(session_id).unwrap().windows.clone();
        assert_eq!(order, vec![w2, w1, w0]);

        let state = tree.to_persist_state();
        let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
            .expect("state rebuilds");
        assert_eq!(restored.session(session_id).unwrap().windows, order);
    }

    /// `respawn-pane`: a live pane refuses without `kill`; a dead pane
    /// restarts in place — same id, window, and layout, the user title
    /// carried to the replacement, a live process again.
    #[test]
    fn respawn_restarts_a_dead_pane_in_place() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let pane = tree.window(window_id).unwrap().panes()[0];
        tree.pane_mut(pane).unwrap().set_user_title("kept");

        // Live process: refused without -k.
        assert!(matches!(
            tree.begin_respawn(pane, false, None, None),
            Err(MuxError::PaneAlive(_pane))
        ));

        // Kill the process the way the reaper observes it: type exit,
        // poll until the OS agrees, mark the death.
        //
        // Enter is `\r`, not `\n`: on newer conhost builds (measured on
        // the Windows-26200 VM: both writes echoed, `exit 0exit 0` on one
        // line, never executed) a `\n`-terminated line is echoed by
        // cmd.exe but never submitted. windows CI runners were straddling
        // the 26100→26200 image rollout, which made this intermittent
        // (run 36601091589). Every other typed-line path in the suite
        // (mux_factory's `type_line`, the daemon's send-keys Enter)
        // already sends `\r`.
        tree.pane_mut(pane).unwrap().write(b"exit 0\r").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while tree.pane_mut(pane).unwrap().poll_running() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let still_running = tree.pane_mut(pane).unwrap().poll_running();
        let exit = tree.pane(pane).unwrap().exit_code();
        let screen_tail: String = tree
            .pane(pane)
            .unwrap()
            .terminal()
            .read()
            .content()
            .chars()
            .rev()
            .take(200)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        assert!(
            !still_running,
            "the shell must exit on `exit 0`; exit_code={exit:?}, screen tail \
             (an echoed command proves delivery, an absent one a dropped \
             write): {screen_tail:?}"
        );
        tree.pane_mut(pane).unwrap().mark_dead();
        assert_eq!(tree.pane(pane).unwrap().exit_code(), Some(0));
        assert!(tree.all_panes_dead());

        // Respawn with the stored command: same pane id, title carried,
        // layout untouched, a live process again.
        let factory = tree.factory();
        let plan = tree.begin_respawn(pane, false, None, None).unwrap();
        assert_eq!(plan.pane_id, pane);
        let respawned = factory
            .create_pane(
                plan.pane_id,
                plan.cols,
                plan.rows,
                plan.command.as_deref(),
                &plan.context(),
            )
            .expect("respawn spawns");
        tree.complete_respawn(plan, respawned).unwrap();
        assert_eq!(tree.window(window_id).unwrap().panes(), vec![pane]);
        assert_eq!(tree.pane(pane).unwrap().user_title(), Some("kept"));
        assert!(tree.pane(pane).unwrap().is_running());
        assert!(!tree.all_panes_dead());
    }

    /// A live `sleep 60` pane, fed `OSC 7 file://{host}{dir}`, and the
    /// cwd `begin_respawn(-k)` plans for it (SEC-128).
    #[cfg(unix)]
    fn respawn_cwd_after_osc7(host: &str, dir: &std::path::Path) -> Option<PathBuf> {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let pane = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, Some("sleep 60"))
            .unwrap();
        tree.pane(pane)
            .unwrap()
            .terminal()
            .write()
            .process(format!("\x1b]7;file://{host}{}\x1b\\", dir.display()).as_bytes());
        assert_eq!(
            tree.pane(pane)
                .unwrap()
                .terminal()
                .read()
                .current_directory(),
            Some(dir.to_str().unwrap()),
            "the OSC 7 report was recorded"
        );
        let plan = tree.begin_respawn(pane, true, None, None).unwrap();
        for id in [first, pane] {
            let _ = tree.pane_mut(id).unwrap().kill();
        }
        plan.cwd
    }

    /// SEC-128: an OSC 7 cwd naming another host is program output about a
    /// remote machine, never a local directory respawn may run in.
    #[cfg(unix)]
    #[test]
    fn respawn_ignores_a_remote_osc7_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = respawn_cwd_after_osc7("remote.invalid", dir.path());
        assert_ne!(cwd.as_deref(), Some(dir.path()), "remote OSC 7 was trusted");
    }

    /// SEC-128: a local OSC 7 report (implicit, `localhost`, or this
    /// machine's own name) naming an existing directory is still used.
    #[cfg(unix)]
    #[test]
    fn respawn_uses_a_local_osc7_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let host = crate::mux::pane::local_hostname().expect("gethostname");
        for h in ["", "localhost", host.as_str()] {
            let cwd = respawn_cwd_after_osc7(h, dir.path());
            assert_eq!(cwd.as_deref(), Some(dir.path()), "host {h:?}");
        }
    }

    /// SEC-128: a held dead pane with no OSC 7 has no cwd to offer (its
    /// reaped PID is not read, SEC-125), so the plan defers to the
    /// factory's cwd rather than the daemon's own working directory.
    #[cfg(unix)]
    #[test]
    fn respawn_of_a_dead_pane_without_osc7_defers_the_cwd() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let pane = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, Some("exit 3"))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let p = tree.pane_mut(pane).unwrap();
            if !p.poll_running() {
                p.mark_dead();
                if p.exit_code().is_some() {
                    break;
                }
            }
            assert!(std::time::Instant::now() < deadline, "never reaped");
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let plan = tree.begin_respawn(pane, false, None, None).unwrap();
        let _ = tree.pane_mut(first).unwrap().kill();
        assert_eq!(plan.cwd, None);
    }

    /// SEC-128: a local OSC 7 path that no longer exists is not used.
    #[cfg(unix)]
    #[test]
    fn respawn_ignores_an_osc7_dir_that_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let cwd = respawn_cwd_after_osc7("localhost", &gone);
        assert_ne!(cwd.as_deref(), Some(gone.as_path()), "missing dir was used");
    }

    /// ARC-089: `respawn-pane -k` reuses the pane id, so the dying
    /// process's SIGHUP output must not reach the id's output sink — it
    /// would land on the replacement's fresh screen in every client.
    #[cfg(unix)]
    #[test]
    fn respawn_does_not_forward_the_old_processes_exit_output() {
        type Chunks = Arc<parking_lot::Mutex<Vec<(u8, Vec<u8>)>>>;
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        // `sleep & wait` lets the trap run inside portable-pty's SIGHUP
        // grace; see `a_killed_panes_late_output_is_not_forwarded`.
        let pane = tree
            .split_pane(
                first,
                SplitDirection::Vertical,
                0.5,
                Some("trap 'echo OLD-PANE-BYE; exit 0' HUP; echo READY-MARK; while :; do sleep 5 & wait; done"),
            )
            .unwrap();
        let chunks: Chunks = Arc::default();
        let tagged = |generation: u8| {
            let chunks = Arc::clone(&chunks);
            move |bytes: &[u8]| chunks.lock().push((generation, bytes.to_vec()))
        };
        let seen_from = |generation: u8| -> String {
            chunks
                .lock()
                .iter()
                .filter(|(g, _)| *g == generation)
                .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
                .collect()
        };
        tree.pane_mut(pane).unwrap().on_output(tagged(0));
        // Read the marker off the screen, not the sink: the shell can print
        // it before `on_output` registers, since `split_pane` spawns first.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !tree
            .pane(pane)
            .unwrap()
            .terminal()
            .read()
            .content()
            .contains("READY-MARK")
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the trap's ready marker never reached the pane screen"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        let factory = tree.factory();
        let plan = tree
            .begin_respawn(pane, true, Some("sleep 60".to_string()), None)
            .unwrap();
        let replacement = factory
            .create_pane(
                plan.pane_id,
                plan.cols,
                plan.rows,
                plan.command.as_deref(),
                &plan.context(),
            )
            .expect("respawn spawns");
        tree.complete_respawn(plan, replacement).unwrap();
        tree.pane_mut(pane).unwrap().on_output(tagged(1));
        // Longer than portable-pty's SIGHUP grace plus the 500 ms reap.
        std::thread::sleep(std::time::Duration::from_millis(700));

        let old = seen_from(0);
        assert!(
            !old.contains("OLD-PANE-BYE"),
            "the old process's exit output reached the respawned id: {old:?}"
        );
        for id in [first, pane] {
            let _ = tree.pane_mut(id).unwrap().kill();
        }
    }

    #[test]
    fn resize_pane_absolute_rejects_an_unknown_pane() {
        let mut tree = tree();
        let result = tree.resize_pane_absolute(PaneId(999), Some(40), None);
        assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
    }

    #[test]
    fn killing_a_pane_resizes_the_survivor_to_the_full_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.kill_pane(second).unwrap();

        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (80, 24),
            "the surviving pane takes the freed extent"
        );
    }

    #[test]
    fn swapping_panes_trades_their_terminal_sizes() {
        // An asymmetric split: swap must resize both terminals to their new
        // geometry, not just exchange tree positions.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.25, None)
            .unwrap();
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (60, 24));
        assert_eq!(
            tree.pane(second).unwrap().terminal().read().size(),
            (20, 24)
        );

        tree.swap_panes(first, second).unwrap();

        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (20, 24));
        assert_eq!(
            tree.pane(second).unwrap().terminal().read().size(),
            (60, 24)
        );
    }

    #[test]
    fn resize_window_refits_every_pane_terminal() {
        // The refresh-client -C landing point: the window's extent changes,
        // the layout re-divides it, the terminals follow.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        tree.split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.resize_window(window_id, 120, 40).unwrap();

        let window = tree.window(window_id).unwrap();
        assert_eq!((window.cols, window.rows), (120, 40));
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (60, 40));
        assert_eq!(
            tree.pane(window.panes()[1])
                .unwrap()
                .terminal()
                .read()
                .size(),
            (60, 40)
        );
    }

    /// QA-182: the audit probe, below the parser cap — 2000 columns of
    /// 40 px cells overflowed `cols * cell_w` in u16 and panicked in
    /// `resize_with_cell_pixels`. The extent now saturates.
    #[test]
    fn oversized_cell_pixels_refit_does_not_panic() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let pane = tree.window(window_id).unwrap().panes()[0];
        tree.resize_window(window_id, 2000, 50).unwrap();
        tree.set_client_cell_pixels(40, 40);
        tree.resize_window(window_id, 2000, 50).unwrap();
        assert_eq!(
            tree.pane(pane).unwrap().terminal().read().size(),
            (2000, 50)
        );
    }

    #[test]
    fn resize_window_rejects_an_unknown_window() {
        let mut tree = tree();
        let result = tree.resize_window(WindowId(999), 120, 40);
        assert!(matches!(result, Err(MuxError::NoSuchWindow(_))));
    }

    #[test]
    fn select_window_changes_the_session_active_index() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let second = tree.new_window(session_id, "logs", 80, 24).unwrap();
        assert_eq!(tree.session(session_id).unwrap().active, 0);

        tree.select_window(second).expect("select succeeds");
        assert_eq!(tree.session(session_id).unwrap().active, 1);
    }

    #[test]
    fn select_window_rejects_an_unknown_window() {
        let mut tree = tree();
        let result = tree.select_window(WindowId(999));
        assert!(matches!(result, Err(MuxError::NoSuchWindow(_))));
    }

    #[test]
    fn rename_window_updates_the_name() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];

        tree.rename_window(window_id, "scratch")
            .expect("rename succeeds");
        assert_eq!(tree.window(window_id).unwrap().name, "scratch");
    }

    #[test]
    fn rename_window_rejects_an_unknown_window() {
        let mut tree = tree();
        let result = tree.rename_window(WindowId(999), "x");
        assert!(matches!(result, Err(MuxError::NoSuchWindow(_))));
    }

    #[test]
    fn kill_window_removes_it_and_its_panes_but_keeps_the_session() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let second = tree.new_window(session_id, "logs", 80, 24).unwrap();
        let pane_id = tree.window(second).unwrap().panes()[0];

        tree.kill_window(second).expect("kill succeeds");
        assert!(tree.window(second).is_none(), "window is gone");
        assert!(tree.pane(pane_id).is_none(), "its pane is gone too");
        assert_eq!(
            tree.session(session_id).unwrap().windows,
            vec![window_id],
            "the surviving window remains"
        );
    }

    #[test]
    fn kill_window_of_the_only_window_cascades_to_the_session() {
        // The same cascade kill_pane already has, entered from the window
        // side: a session with no windows does not survive.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];

        tree.kill_window(window_id).expect("kill succeeds");
        assert!(tree.window(window_id).is_none());
        assert!(
            tree.session(session_id).is_none(),
            "a session with no windows does not survive — matches tmux"
        );
    }

    #[test]
    fn kill_window_rejects_an_unknown_window() {
        let mut tree = tree();
        let result = tree.kill_window(WindowId(999));
        assert!(matches!(result, Err(MuxError::NoSuchWindow(_))));
    }

    #[test]
    fn buffer_starts_empty_and_round_trips_through_set_and_get() {
        let mut tree = tree();
        assert_eq!(tree.get_buffer("default"), None, "no buffer yet");

        tree.set_buffer("default", "hello".to_string());
        assert_eq!(tree.get_buffer("default"), Some("hello"));
    }

    #[test]
    fn set_buffer_overwrites_the_previous_value() {
        let mut tree = tree();
        tree.set_buffer("default", "first".to_string());
        tree.set_buffer("default", "second".to_string());
        assert_eq!(tree.get_buffer("default"), Some("second"));
    }

    /// Two sessions of one window of one pane each, with the panes titled
    /// per the test's needs — the shape every name-target test starts from.
    fn named_tree(first_title: Option<&str>, second_title: Option<&str>) -> MuxTree {
        let (mut tree, _factory) = recording_tree();
        let session = tree.new_session("alpha", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];
        let second_session = tree.new_session("beta", 80, 24).unwrap();
        let second_window = tree.session(second_session).unwrap().windows[0];
        let second = tree.window(second_window).unwrap().panes()[0];
        if let Some(title) = first_title {
            tree.pane_mut(first).unwrap().set_user_title(title);
        }
        if let Some(title) = second_title {
            tree.pane_mut(second).unwrap().set_user_title(title);
        }
        tree
    }

    #[test]
    fn pane_target_resolves_by_user_title_per_kind() {
        let tree = named_tree(Some("build"), None);
        let resolved = tree
            .resolve_pane_target(Target::Name("build".to_string()))
            .unwrap();
        // The titled pane is %0 (first session's first pane).
        assert_eq!(resolved, PaneId(0));
        // The other pane stays reachable by its own distinct title.
        let other = named_tree(None, Some("logs"));
        assert_eq!(
            other
                .resolve_pane_target(Target::Name("logs".to_string()))
                .unwrap(),
            PaneId(1)
        );
    }

    #[test]
    fn window_and_session_targets_resolve_by_name() {
        let mut tree = named_tree(None, None);
        // new_session names the window after the session, so rename one
        // window to prove name resolution is not id-shaped luck.
        tree.rename_window(WindowId(1), "logs").unwrap();
        assert_eq!(
            tree.resolve_window_target(Target::Name("logs".to_string()))
                .unwrap(),
            WindowId(1)
        );
        assert_eq!(
            tree.resolve_session_target(Target::Name("beta".to_string()))
                .unwrap(),
            SessionId(1)
        );
    }

    #[test]
    fn unknown_names_error_without_touching_the_tree() {
        let tree = named_tree(Some("build"), None);
        assert!(matches!(
            tree.resolve_pane_target(Target::Name("nope".to_string())),
            Err(MuxError::NoSuchPaneNamed(n)) if n == "nope"
        ));
        assert!(matches!(
            tree.resolve_window_target(Target::Name("nope".to_string())),
            Err(MuxError::NoSuchWindowNamed(n)) if n == "nope"
        ));
        assert!(matches!(
            tree.resolve_session_target(Target::Name("nope".to_string())),
            Err(MuxError::NoSuchSessionNamed(n)) if n == "nope"
        ));
    }

    #[test]
    fn ambiguous_names_error_listing_sorted_candidate_ids() {
        let tree = named_tree(Some("dup"), Some("dup"));
        match tree.resolve_pane_target(Target::Name("dup".to_string())) {
            Err(MuxError::AmbiguousPaneTarget(name, ids)) => {
                assert_eq!(name, "dup");
                assert_eq!(ids, vec![PaneId(0), PaneId(1)], "candidates in id order");
            }
            other => panic!("ambiguity must error, got {other:?}"),
        }
        // Windows: two sessions' initial windows both carry their
        // session's name, so one shared name makes them ambiguous.
        let mut tree = named_tree(None, None);
        tree.rename_window(WindowId(1), "alpha").unwrap();
        match tree.resolve_window_target(Target::Name("alpha".to_string())) {
            Err(MuxError::AmbiguousWindowTarget(_, ids)) => {
                assert_eq!(ids, vec![WindowId(0), WindowId(1)]);
            }
            other => panic!("ambiguity must error, got {other:?}"),
        }
    }

    #[test]
    fn typed_ids_pass_through_resolution_untouched() {
        let tree = named_tree(Some("%1"), None);
        // A pane TITLED "%1" must not capture id targets: %1 still means
        // pane 1, whose existence the caller reports as before.
        assert_eq!(
            tree.resolve_pane_target(Target::Id(PaneId(1))).unwrap(),
            PaneId(1)
        );
        // And the title "%1" is unreachable BY NAME (the parser classifies
        // sigil-prefixed values as ids), so no name can shadow an id.
        assert!(matches!(
            tree.resolve_pane_target(Target::parse("%1").unwrap()),
            Ok(PaneId(1))
        ));
    }

    #[test]
    fn duplicate_session_names_are_ambiguous_not_silently_picked() {
        let (mut tree, _factory) = recording_tree();
        tree.new_session("dup", 80, 24).unwrap();
        tree.new_session("dup", 80, 24).unwrap();
        match tree.resolve_session_target(Target::Name("dup".to_string())) {
            Err(MuxError::AmbiguousSessionTarget(_, ids)) => {
                assert_eq!(ids, vec![SessionId(0), SessionId(1)]);
            }
            other => panic!("ambiguity must error, got {other:?}"),
        }
    }

    #[test]
    fn rename_session_updates_the_name_for_resolution_and_future_spawns() {
        let mut tree = named_tree(None, None);
        tree.rename_session(SessionId(1), "renamed").unwrap();
        assert_eq!(tree.session(SessionId(1)).unwrap().name, "renamed");
        assert_eq!(
            tree.resolve_session_target(Target::Name("renamed".to_string()))
                .unwrap(),
            SessionId(1)
        );
        assert!(matches!(
            tree.resolve_session_target(Target::Name("beta".to_string())),
            Err(MuxError::NoSuchSessionNamed(n)) if n == "beta"
        ));
        assert!(matches!(
            tree.rename_session(SessionId(9), "x"),
            Err(MuxError::NoSuchSession(_))
        ));
    }

    #[test]
    fn kill_session_removes_every_window_pane_and_the_session() {
        let mut tree = named_tree(None, None);
        // A second window in session 1, so the kill spans multiple windows.
        tree.new_window(SessionId(1), "logs", 80, 24).unwrap();
        let killed = tree.kill_session(SessionId(1)).unwrap();
        assert_eq!(killed.len(), 2, "both windows of the session die");
        for window in &killed {
            assert!(tree.window(*window).is_none());
        }
        assert!(tree.session(SessionId(1)).is_none());
        // The other session is untouched.
        assert!(tree.session(SessionId(0)).is_some());
        assert_eq!(tree.sessions().len(), 1);
        assert!(matches!(
            tree.kill_session(SessionId(1)),
            Err(MuxError::NoSuchSession(_))
        ));
    }
}
