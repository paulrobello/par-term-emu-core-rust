//! The session/window/pane tree: the server's single source of truth.

mod layout_ops;
mod lifecycle;
#[cfg(test)]
mod tests;

use crate::mux::ids::{AnyTarget, IdAllocator, PaneId, SessionId, Target, WindowId, WorkspaceId};
use crate::mux::layout::{LayoutTree, PaneChrome, SplitDirection};
use crate::mux::pane::{MuxError, MuxPane, PaneFactory, SpawnContext};
use par_term_emu_core::color::Color;
use par_term_emu_core::terminal::ObserverDispatchBatch;
use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

/// Deliver the observer batches `tree` parked during resizes and notes,
/// taking the lock only to drain them — the callbacks run with it
/// released. Every path that can re-fit panes or write a note under the
/// tree mutex calls this after its last guard drops.
pub(crate) fn deliver_pending_observer_events(tree: &Mutex<MuxTree>) {
    let batches = tree.lock().take_observer_batches();
    for batch in batches {
        batch.deliver();
    }
}

/// One connected render client's sizing contribution: the grid it reported
/// (`refresh-client -C`) and the window it is displaying. A window's extent
/// is the componentwise minimum over the clients displaying it; a client
/// that never reported a size (a plain control or hook connection) has no
/// entry and constrains nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientView {
    /// The reported grid width in columns.
    pub cols: u16,
    /// The reported grid height in rows.
    pub rows: u16,
    /// The window the client is displaying.
    pub window: WindowId,
    /// The per-pane chrome the client declared (`refresh-client -I`);
    /// none for a client that declared nothing. The window reserves the
    /// merge over its viewers ([`PaneChrome::merge`]).
    pub chrome: PaneChrome,
}

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
    /// The per-pane chrome the window's division reserves: each pane's
    /// PTY is its layout rect less this ([`PaneChrome::pty_size`]), while
    /// the rects themselves — and so the layout string — keep their full
    /// geometry. The merge over the render clients displaying the window;
    /// none (full-rect PTYs) until a declaring client views it. Not
    /// persisted: clients re-declare on attach.
    pub chrome: PaneChrome,
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

