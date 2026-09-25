//! On-disk persistence envelope for the mux tree (par-mux.md Phase 3, D3.2).
//!
//! The artifact is a versioned envelope, not a bare tree: `format_version`
//! lets a future format change migrate old files rather than refuse them,
//! and each window's pane list rides beside its [`LayoutTree`] so restore
//! can spawn every pane's replacement process before re-hanging content
//! (D3.5: spawn first, restore second — startup bytes must not overwrite a
//! restored screen).

use crate::mux::agent_resume::resume_invocation;
use crate::mux::ids::{IdAllocator, PaneId, SessionId, WindowId};
use crate::mux::layout::LayoutTree;
use crate::mux::pane::{MuxError, PaneFactory, SpawnContext};
use crate::mux::tree::{MuxSession, MuxTree, MuxWindow};
use crate::terminal::replay_snapshot::{GridSnapshot, TerminalSnapshot};
use std::collections::HashMap;
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
pub const FORMAT_VERSION: u32 = 2;

/// Ceiling on the scrollback cells one pane contributes to the state file
/// — a persistence bound only; the in-memory pane keeps its full history.
/// Cells dominate the file (~170 serialized bytes each, measured
/// 2026-09-22), so an uncapped 10 000-line pane at 80 cols serializes to a
/// 136 MB state whose final shutdown save held SIGTERM exit for two
/// minutes. 100 000 cells (~1 250 lines at 80 cols, ~17 MB) keeps the
/// newest context while bounding the save far below the exit budget.
const MAX_PERSISTED_SCROLLBACK_CELLS: usize = 100_000;

/// Errors raised while saving or rebuilding persisted mux state.
#[derive(Debug)]
pub enum PersistError {
    /// The state was written under a `format_version` this build does not
    /// know — migrate or refuse, never guess.
    UnsupportedVersion { found: u32, supported: u32 },
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
    /// The id allocator's counters at capture — `(session, window, pane)`,
    /// the next id each kind hands out.
    pub next_ids: (u32, u32, u32),
    /// Every live session, in id order.
    pub sessions: Vec<PersistSession>,
    /// Named paste buffers (`set-buffer`/`show-buffer`).
    pub buffers: HashMap<String, String>,
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
    /// spawn each pane's replacement process before re-hanging content.
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
    let agent = metadata.get("agent")?;
    let session = PersistAgentSession {
        agent: agent.clone(),
        session_id: metadata.get("agent_session_id").cloned(),
        session_path: metadata.get("agent_session_path").cloned(),
        source: metadata.get("agent_source").cloned(),
        resume_argv: metadata.get("agent_resume_argv").cloned(),
    };
    (session.session_id.is_some()
        || session.session_path.is_some()
        || session.resume_argv.is_some())
    .then_some(session)
}

impl MuxTree {
    /// Capture the whole tree into its persisted form (D3.2).
    ///
    /// Sessions serialize in id order so two captures of an unchanged tree
    /// differ only in their timestamps.
    pub fn to_persist_state(&self) -> PersistState {
        let mut sessions: Vec<PersistSession> = self
            .sessions
            .values()
            .map(|session| PersistSession {
                id: session.id.0,
                name: session.name.clone(),
                active_window_index: session.active,
                env: session.env.clone(),
                windows: session
                    .windows
                    .iter()
                    .map(|window_id| self.to_persist_window(*window_id))
                    .collect(),
            })
            .collect();
        sessions.sort_by_key(|session| session.id);

        PersistState {
            format_version: FORMAT_VERSION,
            saved_at_unix_ms: unix_ms(),
            next_ids: self.ids.next_ids(),
            sessions,
            buffers: self.buffers.clone(),
        }
    }

    /// Capture one window and its panes; panes serialize in layout order.
    fn to_persist_window(&self, window_id: WindowId) -> PersistWindow {
        let window = self
            .windows
            .get(&window_id)
            .expect("session window lists only hold live windows");
        let panes = window
            .panes()
            .into_iter()
            .map(|pane_id| {
                let pane = self
                    .panes
                    .get(&pane_id)
                    .expect("layout leaf ids are always live panes");
                PersistPane {
                    id: pane_id.0,
                    terminal: cap_persisted_scrollback(pane.persisted_snapshot()),
                    spawn_command: pane.spawn_command().map(str::to_string),
                    user_title: pane.user_title().map(str::to_string),
                    agent_session: agent_session_from_metadata(pane.metadata()),
                    cwd: pane
                        .persistence_cwd()
                        .map(|dir| dir.to_string_lossy().into_owned()),
                }
            })
            .collect();
        PersistWindow {
            id: window.id.0,
            name: window.name.clone(),
            cols: window.cols,
            rows: window.rows,
            active_pane: window.active.0,
            layout: window.layout.clone(),
            panes,
        }
    }

