//! On-disk persistence envelope for the mux tree (par-mux.md Phase 3, D3.2).
//!
//! The artifact is a versioned envelope, not a bare tree: `format_version`
//! lets a future format change migrate old files rather than refuse them,
//! and each window's pane list rides beside its [`LayoutTree`] so restore
//! can spawn every pane's replacement process before re-hanging content
//! (D3.5: spawn first, restore second — startup bytes must not overwrite a
//! restored screen).

use crate::cell::Cell;
use crate::mux::agent_resume::resume_invocation;
use crate::mux::ids::{IdAllocator, PaneId, SessionId, WindowId, WorkspaceId};
use crate::mux::layout::LayoutTree;
use crate::mux::pane::{snapshot_from_parts, PaneSnapshotParts};
use crate::mux::pane::{MuxError, PaneFactory, SpawnContext};
use crate::mux::tree::{MuxSession, MuxTree, MuxWindow, MuxWorkspace};
use crate::terminal::replay_snapshot::{GridSnapshot, TerminalSnapshot};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The envelope version this build writes, and the only one it accepts.
/// A file carrying any other version is quarantined at the load site and the
/// server starts fresh (D3.2).
///
/// v2 (Phase 6, task 6.1): `PersistPane` gained the optional
/// `agent_session` — a compatible serde read, but the bump keeps the
/// boundary explicit instead of silently partial-reading a v1 file.
///
/// v3 (workspaces): the hierarchy gained the workspace level above
/// sessions — `PersistState` carries `workspaces` + `active_workspace`,
/// and `next_ids` grew the workspace counter. Not backward compatible BY
/// DESIGN (workspaces are first-class, no migration): a v2 file fails the
/// version check and is quarantined, and the daemon starts fresh.
pub const FORMAT_VERSION: u32 = 3;

/// Ceiling on the scrollback cells one pane contributes to the state file
/// — a persistence bound only; the in-memory pane keeps its full history.
/// Cells dominate the file (~170 serialized bytes each, measured
/// 2026-09-22), so an uncapped 10 000-line pane at 80 cols serializes to a
/// 136 MB state whose final shutdown save held SIGTERM exit for two
/// minutes. 100 000 cells (~1 250 lines at 80 cols, ~17 MB) keeps the
/// newest context while bounding the save far below the exit budget.
/// cap: Scrollback cells restored per pane from an untrusted on-disk state file.
const MAX_PERSISTED_SCROLLBACK_CELLS: usize = 100_000;

/// Restore bounds for a state file's window size — the same floor and
/// ceiling the streaming layer enforces on a client's terminal-size request
/// (`crate::streaming::server::{MIN_COLS, MAX_COLS, MIN_ROWS, MAX_ROWS}`),
/// mirrored here because the `mux` feature does not enable `streaming`
/// (ARC-113b).
const MIN_RESTORED_COLS: u16 = 2;
const MIN_RESTORED_ROWS: u16 = 1;
/// cap: Columns one restored window may claim from an untrusted state file.
const MAX_RESTORED_COLS: u16 = 1_000;
/// cap: Rows one restored window may claim from an untrusted state file.
const MAX_RESTORED_ROWS: u16 = 500;

/// Clamp a state file's window size to the restore bounds: the size feeds
/// the factory's grid allocation, so a corrupt or hostile `cols`/`rows`
/// pair must not reach one unchecked.
fn clamp_restored_window_size(cols: u16, rows: u16) -> (u16, u16) {
    (
        cols.clamp(MIN_RESTORED_COLS, MAX_RESTORED_COLS),
        rows.clamp(MIN_RESTORED_ROWS, MAX_RESTORED_ROWS),
    )
}

/// Clamp a restored snapshot's grid dimensions (ARC-113c2): each pane's
/// persisted grids ride the state file beside the window size, and
/// `Grid::restore_from_snapshot` adopts their `cols`/`rows`, `cells`, and
/// `wrapped` wholesale — so a hostile snapshot drives the grid's row-major
/// math and allocation the same way a hostile window size does. Hold each
/// grid to the same bounds, reshaping `cells` and `wrapped` to the clamped
/// shape so the restored grid keeps its invariants.
fn clamp_restored_grid_dims(snapshot: &mut TerminalSnapshot) {
    for grid in [&mut snapshot.grid, &mut snapshot.alt_grid] {
        let cols = grid.cols.clamp(
            usize::from(MIN_RESTORED_COLS),
            usize::from(MAX_RESTORED_COLS),
        );
        let rows = grid.rows.clamp(
            usize::from(MIN_RESTORED_ROWS),
            usize::from(MAX_RESTORED_ROWS),
        );
        grid.cells.resize(rows * cols, Cell::default());
        grid.wrapped.resize(rows, false);
        grid.cols = cols;
        grid.rows = rows;
    }
}

/// Errors raised while saving or rebuilding persisted mux state.
#[derive(Debug)]
pub enum PersistError {
    /// The state was written under a `format_version` this build does not
    /// know — migrate or refuse, never guess.
    UnsupportedVersion {
        /// `format_version` found in the file.
        found: u32,
        /// `format_version` this build supports.
        supported: u32,
    },
    /// Spawning a restored pane's replacement process failed.
    Mux(MuxError),
    /// Writing the state file failed.
    Io(std::io::Error),
    /// Encoding the state failed.
    Serialize(serde_json::Error),
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistError::UnsupportedVersion { found, supported } => {
                write!(
                    f,
                    "unsupported state format_version {found} (supported: {supported})"
                )
            }
            PersistError::Mux(err) => write!(f, "restore failed: {err}"),
            PersistError::Io(err) => write!(f, "state file I/O failed: {err}"),
            PersistError::Serialize(err) => write!(f, "state encoding failed: {err}"),
        }
    }
}

impl std::error::Error for PersistError {}

impl From<MuxError> for PersistError {
    fn from(err: MuxError) -> Self {
        PersistError::Mux(err)
    }
}

impl From<std::io::Error> for PersistError {
    fn from(err: std::io::Error) -> Self {
        PersistError::Io(err)
    }
}

impl From<serde_json::Error> for PersistError {
    fn from(err: serde_json::Error) -> Self {
        PersistError::Serialize(err)
    }
}

/// Current wall clock in Unix milliseconds; `0` if the clock reads before
/// the epoch.
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The whole server's persisted state — the file's top level.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistState {
    /// Envelope version; see [`FORMAT_VERSION`].
    pub format_version: u32,
    /// When this state was captured, Unix milliseconds.
    pub saved_at_unix_ms: u64,
    /// The id allocator's counters at capture — `(workspace, session,
    /// window, pane)`, the next id each kind hands out.
    pub next_ids: (u32, u32, u32, u32),
    /// Every live session, in id order.
    pub sessions: Vec<PersistSession>,
    /// Every live workspace, in id order. A session appears in exactly
    /// one workspace's `sessions` list; a session id absent from
    /// `sessions` above is skipped at restore (defensive against a
    /// hand-edited file, not a migration path).
    pub workspaces: Vec<PersistWorkspace>,
    /// The `+N` number of the daemon's active workspace, when one exists.
    pub active_workspace: Option<u32>,
    /// Named paste buffers (`set-buffer`/`show-buffer`).
    pub buffers: HashMap<String, String>,
}

/// One persisted workspace: the level above sessions.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistWorkspace {
    /// The workspace's `+N` number.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// The member sessions' `$N` numbers, in workspace order.
    pub sessions: Vec<u32>,
    /// Index into `sessions` of the workspace's active session.
    pub active_session_index: usize,
}

/// One persisted session.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistSession {
    /// The session's `$N` number.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// Index into `windows` of the active window.
    pub active_window_index: usize,
    /// The session's windows, in order.
    pub windows: Vec<PersistWindow>,
    /// The session environment. May hold secrets, which is one reason the
    /// file is owner-only. Absent in files written before the field
    /// existed, which load with an empty environment.
    #[cfg_attr(feature = "serde", serde(default))]
    pub env: std::collections::BTreeMap<String, String>,
}

/// One persisted window: its layout tree plus the panes that tree references.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistWindow {
    /// The window's `@N` number.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// Window width in columns.
    pub cols: u16,
    /// Window height in rows.
    pub rows: u16,
    /// The `@N`-style number of the active pane — a plain number, like `id`,
    /// so the envelope carries no newtype coupling.
    pub active_pane: u32,
    /// The interior pane structure; its leaf ids reference `panes` below.
    pub layout: LayoutTree,
    /// The window's panes. Carried beside the layout (D3.2) so restore can
    /// spawn each live pane's replacement process (or build a held-dead
    /// pane, ARC-114) before re-hanging content.
    pub panes: Vec<PersistPane>,
}

/// One persisted pane: its captured terminal and how to respawn its process.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistPane {
    /// The pane's `%N` number.
    pub id: u32,
    /// The pane's full terminal state — screen, scrollback, modes.
    pub terminal: TerminalSnapshot,
    /// The command the pane ran (`None` = the default shell).
    pub spawn_command: Option<String>,
    /// The pane's user title (`select-pane -T`), when set. Skipped when
    /// absent (an untitled pane serializes byte-identically to the older
    /// format) and defaulted on load, so pre-title save files still
    /// restore.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub user_title: Option<String>,
    /// Agent session identity, when a hook claimed this pane (Phase 6,
    /// task 6.1). Skipped when absent so a non-agent pane serializes
    /// byte-identically to the pre-change format.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub agent_session: Option<PersistAgentSession>,
    /// The pane's last cwd (its shell's OSC 7 report, else the child's live
    /// cwd at capture), so a restore re-lands the pane — and a resumed
    /// agent — where it left off instead of in the daemon's start
    /// directory. Skipped when absent and defaulted on load, so pre-cwd
    /// save files still restore.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub cwd: Option<String>,
    /// The OSC 7 hostname that came with [`Self::cwd`], only when the cwd
    /// was an OSC 7 report (a remote shell's report names its host). Restore
    /// seeds it beside the cwd on a held-dead pane so `respawn-pane` still
    /// rejects a remote directory (SEC-128). Skipped when absent and
    /// defaulted on load, so older save files restore as before.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub cwd_host: Option<String>,
    /// The pane was held dead (its process observed exited) at save time
    /// (ARC-114). A restore brings it back held-dead — processless, its
    /// frozen screen intact, `respawn-pane` available — instead of
    /// respawning it. Skipped when false and defaulted on load, so older
    /// save files (and older daemons reading newer ones) treat every pane
    /// as live, which is the pre-ARC-114 respawn behavior.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "std::ops::Not::not")
    )]
    pub dead: bool,
    /// The held-dead pane's recorded exit code; meaningful only with
    /// [`Self::dead`]. Skipped when absent, defaulted on load.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub exit_code: Option<i32>,
}

/// The agent-session identity hooks write into pane metadata, persisted so
/// the resume path (Phase 6 tasks 6.2/6.3) survives a restart — live
/// metadata dies with the process. The wire contract is id-OR-path (the
/// shipped pi/omp extensions send path-only refs), so neither field alone
/// gates the capture: whichever the metadata holds travels. `resume_argv`
/// is the agent's own reported invocation, stored verbatim as a JSON argv
/// string — the hook-first override the per-agent table falls back from.
///
/// Deliberately absent: `agent_session_start_source` (stale and misleading
/// after a restart; task 6.4 reads it from the post-restore report instead)
/// and everything state-shaped (`agent_state`, `agent_state_source`,
/// `agent_seq`) — a restored pane holds no state until its agent reports
/// again, which is why the post-restart roster is empty by design.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistAgentSession {
    /// The agent label (`metadata["agent"]`).
    pub agent: String,
    /// The session id, when the metadata holds one.
    pub session_id: Option<String>,
    /// The transcript path, when the metadata holds one.
    pub session_path: Option<String>,
    /// The reporting script's own source tag.
    pub source: Option<String>,
    /// The reported resume invocation, verbatim JSON argv.
    pub resume_argv: Option<String>,
}

/// Capture one pane's agent-session identity from its metadata, or `None`
/// when nothing the resume path can use is there: identity requires an id,
/// a path, or a reported invocation — an agent label alone is a pane
/// hook-claimed for state but carrying no session worth resuming.
fn agent_session_from_metadata(metadata: &HashMap<String, String>) -> Option<PersistAgentSession> {
    // Through the typed claim (ARC-113c): the same identity fields the
    // roster reads, parsed once. The argv re-renders compact — byte-identical
    // to what the report path stored, so the save format does not drift.
    let claim = crate::mux::hooks::AgentClaim::from_metadata(metadata)?;
    let session = PersistAgentSession {
        agent: claim.agent,
        session_id: claim.session_id,
        session_path: claim.session_path,
        source: claim.source,
        resume_argv: claim
            .resume_argv
            .map(|argv| serde_json::to_string(&argv).unwrap_or_default()),
    };
    (session.session_id.is_some()
        || session.session_path.is_some()
        || session.resume_argv.is_some())
    .then_some(session)
}

/// The collected-under-lock half of a tree save (ARC-032): the structure
/// plus per-pane capture handles, no pane payloads. Built by
/// [`MuxTree::collect_persist_capture`] while the tree lock is held (cheap
/// field reads only); [`PersistCapture::capture`] then finishes the save
/// after the lock drops — the grid-walk snapshots on cache miss and the
/// per-pane cwd syscalls are the expensive half, and they no longer run
/// with every client waiting on the tree.
pub struct PersistCapture {
    next_ids: (u32, u32, u32, u32),
    workspaces: Vec<WorkspaceCapture>,
    active_workspace: Option<WorkspaceId>,
    sessions: Vec<SessionCapture>,
    buffers: HashMap<String, String>,
}

struct WorkspaceCapture {
    id: WorkspaceId,
    name: String,
    active: usize,
    sessions: Vec<SessionId>,
}

struct SessionCapture {
    id: SessionId,
    name: String,
    active: usize,
    env: BTreeMap<String, String>,
    windows: Vec<WindowCapture>,
}

struct WindowCapture {
    id: WindowId,
    name: String,
    cols: u16,
    rows: u16,
    active_pane: PaneId,
    layout: LayoutTree,
    panes: Vec<PaneCapture>,
}

struct PaneCapture {
    id: PaneId,
    parts: PaneSnapshotParts,
    spawn_command: Option<String>,
    user_title: Option<String>,
    agent_session: Option<PersistAgentSession>,
    dead: bool,
    exit_code: Option<i32>,
}