/// One workspace: an ordered set of sessions with one of them active —
/// the level above sessions in the daemon > workspace > session >
/// window > pane hierarchy. Workspaces are first-class (no migration
/// from the flat session world): a session belongs to exactly one
/// workspace for its whole life, and a workspace dies when its last
/// session dies (`kill-workspace` kills the sessions outright).
#[derive(Debug)]
pub struct MuxWorkspace {
    /// This workspace's identifier.
    pub id: WorkspaceId,
    /// Display name.
    pub name: String,
    /// Sessions in order.
    pub sessions: Vec<SessionId>,
    /// Index into `sessions` of the active session.
    pub active: usize,
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
    /// Reserved window id.
    pub window_id: WindowId,
    /// Reserved id of the first pane.
    pub pane_id: PaneId,
    /// Initial grid width in columns.
    pub cols: u16,
    /// Initial grid height in rows.
    pub rows: u16,
    name: String,
    /// The first window's name; the session's own name when `None`.
    window_name: Option<String>,
    env: BTreeMap<String, String>,
    /// The workspace the session links to in phase 3, resolved at begin
    /// (explicit target, else active, else the lazily created default).
    workspace_id: WorkspaceId,
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
    /// The pane grid height in rows.
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
    /// Name the first window `name` instead of after the session.
    pub fn with_window_name(mut self, name: &str) -> Self {
        self.window_name = Some(crate::mux::strip_controls(name).into_owned());
        self
    }

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
    /// Reserved window id.
    pub window_id: WindowId,
    /// Reserved id of the new window's first pane.
    pub pane_id: PaneId,
    session_id: SessionId,
    /// Initial grid width in columns.
    pub cols: u16,
    /// Initial grid height in rows.
    pub rows: u16,
    name: String,
    session_name: String,
    env: BTreeMap<String, String>,
    cwd: Option<PathBuf>,
    /// `new-window -t @N`/`-t %N`: the window the new one sits
    /// immediately after at completion; `None` (the `$N`/name/bare
    /// forms) appends.
    insert_after: Option<WindowId>,
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
    /// Reserved id of the new pane.
    pub pane_id: PaneId,
    window_id: WindowId,
    target: PaneId,
    direction: SplitDirection,
    new_share: f32,
    /// `-b`: the new pane takes `first` (left/top) of the new split
    /// instead of `second` (right/bottom).
    before: bool,
    /// Initial grid width in columns.
    pub cols: u16,
    /// Initial grid height in rows.
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
    pub(crate) workspaces: HashMap<WorkspaceId, MuxWorkspace>,
    pub(crate) sessions: HashMap<SessionId, MuxSession>,
    pub(crate) windows: HashMap<WindowId, MuxWindow>,
    pub(crate) panes: HashMap<PaneId, MuxPane>,
    pub(crate) ids: IdAllocator,
    factory: Arc<dyn PaneFactory>,
    /// Named paste buffers (`set-buffer`/`show-buffer`). A single value per
    /// name, not tmux's numbered stack — the Phase 2 non-goal in par-mux.md D3.
    pub(crate) buffers: HashMap<String, String>,
    /// The daemon's active workspace — the one a bare `new-session`
    /// targets, and what `select-workspace` moves. `None` until the
    /// lazily created default (`main`) or the first explicit
    /// `new-workspace` exists.
    pub(crate) active_workspace: Option<WorkspaceId>,
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
    /// Reverse index session → workspace, for
    /// [`Self::workspace_of_session`]. Maintained only by
    /// [`Self::link_session`] and [`Self::unlink_session`], the only code
    /// that changes which workspace a session belongs to — a session is
    /// immutable in this respect for its whole life.
    pub(crate) session_workspace: HashMap<SessionId, WorkspaceId>,
    /// Per-connection sizing state behind the smallest-attached-client
    /// window rule ([`ClientView`]). Never persisted: a reconnecting
    /// client re-reports through the attach handshake.
    pub(crate) client_views: HashMap<u64, ClientView>,
    /// Observer events from pane resizes and daemon notes, held until the
    /// tree lock drops. A layout mutation re-fits pane terminals under the
    /// tree mutex, and a spawn writes its start-dir note there; an observer
    /// callback run there could block on (or re-enter) that mutex, so
    /// [`Self::sync_pane_sizes`], [`Self::apply_cell_pixels`], and
    /// [`Self::write_pane_note`] park their batches here and every lock
    /// holder that can trigger one drains them through
    /// [`deliver_pending_observer_events`] after releasing it. Only
    /// non-empty batches are kept.
    pending_observer_batches: Vec<ObserverDispatchBatch>,
}

impl MuxTree {
    /// Create an empty tree that builds panes with `factory` (seam S1).
    pub fn new(factory: Box<dyn PaneFactory>) -> Self {
        Self {
            workspaces: HashMap::new(),
            active_workspace: None,
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
            session_workspace: HashMap::new(),
            client_views: HashMap::new(),
            pending_observer_batches: Vec::new(),
        }
    }

    /// Park a resize's or note's observer batch for delivery after the tree lock
    /// drops. Empty batches (no observers, or no events) are dropped here.
    pub(crate) fn defer_observer_batch(&mut self, batch: ObserverDispatchBatch) {
        if !batch.is_empty() {
            self.pending_observer_batches.push(batch);
        }
    }

    /// Write a daemon note into `pane_id`'s terminal ([`MuxPane::write_note`])
    /// with its observer events parked for delivery after the tree lock
    /// drops. A pane that no longer exists is skipped.
    pub(crate) fn write_pane_note(&mut self, pane_id: PaneId, bytes: &[u8]) {
        let Some(pane) = self.panes.get(&pane_id) else {
            return;
        };
        let batch = pane.write_note_deferred(bytes);
        self.defer_observer_batch(batch);
    }

