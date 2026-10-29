//! Saving: the two-phase tree capture (ARC-032), the scrollback cap, and
//! the atomic write with its last-good and snapshot-history tiers.

use super::format::{
    unix_ms, PersistAgentSession, PersistError, PersistPane, PersistSession, PersistState,
    PersistWindow, PersistWorkspace, PersistWorktreeLink, FORMAT_VERSION,
    MAX_PERSISTED_SCROLLBACK_CELLS,
};
use crate::mux::ids::{PaneId, SessionId, WindowId, WorkspaceId};
use crate::mux::layout::LayoutTree;
use crate::mux::pane::{snapshot_from_parts, PaneSnapshotParts};
use crate::mux::tree::MuxTree;
use par_term_emu_core::terminal::replay_snapshot::{GridSnapshot, TerminalSnapshot};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
        state: claim.state,
        state_source: claim.state_source,
        message: claim.message,
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
    respawn_last: bool,
    exit_action: Option<crate::mux::config::ExitAction>,
    worktree: Option<crate::mux::tree::WorktreeLink>,
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
                respawn_last: workspace.respawn_last,
                exit_action: workspace.exit_action.map(|a| a.name().to_string()),
                worktree: workspace.worktree.as_ref().map(PersistWorktreeLink::from),
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
                    respawn_last: workspace.respawn_last,
                    exit_action: workspace.exit_action,
                    worktree: workspace.worktree.clone(),
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
pub(super) fn cap_grid_scrollback(mut grid: GridSnapshot) -> GridSnapshot {
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
    /// The final save of an exit-when-empty daemon: zero sessions or every
    /// pane dead (attached clients do not count), held past the grace with no
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
#[deprecated(
    since = "0.58.1",
    note = "holds the tree lock across fsync; use save_off_lock (ARC-119)"
)]
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
        // A state dir deleted while the daemon ran must not be resurrected
        // by the final save (the measured case: a script's
        // `--stop; rm -rf state; start` — the --stop reply lands ~2.5 s
        // before the process exits, and the save-on-exit used to re-create
        // the deleted dir inside that window). A shutdown-family save
        // whose target dir is gone respects the deletion; earlier saves of
        // a live daemon keep creating the dir (the daemon owns its state
        // until the exit), and a cold start's first save creates it as
        // always.
        if matches!(origin, SaveOrigin::Shutdown | SaveOrigin::ShutdownEmpty) && !parent.exists() {
            par_term_emu_core::debug_error!(
                "MUX",
                "state dir deleted while running; shutdown save skipped: {}",
                parent.display()
            );
            return Ok(());
        }
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
    maintain_snapshot_history(origin, state, target);
    Ok(())
}

/// The snapshot sibling a pane-bearing save keeps beside the state file.
pub(super) fn lastgood_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(".lastgood");
    PathBuf::from(name)
}

/// Whether the state holds a pane anywhere — the difference between "the
/// tree shrank" and "the tree is gone".
pub(super) fn state_has_panes(state: &PersistState) -> bool {
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
        // not a resurrection — the history snapshots clear with it, or they
        // would resurrect through the side door the lastgood removal closed
        // (pinned by a_command_empty_save_clears_the_last_good_snapshot).
        let _ = fs::remove_file(&lastgood);
        for stale in kept_snapshots(target) {
            let _ = fs::remove_file(stale);
        }
    }
    // An empty Shutdown save leaves the snapshot alone — the emptiness may
    // be the reboot race, not the user's intent.
}

/// Snapshot history (herdr's `session-snapshots/`, `SNAPSHOT_INTERVAL`
/// and `SNAPSHOT_LIMIT` as measured there): every pane-bearing save that
/// lands at least [`SNAPSHOT_INTERVAL`] after the newest kept snapshot
/// copies the state file into `snapshots/` beside it, newest-last, capped
/// at [`SNAPSHOT_LIMIT`]. Unlike herdr's layout-fingerprint dedupe, the
/// throttle alone decides: par-mux's state embeds live screens, so
/// successive saves are never layout-identical in bytes, and the interval
/// is the bound that matters.
const SNAPSHOT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);
pub(super) const SNAPSHOT_LIMIT: usize = 48;

/// Where `target`'s history snapshots live: `snapshots/` beside it.
fn snapshots_dir(target: &Path) -> PathBuf {
    target
        .parent()
        .map(|parent| parent.join("snapshots"))
        .unwrap_or_else(|| PathBuf::from("snapshots"))
}

/// The kept snapshots for `target`, oldest first (the `<unix_ms>.state.json`
/// names sort lexically while every name carries the same digit count).
pub(super) fn kept_snapshots(target: &Path) -> Vec<PathBuf> {
    let stem = target
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Ok(entries) = fs::read_dir(snapshots_dir(target)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(&format!("{stem}-")) && name.ends_with(".state.json")
                })
        })
        .collect();
    paths.sort();
    paths
}

/// The history tier of a save: throttled, pane-bearing states only, best
/// effort — a snapshot failure warns and never fails the save itself.
fn maintain_snapshot_history(origin: SaveOrigin, state: &PersistState, target: &Path) {
    if origin == SaveOrigin::Reap || !state_has_panes(state) {
        return;
    }
    if let Err(err) = maintain_snapshot_history_in(target, SNAPSHOT_INTERVAL, SNAPSHOT_LIMIT) {
        log::warn!(
            "par-mux: snapshot history update beside {} failed: {err}",
            target.display()
        );
    }
}

/// [`maintain_snapshot_history`] with injectable throttle and cap (tests).
pub(super) fn maintain_snapshot_history_in(
    target: &Path,
    interval: std::time::Duration,
    keep: usize,
) -> std::io::Result<()> {
    let existing = kept_snapshots(target);
    if let Some(newest) = existing.last() {
        let modified = fs::metadata(newest)?.modified()?;
        if SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age < interval)
        {
            return Ok(());
        }
    }
    let dir = snapshots_dir(target);
    fs::create_dir_all(&dir)?;
    let stem = target
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    fs::copy(
        target,
        dir.join(format!("{stem}-{:.0}.state.json", unix_ms())),
    )?;
    let excess = kept_snapshots(target);
    if excess.len() > keep {
        for stale in &excess[..excess.len() - keep] {
            fs::remove_file(stale)?;
        }
    }
    Ok(())
}
