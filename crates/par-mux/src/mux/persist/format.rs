//! The on-disk envelope: the serde schema types (field order is the wire
//! format), the version constant, the error type, and the restore bounds
//! that validate an untrusted state file's sizes.

use crate::mux::layout::LayoutTree;
use crate::mux::pane::MuxError;
use par_term_emu_core::cell::Cell;
use par_term_emu_core::terminal::replay_snapshot::TerminalSnapshot;
use std::collections::HashMap;
use std::path::PathBuf;
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
pub(super) const MAX_PERSISTED_SCROLLBACK_CELLS: usize = 100_000;

/// Restore bounds for a state file's window size — the same floor and
/// ceiling the streaming layer enforces on a client's terminal-size request
/// (`crate::streaming::server::{MIN_COLS, MAX_COLS, MIN_ROWS, MAX_ROWS}`),
/// mirrored here because the `mux` feature does not enable `streaming`
/// (ARC-113b).
pub(super) const MIN_RESTORED_COLS: u16 = 2;
pub(super) const MIN_RESTORED_ROWS: u16 = 1;
/// cap: Columns one restored window may claim from an untrusted state file.
pub(super) const MAX_RESTORED_COLS: u16 = 1_000;
/// cap: Rows one restored window may claim from an untrusted state file.
pub(super) const MAX_RESTORED_ROWS: u16 = 500;

/// Clamp a state file's window size to the restore bounds: the size feeds
/// the factory's grid allocation, so a corrupt or hostile `cols`/`rows`
/// pair must not reach one unchecked.
pub(super) fn clamp_restored_window_size(cols: u16, rows: u16) -> (u16, u16) {
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
pub(super) fn clamp_restored_grid_dims(snapshot: &mut TerminalSnapshot) {
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
    /// A layout name outside `[A-Za-z0-9._-]+` (or `.`/`..`).
    InvalidLayoutName {
        /// The rejected name.
        name: String,
    },
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
            PersistError::InvalidLayoutName { name } => {
                write!(
                    f,
                    "invalid layout name {name:?} (allowed: A-Z a-z 0-9 . _ -)"
                )
            }
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
pub(super) fn unix_ms() -> u64 {
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
    /// Keep-one-tab respawn ([`crate::mux::tree::MuxWorkspace::respawn_last`]).
    /// Absent (off) in files written before the option existed, and
    /// omitted while off so those files keep their shape.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "std::ops::Not::not")
    )]
    pub respawn_last: bool,
    /// The workspace's explicit exit action
    /// ([`crate::mux::tree::MuxWorkspace::exit_action`], the
    /// `set-exit-action` choice). Absent in files written before the
    /// option existed, and omitted while unset so those files keep
    /// their shape.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub exit_action: Option<String>,
    /// The workspace's git-worktree backing
    /// ([`crate::mux::tree::MuxWorkspace::worktree`]). Absent in files
    /// written before worktree workspaces existed and omitted for an
    /// ordinary workspace, so the envelope version stays put.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub worktree: Option<PersistWorktreeLink>,
}

/// A persisted [`crate::mux::tree::WorktreeLink`]. Paths are stored the
/// way pane cwds are (lossy UTF-8 strings).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistWorktreeLink {
    /// The repo's main checkout — the group key.
    pub repo: String,
    /// This workspace's checkout.
    pub checkout: String,
    /// The checkout's branch; `None` for a detached HEAD.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub branch: Option<String>,
}

impl From<&crate::mux::tree::WorktreeLink> for PersistWorktreeLink {
    fn from(link: &crate::mux::tree::WorktreeLink) -> Self {
        Self {
            repo: link.repo.to_string_lossy().into_owned(),
            checkout: link.checkout.to_string_lossy().into_owned(),
            branch: link.branch.clone(),
        }
    }
}

impl From<&PersistWorktreeLink> for crate::mux::tree::WorktreeLink {
    fn from(link: &PersistWorktreeLink) -> Self {
        Self {
            repo: PathBuf::from(&link.repo),
            checkout: PathBuf::from(&link.checkout),
            branch: link.branch.clone(),
        }
    }
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
/// The last-known state rides along (`state`/`state_source`/`message`) so
/// a restored pane stays on the roster without waiting for its agent's
/// next hook report. Staleness is bounded twice: the liveness sweep clears
/// a claim whose agent is provably out of the pane's process tree (the
/// resumed invocation counts — restore spawns it), and the agent's next
/// accepted report rewrites the state. Absent on older state files
/// (`serde(default)`), where the post-restart roster is simply empty.
///
/// Deliberately absent: `agent_session_start_source` (stale and misleading
/// after a restart; task 6.4 reads it from the post-restore report instead)
/// and the sweep's liveness-miss counters and `agent_seq` (process-local
/// bookkeeping: a restored pane reports its own sequence from scratch).
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
    /// The last-known agent state (`metadata["agent_state"]`).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub state: Option<String>,
    /// Who authored the state: `"hook"` or `"scrape"`
    /// (`metadata["agent_state_source"]`).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub state_source: Option<String>,
    /// The blocked reason, whitespace-collapsed at the report door
    /// (`metadata["agent_message"]`).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub message: Option<String>,
}