    /// Drain the observer batches parked by pane resizes and notes, for delivery
    /// once the tree lock is released. An embedder that drives the tree
    /// directly (no [`crate::mux::MuxServer`]) calls this after its own
    /// mutations and delivers each batch itself.
    pub fn take_observer_batches(&mut self) -> Vec<ObserverDispatchBatch> {
        std::mem::take(&mut self.pending_observer_batches)
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
        let mut session_workspace = HashMap::new();
        for (workspace_id, workspace) in &self.workspaces {
            for session_id in &workspace.sessions {
                session_workspace.insert(*session_id, *workspace_id);
            }
        }
        assert_eq!(
            self.session_workspace, session_workspace,
            "session → workspace index drifted"
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

    /// Resolve a `split-window` target to the pane it splits: `%N` is the
    /// pane itself (whose existence `begin_split` reports), `@N` that
    /// window's active pane, `$N` the session's active window's active
    /// pane, and a name falls through to the pane-title match
    /// [`Self::resolve_pane_target`] does.
    pub fn resolve_split_target(&self, target: AnyTarget) -> Result<PaneId, MuxError> {
        match target {
            AnyTarget::Pane(pane) => Ok(pane),
            AnyTarget::Window(window) => self
                .window(window)
                .map(|w| w.active)
                .ok_or(MuxError::NoSuchWindow(window)),
            AnyTarget::Session(session) => {
                let active = self
                    .session(session)
                    .and_then(|s| s.windows.get(s.active))
                    .copied()
                    .ok_or(MuxError::NoSuchSession(session))?;
                self.window(active)
                    .map(|w| w.active)
                    .ok_or(MuxError::NoSuchSession(session))
            }
            AnyTarget::Name(name) => self.resolve_pane_target(Target::Name(name)),
        }
    }

    /// Resolve a `new-window` target to its session and — when the target
    /// names a window or a pane — the window the new one sits immediately
    /// after at completion (`$N`, names, and the bare form append). A pane
    /// target names the window holding it; unknown ids error per kind.
    pub fn resolve_new_window_target(
        &self,
        target: AnyTarget,
    ) -> Result<(SessionId, Option<WindowId>), MuxError> {
        match target {
            AnyTarget::Session(session) => Ok((session, None)),
            AnyTarget::Window(window) => self
                .session_of_window(window)
                .map(|session| (session, Some(window)))
                .ok_or(MuxError::NoSuchWindow(window)),
            AnyTarget::Pane(pane) => {
                let window = self
                    .window_of_pane(pane)
                    .ok_or(MuxError::NoSuchPane(pane))?;
                self.session_of_window(window)
                    .map(|session| (session, Some(window)))
                    .ok_or(MuxError::NoSuchPane(pane))
            }
            AnyTarget::Name(name) => self
                .resolve_session_target(Target::Name(name))
                .map(|session| (session, None)),
        }
    }

    /// The factory this tree spawns panes with (seam S1), handed out so the
    /// dispatcher can run phase 2 — the spawn itself — OFF the tree lock.
    pub fn factory(&self) -> Arc<dyn PaneFactory> {
        self.factory.clone()
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

    /// Every workspace id currently live.
    pub fn workspaces(&self) -> Vec<WorkspaceId> {
        self.workspaces.keys().copied().collect()
    }

    /// Look up a workspace.
    pub fn workspace(&self, id: WorkspaceId) -> Option<&MuxWorkspace> {
        self.workspaces.get(&id)
    }

    /// The daemon's active workspace — the one a bare `new-session`
    /// targets. `None` when no workspace exists yet.
    pub fn active_workspace(&self) -> Option<WorkspaceId> {
        self.active_workspace
    }

    /// The workspace a session belongs to, if any. A map lookup.
    pub fn workspace_of_session(&self, session: SessionId) -> Option<WorkspaceId> {
        self.session_workspace.get(&session).copied()
    }

    /// The active session of the daemon's active workspace, if any.
    pub fn active_session(&self) -> Option<SessionId> {
        let workspace = self.active_workspace()?;
        let ws = self.workspaces.get(&workspace)?;
        ws.sessions.get(ws.active).copied()
    }

    /// Resolve a workspace target: typed `+N` ids pass through; a name
    /// matches workspace names exactly, across every workspace. Ambiguous
    /// names error with the candidates.
    pub fn resolve_workspace_target(
        &self,
        target: Target<WorkspaceId>,
    ) -> Result<WorkspaceId, MuxError> {
        let name = match target {
            Target::Id(id) => return Ok(id),
            Target::Name(name) => name,
        };
        match match_name(
            self.workspaces
                .iter()
                .filter_map(|(id, workspace)| (workspace.name == name).then_some(*id)),
        ) {
            Match::None => Err(MuxError::NoSuchWorkspaceNamed(name)),
            Match::One(id) => Ok(id),
            Match::Many(ids) => Err(MuxError::AmbiguousWorkspaceTarget(name, ids)),
        }
    }

    /// Append `session_id` to `workspace_id`'s session list and index the
    /// membership. `false` when the workspace no longer exists (the
    /// caller — [`Self::complete_session`]'s phase-3 path — re-anchors to
    /// the default).
    pub(crate) fn link_session(
        &mut self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> bool {
        if !self.workspaces.contains_key(&workspace_id) {
            return false;
        }
        let workspace = self
            .workspaces
            .get_mut(&workspace_id)
            .expect("checked above");
        workspace.sessions.push(session_id);
        // The newly created session becomes the workspace's active one —
        // the same newest-wins rule the flat world applied to bare
        // new-window targeting.
        workspace.active = workspace.sessions.len() - 1;
        self.session_workspace.insert(session_id, workspace_id);
        true
    }

    /// Take `session_id` out of its workspace's session list, clamping the
    /// workspace's active index to the shortened list. A workspace left
    /// with no sessions is REMOVED entirely (its last session died, so the
    /// user closed everything in it) — a workspace born empty
    /// (`new-workspace` before any session joins) is untouched by this
    /// path, since nothing was unlinked from it.
    fn unlink_session(&mut self, session_id: SessionId) {
        let Some(workspace_id) = self.session_workspace.remove(&session_id) else {
            return;
        };
        let Some(workspace) = self.workspaces.get_mut(&workspace_id) else {
            return;
        };
        if let Some(pos) = workspace.sessions.iter().position(|s| *s == session_id) {
            workspace.sessions.remove(pos);
        }
        if workspace.active >= workspace.sessions.len() && !workspace.sessions.is_empty() {
            workspace.active = workspace.sessions.len() - 1;
        }
        if workspace.sessions.is_empty() {
            self.workspaces.remove(&workspace_id);
            if self.active_workspace == Some(workspace_id) {
                // Lowest surviving id, not map order, so the fallback is
                // deterministic.
                self.active_workspace = self.workspaces.keys().min().copied();
            }
        }
    }

    /// The workspace a bare `new-session` lands in: the explicit target
    /// resolved when given, else the active workspace, else a lazily
    /// created default named `main` (selected on creation). Names the
    /// workspace the session will link to in phase 3.
    pub(crate) fn resolve_new_session_workspace(
        &mut self,
        target: Option<Target<WorkspaceId>>,
    ) -> Result<WorkspaceId, MuxError> {
        match target {
            Some(target) => {
                let id = self.resolve_workspace_target(target)?;
                if !self.workspaces.contains_key(&id) {
                    return Err(MuxError::NoSuchWorkspace(id));
                }
                Ok(id)
            }
            None => Ok(self.ensure_default_workspace()),
        }
    }

    /// The lazily created default workspace: `main` with the next id when
    /// no workspace exists, else the current active one.
    pub(crate) fn ensure_default_workspace(&mut self) -> WorkspaceId {
        if let Some(id) = self.active_workspace {
            return id;
        }
        let id = self.ids.next_workspace();
        self.workspaces.insert(
            id,
            MuxWorkspace {
                id,
                name: "main".to_string(),
                sessions: Vec::new(),
                active: 0,
            },
        );
        self.active_workspace = Some(id);
        id
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