    /// Rebuild a tree from persisted state (D3.5): spawn each pane's
    /// replacement process first, then restore its terminal from the
    /// snapshot, so process startup bytes never overwrite restored content.
    ///
    /// The id allocator resumes from the persisted counters, so restored
    /// panes keep their `%N` identities and new panes do not collide.
    pub fn from_persist_state(
        state: &PersistState,
        factory: Box<dyn PaneFactory>,
    ) -> Result<MuxTree, PersistError> {
        if state.format_version != FORMAT_VERSION {
            return Err(PersistError::UnsupportedVersion {
                found: state.format_version,
                supported: FORMAT_VERSION,
            });
        }

        let mut panes = HashMap::new();
        let mut windows = HashMap::new();
        let mut sessions = HashMap::new();
        // Panes whose persisted cwd was gone at restore — they spawned in
        // home and get a visible note after their content is restored.
        let mut cwd_fallbacks: HashMap<u32, String> = HashMap::new();

        for session in &state.sessions {
            let mut window_ids = Vec::with_capacity(session.windows.len());
            for window in &session.windows {
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
                    let resume_argv = pane.agent_session.as_ref().and_then(resume_invocation);
                    // The persisted cwd re-lands the pane where it left off.
                    // A directory that vanished between save and restore
                    // would fail the spawn, so it degrades to home — the
                    // spawn's success may not depend on a directory this
                    // process cannot control — and the pane says so after
                    // its content is restored.
                    let cwd: Option<PathBuf> = match pane.cwd.as_deref().map(Path::new) {
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
                    };
                    let mut created = match resume_argv {
                        Some(argv) => factory.create_argv_pane(
                            PaneId(pane.id),
                            window.cols,
                            window.rows,
                            &argv,
                            &context,
                        )?,
                        None => factory.create_pane(
                            PaneId(pane.id),
                            window.cols,
                            window.rows,
                            pane.spawn_command.as_deref(),
                            &context,
                        )?,
                    };
                    // Identity comes back as metadata so the format
                    // round-trips and task 6.3's hook-first lookup reads it
                    // from the same place it reads a live pane's. Only the
                    // identity keys — no state, no seq, no start source (a
                    // restored pane reports those anew or holds none).
                    if let Some(agent_session) = &pane.agent_session {
                        created.set_metadata("agent", &agent_session.agent);
                        if let Some(id) = &agent_session.session_id {
                            created.set_metadata("agent_session_id", id);
                        }
                        if let Some(path) = &agent_session.session_path {
                            created.set_metadata("agent_session_path", path);
                        }
                        if let Some(source) = &agent_session.source {
                            created.set_metadata("agent_source", source);
                        }
                        if let Some(argv) = &agent_session.resume_argv {
                            created.set_metadata("agent_resume_argv", argv);
                        }
                    }
                    if let Some(title) = &pane.user_title {
                        created.set_user_title(title);
                    }
                    panes.insert(PaneId(pane.id), created);
                }
                for pane in &window.panes {
                    let terminal = panes
                        .get_mut(&PaneId(pane.id))
                        .expect("just inserted above")
                        .terminal();
                    let mut restored = terminal.write();
                    restored.restore_for_new_process(pane.terminal.clone());
                    // After the snapshot re-hangs, so the note is the last
                    // thing on screen rather than scrolled away by it.
                    if let Some(note) = cwd_fallbacks.get(&pane.id) {
                        restored.process(note.as_bytes());
                    }
                }
                window_ids.push(WindowId(window.id));
                windows.insert(
                    WindowId(window.id),
                    MuxWindow {
                        id: WindowId(window.id),
                        name: window.name.clone(),
                        layout: window.layout.clone(),
                        active: PaneId(window.active_pane),
                        cols: window.cols,
                        rows: window.rows,
                    },
                );
            }
            sessions.insert(
                SessionId(session.id),
                MuxSession {
                    id: SessionId(session.id),
                    name: session.name.clone(),
                    windows: window_ids,
                    active: session.active_window_index,
                    env: session.env.clone(),
                },
            );
        }