impl PersistCapture {
    /// Finish the capture off the tree lock: per-pane snapshots (cached
    /// while unchanged) and cwd resolution, then the same shape
    /// [`MuxTree::to_persist_state`] produces — sessions in id order so two
    /// captures of an unchanged tree differ only in their timestamps.
    pub fn capture(self) -> PersistState {
        let mut workspaces: Vec<PersistWorkspace> = self
            .workspaces
            .into_iter()
            .map(|workspace| PersistWorkspace {
                id: workspace.id.0,
                name: workspace.name,
                active_session_index: workspace.active,
                sessions: workspace
                    .sessions
                    .into_iter()
                    .map(|session| session.0)
                    .collect(),
            })
            .collect();
        workspaces.sort_by_key(|workspace| workspace.id);
        let mut sessions: Vec<PersistSession> = self
            .sessions
            .into_iter()
            .map(|session| PersistSession {
                id: session.id.0,
                name: session.name,
                active_window_index: session.active,
                env: session.env,
                windows: session
                    .windows
                    .into_iter()
                    .map(|window| PersistWindow {
                        id: window.id.0,
                        name: window.name,
                        cols: window.cols,
                        rows: window.rows,
                        active_pane: window.active_pane.0,
                        layout: window.layout,
                        panes: window
                            .panes
                            .into_iter()
                            .map(|pane| PersistPane {
                                id: pane.id.0,
                                terminal: cap_persisted_scrollback(snapshot_from_parts(
                                    &pane.parts,
                                )),
                                spawn_command: pane.spawn_command,
                                user_title: pane.user_title,
                                agent_session: pane.agent_session,
                                cwd: pane
                                    .parts
                                    .cwd()
                                    .map(|dir| dir.to_string_lossy().into_owned()),
                                cwd_host: pane.parts.cwd_host(),
                                dead: pane.dead,
                                exit_code: pane.exit_code,
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        sessions.sort_by_key(|session| session.id);

        PersistState {
            format_version: FORMAT_VERSION,
            saved_at_unix_ms: unix_ms(),
            next_ids: self.next_ids,
            workspaces,
            active_workspace: self.active_workspace.map(|id| id.0),
            sessions,
            buffers: self.buffers,
        }
    }
}

impl MuxTree {
    /// Capture the whole tree into its persisted form (D3.2) — the one-shot
    /// form of [`Self::collect_persist_capture`] + [`PersistCapture::capture`],
    /// for callers that do not hold the tree lock through the capture.
    pub fn to_persist_state(&self) -> PersistState {
        self.collect_persist_capture().capture()
    }

    /// The under-lock half of a save (ARC-032): structure and capture
    /// handles only. Pair with [`PersistCapture::capture`] after dropping
    /// the tree lock.
    pub fn collect_persist_capture(&self) -> PersistCapture {
        PersistCapture {
            next_ids: self.ids.next_ids(),
            workspaces: self
                .workspaces
                .values()
                .map(|workspace| WorkspaceCapture {
                    id: workspace.id,
                    name: workspace.name.clone(),
                    active: workspace.active,
                    sessions: workspace.sessions.clone(),
                })
                .collect(),
            active_workspace: self.active_workspace,
            sessions: self
                .sessions
                .values()
                .map(|session| SessionCapture {
                    id: session.id,
                    name: session.name.clone(),
                    active: session.active,
                    env: session.env.clone(),
                    windows: session
                        .windows
                        .iter()
                        .map(|window_id| self.capture_window(*window_id))
                        .collect(),
                })
                .collect(),
            buffers: self.buffers.clone(),
        }
    }

    /// Collect one window and its panes; panes serialize in layout order.
    fn capture_window(&self, window_id: WindowId) -> WindowCapture {
        let window = self
            .windows
            .get(&window_id)
            .expect("session window lists only hold live windows");
        WindowCapture {
            id: window.id,
            name: window.name.clone(),
            cols: window.cols,
            rows: window.rows,
            active_pane: window.active,
            layout: window.layout.clone(),
            panes: window
                .panes()
                .into_iter()
                .map(|pane_id| {
                    let pane = self
                        .panes
                        .get(&pane_id)
                        .expect("layout leaf ids are always live panes");
                    PaneCapture {
                        id: pane_id,
                        parts: pane.snapshot_capture_parts(),
                        spawn_command: pane.spawn_command().map(str::to_string),
                        user_title: pane.user_title().map(str::to_string),
                        agent_session: agent_session_from_metadata(pane.metadata()),
                        dead: pane.dead(),
                        exit_code: pane.exit_code(),
                    }
                })
                .collect(),
        }
    }

    /// Rebuild a tree from persisted state (D3.5): spawn each live pane's
    /// replacement process first (a pane persisted held-dead gets a
    /// processless pane instead, ARC-114), then restore its terminal from the
    /// snapshot, so process startup bytes never overwrite restored content.
    ///
    /// Held-dead entries are honored against the daemon's CURRENT
    /// `remain-on-exit` setting (`crate::mux::config::daemon_remain_on_exit`):
    /// a daemon with auto-remove on drops them, one with the hold on
    /// restores them held-dead — `respawn-pane` available. The
    /// [`Self::from_persist_state_with_policy`] form pins the decision for
    /// tests.
    ///
    /// The id allocator resumes from the persisted counters, so restored
    /// panes keep their `%N` identities and new panes do not collide.
    pub fn from_persist_state(
        state: &PersistState,
        factory: Box<dyn PaneFactory>,
    ) -> Result<MuxTree, PersistError> {
        Self::from_persist_state_with_policy(
            state,
            factory,
            crate::mux::config::daemon_remain_on_exit(),
        )
    }

    /// [`Self::from_persist_state`] with the held-dead policy pinned: with
    /// `remain_on_exit` false (the product default) every persisted dead
    /// entry is dropped — through the kill-pane cascade, so a window whose
    /// last pane was dead closes and an emptied session (and workspace) go
    /// with it; with it true the entry is restored held-dead exactly as
    /// ARC-114 defined.
    pub fn from_persist_state_with_policy(
        state: &PersistState,
        factory: Box<dyn PaneFactory>,
        remain_on_exit: bool,
    ) -> Result<MuxTree, PersistError> {
        if state.format_version != FORMAT_VERSION {
            return Err(PersistError::UnsupportedVersion {
                found: state.format_version,
                supported: FORMAT_VERSION,
            });
        }

        let mut panes = HashMap::new();
        // Each session with its windows in order; linked into the tree
        // through `MuxTree::insert_window` below, which maintains the
        // reverse indexes (ARC-096).
        let mut sessions: Vec<(MuxSession, Vec<MuxWindow>)> = Vec::new();
        // Panes whose persisted cwd was gone at restore — they spawned in
        // home and get a visible note after their content is restored.
        let mut cwd_fallbacks: HashMap<u32, String> = HashMap::new();
        let mut note_batches = Vec::new();

        for session in &state.sessions {
            let mut session_windows = Vec::with_capacity(session.windows.len());
            for window in &session.windows {
                // The state file is untrusted input: the window size feeds
                // the factory's grid allocation, so it is clamped before it
                // reaches one — the same bounds a live client's size
                // request is held to.
                let (cols, rows) = clamp_restored_window_size(window.cols, window.rows);
                for pane in &window.panes {
                    // The effective command (D6.3): a resumable agent
                    // session rewrites what the pane respawns as — the
                    // hook-reported invocation first, the per-agent table
                    // second, handed to the factory as STRUCTURED argv so
                    // Windows can spawn without a cmd.exe string re-parse
                    // (the factory's string path POSIX-quotes, which cmd
                    // treats as literal characters). Every failure mode of
                    // that chain is an Option degrading to the pane's
                    // original `spawn_command`, exactly Phase 3 behavior;
                    // no retry and no probe (an agent that accepts a resume
                    // flag and starts fresh is 6.4's after-the-fact
                    // question, not spawn time's). A chain that BUILDS but
                    // fails at runtime (uninstalled binary, rejected
                    // session id) renders with a fallback tail so the
                    // failure lands the pane on a live shell rather than a
                    // reaper deletion.
                    // A pane persisted held-dead has no process to resume or
                    // to land in a cwd: it is rebuilt processless below, so
                    // neither the agent resume invocation nor the cwd spawn
                    // logic runs for it (ARC-114).
                    let resume_argv = pane
                        .agent_session
                        .as_ref()
                        .filter(|_| !pane.dead)
                        .and_then(resume_invocation);
                    // The persisted cwd re-lands the pane where it left off.
                    // A directory that vanished between save and restore
                    // would fail the spawn, so it degrades to home — the
                    // spawn's success may not depend on a directory this
                    // process cannot control — and the pane says so after
                    // its content is restored.
                    let cwd: Option<PathBuf> =
                        match pane.cwd.as_deref().map(Path::new).filter(|_| !pane.dead) {
                            Some(dir) if dir.is_dir() => Some(dir.to_path_buf()),
                            Some(dir) => {
                                let home = dirs::home_dir().unwrap_or_default();
                                cwd_fallbacks.insert(
                                    pane.id,
                                    format!(
                                        "\r\npar-mux: {} is gone; pane restored in {}\r\n",
                                        dir.display(),
                                        home.display()
                                    ),
                                );
                                home.is_dir().then_some(home)
                            }
                            None => None,
                        };
                    let context = SpawnContext {
                        session: Some((SessionId(session.id), &session.name)),
                        window: Some(WindowId(window.id)),
                        env: Some(&session.env),
                        cwd: cwd.as_deref(),
                        // No client can connect before the accept loop
                        // starts, so restore wires after the tree is built
                        // (`MuxServer::bind_with_tree`); the bytes land in
                        // the grid that clients seed from.
                        output: None,
                    };
                    let mut created = if pane.dead {
                        factory.create_dead_pane(
                            PaneId(pane.id),
                            cols,
                            rows,
                            pane.spawn_command.as_deref(),
                            pane.exit_code,
                        )?
                    } else {
                        match resume_argv {
                            Some(argv) => factory.create_argv_pane(
                                PaneId(pane.id),
                                cols,
                                rows,
                                &argv,
                                &context,
                            )?,
                            None => factory.create_pane(
                                PaneId(pane.id),
                                cols,
                                rows,
                                pane.spawn_command.as_deref(),
                                &context,
                            )?,
                        }
                    };
                    // Identity comes back through the typed claim (ARC-113c)
                    // so the format round-trips and task 6.3's hook-first
                    // lookup reads it from the same place it reads a live
                    // pane's. Only the identity fields — no state, no seq,
                    // no start source (a restored pane reports those anew or
                    // holds none).
                    if let Some(agent_session) = &pane.agent_session {
                        let mut claim = crate::mux::hooks::AgentClaim {
                            agent: agent_session.agent.clone(),
                            session_id: agent_session.session_id.clone(),
                            session_path: agent_session.session_path.clone(),
                            source: agent_session.source.clone(),
                            ..Default::default()
                        };
                        // Verbatim argv passthrough: a malformed (hand-edited)
                        // argv string in the state file cannot parse into the
                        // typed argv, so it is stored raw rather than silently
                        // dropped — loading an old file never loses what the
                        // file said.
                        let mut raw_argv: Option<&str> = None;
                        match agent_session
                            .resume_argv
                            .as_deref()
                            .map(|raw| (raw, serde_json::from_str::<Vec<String>>(raw)))
                        {
                            Some((_, Ok(argv))) => claim.resume_argv = Some(argv),
                            Some((raw, Err(_))) => raw_argv = Some(raw),
                            None => {}
                        }
                        created.set_agent_claim(&claim);
                        if let Some(raw) = raw_argv {
                            created.set_metadata("agent_resume_argv", raw);
                        }
                    }
                    if let Some(title) = &pane.user_title {
                        created.set_user_title(title);
                    }
                    if pane.dead {
                        // `begin_respawn` reads the pane's cwd from the
                        // terminal's OSC 7 state (the dead pane has no
                        // child to ask), and the next save captures it
                        // from the same place — so the persisted cwd is
                        // seeded there rather than lost with the process.
                        if let Some(dir) = &pane.cwd {
                            created.with_terminal_mut(|term| {
                                let si = term.shell_integration_mut();
                                si.set_cwd(dir.clone());
                                si.set_hostname(pane.cwd_host.clone());
                            });
                        }
                    }
                    panes.insert(PaneId(pane.id), created);
                }
                for pane in &window.panes {
                    // Through the geometry-publishing path (QA-195): the
                    // restored cursor and note are what `cursor_position()`
                    // serves before the new process prints anything.
                    let note_batch = panes
                        .get(&PaneId(pane.id))
                        .expect("just inserted above")
                        .with_terminal_mut(|restored| {
                            // ARC-113c2: the snapshot's grid dims are
                            // state-file input exactly like the window size
                            // above, so clamp them before
                            // `restore_for_new_process` adopts them.
                            let mut snapshot = pane.terminal.clone();
                            clamp_restored_grid_dims(&mut snapshot);
                            if pane.dead {
                                // No new process to protect: the frozen
                                // screen returns exactly as the client saw
                                // it, modes and alt screen included.
                                restored.restore_from_snapshot(snapshot);
                            } else {
                                restored.restore_for_new_process(snapshot);
                            }
                            // After the snapshot re-hangs, so the note is the
                            // last thing on screen rather than scrolled away.
                            // Its observer events wait for the terminal
                            // lock to drop and go out with the re-fits'.
                            cwd_fallbacks
                                .get(&pane.id)
                                .map(|note| restored.process_deferred(note.as_bytes()))
                        });
                    note_batches.extend(note_batch);
                }
                session_windows.push(MuxWindow {
                    id: WindowId(window.id),
                    name: crate::mux::strip_controls(&window.name).into_owned(),
                    layout: window.layout.clone(),
                    active: PaneId(window.active_pane),
                    cols,
                    rows,
                    // Zoom is session state, not layout — restored
                    // windows start unzoomed (tmux's behavior).
                    zoomed: None,
                    chrome: Default::default(),
                });
            }
            sessions.push((
                MuxSession {
                    id: SessionId(session.id),
                    name: crate::mux::strip_controls(&session.name).into_owned(),
                    windows: Vec::new(),
                    active: session.active_window_index,
                    env: session.env.clone(),
                },
                session_windows,
            ));
        }

        let mut tree = MuxTree::new(factory);
        for batch in note_batches {
            tree.defer_observer_batch(batch);
        }
        tree.ids = IdAllocator::resume(state.next_ids);
        tree.panes = panes;
        tree.buffers = state.buffers.clone();
        for (session, windows) in sessions {
            let session_id = session.id;
            tree.sessions.insert(session_id, session);
            for window in windows {
                tree.insert_window(session_id, window);
            }
        }
        // Workspaces restore after the sessions they reference; a session
        // id absent from the sessions map (a hand-edited file) is skipped,
        // and a workspace left referencing nothing still restores as an
        // empty workspace — the same state `new-workspace` creates.
        let mut active_workspace = None;
        for workspace in &state.workspaces {
            let workspace_id = WorkspaceId(workspace.id);
            let sessions: Vec<SessionId> = workspace
                .sessions
                .iter()
                .filter(|id| tree.sessions.contains_key(&SessionId(**id)))
                .map(|id| SessionId(*id))
                .collect();
            for session_id in &sessions {
                tree.session_workspace.insert(*session_id, workspace_id);
            }
            let active = workspace
                .active_session_index
                .min(sessions.len().saturating_sub(1));
            tree.workspaces.insert(
                workspace_id,
                MuxWorkspace {
                    id: workspace_id,
                    name: crate::mux::strip_controls(&workspace.name).into_owned(),
                    sessions,
                    active,
                },
            );
            if state.active_workspace == Some(workspace.id) {
                active_workspace = Some(workspace_id);
            }
        }
        // An active pointer naming a workspace the file never carried
        // falls back to the first restored one.
        // Lowest id, not map order, when the file named no live workspace.
        tree.active_workspace = active_workspace.or_else(|| tree.workspaces.keys().min().copied());
        // Panes were spawned at their window's full extent, but the restored
        // layout divides that extent — re-fit every terminal (and PTY) to
        // its geometry, exactly as a live resize would have, so a restart
        // lands in the same state a running server would be in.
        for window_id in tree.windows.keys().copied().collect::<Vec<_>>() {
            tree.sync_pane_sizes(window_id);
        }
        // Auto-remove (remain-on-exit = false): a persisted dead entry is
        // dropped, honoring the CURRENT setting at restore time. The
        // kill-pane cascade does the pruning — window, session, and
        // workspace go with a pane that was a window's last — so the
        // restored tree is exactly what a live daemon with the setting off
        // would have held. No PTY concern: a dead entry is processless.
        if !remain_on_exit {
            let dead_ids: Vec<PaneId> = state
                .sessions
                .iter()
                .flat_map(|s| s.windows.iter())
                .flat_map(|w| w.panes.iter())
                .filter(|pane| pane.dead)
                .map(|pane| PaneId(pane.id))
                .collect();
            for pane_id in dead_ids {
                let _ = tree.kill_pane(pane_id);
            }
        }
        // The tree is still owned here, behind no lock, so the notes' and
        // re-fits' observer events go out now rather than riding into the
        // server.
        for batch in tree.take_observer_batches() {
            batch.deliver();
        }
        Ok(tree)
    }
}

/// The state file for a server listening on `socket_path` (D3.4):
/// `<state_dir>/par-mux/<socket-stem>.state.json`. The socket itself lives
/// in the ephemeral temp dir by design; state must not — pane content is as
/// private as the terminal it came from.
pub fn state_file_path(socket_path: &Path) -> PathBuf {
    state_file_in(&platform_state_dir(), socket_path)
}

/// The platform state dir, falling back to the data dir on platforms
/// without a distinct one (D3.4: macOS has no XDG state dir, so state lives
/// under `~/Library/Application Support` there).
fn platform_state_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
}

/// The pure half of [`state_file_path`]: a socket at
/// `<any>/par-mux-<stem>.sock` maps to `<base>/par-mux/<stem>.state.json`,
/// so two servers on different sockets never share state. `base` is exposed
/// (ARC-017) so `par-mux --state-dir <dir>` can override
/// [`platform_state_dir`] without duplicating this join logic.
pub fn state_file_in(base: &Path, socket_path: &Path) -> PathBuf {
    let stem = socket_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("default");
    base.join("par-mux").join(format!("{stem}.state.json"))
}

/// Atomically persist `tree` to `target` (D3.3): serialize to
/// `<target>.tmp`, fsync, then rename over the target — a crash mid-save
/// leaves either the complete previous state or a leftover tmp the next
/// save overwrites, never a torn file. On Unix the file is created `0600`,
/// the same owner-only posture as the socket (D3.4).
///
/// Cap one pane's persisted snapshot to [`MAX_PERSISTED_SCROLLBACK_CELLS`]
/// scrollback cells, keeping the newest lines (see the const's note for why
/// the bound exists). Both grids are capped; the alternate screen rarely
/// holds scrollback, but uniformity costs nothing.
fn cap_persisted_scrollback(mut snapshot: TerminalSnapshot) -> TerminalSnapshot {
    snapshot.grid = cap_grid_scrollback(snapshot.grid);
    snapshot.alt_grid = cap_grid_scrollback(snapshot.alt_grid);
    snapshot
}

/// Keep only the newest `MAX_PERSISTED_SCROLLBACK_CELLS` scrollback cells
/// of one grid snapshot, dropping the oldest lines. The result is shaped
/// exactly like a younger grid that scrolled only the retained lines:
/// oldest-first contiguous payload (`cells.len() == lines * cols`) and the
/// original `max_scrollback`, so a restored pane still grows to its
/// configured depth. Zones are dropped and clamped at the new floor the
/// same way live eviction does in `push_rows_to_scrollback` — absolute rows
/// and `total_lines_scrolled` keep their frame.
fn cap_grid_scrollback(mut grid: GridSnapshot) -> GridSnapshot {
    let cols = grid.cols;
    if cols == 0 || grid.scrollback_lines == 0 {
        return grid;
    }
    let keep_lines = (MAX_PERSISTED_SCROLLBACK_CELLS / cols).min(grid.scrollback_lines);
    if keep_lines == grid.scrollback_lines {
        return grid;
    }
    // The extraction below indexes the payload through the snapshot
    // invariants (`cells.len() == lines * cols`, `wrapped` one entry per
    // line, oldest first). A snapshot violating that shape is passed
    // through untrimmed rather than sliced on a guess.
    let physical_lines = grid.scrollback_cells.len() / cols;
    if physical_lines < grid.scrollback_lines || grid.scrollback_wrapped.len() != physical_lines {
        return grid;
    }
    let drop_lines = grid.scrollback_lines - keep_lines;
    // Lines are stored oldest-first in the flat payload, so dropping the
    // oldest `drop_lines` is a plain slice of consecutive rows.
    let physicals: Vec<usize> = (drop_lines..grid.scrollback_lines).collect();
    if physicals.iter().any(|&physical| physical >= physical_lines) {
        return grid;
    }
    let mut cells = Vec::with_capacity(keep_lines * cols);
    let mut wrapped = Vec::with_capacity(keep_lines);
    for physical in physicals {
        let base = physical * cols;
        cells.extend_from_slice(&grid.scrollback_cells[base..base + cols]);
        wrapped.push(grid.scrollback_wrapped[physical]);
    }
    grid.scrollback_cells = cells;
    grid.scrollback_wrapped = wrapped;
    grid.scrollback_lines = keep_lines;
    // Zone floor mirrors `evict_zones`: zones wholly below the retained
    // window are gone (a snapshot carries no evicted-zone list), and a
    // straddling zone clamps its start to the floor.
    let floor = grid.total_lines_scrolled.saturating_sub(keep_lines);
    grid.zones = grid
        .zones
        .iter()
        .filter(|zone| zone.abs_row_end >= floor)
        .cloned()
        .map(|mut zone| {
            if zone.abs_row_start < floor {
                zone.abs_row_start = floor;
            }
            zone
        })
        .collect();
    grid
}

/// What triggered a save — decides how the last-good snapshot is treated
/// (see [`write_job`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOrigin {
    /// A successful mutating command. The honest current state, whatever it
    /// holds: pane-bearing updates the snapshot, deliberately empty (the
    /// last pane `kill-pane`d) clears it so the next start is fresh.
    Command,
    /// The reaper noticed a pane's child exited on its own. Never touches
    /// the snapshot — a burst of pane deaths (reboot/logout SIGHUPs the
    /// panes before the daemon's own stop) must not degrade it.
    Reap,
    /// The final save on a requested shutdown. Pane-bearing updates the
    /// snapshot; empty leaves it alone, because the emptiness may be the
    /// race above rather than the user's intent.
    Shutdown,
    /// The final save of an exit-when-empty daemon: no clients, and either
    /// zero sessions or every pane dead, held past the grace period with no
    /// shutdown signal received. An empty tree clears the snapshot (the
    /// user closed everything, so the next start is fresh). An all-dead
    /// tree is pane-bearing and refreshes it, so the next start respawns
    /// those panes (MUX.md, Pane Reaping).
    ShutdownEmpty,
}

/// The synchronous entry the shutdown save and tests use; the per-command
/// path captures a [`PersistState`] under the tree lock and hands it to the
/// server's persist worker, which writes through [`write_job`] off the
/// lock.
pub fn save_to(tree: &MuxTree, target: &Path) -> Result<(), PersistError> {
    write_job(SaveOrigin::Shutdown, &tree.to_persist_state(), target)
}

/// [`save_to`] with an explicit origin. The whole save — capture, cwd
/// syscalls, serialization, fsync — runs while the caller holds `tree`;
/// a caller with the tree behind a mutex should prefer [`save_off_lock`].
pub fn save_to_with_origin(
    tree: &MuxTree,
    target: &Path,
    origin: SaveOrigin,
) -> Result<(), PersistError> {
    write_job(origin, &tree.to_persist_state(), target)
}

/// Save a mutex-held tree with the lock held only for the cheap structure
/// read (ARC-119): collect the capture handles under the lock, drop it,
/// then take the snapshots, resolve cwds, serialize and fsync off it — the
/// discipline every other save already follows. A wedged filesystem during
/// the final save no longer holds the tree mutex the reap tick and late
/// client commands share. A command that lands between the capture and the
/// write is not in this save, as with the periodic path.
pub fn save_off_lock(
    tree: &parking_lot::Mutex<MuxTree>,
    target: &Path,
    origin: SaveOrigin,
) -> Result<(), PersistError> {
    let capture = tree.lock().collect_persist_capture();
    write_job(origin, &capture.capture(), target)
}

/// Serialize and atomically land one already-captured state with the
/// default (command) snapshot semantics — the library-callers' entry.
pub fn write_state(state: &PersistState, target: &Path) -> Result<(), PersistError> {
    write_job(SaveOrigin::Command, state, target)
}

/// Serialize and atomically land one already-captured state (D3.3): write to
/// `<target>.tmp`, fsync, then rename over the target, then maintain the
/// last-good snapshot per `origin`. No tree access — callable from a thread
/// that holds no locks.
pub fn write_job(
    origin: SaveOrigin,
    state: &PersistState,
    target: &Path,
) -> Result<(), PersistError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = target.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    // Owner-only from creation, not chmod-after: the file holds pane
    // content and session environments (which can carry tokens), and a
    // create-then-chmod leaves a umask-readable window. On Windows the file
    // inherits the owner-only ACL of the user-profile state directory.
    let file = {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&tmp)?
    };
    // Buffer the encoder: `to_writer` into a bare File issues one write
    // syscall per JSON fragment, and a scrollback-heavy state (hundreds of
    // thousands of cells) spends minutes in those syscalls — measured
    // 2026-09-22 as the whole of a 122 s final save of a 136 MB state.
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, &state)?;
    let file = writer
        .into_inner()
        .map_err(|err| PersistError::Io(err.into_error()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.sync_all()?;
    drop(file);

    fs::rename(&tmp, target)?;
    maintain_lastgood(origin, state, target);
    Ok(())
}

/// The snapshot sibling a pane-bearing save keeps beside the state file.
fn lastgood_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(".lastgood");
    PathBuf::from(name)
}

/// Whether the state holds a pane anywhere — the difference between "the
/// tree shrank" and "the tree is gone".
fn state_has_panes(state: &PersistState) -> bool {
    state
        .sessions
        .iter()
        .any(|s| s.windows.iter().any(|w| !w.panes.is_empty()))
}

/// Keep `<target>.lastgood` at the newest state the user would want
/// resurrected, per the [`SaveOrigin`] matrix. Best-effort: a failure here
/// leaves the snapshot stale or absent, never the main save damaged.
fn maintain_lastgood(origin: SaveOrigin, state: &PersistState, target: &Path) {
    if origin == SaveOrigin::Reap {
        return;
    }
    let lastgood = lastgood_path(target);
    if state_has_panes(state) {
        if let Err(err) = fs::copy(target, &lastgood) {
            log::warn!(
                "par-mux: last-good snapshot update to {} failed: {err}",
                lastgood.display()
            );
        }
    } else if matches!(origin, SaveOrigin::Command | SaveOrigin::ShutdownEmpty) {
        // Deliberately empty (kill-pane of the last pane, or the daemon
        // exiting because everything closed): the next start must be fresh,
        // not a resurrection.
        let _ = fs::remove_file(&lastgood);
    }
    // An empty Shutdown save leaves the snapshot alone — the emptiness may
    // be the reboot race, not the user's intent.
}

/// What daemon startup found in the state file.
#[derive(Debug)]
pub enum Loaded {
    /// No state file exists — a fresh start.
    Fresh,
    /// A readable, current-version state a rebuild can start from.
    State(Box<PersistState>),
    /// A file existed but was corrupt or carried an unknown version; it has
    /// been renamed aside and the daemon starts fresh (D3.2: unreadable
    /// state never blocks startup — match tmux).
    Quarantined {
        /// The original state-file path.
        from: PathBuf,
        /// The path the file was renamed to.
        to: PathBuf,
    },
}

/// Read the state file at `target`, quarantining a corrupt or
/// unknown-version file aside so the next save cannot overwrite the
/// evidence (D3.2). Every failure degrades to a fresh start; nothing here
/// can block daemon startup.
///
/// A readable but pane-less state falls back to the last-good snapshot
/// when one holds panes: a reboot/logout race kills the panes before the
/// daemon's stop, and the empty tree those deaths persisted is not the
/// layout the user should lose.
pub fn load_or_quarantine(target: &Path) -> Loaded {
    let bytes = match fs::read(target) {
        Ok(bytes) => bytes,
        Err(_) => return Loaded::Fresh,
    };

    let reason = match serde_json::from_slice::<PersistState>(&bytes) {
        Ok(state) if state.format_version == FORMAT_VERSION => {
            if !state_has_panes(&state) {
                if let Some(good) = load_lastgood(target) {
                    log::info!(
                        "par-mux: state {} held no panes; restoring the last-good snapshot",
                        target.display()
                    );
                    return Loaded::State(Box::new(good));
                }
            }
            return Loaded::State(Box::new(state));
        }
        Ok(state) => PersistError::UnsupportedVersion {
            found: state.format_version,
            supported: FORMAT_VERSION,
        },
        Err(err) => PersistError::Serialize(err),
    };

    let mut name = target.as_os_str().to_os_string();
    name.push(format!(".quarantine-{}", unix_ms()));
    let to = PathBuf::from(name);
    match fs::rename(target, &to) {
        Ok(()) => {
            log::warn!(
                "par-mux: state {} was unreadable ({reason}); quarantined as {}",
                target.display(),
                to.display()
            );
            Loaded::Quarantined {
                from: target.to_path_buf(),
                to,
            }
        }
        Err(err) => {
            log::warn!(
                "par-mux: state {} was unreadable ({reason}) and could not be quarantined: {err}",
                target.display()
            );
            Loaded::Fresh
        }
    }
}

/// The pane-bearing snapshot beside `target`, if a usable one exists. A
/// derived cache, not evidence: corrupt or missing degrades to `None`
/// silently rather than quarantining.
fn load_lastgood(target: &Path) -> Option<PersistState> {
    let bytes = fs::read(lastgood_path(target)).ok()?;
    let state = serde_json::from_slice::<PersistState>(&bytes).ok()?;
    (state.format_version == FORMAT_VERSION && state_has_panes(&state)).then_some(state)
}

/// What a read-only peek at the state file predicts the next daemon start
/// will restore. Mirrors [`load_or_quarantine`]'s conclusions (including
/// the last-good fallback) without any of its mutations — the `--restart`
/// pre-flight uses it to tell the caller, on the still-open terminal, what
/// the detached fresh daemon is about to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatePeek {
    /// No state file — the next daemon starts fresh.
    Missing,
    /// The state file exists but is not parseable current-version state —
    /// the next daemon quarantines it aside and starts fresh.
    Unreadable,
    /// Parseable but pane-less with no usable last-good snapshot — the next
    /// daemon restores nothing.
    Empty,
    /// Panes would be restored (from the state file itself or the
    /// last-good snapshot fallback).
    Populated,
}

/// The read-only half of [`load_or_quarantine`]: classify the state file
/// without quarantining or otherwise mutating it. A read or parse failure
/// here stays a classification, never an error.
pub fn peek_state(target: &Path) -> StatePeek {
    let bytes = match fs::read(target) {
        Ok(bytes) => bytes,
        Err(_) => return StatePeek::Missing,
    };
    match serde_json::from_slice::<PersistState>(&bytes) {
        Ok(state) if state.format_version == FORMAT_VERSION => {
            if state_has_panes(&state) || load_lastgood(target).is_some() {
                StatePeek::Populated
            } else {
                StatePeek::Empty
            }
        }
        _ => StatePeek::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::agent_resume::SURVIVING_TAIL;
    use crate::mux::layout::SplitDirection;
    use crate::mux::pane::{MuxPane, PaneFactory, ShellPaneFactory};
    use std::path::PathBuf;

    /// A per-test target path inside a fresh `TempDir`. The directory name
    /// carries OS-provided randomness, so neither parallel tests nor a later
    /// run can share it (a `process::id()`-derived name repeats once the OS
    /// recycles the pid, and a stale state file would then be loaded). The
    /// returned guard removes the directory — target, `.tmp` sibling, and any
    /// quarantined copy — on drop, even when the test panics; bind it to a
    /// name, since `let _` would drop it at once.
    fn temp_target(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("par-mux-persist-{name}-"))
            .tempdir()
            .expect("create persist temp dir");
        let target = dir.path().join("state.json");
        (dir, target)
    }

    /// The tmp sibling `save_to` must never leave behind.
    fn tmp_sibling(target: &Path) -> PathBuf {
        let mut name = target.as_os_str().to_os_string();
        name.push(".tmp");
        PathBuf::from(name)
    }

    pub(super) fn tree() -> MuxTree {
        MuxTree::new(Box::new(ShellPaneFactory::default()))
    }

    /// Commands restores handed the factory, in spawn order.
    type RecordedCommands = Vec<(PaneId, Option<String>)>;

    /// Records the command each restore computed, delegating the actual
    /// spawn to a bounded sleeper: restoring an agent pane must not launch a
    /// real agent CLI from a unit test, and the recorded value is the
    /// assertion target — exactly what `from_persist_state` handed the
    /// factory.
    #[derive(Clone, Default)]
    struct RecordingFactory {
        received: std::sync::Arc<std::sync::Mutex<RecordedCommands>>,
        /// The size each pane was created at, in spawn order — the grid
        /// allocation input (ARC-113b).
        sizes: std::sync::Arc<std::sync::Mutex<Vec<(PaneId, u16, u16)>>>,
    }

    impl RecordingFactory {
        fn command_for(&self, id: PaneId) -> Option<String> {
            self.received
                .lock()
                .unwrap()
                .iter()
                .find(|(pane, _)| *pane == id)
                .and_then(|(_, command)| command.clone())
        }

        /// The size pane `id` was created at — what its grid was allocated
        /// with.
        fn size_for(&self, id: PaneId) -> (u16, u16) {
            self.sizes
                .lock()
                .unwrap()
                .iter()
                .find(|(pane, _, _)| *pane == id)
                .map(|(_, cols, rows)| (*cols, *rows))
                .expect("recorded pane")
        }
    }

    impl PaneFactory for RecordingFactory {
        fn create_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            command: Option<&str>,
            context: &SpawnContext<'_>,
        ) -> Result<MuxPane, MuxError> {
            self.received
                .lock()
                .unwrap()
                .push((id, command.map(str::to_string)));
            self.sizes.lock().unwrap().push((id, cols, rows));
            ShellPaneFactory::default().create_pane(id, cols, rows, Some("sleep 60"), context)
        }

        fn create_dead_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            command: Option<&str>,
            exit_code: Option<i32>,
        ) -> Result<MuxPane, MuxError> {
            ShellPaneFactory::default().create_dead_pane(id, cols, rows, command, exit_code)
        }
    }

    /// Two sessions: the first with a split window plus a second window, the
    /// second plain — enough shape that a round trip has something to lose.
    pub(super) fn populated_tree() -> MuxTree {
        let mut tree = tree();
        let main = tree.new_session("main", 80, 24).unwrap();
        let main_window = tree.session(main).unwrap().windows[0];
        let first = tree.window(main_window).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.25, None)
            .unwrap();
        tree.select_pane(second).unwrap();
        let logs = tree.new_window(main, "logs", 100, 30).unwrap();
        tree.select_window(logs).unwrap();
        tree.set_buffer("default", "hello".to_string());
        tree.new_session("other", 120, 40).unwrap();
        tree
    }

    /// ARC-032: the two-phase capture (collect under the lock, capture
    /// after) produces the same save as the one-shot form, modulo the
    /// timestamp.
    ///
    /// Parity is a claim about a QUIESCENT tree: a pane whose PTY produced
    /// output between the two captures legitimately differs, so each attempt
    /// requires every pane's generation to hold still across both captures
    /// and retries while startup output is still landing.
    #[test]
    fn two_phase_capture_matches_one_shot() {
        let tree = populated_tree();
        let generations = |tree: &MuxTree| {
            tree.sessions()
                .iter()
                .flat_map(|s| tree.session(*s).unwrap().windows.clone())
                .flat_map(|w| tree.window(w).unwrap().panes())
                .map(|p| tree.pane(p).unwrap().update_generation())
                .collect::<Vec<_>>()
        };
        for _ in 0..50 {
            let before = generations(&tree);
            let one_shot = serde_json::to_value(tree.to_persist_state()).unwrap();
            let two_phase = serde_json::to_value(tree.collect_persist_capture().capture()).unwrap();
            if generations(&tree) == before {
                let strip = |mut value: serde_json::Value| {
                    value.as_object_mut().unwrap().remove("saved_at_unix_ms");
                    value
                };
                assert_eq!(strip(one_shot), strip(two_phase));
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("panes never quiesced within 5 s; parity not testable this run");
    }

    /// ARC-032: the capture owns no tree borrow — it completes after the
    /// tree is gone, the lifetime-level guarantee that the expensive half
    /// (grid walks, cwd syscalls) cannot be running under the tree lock.
    #[test]
    fn capture_outlives_the_tree() {
        let capture = {
            let tree = populated_tree();
            tree.collect_persist_capture()
        };
        let state = capture.capture();
        assert!(!state.sessions.is_empty(), "the captured tree had sessions");
    }

    /// Assert every structural fact a round trip must preserve.
    pub(super) fn assert_same_shape(restored: &MuxTree, original: &MuxTree) {
        let mut original_sessions = original.sessions();
        original_sessions.sort();
        let mut restored_sessions = restored.sessions();
        restored_sessions.sort();
        assert_eq!(restored_sessions, original_sessions, "same session ids");

        for id in &original_sessions {
            let a = original.session(*id).unwrap();
            let b = restored.session(*id).unwrap();
            assert_eq!(b.name, a.name, "session {id} name");
            assert_eq!(b.windows, a.windows, "session {id} window ids, in order");
            assert_eq!(b.active, a.active, "session {id} active window index");

            for window_id in &a.windows {
                let wa = original.window(*window_id).unwrap();
                let wb = restored.window(*window_id).unwrap();
                assert_eq!(wb.name, wa.name, "window {window_id} name");
                assert_eq!((wb.cols, wb.rows), (wa.cols, wa.rows));
                assert_eq!(wb.active, wa.active, "window {window_id} active pane");
                assert_eq!(
                    wb.layout, wa.layout,
                    "window {window_id} layout shape (splits, ratios, leaf order)"
                );
                for pane_id in wa.panes() {
                    assert!(restored.pane(pane_id).is_some(), "pane {pane_id} restored");
                    assert_eq!(
                        restored.pane(pane_id).unwrap().spawn_command(),
                        original.pane(pane_id).unwrap().spawn_command(),
                        "pane {pane_id} spawn command"
                    );
                }
            }
        }
    }

    #[test]
    fn round_trip_preserves_sessions_windows_layout_and_active_panes() {
        let original = populated_tree();
        let state = original.to_persist_state();

        assert_eq!(state.format_version, FORMAT_VERSION);
        assert_eq!(state.sessions.len(), 2);
        assert_eq!(
            state.sessions.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![0, 1],
            "sessions serialize in id order"
        );

        let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
            .expect("state this build wrote must restore");
        assert_same_shape(&restored, &original);
        assert_eq!(
            restored.get_buffer("default").map(str::to_string),
            Some("hello".to_string()),
            "named buffers survive the round trip"
        );
    }

    #[test]
    fn session_env_round_trips_and_restored_panes_spawn_with_it() {
        let mut original = populated_tree();
        let main = original
            .sessions()
            .into_iter()
            .find(|id| original.session(*id).unwrap().name == "main")
            .unwrap();
        original
            .set_session_env(main, "TOKEN", Some("s3cret value"))
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("env.state.json");
        save_to(&original, &target).unwrap();
        let Loaded::State(state) = load_or_quarantine(&target) else {
            panic!("the saved file must load back");
        };
        let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        let env = &restored.session(main).unwrap().env;
        assert_eq!(env.get("TOKEN").map(String::as_str), Some("s3cret value"));
        for window in &restored.session(main).unwrap().windows {
            for pane in restored.window(*window).unwrap().panes() {
                assert_eq!(&factory.spawn_of(pane).env, env);
            }
        }
    }

    #[test]
    fn a_state_file_without_session_env_loads_with_an_empty_one() {
        let mut json = serde_json::to_value(populated_tree().to_persist_state()).unwrap();
        for session in json["sessions"].as_array_mut().unwrap() {
            session.as_object_mut().unwrap().remove("env");
        }
        let state: PersistState = serde_json::from_value(json).unwrap();
        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        for id in restored.sessions() {
            assert!(restored.session(id).unwrap().env.is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_state_file_is_owner_only_even_over_a_stale_world_readable_tmp() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("s.state.json");
        let tmp = dir.path().join("s.state.json.tmp");
        fs::write(&tmp, b"stale").unwrap();
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)).unwrap();
        save_to(&populated_tree(), &target).unwrap();
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn restore_spawns_each_pane_with_its_session_and_window_identity() {
        let original = populated_tree();
        let state = original.to_persist_state();
        let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone()))
            .expect("state this build wrote must restore");
        for session_id in restored.sessions() {
            let session = restored.session(session_id).unwrap();
            for window_id in &session.windows {
                for pane_id in restored.window(*window_id).unwrap().panes() {
                    let spawn = factory.spawn_of(pane_id);
                    assert_eq!(spawn.session, Some((session_id, session.name.clone())));
                    assert_eq!(spawn.window, Some(*window_id));
                }
            }
        }
    }

    /// SEC-209: names in a (hand-edited or older) save file are assigned
    /// directly on restore, so restore strips them too.
    #[test]
    fn restore_strips_control_characters_from_tree_names() {
        let mut tree = tree();
        let workspace = tree.new_workspace("ws");
        let session = tree.new_session("s", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let mut state = tree.to_persist_state();
        for ws in &mut state.workspaces {
            ws.name = "w\x1bs".to_string();
        }
        for s in &mut state.sessions {
            s.name = "s\x07n".to_string();
            for w in &mut s.windows {
                w.name = "w\u{9b}n".to_string();
            }
        }
        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        assert_eq!(restored.workspace(workspace).unwrap().name, "ws");
        assert_eq!(restored.session(session).unwrap().name, "sn");
        assert_eq!(restored.window(window).unwrap().name, "wn");
    }

    #[test]
    fn round_trip_preserves_user_titles() {
        let mut tree = tree();
        let session = tree.new_session("titled", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let pane_id = tree.window(window).unwrap().panes()[0];
        tree.pane_mut(pane_id)
            .expect("the session's pane exists")
            .set_user_title("keep me");

        let state = tree.to_persist_state();
        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        assert_eq!(
            restored.pane(pane_id).and_then(|p| p.user_title()),
            Some("keep me"),
            "the user title survives the round trip"
        );
    }

    #[test]
    fn a_save_file_without_the_title_field_still_loads() {
        // serde(default): a save file written before user titles existed
        // carries no `user_title` key and must decode with no title rather
        // than fail — the strip below simulates exactly that older file.
        let mut tree = tree();
        let session = tree.new_session("old", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let pane_id = tree.window(window).unwrap().panes()[0];
        tree.pane_mut(pane_id)
            .unwrap()
            .set_user_title("dropped by the strip");

        let json = serde_json::to_string(&tree.to_persist_state()).unwrap();
        assert!(
            json.contains("user_title"),
            "positive control: the field is on the wire"
        );
        let old_format = json.replace(",\"user_title\":\"dropped by the strip\"", "");
        assert!(
            !old_format.contains("user_title"),
            "the strip removed the only occurrence"
        );

        let state: PersistState =
            serde_json::from_str(&old_format).expect("a pre-title save file decodes");
        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        assert_eq!(
            restored.pane(pane_id).and_then(|p| p.user_title()),
            None,
            "a missing field decodes as no title, not an error"
        );
    }

    /// A pane's last reported cwd (OSC 7 first, the process's live cwd
    /// second) is captured on save and handed back to the factory on
    /// restore, so a resumed pane — shell or agent — lands where it left
    /// off instead of in the daemon's start directory. Unix only: the
    /// OSC 7 URL is built from a `/`-rooted tempdir path, and the fallback
    /// probe below needs a pid→cwd source Windows does not offer.
    #[test]
    #[cfg(unix)]
    fn osc7_cwd_is_captured_and_restored_panes_spawn_in_it() {
        let mut tree = tree();
        let session = tree.new_session("cwd", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let pane_id = tree.window(window).unwrap().panes()[0];
        let dir = tempfile::tempdir().unwrap();
        let reported = dir.path().join("project");
        std::fs::create_dir(&reported).unwrap();
        // The shell-integration report a pane with OSC 7 hooks emits on
        // every prompt — the primary cwd source.
        tree.pane(pane_id)
            .unwrap()
            .terminal()
            .write()
            .process(format!("\x1b]7;file://localhost{}\x1b\\", reported.display()).as_bytes());

        let state = tree.to_persist_state();
        assert_eq!(
            state.sessions[0].windows[0].panes[0].cwd.as_deref(),
            Some(reported.to_str().unwrap()),
            "positive control: the OSC 7 cwd is on the wire"
        );

        let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        assert_eq!(
            factory.spawn_of(pane_id).cwd.as_deref(),
            Some(reported.as_path()),
            "the restored spawn carries the persisted cwd"
        );
        assert!(restored.pane(pane_id).is_some());
    }

    /// Without OSC 7 the pane's live process cwd is captured instead — a
    /// plain shell that never reported anything still restores where the
    /// user had `cd`-ed, because the child's own cwd is readable. Unix
    /// only: `process_cwd` has no Windows implementation (no documented
    /// pid→cwd API), so there is nothing to assert there.
    #[test]
    #[cfg(unix)]
    fn a_pane_without_osc7_persists_its_process_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let factory = ShellPaneFactory {
            cwd: Some(dir.path().to_path_buf()),
            ..ShellPaneFactory::default()
        };
        let pane = factory
            .create_pane(
                PaneId(1),
                80,
                24,
                Some("sleep 60"),
                &SpawnContext::default(),
            )
            .unwrap();
        // The kernel resolves symlinks in the vnode path (`/var` →
        // `/private/var` on macOS), so compare canonical-to-canonical.
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(
            pane.persistence_cwd().as_deref(),
            Some(canonical.as_path()),
            "no OSC 7 report falls back to the process's live cwd"
        );
    }

    /// OSC 7 wins when both sources exist: the shell's report is the
    /// pane's logical cwd even if the child process has moved. Unix only,
    /// for the same reason as the process-cwd test above (the report URL
    /// is `/`-rooted; the losing fallback is unix's pid→cwd probe).
    #[test]
    #[cfg(unix)]
    fn osc7_wins_over_the_process_cwd() {
        let spawn_dir = tempfile::tempdir().unwrap();
        let factory = ShellPaneFactory {
            cwd: Some(spawn_dir.path().to_path_buf()),
            ..ShellPaneFactory::default()
        };
        let pane = factory
            .create_pane(
                PaneId(1),
                80,
                24,
                Some("sleep 60"),
                &SpawnContext::default(),
            )
            .unwrap();
        let reported = tempfile::tempdir().unwrap();
        pane.terminal().write().process(
            format!("\x1b]7;file://localhost{}\x1b\\", reported.path().display()).as_bytes(),
        );
        assert_eq!(
            pane.persistence_cwd().as_deref(),
            Some(reported.path()),
            "the OSC 7 report outranks the process cwd"
        );
    }

    /// A persisted cwd whose directory no longer exists must not fail the
    /// restore: the pane spawns in home and the pane itself says so.
    #[test]
    fn a_gone_persisted_cwd_falls_back_to_home_with_a_visible_message() {
        let mut tree = tree();
        let session = tree.new_session("gone", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let pane_id = tree.window(window).unwrap().panes()[0];
        let mut state = tree.to_persist_state();
        state.sessions[0].windows[0].panes[0].cwd = Some("/par-mux-test-no-such-dir".to_string());

        let mut restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
                .expect("a gone cwd degrades, never fails the restore");
        assert!(
            restored.pane_mut(pane_id).unwrap().poll_running(),
            "the pane is alive in the fallback cwd"
        );
        let snapshot = restored
            .pane(pane_id)
            .unwrap()
            .terminal()
            .read()
            .capture_snapshot();
        let text: String = snapshot.grid.cells.iter().map(|c| c.c).collect();
        assert!(
            text.contains("par-mux: /par-mux-test-no-such-dir is gone"),
            "the fallback announced itself in the pane; screen held: {text}"
        );
        // QA-195 consistency check: the wait-free cursor matches the
        // terminal's after a restore. Not isolating — the re-fit that ends
        // `from_persist_state` republishes too. Both are read under the
        // terminal's read lock, since every publish holds its write lock.
        let pane = restored.pane(pane_id).unwrap();
        let (published, cursor) = {
            let terminal = pane.terminal();
            let term = terminal.read();
            (
                pane.published_cursor(),
                (term.cursor().col, term.cursor().row),
            )
        };
        assert_eq!(
            published, cursor,
            "the geometry mirror follows the restored screen"
        );
    }

    /// Records whether a zone eviction reached the observer and whether the
    /// pane's terminal lock was free while the callback ran (a delivery
    /// inside `with_terminal_mut` fails `try_read` on the delivering thread).
    #[cfg(unix)]
    struct TerminalLockProbe {
        terminal: std::sync::Weak<parking_lot::RwLock<crate::terminal::Terminal>>,
        scrolled_out: std::sync::atomic::AtomicBool,
        terminal_lock_was_free: std::sync::atomic::AtomicBool,
    }

    #[cfg(unix)]
    impl crate::terminal::observer::TerminalObserver for TerminalLockProbe {
        fn on_zone_event(&self, event: &crate::terminal::TerminalEvent) {
            use std::sync::atomic::Ordering;
            if matches!(
                event,
                crate::terminal::TerminalEvent::ZoneScrolledOut { .. }
            ) {
                let free = self
                    .terminal
                    .upgrade()
                    .is_some_and(|terminal| terminal.try_read().is_some());
                self.terminal_lock_was_free.store(free, Ordering::SeqCst);
                self.scrolled_out.store(true, Ordering::SeqCst);
            }
        }
    }

    /// Spawns quiet `sleep 60` panes, each carrying a [`TerminalLockProbe`]
    /// attached before restore touches its terminal.
    #[cfg(unix)]
    #[derive(Default)]
    struct RestoreProbeFactory {
        probes: std::sync::Arc<std::sync::Mutex<Vec<std::sync::Arc<TerminalLockProbe>>>>,
    }

    #[cfg(unix)]
    impl PaneFactory for RestoreProbeFactory {
        fn create_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            _command: Option<&str>,
            context: &SpawnContext<'_>,
        ) -> Result<MuxPane, MuxError> {
            let pane = ShellPaneFactory::default().create_pane(
                id,
                cols,
                rows,
                Some("sleep 60"),
                context,
            )?;
            let terminal = pane.terminal();
            let probe = std::sync::Arc::new(TerminalLockProbe {
                terminal: std::sync::Arc::downgrade(&terminal),
                scrolled_out: std::sync::atomic::AtomicBool::new(false),
                terminal_lock_was_free: std::sync::atomic::AtomicBool::new(false),
            });
            terminal.write().add_observer(probe.clone());
            self.probes.lock().unwrap().push(probe);
            Ok(pane)
        }

        fn create_dead_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            command: Option<&str>,
            exit_code: Option<i32>,
        ) -> Result<MuxPane, MuxError> {
            ShellPaneFactory::default().create_dead_pane(id, cols, rows, command, exit_code)
        }
    }

    /// Card 01a119fc: the gone-cwd note restore writes into a restored pane
    /// raises observer events (here a zone the note scrolls out of a
    /// scrollback parked at its cap). They must not be lost, and they are
    /// delivered after the pane's terminal write lock drops — not inline
    /// inside `with_terminal_mut`.
    #[cfg(unix)]
    #[test]
    fn restore_note_observer_events_arrive_after_the_terminal_lock_drops() {
        use std::sync::atomic::Ordering;
        let mut tree = tree();
        let session = tree.new_session("note", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let pane_id = tree.window(window).unwrap().panes()[0];
        let mut state = tree.to_persist_state();
        drop(tree);

        // A screen holding one completed command zone at the top of a
        // scrollback filled exactly to its cap, cursor on the bottom row:
        // the first line the note scrolls evicts that zone.
        let (cols, rows) = (80usize, 24usize);
        let mut term = crate::terminal::Terminal::with_scrollback(cols, rows, 50);
        let mut bytes =
            b"\x1b]133;A\x07$ \x1b]133;B\x07cmd\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07".to_vec();
        let line = "x".repeat(cols);
        let fill = term.grid().max_scrollback() - term.grid().scrollback_len() + rows - 3;
        for _ in 0..fill {
            bytes.extend_from_slice(b"\r\n");
            bytes.extend_from_slice(line.as_bytes());
        }
        term.process(&bytes);
        assert_eq!(term.grid().scrollback_len(), term.grid().max_scrollback());
        assert_eq!(term.cursor().row, rows - 1, "cursor on the bottom row");
        assert_eq!(term.get_zones().len(), 3, "no zone evicted yet");
        let pane = &mut state.sessions[0].windows[0].panes[0];
        pane.terminal = term.capture_snapshot();
        pane.cwd = Some("/par-mux-test-no-such-dir".to_string());

        let factory = RestoreProbeFactory::default();
        let probes = factory.probes.clone();
        let mut restored = MuxTree::from_persist_state(&state, Box::new(factory))
            .expect("a gone cwd degrades, never fails the restore");
        let probe = probes.lock().unwrap()[0].clone();
        assert!(
            restored.take_observer_batches().is_empty(),
            "restore drained its own batches"
        );

        assert!(
            probe.scrolled_out.load(Ordering::SeqCst),
            "the note's ZoneScrolledOut reached the observer by the time restore returned"
        );
        assert!(
            probe.terminal_lock_was_free.load(Ordering::SeqCst),
            "the observer ran under the pane's terminal write lock"
        );
        let snapshot = restored
            .pane(pane_id)
            .unwrap()
            .terminal()
            .read()
            .capture_snapshot();
        let text: String = snapshot.grid.cells.iter().map(|c| c.c).collect();
        assert!(text.contains("par-mux: /par-mux-test-no-such-dir is gone"));
    }

    /// serde(default): a save file written before pane cwds existed carries
    /// no `cwd` key and must decode — and spawn — with the factory default.
    #[test]
    fn a_save_file_without_the_cwd_field_still_loads() {
        let mut tree = tree();
        let session = tree.new_session("old", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let pane_id = tree.window(window).unwrap().panes()[0];
        tree.pane(pane_id)
            .unwrap()
            .terminal()
            .write()
            .process(b"\x1b]7;file://localhost/tmp\x1b\\");

        let json = serde_json::to_string(&tree.to_persist_state()).unwrap();
        assert!(
            json.contains("\"cwd\""),
            "positive control: the field is on the wire"
        );
        let old_format = json.replace(",\"cwd\":\"/tmp\"", "");
        assert!(
            !old_format.contains("\"cwd\""),
            "the strip removed the only occurrence"
        );

        let state: PersistState =
            serde_json::from_str(&old_format).expect("a pre-cwd save file decodes");
        let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        assert_eq!(
            factory.spawn_of(pane_id).cwd,
            None,
            "a missing field decodes as no cwd, not an error"
        );
        assert!(restored.pane(pane_id).is_some());
    }

    /// A one-pane tree on [`RecordingFactory`] (a silent `sleep`, so no
    /// prompt bytes race the assertions) whose pane carries a frozen screen
    /// that sets up an alt screen and mouse mode, then is marked held-dead
    /// with `exit_code`. Returns the tree and the pane id.
    fn held_dead_tree(exit_code: Option<i32>) -> (MuxTree, PaneId) {
        let mut tree = MuxTree::new(Box::new(RecordingFactory::default()));
        let session = tree.new_session("dead", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let pane_id = tree.window(window).unwrap().panes()[0];
        let pane = tree.pane_mut(pane_id).unwrap();
        pane.terminal()
            .write()
            .process(b"MAIN-LINE\x1b[?1049h\x1b[?1000hALT-LINE");
        pane.set_metadata("agent", "claude");
        pane.set_metadata("agent_session_id", "sess-114");
        pane.mark_dead_with_code(exit_code);
        (tree, pane_id)
    }

    /// ARC-114: a held-dead pane survives a save/restore as a held-dead
    /// pane — no process, exit code kept, screen byte-for-byte (the alt
    /// screen and mouse mode a respawned pane would have reset stay), and
    /// the spawn command retained for a later `respawn-pane`.
    #[test]
    fn a_held_dead_pane_round_trips_as_held_dead() {
        let (tree, pane_id) = held_dead_tree(Some(3));
        let state = tree.to_persist_state();
        let saved = &state.sessions[0].windows[0].panes[0];
        assert!(saved.dead, "positive control: dead is on the wire");
        assert_eq!(saved.exit_code, Some(3));
        assert!(saved.terminal.alt_screen_active, "control: alt screen up");

        let json = serde_json::to_string(&state).unwrap();
        let state: PersistState = serde_json::from_str(&json).unwrap();
        let factory = RecordingFactory::default();
        let restored =
            MuxTree::from_persist_state_with_policy(&state, Box::new(factory.clone()), true)
                .unwrap();

        assert!(
            factory.command_for(pane_id).is_none() && factory.sizes.lock().unwrap().is_empty(),
            "a dead pane is never handed to the spawning factory path"
        );
        let pane = restored.pane(pane_id).unwrap();
        assert!(pane.dead());
        assert_eq!(pane.exit_code(), Some(3), "the code is not clobbered");
        assert!(pane.child_pid().is_none(), "no process behind it");
        assert_eq!(pane.spawn_command(), Some("sleep 60"));

        // Identity is re-hung for a later respawn, but no resume invocation
        // ran (the factory above was never asked to spawn anything).
        let resaved = restored.to_persist_state();
        assert!(saved.agent_session.is_some(), "control: identity was saved");
        assert_eq!(
            resaved.sessions[0].windows[0].panes[0].agent_session, saved.agent_session,
            "the agent identity survives a dead restore"
        );

        let want = &state.sessions[0].windows[0].panes[0].terminal;
        let got = pane.terminal().read().capture_snapshot();
        assert_eq!(got.grid.cells, want.grid.cells, "main screen identical");
        assert_eq!(
            got.alt_grid.cells, want.alt_grid.cells,
            "alt screen identical"
        );
        assert!(got.alt_screen_active, "alt screen not reset to primary");
        assert_eq!(got.cursor.col, want.cursor.col);
        assert_eq!(got.cursor.row, want.cursor.row);
        assert_ne!(
            pane.terminal().read().mouse_mode(),
            crate::mouse::MouseMode::Off,
            "mouse mode kept: there is no new process to protect"
        );
        let text: String = got.alt_grid.cells.iter().map(|c| c.c).collect();
        assert!(text.contains("ALT-LINE"), "the frozen content is real");
    }

    /// remain-on-exit off (the default): a persisted dead entry is DROPPED
    /// at restore, honoring the current setting — and the kill-pane cascade
    /// takes its window, session, and workspace with it when it was the
    /// last pane. A live pane in the same save is untouched.
    #[test]
    fn auto_remove_restore_drops_dead_entries() {
        let (mut tree, dead_id) = held_dead_tree(Some(3));
        let live_id = tree
            .split_pane(dead_id, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        let state = tree.to_persist_state();

        let restored = MuxTree::from_persist_state_with_policy(
            &state,
            Box::new(RecordingFactory::default()),
            false,
        )
        .unwrap();
        assert!(restored.pane(dead_id).is_none(), "the dead entry is gone");
        assert!(
            restored.pane(live_id).is_some(),
            "the live pane survives the prune"
        );
        assert!(
            !restored.sessions.is_empty(),
            "the window still has a live pane, so the session stays"
        );
        tree_consistency(&restored);
    }

    /// remain-on-exit off, dead-only save: the whole window, session, and
    /// workspace cascade — the restored tree is empty, exactly what a live
    /// daemon with the setting off would have been left holding.
    #[test]
    fn auto_remove_restore_of_a_dead_only_save_empties_the_tree() {
        let (tree, _pane_id) = held_dead_tree(Some(3));
        let state = tree.to_persist_state();
        let restored = MuxTree::from_persist_state_with_policy(
            &state,
            Box::new(RecordingFactory::default()),
            false,
        )
        .unwrap();
        assert!(
            restored.panes.is_empty() && restored.sessions.is_empty(),
            "the dead-only save restores to nothing"
        );
        tree_consistency(&restored);
    }

    /// remain-on-exit on: the pre-existing ARC-114 behavior — the dead
    /// entry restores held-dead even though auto-remove is the product
    /// default (the daemon was configured to hold).
    #[test]
    fn hold_restore_keeps_a_dead_entry_held_dead() {
        let (tree, pane_id) = held_dead_tree(Some(3));
        let state = tree.to_persist_state();
        let restored = MuxTree::from_persist_state_with_policy(
            &state,
            Box::new(RecordingFactory::default()),
            true,
        )
        .unwrap();
        assert!(restored.pane(pane_id).unwrap().dead());
        assert_eq!(restored.pane(pane_id).unwrap().exit_code(), Some(3));
    }

    /// A dead pane with an unknown exit code stays dead with `None`, and a
    /// live pane in the same save is respawned as before.
    #[test]
    fn dead_state_is_per_pane_and_live_panes_still_respawn() {
        let (mut tree, dead_id) = held_dead_tree(None);
        let live_id = tree
            .split_pane(dead_id, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        let state = tree.to_persist_state();
        let factory = RecordingFactory::default();
        let mut restored =
            MuxTree::from_persist_state_with_policy(&state, Box::new(factory.clone()), true)
                .unwrap();
        assert!(restored.pane(dead_id).unwrap().dead());
        assert_eq!(restored.pane(dead_id).unwrap().exit_code(), None);
        assert!(!restored.pane(live_id).unwrap().dead());
        assert!(restored.pane_mut(live_id).unwrap().poll_running());
        assert!(factory.command_for(live_id).is_some(), "live pane spawned");
        assert!(factory.command_for(dead_id).is_none(), "dead pane did not");
    }

    /// ARC-114 compat: a save file without the new fields (any pre-ARC-114
    /// daemon's) decodes and restores every pane alive, and a live pane
    /// serializes without them so older daemons read newer files unchanged.
    #[test]
    fn a_save_file_without_the_dead_fields_loads_alive() {
        let (tree, pane_id) = held_dead_tree(Some(5));
        let mut value = serde_json::to_value(tree.to_persist_state()).unwrap();
        let pane = &mut value["sessions"][0]["windows"][0]["panes"][0];
        assert_eq!(pane["dead"], true, "positive control: on the wire");
        assert_eq!(pane["exit_code"], 5, "positive control: on the wire");
        let pane = pane.as_object_mut().unwrap();
        pane.remove("dead");
        pane.remove("exit_code");
        let state: PersistState = serde_json::from_value(value).expect("old file decodes");
        let mut restored = MuxTree::from_persist_state_with_policy(
            &state,
            Box::new(RecordingFactory::default()),
            true,
        )
        .unwrap();
        assert!(!restored.pane(pane_id).unwrap().dead());
        assert_eq!(restored.pane(pane_id).unwrap().exit_code(), None);
        assert!(
            restored.pane_mut(pane_id).unwrap().poll_running(),
            "respawned"
        );

        let live = serde_json::to_value(populated_tree().to_persist_state()).unwrap();
        for pane in live["sessions"][0]["windows"][0]["panes"]
            .as_array()
            .unwrap()
        {
            assert!(
                pane.get("dead").is_none() && pane.get("exit_code").is_none(),
                "a live pane serializes byte-compatibly with the older format"
            );
        }
    }

    /// The persisted cwd of a dead pane survives the restore (where
    /// `begin_respawn` reads it) without any spawn-time landing logic.
    #[cfg(unix)]
    #[test]
    fn a_dead_panes_cwd_survives_restore_for_respawn() {
        let (tree, pane_id) = held_dead_tree(Some(0));
        tree.pane(pane_id)
            .unwrap()
            .terminal()
            .write()
            .process(b"\x1b]7;file://localhost/tmp\x1b\\");
        let state = tree.to_persist_state();
        assert_eq!(
            state.sessions[0].windows[0].panes[0].cwd.as_deref(),
            Some("/tmp")
        );
        let mut restored = MuxTree::from_persist_state_with_policy(
            &state,
            Box::new(RecordingFactory::default()),
            true,
        )
        .unwrap();
        assert_eq!(
            restored.pane(pane_id).unwrap().persistence_cwd().as_deref(),
            Some(Path::new("/tmp")),
            "re-saving a restored dead pane keeps its cwd"
        );
        let plan = restored.begin_respawn(pane_id, false, None, None).unwrap();
        assert_eq!(plan.cwd.as_deref(), Some(Path::new("/tmp")));
    }

    #[test]
    fn restored_allocator_resumes_without_colliding() {
        let original = populated_tree();
        let state = original.to_persist_state();

        let mut restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        assert_eq!(
            restored.to_persist_state().next_ids,
            state.next_ids,
            "the allocator's counters travel with the state"
        );

        let fresh = restored.new_session("fresh", 80, 24).unwrap();
        assert_eq!(
            fresh,
            SessionId(state.next_ids.1),
            "a restored server's first new session must take the persisted next id"
        );
    }

    /// Workspaces round-trip: members, order, active pointers, and names
    /// all travel, and a restored bare `new-session` lands in the
    /// persisted ACTIVE workspace.
    #[test]
    fn workspaces_round_trip_through_persistence() {
        let mut original = populated_tree();
        let ws_main = original
            .resolve_new_session_workspace(None)
            .expect("populated_tree created its default");
        let ws_dev = original.new_workspace("dev");
        original.rename_workspace(ws_main, "prod").unwrap();
        let expected_main = original.workspace(ws_main).unwrap().sessions.clone();
        original.select_workspace(ws_dev).unwrap();
        let s3 = original.new_session("in-dev", 80, 24).unwrap();

        let state = original.to_persist_state();
        assert_eq!(state.active_workspace, Some(ws_dev.0));
        assert_eq!(
            state
                .workspaces
                .iter()
                .find(|w| w.id == ws_main.0)
                .unwrap()
                .name,
            "prod"
        );

        let mut restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        assert_eq!(restored.workspaces().len(), 2);
        assert_eq!(restored.active_workspace(), Some(ws_dev));
        assert_eq!(
            restored.workspace(ws_main).unwrap().sessions,
            expected_main,
            "workspace membership and order travel"
        );
        assert_eq!(
            restored.workspace(ws_dev).unwrap().sessions,
            vec![s3],
            "the post-rename session landed in the active workspace"
        );
        // A restored bare new-session lands in the persisted active
        // workspace — not a fresh default.
        let fresh = restored.new_session("fresh", 80, 24).unwrap();
        assert_eq!(restored.workspace_of_session(fresh), Some(ws_dev));
        tree_consistency(&restored);
    }

    /// A v2-shaped state file (pre-workspaces) fails the version gate and
    /// is quarantined rather than partially read — the documented
    /// no-migration policy.
    #[test]
    fn a_pre_workspace_state_file_is_quarantined() {
        let (dir, target) = temp_target("v2-quarantine");
        let mut value = serde_json::to_value(populated_tree().to_persist_state()).unwrap();
        value["format_version"] = serde_json::json!(2);
        value.as_object_mut().unwrap().remove("workspaces");
        value.as_object_mut().unwrap().remove("active_workspace");
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&target, &bytes).unwrap();

        match load_or_quarantine(&target) {
            Loaded::Quarantined { from, .. } => assert_eq!(from, target),
            other => panic!("a v2 file must quarantine, got {other:?}"),
        }
        drop(dir);
    }

    /// Assert helper: brute-force the reverse indexes on a restored tree.
    fn tree_consistency(tree: &MuxTree) {
        tree.assert_indexes_consistent();
        for session in tree.sessions() {
            assert!(
                tree.workspace_of_session(session).is_some(),
                "every restored session belongs to a workspace"
            );
        }
    }

    /// SEC-128: a dead pane whose OSC 7 report named a remote host saves and
    /// restores with the host beside the cwd, so `respawn_cwd` still sees the
    /// report as remote.
    #[cfg(unix)]
    #[test]
    fn a_dead_panes_remote_osc7_host_survives_restore() {
        let (tree, pane_id) = held_dead_tree(Some(0));
        tree.pane(pane_id)
            .unwrap()
            .terminal()
            .write()
            .process(b"\x1b]7;file://remote-host/some/path\x1b\\");
        let state = tree.to_persist_state();
        let saved = &state.sessions[0].windows[0].panes[0];
        assert_eq!(saved.cwd.as_deref(), Some("/some/path"));
        assert_eq!(saved.cwd_host.as_deref(), Some("remote-host"));
        let restored = MuxTree::from_persist_state_with_policy(
            &state,
            Box::new(RecordingFactory::default()),
            true,
        )
        .unwrap();
        let pane = restored.pane(pane_id).unwrap();
        let term = pane.terminal();
        let term = term.read();
        assert_eq!(term.current_directory(), Some("/some/path"));
        assert_eq!(term.shell_integration().hostname(), Some("remote-host"));
    }

    /// A save file written before `cwd_host` existed decodes with no host
    /// (today's behavior), and a pane without one serializes without the key.
    #[cfg(unix)]
    #[test]
    fn a_save_file_without_cwd_host_loads_as_before() {
        let (tree, pane_id) = held_dead_tree(Some(0));
        tree.pane(pane_id)
            .unwrap()
            .terminal()
            .write()
            .process(b"\x1b]7;file://remote-host/some/path\x1b\\");
        let mut value = serde_json::to_value(tree.to_persist_state()).unwrap();
        let pane = value["sessions"][0]["windows"][0]["panes"][0]
            .as_object_mut()
            .unwrap();
        assert_eq!(
            pane["cwd_host"], "remote-host",
            "positive control: on the wire"
        );
        pane.remove("cwd_host");
        let state: PersistState = serde_json::from_value(value).expect("old file decodes");
        assert_eq!(state.sessions[0].windows[0].panes[0].cwd_host, None);
        let restored = MuxTree::from_persist_state_with_policy(
            &state,
            Box::new(RecordingFactory::default()),
            true,
        )
        .unwrap();
        let pane = restored.pane(pane_id).unwrap();
        let term = pane.terminal();
        let term = term.read();
        assert_eq!(term.current_directory(), Some("/some/path"));
        assert_eq!(term.shell_integration().hostname(), None);
        drop(term);

        let (plain, _) = held_dead_tree(Some(0));
        let json = serde_json::to_string(&plain.to_persist_state()).unwrap();
        assert!(!json.contains("cwd_host"), "absent host is not serialized");
    }

    #[test]
    fn unknown_format_version_is_refused() {
        let original = populated_tree();
        let mut state = original.to_persist_state();
        state.format_version = FORMAT_VERSION + 1;

        let result = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()));
        assert!(
            matches!(
                result,
                Err(PersistError::UnsupportedVersion { found, supported })
                    if found == FORMAT_VERSION + 1 && supported == FORMAT_VERSION
            ),
            "an unknown envelope version must refuse, not guess"
        );
    }

    /// ARC-113b: a corrupt or hostile state file's window size must not
    /// reach the factory's grid allocation unchecked — zero collapses the
    /// grid's column math and `u16::MAX` (the persisted field's ceiling)
    /// asks the allocator for tens of gigabytes. The restore clamps to the
    /// same bounds a live client's size request is held to, and the
    /// clamped size is both what the factory spawned the pane at and what
    /// the restored window reports.
    #[test]
    fn restore_clamps_a_hostile_window_size() {
        let mut state = populated_tree().to_persist_state();
        let zero_window = state.sessions[0].windows[0].id;
        let huge_window = state.sessions[0].windows[1].id;
        let zero_pane = state.sessions[0].windows[0].panes[0].id;
        let huge_pane = state.sessions[0].windows[1].panes[0].id;
        state.sessions[0].windows[0].cols = 0;
        state.sessions[0].windows[0].rows = 0;
        state.sessions[0].windows[1].cols = u16::MAX;
        state.sessions[0].windows[1].rows = u16::MAX;

        let factory = RecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();

        assert_eq!(
            restored.window(WindowId(zero_window)).unwrap().cols,
            MIN_RESTORED_COLS,
            "a zero-size window restores at the column floor"
        );
        assert_eq!(
            restored.window(WindowId(zero_window)).unwrap().rows,
            MIN_RESTORED_ROWS,
            "a zero-size window restores at the row floor"
        );
        assert_eq!(
            factory.size_for(PaneId(zero_pane)),
            (MIN_RESTORED_COLS, MIN_RESTORED_ROWS)
        );
        assert_eq!(
            restored.window(WindowId(huge_window)).unwrap().cols,
            MAX_RESTORED_COLS,
            "a u16::MAX window restores at the column ceiling"
        );
        assert_eq!(
            restored.window(WindowId(huge_window)).unwrap().rows,
            MAX_RESTORED_ROWS,
            "a u16::MAX window restores at the row ceiling"
        );
        assert_eq!(
            factory.size_for(PaneId(huge_pane)),
            (MAX_RESTORED_COLS, MAX_RESTORED_ROWS)
        );
    }

    /// ARC-113c2: a hostile snapshot's grid dims must clamp to the same
    /// bounds as the window size, and the cell/wrapped vecs must be
    /// reshaped to the clamped shape — `Grid::restore_from_snapshot`
    /// adopts all of them wholesale.
    #[test]
    fn clamp_restored_grid_dims_clamps_hostile_dims_and_reshapes_cells() {
        let tree = populated_tree();
        let session_id = *tree.sessions.keys().next().unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let pane_id = tree.window(window_id).unwrap().panes()[0];
        let mut snap = tree
            .pane(pane_id)
            .unwrap()
            .terminal()
            .read()
            .capture_snapshot();

        snap.grid.cols = 0;
        snap.grid.rows = 0;
        clamp_restored_grid_dims(&mut snap);
        assert_eq!(snap.grid.cols, usize::from(MIN_RESTORED_COLS));
        assert_eq!(snap.grid.rows, usize::from(MIN_RESTORED_ROWS));
        assert_eq!(snap.grid.cells.len(), snap.grid.cols * snap.grid.rows);
        assert_eq!(snap.grid.wrapped.len(), snap.grid.rows);

        snap.grid.cols = usize::from(u16::MAX);
        snap.grid.rows = usize::from(u16::MAX);
        clamp_restored_grid_dims(&mut snap);
        assert_eq!(snap.grid.cols, usize::from(MAX_RESTORED_COLS));
        assert_eq!(snap.grid.rows, usize::from(MAX_RESTORED_ROWS));
        assert_eq!(snap.grid.cells.len(), snap.grid.cols * snap.grid.rows);
        assert_eq!(snap.grid.wrapped.len(), snap.grid.rows);
    }

    /// ARC-113c2 end to end: a state file carrying hostile grid dims in a
    /// pane's snapshot must restore, with the pane's terminal left inside
    /// the restored bounds and readable rather than panicking on collapsed
    /// or giant row-major math.
    #[test]
    fn restore_survives_a_hostile_snapshot_grid_size() {
        let mut state = populated_tree().to_persist_state();
        let zero_pane = state.sessions[0].windows[0].panes[0].id;
        let huge_pane = state.sessions[0].windows[1].panes[0].id;
        state.sessions[0].windows[0].panes[0].terminal.grid.cols = 0;
        state.sessions[0].windows[0].panes[0].terminal.grid.rows = 0;
        state.sessions[0].windows[1].panes[0].terminal.grid.cols = usize::from(u16::MAX);
        state.sessions[0].windows[1].panes[0].terminal.grid.rows = usize::from(u16::MAX);

        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();

        for pane_id in [PaneId(zero_pane), PaneId(huge_pane)] {
            let terminal = restored.pane(pane_id).expect("restored pane").terminal();
            let (cols, rows) = terminal.read().size();
            assert!(
                (usize::from(MIN_RESTORED_COLS)..=usize::from(MAX_RESTORED_COLS)).contains(&cols)
                    && (usize::from(MIN_RESTORED_ROWS)..=usize::from(MAX_RESTORED_ROWS))
                        .contains(&rows),
                "restored pane is {cols}x{rows}, outside the restored bounds"
            );
            let _ = terminal.read().content();
        }
    }

    /// A pane saved while a full-screen app held the alternate screen is
    /// respawned with a fresh shell, which must write to the main screen:
    /// left on the old app's alternate screen, its output never scrolls
    /// into history, so the pane has no scrollback after a restart.
    #[cfg(unix)]
    #[test]
    fn restore_returns_a_full_screen_pane_to_the_main_screen() {
        let mut tree = tree();
        let session = tree.new_session("main", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];
        let quiet = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, Some("sleep 60"))
            .unwrap();
        {
            let terminal = tree.pane(quiet).unwrap().terminal();
            let mut guard = terminal.write();
            guard.process(b"shell line\r\n\x1b[?1049h\x1b[?1h\x1b=\x1b[?1000h\x1b[3;10rtui");
            assert!(guard.is_alt_screen_active(), "setup: app on the alt screen");
        }

        let state = tree.to_persist_state();
        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        let terminal = restored.pane(quiet).unwrap().terminal();
        let mut guard = terminal.write();
        assert!(
            !guard.is_alt_screen_active(),
            "the new shell writes to the main screen"
        );
        let after = guard.capture_snapshot();
        assert!(
            !after.application_cursor,
            "the app's cursor-key mode is gone"
        );
        assert!(!after.application_keypad, "the app's keypad mode is gone");
        assert_eq!(after.mouse_mode, crate::mouse::MouseMode::Off);
        assert_eq!(
            (after.scroll_region_top, after.scroll_region_bottom),
            (0, after.rows - 1),
            "the app's scroll region is gone"
        );
        let screen_text: String = after.grid.cells.iter().map(|c| c.c).collect();
        assert!(
            screen_text.contains("shell line"),
            "the main screen's content survives"
        );

        for i in 0..40 {
            guard.process(format!("new line {i:02}\r\n").as_bytes());
        }
        assert!(
            guard.capture_snapshot().grid.scrollback_lines > 0,
            "new output scrolls into the main screen's history"
        );
    }

    /// Screen content and scrollback survive the round trip. The content
    /// pane runs `sleep` so its process emits nothing — the only bytes in
    /// its terminal are the ones the test fed directly, making the
    /// before/after comparison deterministic.
    #[cfg(unix)]
    #[test]
    fn round_trip_preserves_screen_content_and_scrollback() {
        let mut tree = tree();
        let session = tree.new_session("main", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];
        let quiet = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, Some("sleep 60"))
            .unwrap();

        let terminal = tree.pane(quiet).unwrap().terminal();
        {
            let mut guard = terminal.write();
            for i in 0..40 {
                guard.process(format!("scroll line {i:02}\r\n").as_bytes());
            }
            guard.process(b"ZQX-VISIBLE-TAIL");
        }
        let before = terminal.read().capture_snapshot();
        assert!(
            before.grid.scrollback_lines > 0,
            "test setup must push real content into scrollback"
        );

        let state = tree.to_persist_state();
        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();

        let after = restored
            .pane(quiet)
            .unwrap()
            .terminal()
            .read()
            .capture_snapshot();
        assert_eq!(
            after.grid.scrollback_lines, before.grid.scrollback_lines,
            "scrollback line count survives"
        );
        assert_eq!(
            after.grid.total_lines_scrolled, before.grid.total_lines_scrolled,
            "scroll history survives"
        );
        let scrollback_text: String = after.grid.scrollback_cells.iter().map(|c| c.c).collect();
        assert!(
            scrollback_text.contains("scroll line 05"),
            "scrollback content survives the round trip"
        );
        let screen_text: String = after.grid.cells.iter().map(|c| c.c).collect();
        assert!(
            screen_text.contains("ZQX-VISIBLE-TAIL"),
            "visible content survives the round trip"
        );

        let pane = restored.pane(quiet).unwrap();
        assert!(
            pane.is_running(),
            "the restored pane's process is new, but running"
        );
        assert_eq!(pane.spawn_command(), Some("sleep 60"));
    }

    #[test]
    fn save_lands_at_the_target_with_no_tmp_leftover_and_round_trips() {
        let (_dir, target) = temp_target("save");
        let original = populated_tree();

        save_to(&original, &target).expect("save succeeds");

        // Rename semantics (D3.3): the complete state is AT the target and
        // the tmp sibling is gone — a crash mid-save can only leave the old
        // state or this tmp, never a torn target.
        assert!(target.exists(), "state lands at the target");
        assert!(
            !tmp_sibling(&target).exists(),
            "no .tmp sibling survives a completed save"
        );

        match load_or_quarantine(&target) {
            Loaded::State(state) => {
                assert_eq!(state.format_version, FORMAT_VERSION);
                assert_eq!(state.sessions.len(), 2);
                let restored =
                    MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
                        .expect("saved state restores");
                assert_same_shape(&restored, &original);
            }
            other => panic!("expected a readable state file, got {other:?}"),
        }
    }

    /// Write bytes straight to `target`, creating the parent — the shape a
    /// torn or foreign write leaves on disk (no save_to on the path).
    fn write_raw(target: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(target.parent().expect("temp targets have a parent")).unwrap();
        std::fs::write(target, bytes).unwrap();
    }

    #[test]
    fn torn_write_is_quarantined_and_starts_fresh() {
        let (_dir, target) = temp_target("torn");
        let full = serde_json::to_string(&populated_tree().to_persist_state()).unwrap();
        // Half the bytes made it to "disk" — exactly what a torn
        // non-atomic write leaves behind.
        write_raw(&target, &full.as_bytes()[..full.len() / 2]);

        match load_or_quarantine(&target) {
            Loaded::Quarantined { from, to } => {
                assert_eq!(from, target);
                assert!(to.exists(), "the torn file is preserved aside, not deleted");
                assert!(!target.exists(), "the load path is clear for the next save");
            }
            other => panic!("expected quarantine, got {other:?}"),
        }
    }

    #[test]
    fn unknown_version_state_is_quarantined() {
        let (_dir, target) = temp_target("version");
        let mut state = populated_tree().to_persist_state();
        state.format_version = FORMAT_VERSION + 7;
        write_raw(&target, serde_json::to_string(&state).unwrap().as_bytes());

        assert!(matches!(
            load_or_quarantine(&target),
            Loaded::Quarantined { .. }
        ));
        assert!(!target.exists());
    }

    #[test]
    fn missing_state_file_is_a_quiet_fresh_start() {
        let (_dir, target) = temp_target("missing");
        assert!(matches!(load_or_quarantine(&target), Loaded::Fresh));
        assert!(!target.exists(), "a fresh start must not create anything");
    }

    // --- the last-good snapshot: the shutdown race cannot wipe the layout ---

    /// The reboot/logout race in three deterministic saves: a pane-bearing
    /// structural save, then the reaper persisting a burst of pane deaths
    /// (empty), then the daemon's own final save (still empty). The load
    /// must resurrect the pre-exit layout, not the raced emptiness.
    #[test]
    fn reap_saves_cannot_degrade_the_last_good_snapshot() {
        let (_dir, target) = temp_target("race");
        let populated = populated_tree().to_persist_state();
        let empty = tree().to_persist_state();
        assert!(state_has_panes(&populated) && !state_has_panes(&empty));

        write_job(SaveOrigin::Command, &populated, &target).unwrap();
        write_job(SaveOrigin::Reap, &empty, &target).unwrap();
        write_job(SaveOrigin::Shutdown, &empty, &target).unwrap();

        match load_or_quarantine(&target) {
            Loaded::State(state) => assert!(
                state_has_panes(&state),
                "the pre-exit layout is restored, not the raced emptiness"
            ),
            other => panic!("a readable state file loaded as {other:?}"),
        }
    }

    /// A deliberately emptied tree (kill-pane of the last pane, a Command
    /// save) clears the snapshot: the next start is fresh, not a
    /// resurrection of panes the user closed on purpose.
    #[test]
    fn a_command_empty_save_clears_the_last_good_snapshot() {
        let (_dir, target) = temp_target("deliberate");
        write_job(
            SaveOrigin::Command,
            &populated_tree().to_persist_state(),
            &target,
        )
        .unwrap();
        write_job(SaveOrigin::Command, &tree().to_persist_state(), &target).unwrap();
        match load_or_quarantine(&target) {
            Loaded::State(state) => assert!(
                !state_has_panes(&state),
                "a deliberate empty is the honest state — no resurrection"
            ),
            other => panic!("a readable state file loaded as {other:?}"),
        }
    }

    /// The exit-when-empty daemon's final save is deliberate the same way:
    /// everything closed, no signal raced it, so the snapshot goes and the
    /// next start is fresh. Contrast with the Shutdown arm above, where the
    /// emptiness may be the reboot race and the snapshot must survive.
    #[test]
    fn an_exit_when_empty_final_save_clears_the_last_good_snapshot() {
        let (_dir, target) = temp_target("empty-exit");
        write_job(
            SaveOrigin::Command,
            &populated_tree().to_persist_state(),
            &target,
        )
        .unwrap();
        write_job(
            SaveOrigin::ShutdownEmpty,
            &tree().to_persist_state(),
            &target,
        )
        .unwrap();
        match load_or_quarantine(&target) {
            Loaded::State(state) => assert!(
                !state_has_panes(&state),
                "the empty exit is deliberate — no resurrection"
            ),
            other => panic!("a readable state file loaded as {other:?}"),
        }
    }

    /// ARC-097: exit-when-empty also fires when every pane is dead. That
    /// tree is pane-bearing, so the ShutdownEmpty save refreshes the
    /// snapshot instead of clearing it — the next start respawns those
    /// panes (MUX.md, Pane Reaping).
    #[test]
    fn shutdown_empty_with_dead_panes_refreshes_lastgood() {
        let (_dir, target) = temp_target("all-dead-exit");
        write_job(SaveOrigin::Command, &tree().to_persist_state(), &target).unwrap();
        assert!(
            !lastgood_path(&target).exists(),
            "precondition: no snapshot"
        );

        let dead = populated_tree().to_persist_state();
        assert!(state_has_panes(&dead));
        write_job(SaveOrigin::ShutdownEmpty, &dead, &target).unwrap();
        let snapshot: PersistState =
            serde_json::from_slice(&fs::read(lastgood_path(&target)).unwrap()).unwrap();
        assert!(
            state_has_panes(&snapshot),
            "an all-dead exit keeps its panes for the next start"
        );
    }

    /// ARC-119: the off-lock shutdown save lands the tree's structure,
    /// applies the origin's snapshot rule, and leaves the tree mutex free.
    /// Content parity with the one-shot capture is
    /// `two_phase_capture_matches_one_shot` (`to_persist_state` is the
    /// same two calls); live shells make a byte comparison racy here.
    #[test]
    fn save_off_lock_writes_the_tree_and_releases_the_lock() {
        let (_dir, target) = temp_target("off-lock");
        let tree = parking_lot::Mutex::new(populated_tree());
        save_off_lock(&tree, &target, SaveOrigin::Shutdown).unwrap();
        assert!(
            tree.try_lock().is_some(),
            "the tree mutex is free once the save returns"
        );
        let written: PersistState = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
        let expected = tree.lock().to_persist_state();
        let shape = |state: &PersistState| {
            state
                .sessions
                .iter()
                .map(|s| {
                    let windows: Vec<_> = s
                        .windows
                        .iter()
                        .map(|w| (w.id, w.panes.iter().map(|p| p.id).collect::<Vec<_>>()))
                        .collect();
                    (s.id, s.name.clone(), windows)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(shape(&written), shape(&expected));
        assert_eq!(written.next_ids, expected.next_ids);
        assert_eq!(written.buffers, expected.buffers);
        assert!(
            lastgood_path(&target).exists(),
            "a pane-bearing Shutdown save refreshes the snapshot"
        );
    }

    /// Positive control for the fallback: an empty state with NO snapshot
    /// behind it loads empty — the fallback cannot invent panes.
    #[test]
    fn an_empty_state_without_a_snapshot_loads_empty() {
        let (_dir, target) = temp_target("no-snapshot");
        write_job(SaveOrigin::Shutdown, &tree().to_persist_state(), &target).unwrap();
        match load_or_quarantine(&target) {
            Loaded::State(state) => assert!(!state_has_panes(&state)),
            other => panic!("a readable state file loaded as {other:?}"),
        }
    }

    /// The snapshot is a cache, not evidence: a corrupt one is ignored
    /// (not quarantined), and the empty main state loads as itself.
    #[test]
    fn a_corrupt_lastgood_is_ignored() {
        let (_dir, target) = temp_target("corrupt-good");
        write_job(
            SaveOrigin::Command,
            &populated_tree().to_persist_state(),
            &target,
        )
        .unwrap();
        let lastgood = lastgood_path(&target);
        assert!(lastgood.exists(), "a pane-bearing save wrote the snapshot");
        std::fs::write(&lastgood, b"not json").unwrap();
        write_job(SaveOrigin::Reap, &tree().to_persist_state(), &target).unwrap();
        match load_or_quarantine(&target) {
            Loaded::State(state) => assert!(!state_has_panes(&state)),
            other => panic!("a readable state file loaded as {other:?}"),
        }
    }

    #[test]
    fn state_files_are_keyed_by_socket_stem_under_the_par_mux_dir() {
        // default_socket_path produces `<base>/par-mux-<name>.sock`, so the
        // stem that keys the state file is `par-mux-<name>` in full.
        let base = Path::new("/base");
        let a = state_file_in(base, Path::new("/tmp/par-mux-alpha.sock"));
        let b = state_file_in(base, Path::new("/tmp/par-mux-beta.sock"));
        assert!(a.ends_with("par-mux/par-mux-alpha.state.json"));
        assert!(b.ends_with("par-mux/par-mux-beta.state.json"));
        assert_ne!(a, b, "two servers on different sockets never share state");
    }

    /// One pane carrying the given metadata keys — the shared base for the
    /// agent-identity tests.
    fn tree_with_metadata(metadata: &[(&str, &str)]) -> (MuxTree, PaneId) {
        let mut tree = tree();
        let session = tree.new_session("agents", 80, 24).unwrap();
        let pane_id = tree
            .session(session)
            .unwrap()
            .windows
            .iter()
            .filter_map(|window| tree.window(*window))
            .flat_map(|window| window.panes())
            .next()
            .unwrap();
        let pane = tree.pane_mut(pane_id).unwrap();
        for (key, value) in metadata {
            pane.set_metadata(key, value);
        }
        (tree, pane_id)
    }

    /// One pane whose metadata carries the full hook-reported identity.
    fn tree_with_agent_pane() -> (MuxTree, PaneId) {
        let (mut tree, pane_id) = tree_with_metadata(&[
            ("agent", "pi"),
            ("agent_session_id", "s-1"),
            ("agent_session_path", "/tmp/pi-session.jsonl"),
            ("agent_source", "par-mux:pi"),
            (
                "agent_resume_argv",
                r#"["pi","--session","/tmp/pi-session.jsonl"]"#,
            ),
            // Keys that must NOT travel: state-shaped and
            // provenance-of-start.
            ("agent_state", "working"),
            ("agent_state_source", "hook"),
            ("agent_seq", "1000"),
            ("agent_session_start_source", "startup"),
        ]);
        // Nor the typed, display-only half of the claim (ARC-113).
        let pane = tree.pane_mut(pane_id).unwrap();
        pane.telemetry = Some(crate::mux::hooks::StoredTelemetry {
            sampled_at_unix_ms: 1,
            canonical_b64: "e30=".to_string(),
        });
        pane.seq_by_source.insert("par-mux:pi".to_string(), 1000);
        (tree, pane_id)
    }

    /// The resume spawn re-lands in the pane's persisted cwd: the agent's
    /// invocation (claude resolves transcripts per cwd) must run where the
    /// session lived, not in the daemon's start directory. Unix only: the
    /// persisted cwd is seeded through a `/`-rooted OSC 7 URL.
    #[test]
    #[cfg(unix)]
    fn a_resume_invocation_spawns_in_the_persisted_cwd() {
        let (tree, pane_id) = tree_with_agent_pane();
        let dir = tempfile::tempdir().unwrap();
        let reported = dir.path().join("repo");
        std::fs::create_dir(&reported).unwrap();
        tree.pane(pane_id)
            .unwrap()
            .terminal()
            .write()
            .process(format!("\x1b]7;file://localhost{}\x1b\\", reported.display()).as_bytes());
        let state = tree.to_persist_state();

        let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        let spawn = factory.spawn_of(pane_id);
        assert_eq!(
            spawn.cwd.as_deref(),
            Some(reported.as_path()),
            "the resume spawn carries the persisted cwd"
        );
        assert!(
            restored.pane(pane_id).is_some()
                && state.sessions[0].windows[0].panes[0]
                    .agent_session
                    .is_some(),
            "positive control: this pane restored through the resume path"
        );
    }

    /// A released pane restores through its ORIGINAL command, not the
    /// resume invocation: the release cleared the session identity, so
    /// persistence has nothing to resume and Phase 3 behavior resumes.
    #[test]
    fn a_released_pane_restores_through_the_original_command() {
        let (tree, pane_id) = tree_with_agent_pane();
        let tree = std::sync::Arc::new(parking_lot::Mutex::new(tree));
        let release = format!(
            r#"{{"id":"t-9","method":"pane.release_agent","params":{{"pane_id":"{pane_id}","agent":"pi","seq":2000,"source":"par-mux:test"}}}}"#
        );
        let (reply, _) = crate::mux::hooks::handle_report(&release, &tree);
        assert!(reply.contains(r#""result":"ok""#), "released: {reply}");
        let tree = std::sync::Arc::into_inner(tree)
            .expect("the test holds the only reference")
            .into_inner();

        let state = tree.to_persist_state();
        assert!(
            state.sessions[0].windows[0].panes[0]
                .agent_session
                .is_none(),
            "the release cleared what the resume path needs"
        );

        let factory = RecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        assert_eq!(
            factory.command_for(pane_id),
            None,
            "the pane respawns its original command (the default shell), not a resume"
        );
        assert!(restored.pane(pane_id).is_some());
    }

    #[test]
    fn agent_session_identity_round_trips() {
        let (original, pane_id) = tree_with_agent_pane();
        let state = original.to_persist_state();
        let persisted = state.sessions[0].windows[0].panes[0]
            .agent_session
            .as_ref()
            .expect("the agent pane carries its identity");
        assert_eq!(
            persisted,
            &PersistAgentSession {
                agent: "pi".to_string(),
                session_id: Some("s-1".to_string()),
                session_path: Some("/tmp/pi-session.jsonl".to_string()),
                source: Some("par-mux:pi".to_string()),
                resume_argv: Some(r#"["pi","--session","/tmp/pi-session.jsonl"]"#.to_string()),
            },
            "capture reads exactly the identity keys, nothing state-shaped"
        );

        // Restore writes the identity back as metadata, so a re-capture of
        // the restored tree holds the same identity — the format round-trip.
        // The recording factory keeps the real pi CLI out of the test while
        // proving what restore actually spawned (task 6.3): the reported
        // invocation, verbatim.
        let factory = RecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        let expected = if cfg!(windows) {
            // The POSIX surviving tail cannot cross cmd.exe; on Windows the
            // recorded spawn is the bare invocation (the real argv path is
            // covered by a_windows_resume_receives_the_exact_arguments).
            "'pi' '--session' '/tmp/pi-session.jsonl'".to_string()
        } else {
            format!("'pi' '--session' '/tmp/pi-session.jsonl'{SURVIVING_TAIL}")
        };
        assert_eq!(
            factory.command_for(pane_id).as_deref(),
            Some(expected.as_str()),
            "restore spawns the hook-reported invocation, not the original command"
        );
        let recaptured = restored.to_persist_state();
        assert_eq!(
            recaptured.sessions[0].windows[0].panes[0].agent_session,
            state.sessions[0].windows[0].panes[0].agent_session,
            "identity survives tree -> format -> tree -> format"
        );

        // The restored pane holds identity metadata and none of the
        // state-shaped keys — the post-restart roster stays empty until
        // the agent reports again.
        let pane = restored.pane(pane_id).unwrap();
        assert_eq!(pane.metadata().get("agent").map(String::as_str), Some("pi"));
        assert!(!pane.metadata().contains_key("agent_state"));
        assert!(!pane.metadata().contains_key("agent_seq"));
        assert!(
            !pane.metadata().contains_key("agent_session_start_source"),
            "start source describes the PREVIOUS process's start — stale after a restart"
        );
        assert!(
            pane.telemetry.is_none(),
            "telemetry is display-only and ephemeral — a restored pane serves absent, never a stale sample"
        );
        assert!(
            pane.seq_by_source.is_empty(),
            "sequence stamps are volatile — the restarted agent's first report is never stale"
        );
    }

    /// ARC-113c: a PRE-CHANGE state file still loads. The on-disk identity
    /// shape is unchanged by the typed claim — the fixture hand-writes the
    /// legacy `agent_session` block (argv as a verbatim JSON string,
    /// nothing state-shaped) into an otherwise captured state, and the
    /// restored pane's claim reads back through the typed view.
    #[test]
    fn a_pre_change_agent_identity_json_loads_and_reads_typed() {
        let (original, pane_id) = tree_with_agent_pane();
        let mut value = serde_json::to_value(original.to_persist_state()).unwrap();
        value["sessions"][0]["windows"][0]["panes"][0]["agent_session"] = serde_json::json!({
            "agent": "pi",
            "session_id": "s-1",
            "session_path": "/tmp/pi-session.jsonl",
            "source": "par-mux:pi",
            "resume_argv": r#"["pi","--session","/tmp/pi-session.jsonl"]"#
        });
        let json = serde_json::to_string(&value).unwrap();
        assert!(
            json.contains(r#""resume_argv":"[\"pi\",\"--session\",\"/tmp/pi-session.jsonl\"]""#),
            "the on-disk shape must keep argv a JSON string, not an array: {json}"
        );

        let state: PersistState = serde_json::from_str(&json).unwrap();
        let factory = RecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        let pane = restored.pane(pane_id).unwrap();
        assert_eq!(pane.metadata().get("agent").map(String::as_str), Some("pi"));
        assert_eq!(
            pane.metadata().get("agent_session_id").map(String::as_str),
            Some("s-1")
        );
        assert_eq!(
            pane.metadata()
                .get("agent_session_path")
                .map(String::as_str),
            Some("/tmp/pi-session.jsonl")
        );
        assert_eq!(
            pane.metadata().get("agent_source").map(String::as_str),
            Some("par-mux:pi")
        );
        assert_eq!(
            pane.metadata().get("agent_resume_argv").map(String::as_str),
            Some(r#"["pi","--session","/tmp/pi-session.jsonl"]"#),
            "a valid argv lands byte-identical, not normalized"
        );

        let claim = pane.agent_claim().expect("restored identity is a claim");
        assert_eq!(claim.agent, "pi");
        assert_eq!(claim.session_id.as_deref(), Some("s-1"));
        assert_eq!(claim.session_path.as_deref(), Some("/tmp/pi-session.jsonl"));
        assert_eq!(claim.source.as_deref(), Some("par-mux:pi"));
        assert_eq!(
            claim.resume_argv.as_deref(),
            Some(
                &[
                    "pi".to_string(),
                    "--session".to_string(),
                    "/tmp/pi-session.jsonl".to_string()
                ][..]
            )
        );
        assert_eq!(
            claim.state, None,
            "the old file carries nothing state-shaped"
        );
        assert_eq!(claim.seq, None);
        assert_eq!(claim.session_start_source, None);
    }

    /// The pi/omp shape on the wire today: path-only identity plus the
    /// reported invocation, no session id at all.
    #[test]
    fn pi_shaped_path_only_identity_round_trips() {
        let mut tree = tree();
        let session = tree.new_session("agents", 80, 24).unwrap();
        let pane_id = tree
            .session(session)
            .unwrap()
            .windows
            .iter()
            .filter_map(|window| tree.window(*window))
            .flat_map(|window| window.panes())
            .next()
            .unwrap();
        let pane = tree.pane_mut(pane_id).unwrap();
        pane.set_metadata("agent", "omp");
        pane.set_metadata("agent_session_path", "/tmp/omp-session.jsonl");
        pane.set_metadata(
            "agent_resume_argv",
            r#"["omp","--resume=/tmp/omp-session.jsonl"]"#,
        );

        let state = tree.to_persist_state();
        assert_eq!(
            state.sessions[0].windows[0].panes[0].agent_session,
            Some(PersistAgentSession {
                agent: "omp".to_string(),
                session_id: None,
                session_path: Some("/tmp/omp-session.jsonl".to_string()),
                source: None,
                resume_argv: Some(r#"["omp","--resume=/tmp/omp-session.jsonl"]"#.to_string()),
            }),
            "a path-only ref is identity enough — the id-OR-path wire contract"
        );

        let factory = RecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        assert!(
            restored
                .pane(pane_id)
                .unwrap()
                .metadata()
                .contains_key("agent_resume_argv"),
            "the override 6.2 reads survives the restart"
        );
        let expected = if cfg!(windows) {
            "'omp' '--resume=/tmp/omp-session.jsonl'".to_string()
        } else {
            format!("'omp' '--resume=/tmp/omp-session.jsonl'{SURVIVING_TAIL}")
        };
        assert_eq!(
            factory.command_for(pane_id).as_deref(),
            Some(expected.as_str()),
            "restore spawns the reported invocation for the path-only shape too"
        );
    }

    /// Task 6.3: an agent with NO reported invocation gets the table's argv
    /// through the same restore chain.
    #[test]
    fn restore_spawns_a_table_resume_for_an_agent_without_a_reported_invocation() {
        let (original, pane_id) =
            tree_with_metadata(&[("agent", "claude"), ("agent_session_id", "abc-123")]);
        let state = original.to_persist_state();
        let factory = RecordingFactory::default();
        let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        let expected = if cfg!(windows) {
            "'claude' '--resume' 'abc-123'".to_string()
        } else {
            format!("'claude' '--resume' 'abc-123'{SURVIVING_TAIL}")
        };
        assert_eq!(
            factory.command_for(pane_id).as_deref(),
            Some(expected.as_str()),
            "the table builds the invocation when nothing was reported"
        );
        assert_eq!(
            restored
                .pane(pane_id)
                .unwrap()
                .metadata()
                .get("agent")
                .map(String::as_str),
            Some("claude"),
            "identity still lands as metadata for the next cycle"
        );
    }

    /// Task 6.3 degradation 1: an agent with no table entry and no report
    /// falls back to the pane's original spawn command.
    #[test]
    fn an_agent_with_no_table_entry_and_no_report_falls_back_to_the_original_command() {
        let mut tree = tree();
        let session = tree.new_session("agents", 80, 24).unwrap();
        let main_window = tree.session(session).unwrap().windows[0];
        let first = tree.window(main_window).unwrap().panes()[0];
        let pane_id = tree
            .split_pane(first, SplitDirection::Vertical, 0.25, Some("sleep 60"))
            .unwrap();
        let pane = tree.pane_mut(pane_id).unwrap();
        pane.set_metadata("agent", "kimi");
        pane.set_metadata("agent_session_id", "kimi-arc-1");

        let state = tree.to_persist_state();
        // Real factory: the fallback command is the original sleep, safe to
        // actually spawn.
        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        assert_eq!(
            restored.pane(pane_id).unwrap().spawn_command(),
            Some("sleep 60"),
            "no table entry means fresh-spawn behavior, not an error"
        );
    }

    /// Task 6.3 degradation 2: a session ref the table cannot use (a path
    /// for an id-only agent) falls back the same way.
    #[test]
    fn a_ref_the_table_cannot_use_falls_back_to_the_original_command() {
        let (original, pane_id) =
            tree_with_metadata(&[("agent", "claude"), ("agent_session_path", "/tmp/s.jsonl")]);
        let state = original.to_persist_state();
        let factory = RecordingFactory::default();
        MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
        assert_eq!(
            factory.command_for(pane_id),
            None,
            "claude cannot resume from a path — the pane restores as it was"
        );
    }

    /// Task 6.3 degradation 3: an agent binary that no longer exists. No
    /// probe detects the absence at restore time — the invocation spawns
    /// anyway, carrying the surviving tail, so the runtime failure lands
    /// in the pane (shell error message, fallback shell) and not in the
    /// restore chain.
    #[test]
    fn an_absent_agent_binary_degrades_inside_the_pane_not_the_restore() {
        let (original, pane_id) = tree_with_metadata(&[
            ("agent", "pi"),
            ("agent_session_id", "s-1"),
            (
                "agent_resume_argv",
                r#"["par-mux-test-no-such-binary","--resume","s-1"]"#,
            ),
        ]);
        let state = original.to_persist_state();
        let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
            .expect("restore completes — an absent binary is the pane's problem");
        let expected = if cfg!(windows) {
            "'par-mux-test-no-such-binary' '--resume' 's-1'".to_string()
        } else {
            format!("'par-mux-test-no-such-binary' '--resume' 's-1'{SURVIVING_TAIL}")
        };
        assert_eq!(
            restored.pane(pane_id).unwrap().spawn_command(),
            Some(expected.as_str()),
            "no probe swapped in a fallback — the failure is contained in the pane"
        );
    }

    /// The D6.3 promise behind degradation 3: a restored agent pane whose
    /// resume invocation exits non-zero must stay ALIVE — the reaper never
    /// gets a dead pane, so the restored screen, scrollback, and agent
    /// identity survive for a retry or an explicit `kill-pane`. Runs the
    /// real spawn path (`sh -c` with the missing binary), unix only.
    #[test]
    #[cfg(unix)]
    fn a_failed_resume_leaves_a_live_pane_with_its_restored_history() {
        let (tree, pane_id) = tree_with_metadata(&[
            ("agent", "pi"),
            ("agent_session_id", "s-1"),
            (
                "agent_resume_argv",
                r#"["par-mux-test-no-such-binary","--resume","s-1"]"#,
            ),
        ]);
        // Markers written straight into the terminal (not through the
        // child): 40 lines of scrollback plus on-screen content that the
        // restore must still carry after the resume fails.
        {
            let terminal = tree.pane(pane_id).unwrap().terminal();
            let mut guard = terminal.write();
            for i in 0..40u16 {
                guard.process(format!("PRE-MARK-{:02}\r\n", i).as_bytes());
            }
            guard.process(b"ON-SCREEN-MARK\r\n");
        }
        let state = tree.to_persist_state();

        // Restore through the REAL factory: the pane's process is
        // `sh -c '<missing binary> ... || { ...; exec shell }'`. sh fails
        // near-instantly (127); the tail decides whether the pane survives.
        let mut restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
                .expect("restore completes");

        // Give the failed exec far longer than `sh` needs to die, requiring
        // the pane running throughout — pre-fix, poll_running() flips false
        // within the first poll and the daemon-side reaper would have
        // persisted the pane's deletion by now. Poll for the fallback's
        // marker instead of sleeping a fixed span: under a parallel test
        // run's process-spawn load, the printf can take seconds to land
        // (QA-133 measured 20/20 solo passes at the old fixed 3s, and a
        // ~1-in-20 failure only under the parallel `mux::` filter). A
        // marker that never arrives still fails below.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut text = String::new();
        loop {
            assert!(
                restored.pane_mut(pane_id).unwrap().poll_running(),
                "the pane must survive its failed resume"
            );
            let snapshot = restored
                .pane(pane_id)
                .unwrap()
                .terminal()
                .read()
                .capture_snapshot();
            text.clear();
            text.extend(snapshot.grid.scrollback_cells.iter().map(|c| c.c));
            text.extend(snapshot.grid.cells.iter().map(|c| c.c));
            if (text.contains("PRE-MARK-00")
                && text.contains("PRE-MARK-39")
                && text.contains("par-mux: agent resume failed"))
                || std::time::Instant::now() >= deadline
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }

        // The restored history and the fallback's own trace are both still
        // readable (screen or scrollback — the shell's startup bytes may
        // have scrolled the markers up).
        assert!(
            text.contains("PRE-MARK-00") && text.contains("PRE-MARK-39"),
            "restored scrollback survived the failed resume"
        );
        assert!(
            text.contains("par-mux: agent resume failed"),
            "the fallback announced itself in the pane"
        );

        // The agent identity survives the next persist — only an explicit
        // kill-pane (or kill-window) removes it.
        let recaptured = restored.to_persist_state();
        let session = recaptured.sessions[0].windows[0].panes[0]
            .agent_session
            .as_ref()
            .expect("agent identity survived the failed resume");
        assert_eq!(session.agent, "pi");
        assert_eq!(
            session.resume_argv.as_deref(),
            Some(r#"["par-mux-test-no-such-binary","--resume","s-1"]"#)
        );
    }

    /// Card 01a0d9b38c98: a Windows restore hands the resume argv to the
    /// process verbatim, with no cmd.exe string re-parse in between (the
    /// POSIX single-quote rendering made cmd treat `'claude'` as the
    /// program name, so every resume failed). The observer is
    /// powershell.exe — a PE, so the direct transport — printing each
    /// argument between markers; a session path carrying a space, an `&`,
    /// and a quote must arrive byte-identical.
    #[test]
    #[cfg(windows)]
    fn a_windows_resume_receives_the_exact_arguments() {
        let script = std::env::temp_dir().join("par-mux-resume-observer.ps1");
        std::fs::write(
            &script,
            "foreach ($a in $args) { Write-Output \"ARG<$a>\" }\r\n",
        )
        .unwrap();
        let tricky = "C:\\my sessions\\s 1 & continue's.txt";
        // Serialize with serde_json — hand-splicing Windows paths into a
        // JSON literal produces invalid \-escapes, which the resume chain
        // would (correctly) reject back to the table.
        let argv = serde_json::to_string(&vec![
            "powershell.exe".to_string(),
            "-NoProfile".to_string(),
            // The default policy on a stock Windows blocks .ps1 files.
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script.display().to_string(),
            tricky.to_string(),
            "plain".to_string(),
        ])
        .unwrap();
        let (tree, pane_id) = tree_with_metadata(&[
            ("agent", "pi"),
            ("agent_session_id", "s-1"),
            ("agent_resume_argv", &argv),
        ]);
        let state = tree.to_persist_state();
        let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
            .expect("restore completes");

        // powershell's first start under a fresh ConPTY can take seconds.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
        let text = loop {
            let snapshot = restored
                .pane(pane_id)
                .unwrap()
                .terminal()
                .read()
                .capture_snapshot();
            let mut text: String = snapshot.grid.scrollback_cells.iter().map(|c| c.c).collect();
            text.extend(snapshot.grid.cells.iter().map(|c| c.c));
            if text.contains("ARG<plain>") || std::time::Instant::now() >= deadline {
                break text;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        };
        assert!(
            text.contains(&format!("ARG<{tricky}>")),
            "the tricky argument arrived byte-identical, saw: {text:?}"
        );
        assert!(
            text.contains("ARG<plain>"),
            "every argument arrived, saw: {text:?}"
        );
        let _ = std::fs::remove_file(&script);
    }

    /// Task 6.3 criterion 3: a non-agent pane restores byte-identically to
    /// Phase 3 — same spawn command through the real factory, and the
    /// restore invents no agent session.
    #[test]
    fn non_agent_panes_restore_through_the_unchanged_phase3_path() {
        let original = populated_tree();
        let state = original.to_persist_state();
        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        assert_same_shape(&restored, &original);
        let recaptured = restored.to_persist_state();
        assert!(recaptured
            .sessions
            .iter()
            .flat_map(|session| session.windows.iter())
            .flat_map(|window| window.panes.iter())
            .all(|pane| pane.agent_session.is_none()));
    }

    #[test]
    fn panes_without_resumable_identity_serialize_agent_session_none() {
        // A non-agent pane…
        let bare = populated_tree();
        let state = bare.to_persist_state();
        let persisted_panes: Vec<&PersistPane> = state
            .sessions
            .iter()
            .flat_map(|s| s.windows.iter().flat_map(|w| w.panes.iter()))
            .collect();
        assert!(!persisted_panes.is_empty());
        assert!(
            persisted_panes
                .iter()
                .all(|pane| pane.agent_session.is_none()),
            "no metadata, no agent_session"
        );

        // …and a pane hook-claimed for STATE but carrying no session —
        // an agent label alone is nothing the resume path can use.
        let mut tree = tree();
        let session = tree.new_session("agents", 80, 24).unwrap();
        let pane_id = tree
            .session(session)
            .unwrap()
            .windows
            .iter()
            .filter_map(|window| tree.window(*window))
            .flat_map(|window| window.panes())
            .next()
            .unwrap();
        tree.pane_mut(pane_id)
            .unwrap()
            .set_metadata("agent", "kimi");
        let state = tree.to_persist_state();
        assert_eq!(
            state.sessions[0].windows[0].panes[0].agent_session, None,
            "label-only panes carry nothing to resume"
        );
    }

    /// The version this build just superseded is refused exactly like any
    /// unknown one — the bump must be a boundary, not a silent partial read.
    #[test]
    fn previous_format_version_is_refused() {
        let original = populated_tree();
        let mut state = original.to_persist_state();
        state.format_version = FORMAT_VERSION - 1;

        let result = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()));
        assert!(
            matches!(
                result,
                Err(PersistError::UnsupportedVersion { found, supported })
                    if found == FORMAT_VERSION - 1 && supported == FORMAT_VERSION
            ),
            "a v1 file must be refused, not partially read as v2"
        );
    }
}

/// The envelope must survive serialize → deserialize whole — LayoutTree and
/// the id newtypes serialize inside it (D3.2), which no other test exercises.
#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::tests::{assert_same_shape, populated_tree, tree};
    use super::*;
    use crate::mux::pane::ShellPaneFactory;

    #[test]
    fn envelope_survives_serde_round_trip() {
        let original = populated_tree();
        let state = original.to_persist_state();

        let json = serde_json::to_string(&state).expect("envelope serializes");
        let revived: PersistState = serde_json::from_str(&json).expect("envelope deserializes");
        assert_eq!(revived.format_version, FORMAT_VERSION);
        assert_eq!(revived.next_ids, state.next_ids);

        // Restoring through the revived envelope must land in the same
        // state as restoring through the original, byte for byte in shape.
        let direct =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        let via_json = MuxTree::from_persist_state(&revived, Box::new(ShellPaneFactory::default()))
            .expect("revived envelope restores");
        assert_same_shape(&via_json, &direct);
        assert_eq!(
            via_json.get_buffer("default").map(str::to_string),
            Some("hello".to_string())
        );
    }

    /// The skip-when-absent rule that keeps non-agent panes byte-identical
    /// to the pre-6.1 format: the serialized envelope of a metadata-less
    /// tree never contains the new key.
    #[test]
    fn non_agent_envelope_carries_no_agent_session_key() {
        let json = serde_json::to_string(&populated_tree().to_persist_state())
            .expect("envelope serializes");
        assert!(
            !json.contains("agent_session"),
            "the new field must skip when absent: {json}"
        );

        // And when present, it round-trips through the JSON itself — the
        // wire form of the identity, not just the in-memory struct.
        let mut tree = tree();
        let session = tree.new_session("agents", 80, 24).unwrap();
        let pane_id = tree
            .session(session)
            .unwrap()
            .windows
            .iter()
            .filter_map(|window| tree.window(*window))
            .flat_map(|window| window.panes())
            .next()
            .unwrap();
        let pane = tree.pane_mut(pane_id).unwrap();
        pane.set_metadata("agent", "pi");
        pane.set_metadata("agent_session_path", "/tmp/pi.jsonl");
        pane.set_metadata("agent_resume_argv", r#"["pi","--session","/tmp/pi.jsonl"]"#);

        let json = serde_json::to_string(&tree.to_persist_state()).expect("serializes");
        let revived: PersistState = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(
            revived.sessions[0].windows[0].panes[0].agent_session,
            Some(PersistAgentSession {
                agent: "pi".to_string(),
                session_id: None,
                session_path: Some("/tmp/pi.jsonl".to_string()),
                source: None,
                resume_argv: Some(r#"["pi","--session","/tmp/pi.jsonl"]"#.to_string()),
            }),
            "identity survives the JSON wire form"
        );
    }

    /// A grid snapshot whose scrollback ring holds `lines` logical lines of
    /// `cols` cells: logical line i is filled with the marker byte for i,
    /// laid out through the live logical→physical mapping so a rotated
    /// `start` exercises the extraction, and every 3rd logical line is
    /// flagged wrapped.
    fn ring_snapshot(lines: usize, cols: usize, start: usize, max: usize) -> GridSnapshot {
        // Snapshots carry the flat payload OLDEST-FIRST (one line per
        // `cols`, `scrollback_start` field removed with the per-line
        // storage). `start` and `max` are kept in the signature for the
        // cap-depth math of the callers.
        let _ = start;
        let physical_lines = lines.min(max);
        let mut cells = vec![crate::cell::Cell::default(); physical_lines * cols];
        let mut wrapped = vec![false; physical_lines];
        for logical in 0..lines.min(physical_lines) {
            for c in 0..cols {
                cells[logical * cols + c] = marker_cell(logical);
            }
            wrapped[logical] = logical % 3 == 0;
        }
        GridSnapshot {
            cells: Vec::new(),
            scrollback_cells: cells,
            scrollback_lines: physical_lines,
            max_scrollback: max,
            cols,
            rows: 24,
            wrapped: Vec::new(),
            scrollback_wrapped: wrapped,
            zones: Vec::new(),
            total_lines_scrolled: lines,
        }
    }

    /// A default cell carrying one marker character, so extracted rows are
    /// identifiable by their logical line.
    fn marker_cell(line: usize) -> crate::cell::Cell {
        let mut cell = crate::cell::Cell::default();
        cell.c = char::from_u32((line % 10) as u32 + '0' as u32).unwrap_or('0');
        cell
    }

    /// The marker of the first logical line retained after capping
    /// `lines` down to the const's line budget.
    fn first_kept_marker(lines: usize, cols: usize) -> char {
        let keep = (MAX_PERSISTED_SCROLLBACK_CELLS / cols).min(lines);
        marker_cell(lines - keep).c
    }

    #[test]
    fn cap_scrollback_drops_oldest_lines_from_a_linear_ring() {
        let cols = 80;
        let lines = MAX_PERSISTED_SCROLLBACK_CELLS / cols + 250;
        let keep = MAX_PERSISTED_SCROLLBACK_CELLS / cols;
        let grid = cap_grid_scrollback(ring_snapshot(lines, cols, 0, 10_000));

        assert_eq!(grid.scrollback_lines, keep);
        assert_eq!(grid.scrollback_cells.len(), keep * cols);
        assert_eq!(grid.scrollback_wrapped.len(), keep);
        assert_eq!(grid.max_scrollback, 10_000, "restored pane keeps its depth");
        assert_eq!(grid.total_lines_scrolled, lines, "absolute frame is kept");
        assert_eq!(
            grid.scrollback_cells[0].c,
            first_kept_marker(lines, cols),
            "the oldest retained line leads the ring"
        );
        // Wrapped flags follow their logical lines: the first retained
        // logical line is `lines - keep`, flagged wrapped iff divisible by 3.
        let first_retained = lines - keep;
        assert_eq!(grid.scrollback_wrapped[0], first_retained.is_multiple_of(3));
    }

    #[test]
    fn cap_scrollback_follows_a_rotated_ring() {
        let cols = 80;
        let keep = MAX_PERSISTED_SCROLLBACK_CELLS / cols;
        let max = 2_000;
        let start = 737;
        let grid = cap_grid_scrollback(ring_snapshot(max, cols, start, max));

        assert_eq!(grid.scrollback_lines, keep);
        // Every retained slot must carry the marker of its logical line:
        // slot j holds logical line (max - keep + j).
        for (slot, expected_logical) in (max - keep..max).enumerate() {
            assert_eq!(
                grid.scrollback_cells[slot * cols].c,
                marker_cell(expected_logical).c,
                "slot {slot} must hold logical line {expected_logical}"
            );
        }
    }

    #[test]
    fn cap_scrollback_evicts_and_clamps_zones_at_the_new_floor() {
        let cols = 80;
        let keep = MAX_PERSISTED_SCROLLBACK_CELLS / cols;
        let lines = keep + 750;
        let mut snapshot = ring_snapshot(lines, cols, 0, 10_000);
        let floor = lines - keep;
        let mut old = crate::zone::Zone::new(1, crate::zone::ZoneType::Output, 0, None);
        old.abs_row_end = floor.saturating_sub(1);
        let mut straddling =
            crate::zone::Zone::new(2, crate::zone::ZoneType::Output, floor - 10, None);
        straddling.abs_row_end = floor + 10;
        let mut recent = crate::zone::Zone::new(3, crate::zone::ZoneType::Output, floor + 5, None);
        recent.abs_row_end = floor + 20;
        snapshot.zones = vec![old, straddling, recent];

        let grid = cap_grid_scrollback(snapshot);
        let ids: Vec<usize> = grid.zones.iter().map(|z| z.id).collect();
        assert_eq!(ids, vec![2, 3], "the zone wholly below the floor is gone");
        assert_eq!(grid.zones[0].abs_row_start, floor, "straddler clamps");
        assert_eq!(grid.zones[1].abs_row_start, floor + 5, "recent zone intact");
    }

    #[test]
    fn cap_scrollback_leaves_a_small_ring_untouched() {
        let cols = 80;
        let lines = 40;
        let snapshot = ring_snapshot(lines, cols, 0, 10_000);
        let cells = snapshot.scrollback_cells.clone();
        let grid = cap_grid_scrollback(snapshot);
        assert_eq!(grid.scrollback_lines, lines);
        assert_eq!(grid.scrollback_cells, cells, "under the cap, nothing moves");
    }

    /// The capture path applies the cap: a pane whose scrollback exceeds
    /// the budget persists only the newest lines, while its in-memory
    /// snapshot cache keeps the full history. The content pane runs
    /// `sleep` so its process emits nothing beyond the fed lines.
    #[cfg(unix)]
    #[test]
    fn persisted_state_caps_scrollback_to_the_newest_lines() {
        use crate::mux::SplitDirection;

        let mut tree = tree();
        let session = tree.new_session("main", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];
        let quiet = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, Some("sleep 60"))
            .unwrap();
        let terminal = tree.pane(quiet).unwrap().terminal();
        // The split pane's width is whatever the layout gave it; the line
        // budget is derived from the actual pane, not assumed.
        let (cols, _rows) = terminal.read().size();
        let keep = MAX_PERSISTED_SCROLLBACK_CELLS / cols;
        {
            let mut guard = terminal.write();
            for i in 0..keep + 100 {
                guard.process(format!("L{i:06}\r\n").as_bytes());
            }
        }
        let state = tree.to_persist_state();
        // Panes serialize in layout order; the split pane is the second.
        let pane = &state.sessions[0].windows[0].panes[1];
        assert_eq!(
            pane.terminal.grid.scrollback_lines, keep,
            "persisted scrollback is capped to the budget"
        );
        let persisted_text: String = pane
            .terminal
            .grid
            .scrollback_cells
            .iter()
            .map(|c| c.c)
            .collect();
        // The last `rows` fed lines sit on the visible screen; keep+50 is
        // well inside the retained window, L000000 well below it.
        assert!(
            persisted_text.contains(&format!("L{:06}", keep + 50)),
            "the retained window is the newest content"
        );
        assert!(
            !persisted_text.contains("L000000"),
            "the oldest lines are dropped from the persisted form"
        );
        // The pane's in-memory cache is uncapped: the next capture still
        // sees the full history.
        let full = tree.pane(quiet).unwrap().persisted_snapshot();
        assert!(
            full.grid.scrollback_lines > keep,
            "the in-memory snapshot keeps the pane's full history ({})",
            full.grid.scrollback_lines
        );
    }
}