        let mut tree = MuxTree::new(factory);
        tree.ids = IdAllocator::resume(state.next_ids);
        tree.sessions = sessions;
        tree.windows = windows;
        tree.panes = panes;
        tree.buffers = state.buffers.clone();
        // Panes were spawned at their window's full extent, but the restored
        // layout divides that extent — re-fit every terminal (and PTY) to
        // its geometry, exactly as a live resize would have, so a restart
        // lands in the same state a running server would be in.
        for window_id in tree.windows.keys().copied().collect::<Vec<_>>() {
            tree.sync_pane_sizes(window_id);
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
/// contiguous ring (`scrollback_start == 0`, `cells.len() == lines * cols`)
/// and the original `max_scrollback`, so a restored pane still grows to its
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
    // The extraction below indexes the ring through the live grid's
    // invariants (`cells.len() == min(lines, max) * cols`, `wrapped` one
    // entry per physical line). A snapshot violating that shape is passed
    // through untrimmed rather than sliced on a guess.
    let physical_lines = grid.scrollback_cells.len() / cols;
    if physical_lines < grid.scrollback_lines || grid.scrollback_wrapped.len() != physical_lines {
        return grid;
    }
    let ring_capacity = grid.max_scrollback.max(physical_lines);
    let drop_lines = grid.scrollback_lines - keep_lines;
    // Same logical→physical mapping as `scrollback_physical_index`,
    // validated up front so a snapshot with an inconsistent ring is passed
    // through rather than sliced out of range mid-extraction.
    let physicals: Vec<usize> = (drop_lines..grid.scrollback_lines)
        .map(|logical| (grid.scrollback_start + logical) % ring_capacity)
        .collect();
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
    grid.scrollback_start = 0;
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
}

/// The synchronous entry the shutdown save and tests use; the per-command
/// path captures a [`PersistState`] under the tree lock and hands it to the
/// server's persist worker, which writes through [`write_job`] off the
/// lock.
pub fn save_to(tree: &MuxTree, target: &Path) -> Result<(), PersistError> {
    write_job(SaveOrigin::Shutdown, &tree.to_persist_state(), target)
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
    } else if origin == SaveOrigin::Command {
        // Deliberately empty (kill-pane of the last pane): the next start
        // must be fresh, not a resurrection.
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
    Quarantined { from: PathBuf, to: PathBuf },
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
            ShellPaneFactory::default().create_pane(id, cols, rows, Some("sleep 60"), context)
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
    /// off instead of in the daemon's start directory.
    #[test]
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
    /// user had `cd`-ed, because the child's own cwd is readable.
    #[test]
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
    /// pane's logical cwd even if the child process has moved.
    #[test]
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
            SessionId(state.next_ids.0),
            "a restored server's first new session must take the persisted next id"
        );
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
        tree_with_metadata(&[
            ("agent", "pi"),
            ("agent_session_id", "s-1"),
            ("agent_session_path", "/tmp/pi-session.jsonl"),
            ("agent_source", "par-mux:pi"),
            (
                "agent_resume_argv",
                r#"["pi","--session","/tmp/pi-session.jsonl"]"#,
            ),
            // Keys that must NOT travel: state-shaped and provenance-of-start.
            ("agent_state", "working"),
            ("agent_state_source", "hook"),
            ("agent_seq", "1000"),
            ("agent_session_start_source", "startup"),
        ])
    }

    /// The resume spawn re-lands in the pane's persisted cwd: the agent's
    /// invocation (claude resolves transcripts per cwd) must run where the
    /// session lived, not in the daemon's start directory.
    #[test]
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
        assert_eq!(
            factory.command_for(pane_id).as_deref(),
            Some(format!("'pi' '--session' '/tmp/pi-session.jsonl'{SURVIVING_TAIL}").as_str()),
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
        assert_eq!(
            factory.command_for(pane_id).as_deref(),
            Some(format!("'omp' '--resume=/tmp/omp-session.jsonl'{SURVIVING_TAIL}").as_str()),
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
        assert_eq!(
            factory.command_for(pane_id).as_deref(),
            Some(format!("'claude' '--resume' 'abc-123'{SURVIVING_TAIL}").as_str()),
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
        // persisted the pane's deletion by now.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            assert!(
                restored.pane_mut(pane_id).unwrap().poll_running(),
                "the pane must survive its failed resume"
            );
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }

        // The restored history and the fallback's own trace are both still
        // readable (screen or scrollback — the shell's startup bytes may
        // have scrolled the markers up).
        let snapshot = restored
            .pane(pane_id)
            .unwrap()
            .terminal()
            .read()
            .capture_snapshot();
        let mut text: String = snapshot.grid.scrollback_cells.iter().map(|c| c.c).collect();
        text.extend(snapshot.grid.cells.iter().map(|c| c.c));
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
        let physical_lines = lines.min(max);
        let mut cells = vec![crate::cell::Cell::default(); physical_lines * cols];
        let mut wrapped = vec![false; physical_lines];
        for logical in 0..lines {
            let physical = (start + logical) % max.max(physical_lines);
            for c in 0..cols {
                cells[physical * cols + c] = marker_cell(logical);
            }
            wrapped[physical] = logical % 3 == 0;
        }
        GridSnapshot {
            cells: Vec::new(),
            scrollback_cells: cells,
            scrollback_start: start,
            scrollback_lines: lines,
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
        crate::cell::Cell {
            c: char::from_u32((line % 10) as u32 + '0' as u32).unwrap_or('0'),
            ..crate::cell::Cell::default()
        }
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
        assert_eq!(grid.scrollback_start, 0);
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
        assert_eq!(grid.scrollback_start, 0, "the capped ring is contiguous");
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
