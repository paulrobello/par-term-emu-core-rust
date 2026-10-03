//! Panes: PTY ownership, output plumbing, and the factory seam.

use crate::mux::agent_resume::render_surviving;
use crate::mux::ids::{PaneId, SessionId, WindowId, WorkspaceId};
use crate::pty_error::PtyError;
use crate::pty_session::{OutputCallback, PtyInputHandle, PtySession};
use crate::terminal::replay_snapshot::TerminalSnapshot;
use crate::terminal::Terminal;
use parking_lot::{Mutex, RwLock};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Errors raised while creating or driving a pane, window, or session.
#[derive(Debug)]
pub enum MuxError {
    /// The underlying PTY layer failed.
    Pty(PtyError),
    /// The requested pane does not exist.
    NoSuchPane(PaneId),
    /// The requested window does not exist.
    NoSuchWindow(WindowId),
    /// The requested session does not exist.
    NoSuchSession(SessionId),
    /// A pane name (user title) matched no pane.
    NoSuchPaneNamed(String),
    /// A window name matched no window.
    NoSuchWindowNamed(String),
    /// A session name matched no session.
    NoSuchSessionNamed(String),
    /// A pane name (user title) matched more than one pane; the candidates
    /// are listed so the caller can disambiguate with an id.
    AmbiguousPaneTarget(String, Vec<PaneId>),
    /// A window name matched more than one window.
    AmbiguousWindowTarget(String, Vec<WindowId>),
    /// A session name matched more than one session.
    AmbiguousSessionTarget(String, Vec<SessionId>),
    /// The requested workspace does not exist.
    NoSuchWorkspace(WorkspaceId),
    /// A workspace name matched no workspace.
    NoSuchWorkspaceNamed(String),
    /// A workspace name matched more than one workspace.
    AmbiguousWorkspaceTarget(String, Vec<WorkspaceId>),
    /// The two panes are not in the same window, so their positions cannot
    /// be exchanged.
    PanesInDifferentWindows(PaneId, PaneId),
    /// A pane move named the same pane as source and target.
    SamePane(PaneId),
    /// The pane's process is still running — `respawn-pane` without
    /// `-k` refuses to kill it.
    PaneAlive(PaneId),
    /// The two windows are not in the same session, so their positions
    /// cannot be exchanged.
    WindowsInDifferentSessions(WindowId, WindowId),
    /// The pane has no bordering split of the requested orientation to
    /// adjust.
    PaneNotResizable(PaneId),
}

impl std::fmt::Display for MuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MuxError::Pty(err) => write!(f, "pty error: {err}"),
            MuxError::NoSuchPane(id) => write!(f, "no such pane: {id}"),
            MuxError::NoSuchWindow(id) => write!(f, "no such window: {id}"),
            MuxError::NoSuchSession(id) => write!(f, "no such session: {id}"),
            MuxError::NoSuchPaneNamed(name) => write!(f, "no such pane: {name}"),
            MuxError::NoSuchWindowNamed(name) => write!(f, "no such window: {name}"),
            MuxError::NoSuchSessionNamed(name) => write!(f, "no such session: {name}"),
            MuxError::AmbiguousPaneTarget(name, ids) => {
                write!(
                    f,
                    "ambiguous pane target: {name} (matching: {})",
                    join_ids(ids)
                )
            }
            MuxError::AmbiguousWindowTarget(name, ids) => {
                write!(
                    f,
                    "ambiguous window target: {name} (matching: {})",
                    join_ids(ids)
                )
            }
            MuxError::AmbiguousSessionTarget(name, ids) => {
                write!(
                    f,
                    "ambiguous session target: {name} (matching: {})",
                    join_ids(ids)
                )
            }
            MuxError::NoSuchWorkspace(id) => write!(f, "no such workspace: {id}"),
            MuxError::NoSuchWorkspaceNamed(name) => write!(f, "no such workspace: {name}"),
            MuxError::AmbiguousWorkspaceTarget(name, ids) => {
                write!(
                    f,
                    "ambiguous workspace target: {name} (matching: {})",
                    join_ids(ids)
                )
            }
            MuxError::PanesInDifferentWindows(a, b) => {
                write!(f, "panes {a} and {b} are in different windows")
            }
            MuxError::SamePane(id) => {
                write!(f, "pane {id} cannot be moved onto itself")
            }
            MuxError::PaneAlive(id) => {
                write!(
                    f,
                    "pane {id} is still running; respawn-pane -k kills it first"
                )
            }
            MuxError::WindowsInDifferentSessions(a, b) => {
                write!(f, "windows {a} and {b} are in different sessions")
            }
            MuxError::PaneNotResizable(id) => {
                write!(f, "pane {id} cannot be resized in that direction")
            }
        }
    }
}

impl std::error::Error for MuxError {}

/// Comma-separate ids for an ambiguity error's candidate list — each id's
/// own Display carries its sigil, so the message reads `(matching: %0, %2)`.
fn join_ids(ids: &[impl std::fmt::Display]) -> String {
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl From<PtyError> for MuxError {
    fn from(err: PtyError) -> Self {
        MuxError::Pty(err)
    }
}

/// Default scrollback retained per pane.
const DEFAULT_SCROLLBACK: usize = 10_000;

/// One pane: a PTY, its terminal emulator, and its metadata.
///
/// `metadata` is extension seam S2 — empty in the base server, and the place an
/// agent layer later records `agent`, `agent_status`, and session identity
/// without changing this struct.
pub struct MuxPane {
    id: PaneId,
    session: PtySession,
    /// The command this pane's process was spawned with (`None` = default
    /// shell), recorded at spawn so a save/restore cycle (par-mux.md D3.5)
    /// can respawn the same program.
    spawn_command: Option<String>,
    /// The user-set pane title (`select-pane -T`): sticky — the program's
    /// OSC 0/2 title never overwrites it, the documented divergence from
    /// tmux recorded in par-mux.md. `None` = no user title;
    /// [`MuxPane::effective_title`] falls back to the terminal's live OSC
    /// title instead.
    user_title: Option<String>,
    /// The process has been observed dead by a liveness pass. The pane is
    /// HELD in this state (remain-on-exit): its frozen screen stays for
    /// `respawn-pane` to restart in place.
    dead: bool,
    /// The recorded exit code, valid once `dead` — `None` when the OS had
    /// no exit status to report as the death was observed.
    exit_code: Option<i32>,
    metadata: HashMap<String, String>,
    /// The hook-reported telemetry, parsed and encoded once on write so the
    /// roster reads it without re-parsing JSON under the tree lock
    /// (ARC-113). Ephemeral like the rest of the claim: never persisted,
    /// cleared with it ([`crate::mux::hooks::clear_agent_claim`]).
    pub(crate) telemetry: Option<crate::mux::hooks::StoredTelemetry>,
    /// The host probe's typed sample, each field stamped (ARC-113).
    pub(crate) host_telemetry: Option<crate::mux::host_probe::HostTelemetry>,
    /// The last accepted report `seq` per reporting source (herdr's
    /// `hook_report_sequences`; ARC-113). Reports without a source share
    /// the empty-string bucket.
    pub(crate) seq_by_source: HashMap<String, u64>,
    /// Last persistence snapshot, valid while the terminal has not changed
    /// since it was taken — see [`MuxPane::persisted_snapshot`]. Shared
    /// handle so a capture started under the tree lock can finish OFF it
    /// (ARC-032).
    snapshot_cache: Arc<Mutex<Option<SnapshotCacheEntry>>>,
    /// The pane's own hook-only endpoint (ENH-039), bound by the factory
    /// when the daemon runs with `--pane-endpoints`. `Drop` removes the
    /// endpoint's socket file when the pane is killed or respawned; a crash
    /// leaves the remnant for the daemon's startup sweep. Held for its
    /// `Drop` side effect only — nothing reads the value.
    #[allow(dead_code)]
    pub(crate) pane_endpoint: Option<crate::mux::server::PaneEndpoint>,
}

/// The validity key of a cached snapshot: the pane's PTY generation (bumped
/// on every processed read) plus the terminal size. The size rides along
/// because a resize re-fits the grid without producing PTY output, so the
/// generation alone would keep serving a stale-geometry snapshot.
#[derive(Clone, Copy, PartialEq, Eq)]
struct SnapshotCacheKey {
    generation: u64,
    cols: usize,
    rows: usize,
}

struct SnapshotCacheEntry {
    key: SnapshotCacheKey,
    snapshot: TerminalSnapshot,
}

/// Everything [`MuxPane::persisted_snapshot`] needs, without the pane —
/// collected under the tree lock in cheap field reads (ARC-032), so the
/// expensive half (the grid walk on a cache miss, the cwd syscalls) can
/// run after the lock drops, from [`snapshot_from_parts`].
pub(crate) struct PaneSnapshotParts {
    terminal: Arc<RwLock<Terminal>>,
    generation: u64,
    cache: Arc<Mutex<Option<SnapshotCacheEntry>>>,
    child_pid: Option<u32>,
}

impl PaneSnapshotParts {
    /// [`MuxPane::persistence_cwd`]'s logic over collected parts (ARC-032):
    /// the terminal's OSC 7 reported cwd first, then the child's live cwd
    /// via syscall — both off the tree lock.
    pub(crate) fn cwd(&self) -> Option<std::path::PathBuf> {
        if let Some(reported) = self.terminal.read().current_directory() {
            return Some(std::path::PathBuf::from(reported));
        }
        self.child_pid.and_then(process_cwd)
    }

    /// The OSC 7 hostname that came with [`Self::cwd`], only when that cwd
    /// is the OSC 7 report. A cwd read from the child's kernel state has no
    /// OSC 7 host, so this is `None` then. Persisted beside the cwd so a
    /// restored dead pane still knows a remote report is remote (SEC-128).
    pub(crate) fn cwd_host(&self) -> Option<String> {
        let term = self.terminal.read();
        term.current_directory()?;
        term.shell_integration().hostname().map(str::to_string)
    }

    /// The host-probe target (SEC-115): only the child's kernel-reported
    /// cwd — pane output must never choose the directory the daemon runs
    /// git in, and OSC 7 is program output. `None` (no child, reaped pid,
    /// platform without a pid→cwd path) skips the pane entirely: absent
    /// telemetry beats probing a directory output picked. Contrast
    /// [`Self::cwd`], which keeps the OSC 7 preference where trust is not
    /// load-bearing (persistence and session restore).
    pub(crate) fn probe_cwd(&self) -> Option<std::path::PathBuf> {
        self.child_pid.and_then(process_cwd)
    }
}

/// [`MuxPane::persisted_snapshot`]'s logic over collected parts: serve the
/// cached capture while the terminal has not changed, else walk the grid
/// and cache the result. Holds no tree lock, only the pane's own cache
/// slot and terminal read lock.
pub(crate) fn snapshot_from_parts(parts: &PaneSnapshotParts) -> TerminalSnapshot {
    let PaneSnapshotParts {
        terminal,
        generation,
        cache,
        child_pid: _,
    } = parts;
    let (cols, rows) = terminal.read().size();
    let key = SnapshotCacheKey {
        generation: *generation,
        cols,
        rows,
    };
    let mut cache = cache.lock();
    if let Some(entry) = cache.as_ref() {
        if entry.key == key {
            return entry.snapshot.clone();
        }
    }
    let snapshot = terminal.read().capture_snapshot();
    *cache = Some(SnapshotCacheEntry {
        key,
        snapshot: snapshot.clone(),
    });
    snapshot
}

impl MuxPane {
    /// This pane's identifier.
    pub fn id(&self) -> PaneId {
        self.id
    }

    /// The command this pane was spawned with, if any — what a restore
    /// respawns.
    pub fn spawn_command(&self) -> Option<&str> {
        self.spawn_command.as_deref()
    }

    /// The user-set title (`select-pane -T`), when one is set.
    pub fn user_title(&self) -> Option<&str> {
        self.user_title.as_deref()
    }

    /// Set or clear the user title (empty string = clear). Returns whether
    /// the stored value changed, so the dispatcher can skip broadcasting a
    /// no-op `%pane-title-changed`.
    pub fn set_user_title(&mut self, title: &str) -> bool {
        let new = if title.is_empty() {
            None
        } else {
            Some(title.to_string())
        };
        if self.user_title == new {
            false
        } else {
            self.user_title = new;
            true
        }
    }

    /// The title clients should display for this pane: the user title when
    /// one is set, else the pane terminal's current OSC 0/2 title (empty
    /// when the program set neither).
    pub fn effective_title(&self) -> String {
        self.user_title
            .clone()
            .unwrap_or_else(|| self.session.terminal().read().title().to_string())
    }

    /// The terminal emulator backing this pane. Mutate through
    /// [`Self::with_terminal_mut`] instead of this lock, so the session's
    /// geometry mirror stays current (QA-195).
    pub fn terminal(&self) -> Arc<RwLock<Terminal>> {
        self.session.terminal()
    }

    /// Run `f` with exclusive access to the pane's terminal, then republish
    /// the session's wait-free geometry mirror, so `cursor_position()` and
    /// `size()` reflect what `f` did (QA-195).
    pub fn with_terminal_mut<R>(&self, f: impl FnOnce(&mut Terminal) -> R) -> R {
        self.session.with_terminal_mut(f)
    }

    /// The session's wait-free published cursor — what a mirror-reading
    /// consumer sees, as opposed to the terminal's own cursor.
    #[cfg(test)]
    pub(crate) fn published_cursor(&self) -> (usize, usize) {
        self.session.cursor_position()
    }

    /// The cwd persistence should capture for this pane: the shell's OSC 7
    /// report first (the pane's logical cwd, kept current by every
    /// prompt), the child process's live cwd second (a shell without
    /// integration hooks). `None` when neither source yields a path.
    pub fn persistence_cwd(&self) -> Option<std::path::PathBuf> {
        if let Some(reported) = self.terminal().read().current_directory() {
            return Some(std::path::PathBuf::from(reported));
        }
        self.session.child_pid().and_then(process_cwd)
    }

    /// The default cwd `respawn-pane` restarts this pane in (SEC-128).
    /// OSC 7 is program output, including a remote host's over SSH, so it
    /// is used only when it names this machine and the directory exists
    /// here. Otherwise the live child's kernel-reported cwd (the `-k` case),
    /// which is `None` once the child is reaped (SEC-125). Unlike
    /// [`Self::persistence_cwd`], which keeps OSC 7 first for restore.
    pub fn respawn_cwd(&self) -> Option<std::path::PathBuf> {
        let reported = {
            let term = self.terminal();
            let term = term.read();
            term.current_directory()
                .filter(|_| osc7_host_is_local(term.shell_integration().hostname()))
                .map(std::path::PathBuf::from)
        };
        if let Some(dir) = reported.filter(|dir| dir.is_dir()) {
            return Some(dir);
        }
        self.session.child_pid().and_then(process_cwd)
    }

    /// The pane's persistence snapshot, reusing the cached capture while the
    /// terminal has not changed since it was taken.
    ///
    /// Saves capture every pane on every structural command, but a pane only
    /// changes through PTY output (which bumps the session's update
    /// generation) or a resize (which changes the size) — so an idle pane
    /// costs one `Vec<Cell>` clone instead of a full grid walk. Keyed on
    /// both, per [`SnapshotCacheKey`].
    pub fn persisted_snapshot(&self) -> TerminalSnapshot {
        snapshot_from_parts(&self.snapshot_capture_parts())
    }

    /// The collect-under-lock half of [`Self::persisted_snapshot`]
    /// (ARC-032): terminal handle, PTY generation, and the shared cache
    /// slot, all cheap field reads safe to take while the tree lock is
    /// held.
    pub(crate) fn snapshot_capture_parts(&self) -> PaneSnapshotParts {
        PaneSnapshotParts {
            terminal: self.session.terminal(),
            generation: self.session.update_generation(),
            cache: Arc::clone(&self.snapshot_cache),
            child_pid: self.session.child_pid(),
        }
    }

    /// Whether the pane's child process is still running.
    pub fn is_running(&self) -> bool {
        self.session.is_running()
    }

    /// The pane's update generation — bumped on every processed PTY read,
    /// so it is the liveness signal a test (or embedder) can poll to know a
    /// pane's terminal has stopped changing.
    pub fn update_generation(&self) -> u64 {
        self.session.update_generation()
    }

    /// Liveness for the reaper's periodic pass — the reader flag plus the OS
    /// child handle. On Windows ConPTY the reader never observes EOF after
    /// the child exits, so [`Self::is_running`] alone would leave an exited
    /// pane in the tree forever; see [`PtySession::poll_running`].
    pub fn poll_running(&mut self) -> bool {
        self.session.poll_running()
    }

    /// Whether a liveness pass has observed this pane's process dead.
    /// A dead pane is held (remain-on-exit): its frozen screen stays in
    /// the tree until `respawn-pane` restarts it or `kill-pane` removes
    /// it.
    pub fn dead(&self) -> bool {
        self.dead
    }

    /// Record the death this pane's process has already been observed to
    /// reach ([`Self::poll_running`] returned false), capturing the exit
    /// code while the child handle can still be asked. An earlier reap
    /// serves its recorded code (SEC-125). Best-effort: a code the OS
    /// could not report records `None`.
    pub fn mark_dead(&mut self) {
        let code = self.session.try_wait().ok().flatten();
        self.mark_dead_with_code(code);
    }

    /// Record a death with an already-known exit code, without asking the
    /// OS child handle. The restore path's seam (ARC-114): a pane persisted
    /// held-dead comes back with no process, so [`Self::mark_dead`] would
    /// read no handle and clobber the persisted code with `None`.
    pub fn mark_dead_with_code(&mut self, exit_code: Option<i32>) {
        self.dead = true;
        self.exit_code = exit_code;
    }

    /// The exit code captured at [`Self::mark_dead`] — `None` before the
    /// death was observed or when the code was unreadable.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// The pane's child process id, if it has been spawned.
    pub fn child_pid(&self) -> Option<u32> {
        self.session.child_pid()
    }

    /// Read-only view of this pane's metadata (seam S2).
    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }

    /// Record a metadata entry (seam S2).
    pub fn set_metadata(&mut self, key: &str, value: &str) {
        self.metadata.insert(key.to_string(), value.to_string());
    }

    /// Remove metadata entries (seam S2). The scrape tier's honesty rule:
    /// a pattern that stops matching must clear its earlier guess rather
    /// than leave a stale state on the pane.
    pub fn clear_metadata(&mut self, keys: &[&str]) {
        for key in keys {
            self.metadata.remove(*key);
        }
    }

    /// The pane's agent claim, typed ([`crate::mux::hooks::AgentClaim`],
    /// ARC-113c) — or `None` when no hook or factory has claimed this pane.
    pub(crate) fn agent_claim(&self) -> Option<crate::mux::hooks::AgentClaim> {
        crate::mux::hooks::AgentClaim::from_metadata(&self.metadata)
    }

    /// Replace the pane's whole agent claim: every field renders to its
    /// stringly key and every cleared field's key is removed, so a stale
    /// value cannot survive a write that did not carry it.
    pub(crate) fn set_agent_claim(&mut self, claim: &crate::mux::hooks::AgentClaim) {
        claim.write_to_metadata(&mut self.metadata);
    }

    /// Mutate the pane's existing agent claim in place. No-op on an
    /// unclaimed pane — every caller runs behind an `agent`-label guard
    /// (a claim exists wherever the claim is edited).
    pub(crate) fn update_agent_claim<R>(
        &mut self,
        f: impl FnOnce(&mut crate::mux::hooks::AgentClaim) -> R,
    ) -> Option<R> {
        let mut claim = self.agent_claim()?;
        let result = f(&mut claim);
        self.set_agent_claim(&claim);
        Some(result)
    }

    /// Install the sink that receives raw PTY output for this pane.
    ///
    /// The server wires this to the control-mode emitter so bytes become
    /// `%output` lines as they are produced — the push path that a snapshot
    /// API cannot provide.
    pub fn on_output<F>(&mut self, callback: F)
    where
        F: Fn(&[u8]) + Send + Sync + 'static,
    {
        // OutputCallback is Arc<dyn Fn(&[u8]) + Send + Sync>, so the sink is
        // shared, not boxed.
        self.session.set_output_callback(Arc::new(callback));
    }

    /// [`Self::on_output`] for a sink already shared as an [`OutputSink`] —
    /// the dispatcher re-installs the one it handed the spawn, so a factory
    /// that ignored [`SpawnContext::output`] still forwards (ARC-103).
    pub fn on_output_sink(&mut self, sink: OutputSink) {
        self.session.set_output_callback(sink.0);
    }

    /// Feed `bytes` to the pane's terminal as if the program printed them
    /// — a daemon note such as a gone start directory. Goes through the
    /// geometry-publishing write path, so `cursor_position()` is current
    /// afterwards (QA-195).
    pub fn write_note(&self, bytes: &[u8]) {
        self.with_terminal_mut(|term| term.process(bytes));
    }

    /// Stop forwarding this pane's output (ARC-089). On return, no sink
    /// call is in flight and none will start: the reader invokes the sink
    /// while holding the same lock this takes. The terminal keeps
    /// processing output; only the forward stops.
    pub fn detach_output(&mut self) {
        self.session.clear_output_callback();
    }

    /// Write client input to the pane's PTY.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), MuxError> {
        self.session.write(bytes).map_err(MuxError::from)
    }

    /// The pane's input path for a write issued with no tree lock held
    /// (QA-225): snapshot this under the tree mutex, drop the mutex, then
    /// write — a paste can block in `write_all` on a full PTY buffer.
    pub(crate) fn input_handle(&self) -> Result<PtyInputHandle, MuxError> {
        self.session
            .input_handle()
            .ok_or(MuxError::Pty(PtyError::NotStartedError))
    }

    /// Resize the pane.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), MuxError> {
        self.session.resize(cols, rows).map_err(MuxError::from)
    }

    /// [`Self::resize`] with the client's per-cell pixel size, so
    /// XTWINOPS reports (`CSI 14 t`/`16 t`), `TIOCGWINSZ`, and image
    /// cell-span math all carry the size the client actually renders
    /// cells at instead of the 10×20 construction default. Total text-area
    /// pixels are derived per pane (`cols × cell_w`) because the protocol
    /// carries the cell size — the one renderer-metric every pane shares —
    /// while grid extents differ per pane.
    pub fn resize_with_cell_pixels(
        &mut self,
        cols: u16,
        rows: u16,
        cell_w: u16,
        cell_h: u16,
    ) -> Result<(), MuxError> {
        self.session
            .resize_with_pixels(
                cols,
                rows,
                crate::pty_session::pixel_extent(cols, cell_w),
                crate::pty_session::pixel_extent(rows, cell_h),
            )
            .map_err(MuxError::from)
    }

    /// Terminate the pane's child process. Its output stops forwarding
    /// first, so a SIGHUP handler's parting bytes never reach the sink.
    pub fn kill(&mut self) -> Result<(), MuxError> {
        self.detach_output();
        self.session.kill().map_err(MuxError::from)
    }
}

/// A pane output sink carried into the spawn so it is installed before the
/// reader thread starts (ARC-103). A newtype so [`SpawnContext`] keeps
/// `Debug`.
#[derive(Clone)]
pub struct OutputSink(pub OutputCallback);

impl std::fmt::Debug for OutputSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OutputSink(..)")
    }
}

/// Where a new pane lands: the identity and environment its spawn inherits.
///
/// The tree builds one per spawn from the session and window the pane is
/// created in. [`Default`] is a pane outside any session (tests, embedders
/// driving a factory directly): no identity vars, no session environment,
/// no per-spawn cwd, no output sink.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpawnContext<'a> {
    /// The owning session's id and name, exported as `PAR_MUX_SESSION_ID`
    /// and `PAR_MUX_SESSION`.
    pub session: Option<(SessionId, &'a str)>,
    /// The owning window, exported as `PAR_MUX_WINDOW_ID`.
    pub window: Option<WindowId>,
    /// The session's environment (`set-environment`, `new-session -e`),
    /// applied on top of the daemon's own environment.
    pub env: Option<&'a BTreeMap<String, String>>,
    /// The spawn's working directory — the per-pane override a restore
    /// hands the factory so a pane re-lands where it left off. Wins over
    /// [`ShellPaneFactory::cwd`] (the daemon-wide default); `None` keeps
    /// the factory's value.
    pub cwd: Option<&'a std::path::Path>,
    /// The pane's output sink, installed on the PTY before the child
    /// spawns so the first bytes it prints are forwarded. Factories that
    /// build their own `PtySession` must do the same, or they lose the
    /// first output (the dispatcher re-installs it after the spawn).
    pub output: Option<&'a OutputSink>,
}

/// Creates panes on demand — extension seam S1.
///
/// Mirrors [`crate::streaming::SessionFactory`] deliberately. An agent layer
/// later ships an implementation that spawns an agent CLI, seeds the child
/// environment, and tags [`MuxPane::metadata`], with no change to this trait
/// or to the server that calls it.
pub trait PaneFactory: Send + Sync {
    /// Create a pane, spawning its process.
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError>;

    /// Create a pane from STRUCTURED argv — the restore path's agent-resume
    /// seam. Windows cannot carry an argv through a cmd.exe string
    /// re-parse (the spawn layer quotes for CreateProcess, which cmd then
    /// re-tokenizes with its own rules), so implementors that can spawn
    /// argv without a shell should override. The default renders through
    /// [`agent_resume::render_surviving`] into the string path — exact on
    /// POSIX `sh`, and the historical behavior every factory shipped with.
    fn create_argv_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        argv: &[String],
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        if argv.is_empty() {
            return self.create_pane(id, cols, rows, None, context);
        }
        self.create_pane(id, cols, rows, Some(&render_surviving(argv)), context)
    }

    /// Create a pane that is born held-dead (ARC-114): no process is
    /// spawned. The restore path's seam for a pane persisted as dead, whose
    /// frozen screen is re-hung afterwards and whose `respawn-pane` later
    /// starts a process in place. No default implementation: a
    /// [`MuxPane`] always owns a session, which only a factory knows how
    /// to build.
    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<MuxPane, MuxError>;
}

/// The default factory: spawns the user's shell, or an explicit command.
#[derive(Debug, Default)]
pub struct ShellPaneFactory {
    /// Working directory for new panes; the process default when `None`.
    pub cwd: Option<std::path::PathBuf>,
    /// Control-socket path exported as `PAR_MUX_SOCKET` (with
    /// `PAR_MUX_ENV=1`) so hook scripts running inside a pane can report
    /// agent state back to this server — the Phase 5 env contract.
    /// `None` (tests, embedders without a socket) seeds nothing.
    pub socket_path: Option<String>,
    /// The daemon executable, exported as `PAR_MUX_BIN` so a pane script can
    /// run client mode (`$PAR_MUX_BIN --socket "$PAR_MUX_SOCKET" --cmd …`)
    /// without `par-mux` on `PATH`. Set by the binary: in library code
    /// `current_exe()` would name whatever process embeds the server.
    pub bin_path: Option<String>,
    /// ENH-039: when set, each pane gets its own hook-only socket endpoint
    /// beside the control socket, and `PAR_MUX_SOCKET` names THAT instead
    /// of the full control socket — a pane's child processes can report
    /// their agent state but cannot drive other panes or the server. The
    /// default stays `None` (mode off): the in-pane full-socket fallback is
    /// a core feature; see docs/MUX.md for the trade and the default-flip
    /// gate.
    pub pane_endpoint_tx: Option<crate::mux::server::PaneEndpointTx>,
    /// With `pane_endpoint_tx`: also export `PAR_MUX_CONTROL_SOCKET`
    /// (naming the full control socket) in every pane, so an agent-driven
    /// pane can opt back into full control. Independently granted per
    /// session by `PAR_MUX_CONTROL=1` in the session's environment.
    pub expose_control_socket: bool,
}

impl ShellPaneFactory {
    /// A PTY session carrying the spawn's cwd and env contract — the shared
    /// front half of every spawn path (shell command, structured argv,
    /// default shell).
    fn configured_session(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        context: &SpawnContext<'_>,
        pane_socket: Option<&str>,
    ) -> PtySession {
        let mut session = PtySession::new(cols as usize, rows as usize, DEFAULT_SCROLLBACK);

        // The per-spawn cwd (a restore re-landing a pane where it left off)
        // outranks the factory-wide default.
        let cwd = context.cwd.or(self.cwd.as_deref());
        if let Some(cwd) = cwd {
            session.set_cwd(cwd);
        }
        // The pane id is exported so hooks running inside the pane can identify
        // themselves back to the server — the mechanism seam S1's agent layer
        // relies on. The socket path and gate variable complete herdr's env
        // contract (`HERDR_ENV`/`HERDR_SOCKET_PATH`, renamed): a ported hook
        // script checks all three before reporting.
        // Session env first: the command builder applies vars in order, so
        // the PAR_MUX_* identity set after it cannot be overridden by a
        // client's set-environment.
        // PAR_MUX_CONTROL=1 in the session's env grants the pane the full
        // control socket even in endpoint mode (ENH-039); read before the
        // loop below consumes the iterator.
        let session_granted = context
            .env
            .into_iter()
            .flatten()
            .any(|(name, value)| name == "PAR_MUX_CONTROL" && value == "1");
        for (name, value) in context.env.into_iter().flatten() {
            session.set_env(name, value);
        }
        session.set_env("PAR_MUX_PANE_ID", &id.to_string());
        if let Some(control) = &self.socket_path {
            match pane_socket {
                // Endpoint mode, endpoint bound: PAR_MUX_SOCKET names the
                // pane's OWN hook-only socket (ENH-039). The full socket is
                // exported as PAR_MUX_CONTROL_SOCKET only when opted in —
                // the factory flag, or PAR_MUX_CONTROL=1 in the session env.
                Some(pane_sock) => {
                    session.set_env("PAR_MUX_SOCKET", pane_sock);
                    session.set_env("PAR_MUX_ENV", "1");
                    if self.expose_control_socket || session_granted {
                        session.set_env("PAR_MUX_CONTROL_SOCKET", control);
                    }
                }
                // Endpoint mode but no endpoint bound for this pane (the
                // per-daemon cap, or the path over the platform
                // socket-address limit): the contract marker still names
                // this a pane, but NO socket is exported — least privilege
                // does not fall back to the full control socket.
                None if self.pane_endpoint_tx.is_some() => {
                    session.set_env("PAR_MUX_ENV", "1");
                }
                // Endpoints off: the full-socket contract, byte-identical
                // to the pre-ENH-039 behavior.
                None => {
                    session.set_env("PAR_MUX_SOCKET", control);
                    session.set_env("PAR_MUX_ENV", "1");
                }
            }
        }
        // Fixed at spawn, as tmux's TMUX/TMUX_PANE are: a later
        // rename-session or cross-window swap-pane leaves these stale. The
        // ids stay valid; the name is advisory.
        if let Some((session_id, name)) = context.session {
            session.set_env("PAR_MUX_SESSION_ID", &session_id.to_string());
            session.set_env("PAR_MUX_SESSION", name);
        }
        if let Some(window) = context.window {
            session.set_env("PAR_MUX_WINDOW_ID", &window.to_string());
        }
        if let Some(bin) = &self.bin_path {
            session.set_env("PAR_MUX_BIN", bin);
        }
        // Before the spawn: the reader thread reads this slot from its
        // first read(), so no output can precede the sink.
        if let Some(OutputSink(sink)) = context.output {
            session.set_output_callback(Arc::clone(sink));
        }
        session
    }

    /// Bind this pane's hook-only endpoint (ENH-039) when endpoint mode is
    /// on. `None` = mode off (silent), or the bind failed — the per-daemon
    /// cap, or the path over the platform socket-address limit — which is
    /// logged once here; either way the pane's env contract exports no
    /// socket rather than falling back to the full control socket.
    fn bind_pane_endpoint(&self, id: PaneId) -> Option<crate::mux::server::PaneEndpoint> {
        let control = self.socket_path.as_deref()?;
        let tx = self.pane_endpoint_tx.as_ref()?;
        match crate::mux::server::PaneEndpoint::bind(std::path::Path::new(control), id, tx.clone())
        {
            Ok(endpoint) => Some(endpoint),
            Err(err) => {
                log::warn!(
                    "par-mux: no hook-only endpoint for {id} ({err}); exporting no PAR_MUX_SOCKET"
                );
                None
            }
        }
    }

    /// The shared back half: wrap a spawned session as a pane.
    fn finish_pane(
        id: PaneId,
        session: PtySession,
        spawn_command: Option<String>,
        pane_endpoint: Option<crate::mux::server::PaneEndpoint>,
    ) -> MuxPane {
        // Client mirrors rebuild their grid from the raw PTY bytes forwarded
        // over %output — this terminal's read of a kitty t=t temp file must
        // not delete it, or the mirrors' later read finds nothing. The
        // client that renders the graphic deletes it. The file-media gate
        // (SEC-101) lives in the *client's* terminal state: the daemon's
        // read only passes for a spec-named file under a temp root, and
        // the rendering client's delete re-checks the same gate.
        session.terminal().write().set_retain_kitty_temp_files(true);
        MuxPane {
            id,
            session,
            spawn_command,
            user_title: None,
            dead: false,
            exit_code: None,
            metadata: HashMap::new(),
            telemetry: None,
            host_telemetry: None,
            seq_by_source: HashMap::new(),
            snapshot_cache: Arc::new(Mutex::new(None)),
            pane_endpoint,
        }
    }
}

impl PaneFactory for ShellPaneFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        let endpoint = self.bind_pane_endpoint(id);
        let pane_socket = endpoint.as_ref().map(|e| e.socket_path_string());
        let mut session = self.configured_session(id, cols, rows, context, pane_socket.as_deref());

        match command {
            Some(cmd) => {
                let shell = PtySession::get_default_shell();
                // POSIX shells take `-c <cmd>`; cmd.exe takes `/C <cmd>` —
                // with `-c` it ignores the flag and drops to an interactive
                // prompt, so the pane's command never runs on Windows.
                #[cfg(windows)]
                let run_flag = "/C";
                #[cfg(not(windows))]
                let run_flag = "-c";
                session.spawn(&shell, &[run_flag, cmd])?;
            }
            None => session.spawn_shell()?,
        }

        Ok(Self::finish_pane(
            id,
            session,
            command.map(str::to_string),
            endpoint,
        ))
    }

    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<MuxPane, MuxError> {
        // Never spawned: no process, no reader, and no hook-only endpoint
        // (a dead pane has no child to hand one to; `respawn-pane` goes
        // through `create_pane`, which binds it).
        let session = self.configured_session(id, cols, rows, &SpawnContext::default(), None);
        let mut pane = Self::finish_pane(id, session, command.map(str::to_string), None);
        pane.mark_dead_with_code(exit_code);
        Ok(pane)
    }

    /// Windows: the resume argv spawns without a shell when `argv[0]`
    /// resolves to a PE image, and through a self-deleting cmd bridge for
    /// the npm `.cmd` shims — the transport choice lives in
    /// [`win_resume`]. The recorded `spawn_command` stays the POSIX
    /// rendering: it round-trips identity through persistence and is
    /// never read back for execution.
    #[cfg(windows)]
    fn create_argv_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        argv: &[String],
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        if argv.is_empty() {
            return self.create_pane(id, cols, rows, None, context);
        }
        let endpoint = self.bind_pane_endpoint(id);
        let pane_socket = endpoint.as_ref().map(|e| e.socket_path_string());
        let mut session = self.configured_session(id, cols, rows, context, pane_socket.as_deref());
        super::win_resume::spawn_resume_argv(&mut session, argv)?;
        Ok(Self::finish_pane(
            id,
            session,
            Some(crate::mux::agent_resume::render_argv(argv)),
            endpoint,
        ))
    }
}

/// The Phase 5 agent layer's factory (seam S1): an agent CLI pane.
///
/// Wraps [`ShellPaneFactory`]'s spawn path — same PTY plumbing, same env
/// contract — and adds the one thing that distinguishes an agent pane:
/// `metadata["agent"]` tagged at spawn, so the pane shows up in the roster
/// (and, in Phase 6, in the persisted layout) without waiting for its first
/// hook report. Embedders and the Phase 6 resume path construct trees with
/// this factory; the daemon's default stays [`ShellPaneFactory`], because an
/// agent pane is just a command pane until a hook claims it. Hook
/// INSTALLATION (writing the agent's own hook config) is out of scope by
/// design.
pub struct AgentPaneFactory {
    /// The agent label recorded as `metadata["agent"]` — what `list-agents`
    /// and `%agent-state-changed` report.
    pub agent: String,
    /// Working directory for new panes; the process default when `None`.
    pub cwd: Option<std::path::PathBuf>,
    /// Control-socket path exported with the hook env contract (see
    /// [`ShellPaneFactory::socket_path`]).
    pub socket_path: Option<String>,
    /// Daemon executable exported as `PAR_MUX_BIN` (see
    /// [`ShellPaneFactory::bin_path`]).
    pub bin_path: Option<String>,
}

impl AgentPaneFactory {
    /// Spawn through a [`ShellPaneFactory`] carrying this factory's cwd,
    /// socket and bin path, then tag the pane's agent identity — the body
    /// both spawn paths share (QA-214).
    fn spawn_tagged(
        &self,
        spawn: impl FnOnce(&ShellPaneFactory) -> Result<MuxPane, MuxError>,
    ) -> Result<MuxPane, MuxError> {
        let shell = ShellPaneFactory {
            cwd: self.cwd.clone(),
            socket_path: self.socket_path.clone(),
            bin_path: self.bin_path.clone(),
            ..Default::default()
        };
        let mut pane = spawn(&shell)?;
        pane.set_metadata("agent", &self.agent);
        Ok(pane)
    }
}

impl PaneFactory for AgentPaneFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        self.spawn_tagged(|shell| shell.create_pane(id, cols, rows, command, context))
    }

    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<MuxPane, MuxError> {
        self.spawn_tagged(|shell| shell.create_dead_pane(id, cols, rows, command, exit_code))
    }

    /// Delegates to [`ShellPaneFactory`]'s argv path so a Windows resume
    /// keeps the shell-free transport, then tags the agent identity the
    /// same way [`AgentPaneFactory::create_pane`] does.
    fn create_argv_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        argv: &[String],
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        self.spawn_tagged(|shell| shell.create_argv_pane(id, cols, rows, argv, context))
    }
}

/// The live working directory of process `pid`, for panes whose shell never
/// reported OSC 7. Best-effort by design: a reaped or reparented child, a
/// sandboxed reader, or an OS without a pid→cwd path all yield `None`, and
/// the caller falls back to the spawn-time default.
#[cfg(target_os = "linux")]
fn process_cwd(pid: u32) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(target_os = "macos")]
fn process_cwd(pid: u32) -> Option<std::path::PathBuf> {
    // proc_pidinfo writes a vnode path — the kernel-side equivalent of
    // Linux's /proc/<pid>/cwd symlink.
    // SAFETY: `proc_vnodepathinfo` is a plain C struct, so all-zero is a valid
    // value. proc_pidinfo gets a pointer to that live local plus its exact
    // size, so the kernel writes only inside it. The final slice reinterprets
    // `c_char` as `u8` (same size and alignment) over `bytes[..end]`, where
    // `end` is a NUL index found inside `bytes`, which borrows `info` for the
    // slice's whole lifetime.
    unsafe {
        let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
        let size = libc::proc_pidinfo(
            pid as libc::pid_t,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int,
        );
        if size <= 0 {
            return None;
        }
        // libc models the flat `[c_char; MAXPATHLEN]` as `[[c_char; 32]; 32]`
        // for old-rustc compatibility; flatten before scanning for the NUL.
        let bytes = info.pvi_cdir.vip_path.as_flattened();
        let end = bytes.iter().position(|&b| b == 0)?;
        let raw = std::slice::from_raw_parts(bytes.as_ptr() as *const u8, end);
        std::str::from_utf8(raw).ok().map(std::path::PathBuf::from)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_cwd(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

/// This machine's hostname, or `None` when the OS will not say.
#[cfg(unix)]
pub(crate) fn local_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is a live, writable 256-byte buffer and the length
    // passed is its exact size, so gethostname(3) writes only inside it.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }
    // POSIX leaves a truncated name unterminated; treat that as unknown.
    let end = buf.iter().position(|&b| b == 0)?;
    let name = std::str::from_utf8(&buf[..end]).ok()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// This machine's hostname, or `None` when the OS will not say.
#[cfg(windows)]
pub(crate) fn local_hostname() -> Option<String> {
    std::env::var("COMPUTERNAME").ok().filter(|n| !n.is_empty())
}

/// This machine's hostname, or `None` when the OS will not say.
#[cfg(not(any(unix, windows)))]
pub(crate) fn local_hostname() -> Option<String> {
    None
}

/// Whether an OSC 7 hostname names this machine (SEC-128). `None` is local:
/// the parser already folds an empty host and `localhost` into it. Any
/// other name must match the local hostname, full or short form, case
/// insensitively. Shells commonly emit `file://$HOSTNAME/path`, so `None`
/// alone would reject every local zsh/bash integration. An unknown local
/// name trusts nothing.
fn osc7_host_is_local(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return true;
    };
    let Some(local) = local_hostname() else {
        return false;
    };
    let short = |name: &str| name.split('.').next().unwrap_or(name).to_string();
    host.eq_ignore_ascii_case(&local) || short(host).eq_ignore_ascii_case(&short(&local))
}

/// A factory recording the [`SpawnContext`] of every spawn, for tests that
/// assert what each tree path hands the factory.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// One spawn's context, owned.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct RecordedSpawn {
        pub pane: PaneId,
        pub session: Option<(SessionId, String)>,
        pub window: Option<WindowId>,
        pub env: BTreeMap<String, String>,
        pub cwd: Option<std::path::PathBuf>,
    }

    /// Records each spawn's context, then spawns a bounded sleeper.
    #[derive(Clone, Default)]
    pub(crate) struct ContextRecordingFactory {
        pub spawns: Arc<Mutex<Vec<RecordedSpawn>>>,
    }

    impl ContextRecordingFactory {
        pub(crate) fn spawn_of(&self, pane: PaneId) -> RecordedSpawn {
            self.spawns
                .lock()
                .iter()
                .find(|s| s.pane == pane)
                .cloned()
                .unwrap_or_else(|| panic!("no spawn recorded for {pane}"))
        }
    }

    impl PaneFactory for ContextRecordingFactory {
        fn create_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            _command: Option<&str>,
            context: &SpawnContext<'_>,
        ) -> Result<MuxPane, MuxError> {
            self.spawns.lock().push(RecordedSpawn {
                pane: id,
                session: context.session.map(|(id, name)| (id, name.to_string())),
                window: context.window,
                env: context.env.cloned().unwrap_or_default(),
                cwd: context.cwd.map(std::path::Path::to_path_buf),
            });
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn shell_factory_creates_a_running_pane() {
        let factory = ShellPaneFactory::default();
        let pane = factory
            .create_pane(PaneId(0), 80, 24, None, &SpawnContext::default())
            .expect("shell pane should spawn");
        assert_eq!(pane.id(), PaneId(0));
        assert!(
            pane.is_running(),
            "a freshly spawned shell should be running"
        );
        assert!(pane.child_pid().is_some(), "a spawned pane has a child pid");
    }

    #[test]
    fn factory_honors_an_explicit_command() {
        let factory = ShellPaneFactory::default();
        let pane = factory
            .create_pane(
                PaneId(1),
                80,
                24,
                Some("echo par-mux"),
                &SpawnContext::default(),
            )
            .expect("command pane should spawn");
        assert!(pane.child_pid().is_some());
    }

    #[test]
    fn output_callback_receives_pty_bytes() {
        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(
                PaneId(2),
                80,
                24,
                Some("echo par-mux-marker"),
                &SpawnContext::default(),
            )
            .expect("pane should spawn");

        let seen = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&seen);
        pane.on_output(move |bytes: &[u8]| {
            if !bytes.is_empty() {
                counter.fetch_add(bytes.len(), Ordering::Relaxed);
            }
        });

        // The shell needs a moment to run and flush. Poll rather than sleep a
        // fixed duration so a fast machine does not wait and a slow one does.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while seen.load(Ordering::Relaxed) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert!(
            seen.load(Ordering::Relaxed) > 0,
            "output callback should have received PTY bytes within 5s"
        );
    }

    /// ARC-103: a sink carried in the spawn context is live before the
    /// reader starts, so output printed immediately at spawn is forwarded
    /// without any `on_output` call.
    #[cfg(unix)]
    #[test]
    fn spawn_context_output_sink_sees_the_first_byte() {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&collected);
        let sink = OutputSink(Arc::new(move |bytes: &[u8]| {
            into.lock().extend_from_slice(bytes)
        }));
        let _pane = ShellPaneFactory::default()
            .create_pane(
                PaneId(12),
                80,
                24,
                Some("printf FIRST-BYTE-MARK; exec sleep 5"),
                &SpawnContext {
                    output: Some(&sink),
                    ..SpawnContext::default()
                },
            )
            .expect("pane should spawn");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !String::from_utf8_lossy(&collected.lock()).contains("FIRST-BYTE-MARK") {
            assert!(
                std::time::Instant::now() < deadline,
                "the context sink never saw the first output: {:?}",
                String::from_utf8_lossy(&collected.lock())
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    /// ARC-089: a killed pane's exit output (a SIGHUP trap, a TUI's
    /// alt-screen exit) must not reach its sink — the id may already
    /// belong to a respawned replacement.
    #[cfg(unix)]
    #[test]
    fn a_killed_panes_late_output_is_not_forwarded() {
        // `sleep & wait`, not a foreground sleep: the shell runs a trap only
        // after its foreground child returns, which a 1 s sleep delays past
        // portable-pty's ~200 ms SIGHUP grace, so SIGKILL would win and the
        // test would pass without the fix. `wait` returns on the signal.
        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(
                PaneId(3),
                80,
                24,
                Some("trap 'echo OLD-PANE-BYE; exit 0' HUP; echo READY-MARK; while :; do sleep 5 & wait; done"),
                &SpawnContext::default(),
            )
            .expect("pane should spawn");
        let collected = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&collected);
        pane.on_output(move |bytes: &[u8]| sink.lock().extend_from_slice(bytes));

        // The marker proves the trap is installed; a SIGHUP before it
        // kills the shell without running the trap. Read it off the screen:
        // the shell can print it before `on_output` registers.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !pane.terminal().read().content().contains("READY-MARK") {
            assert!(
                std::time::Instant::now() < deadline,
                "the ready marker never reached the pane screen"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        pane.kill().expect("kill succeeds");
        std::thread::sleep(std::time::Duration::from_millis(300));

        let seen = String::from_utf8_lossy(&collected.lock()).into_owned();
        assert!(
            !seen.contains("OLD-PANE-BYE"),
            "the dying process's output was forwarded: {seen:?}"
        );
    }

    /// Card 01a0d9e6f2ef70e383213a2911689228: a kitty `t=t` (temp file)
    /// image must render in a mux pane. The daemon-side terminal processes
    /// the PTY bytes first, but client mirrors rebuild their grid from the
    /// same raw bytes forwarded over `%output` — and the t=t contract has
    /// the reading terminal delete the file. If the daemon's read deletes
    /// it, no client can ever load the graphic. The daemon must retain the
    /// file; the client mirror that actually renders it is the one whose
    /// read deletes it.
    ///
    /// Unix-only, measured on the Windows 11 VM (card 01a0defb, 2026-09-26):
    /// conhost's VT parser consumes an APC (`ESC _ … ESC \`) and never
    /// re-emits it to the ConPTY reader. A pane child that printed
    /// `ALIVE`, the escape, `DONE` delivered both markers to the daemon
    /// sink with the escape missing — through a cmd /C line, a
    /// `powershell -Command` argv, and a `powershell -File` script alike
    /// (the persist resume control passes on the same transport in
    /// 0.34s). No pane child on Windows can deliver kitty APC over the
    /// PTY, so the daemon-retain/mirror-delete contract is asserted where
    /// the medium exists.
    #[cfg(unix)]
    #[test]
    fn kitty_temp_file_graphic_survives_for_client_mirrors() {
        use base64::Engine as _;

        // A real 2x1 PNG so decode_pixels succeeds in both terminals.
        let img = image::RgbaImage::from_pixel(2, 1, image::Rgba([9, 8, 7, 6]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        // The t=t sender's temp file. The escape carries its path
        // base64-encoded — the wire payload is always base64; the file
        // medium resolves the path after decode. Real senders name the
        // file per kitty's spec, which the SEC-101 file-media gate
        // requires before the daemon terminal will load (and a client
        // delete) it.
        let mut temp = tempfile::Builder::new()
            .prefix("tty-graphics-protocol-")
            .tempfile()
            .expect("temp file");
        std::io::Write::write_all(&mut temp, &png).unwrap();
        let path = temp.path().to_path_buf();
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(path.to_string_lossy().as_bytes());
        // Single-quoted printf: the path is tempfile-safe (alphanumerics,
        // '/', '.', '_', '-') and the escape contains no single quotes.
        // The leading sleep keeps the child's output behind the test's
        // sink registration — create_pane spawns before on_output can be
        // called, so an immediate printf races the callback wiring.
        let command = format!("sleep 1; printf '%s' '\x1b_Ga=T,f=100,t=t;{encoded}\x1b\\'");

        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(PaneId(9), 80, 24, Some(&command), &SpawnContext::default())
            .expect("pane should spawn");

        // Capture exactly what pane_output_sink would forward to clients.
        let sink_bytes = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&sink_bytes);
        pane.on_output(move |bytes: &[u8]| captured.lock().extend_from_slice(bytes));

        // Wait until the sink holds the printf's complete escape — it ends
        // with the ST terminator. Under the reordered reader loop the output
        // callback fires after the bytes are applied to the daemon terminal,
        // so complete sink bytes imply the graphic is already in the
        // daemon's store.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !sink_bytes.lock().ends_with(b"\x1b\\") && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let sink_text = String::from_utf8_lossy(&sink_bytes.lock().clone()).to_string();
        assert_eq!(
            pane.terminal()
                .read()
                .graphics
                .graphics_store
                .all_graphics()
                .len(),
            1,
            "daemon terminal should hold the t=t graphic; sink bytes so far: {sink_text:?}"
        );

        assert!(
            path.exists(),
            "daemon processing must not delete the t=t file client mirrors still need to read"
        );

        // The client mirror replays the forwarded bytes and must render the
        // same graphic — a plain Terminal, exactly what a mux client embeds.
        let mut mirror = crate::terminal::Terminal::new(80, 24);
        mirror.process(&sink_bytes.lock().clone());
        assert_eq!(
            mirror.graphics.graphics_store.all_graphics().len(),
            1,
            "client mirror must render the t=t graphic from forwarded bytes"
        );

        assert!(
            !path.exists(),
            "the rendering client mirror deletes the temp file after its read"
        );
    }

    #[test]
    fn metadata_starts_empty_and_accepts_entries() {
        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(PaneId(3), 80, 24, None, &SpawnContext::default())
            .unwrap();
        assert!(
            pane.metadata().is_empty(),
            "metadata starts empty (seam S2)"
        );
        pane.set_metadata("agent", "claude");
        assert_eq!(
            pane.metadata().get("agent").map(String::as_str),
            Some("claude")
        );
    }

    /// ARC-113c: the typed claim renders to exactly the stringly keys the
    /// roster and the save format read — including the agent_seq rename and
    /// the compact JSON argv — and parses back losslessly.
    #[test]
    fn agent_claim_round_trips_through_the_stringly_map() {
        use crate::mux::hooks::{AgentClaim, AGENT_CLAIM_KEYS};

        let full = AgentClaim {
            agent: "pi".to_string(),
            state: Some("blocked".to_string()),
            state_source: Some("hook".to_string()),
            message: Some("need approval".to_string()),
            source: Some("par-mux:pi".to_string()),
            seq: Some(1000),
            session_id: Some("s-1".to_string()),
            session_path: Some("/tmp/pi-session.jsonl".to_string()),
            session_start_source: Some("startup".to_string()),
            resume_argv: Some(vec!["pi".to_string(), "--session".to_string()]),
            liveness_misses: Some(2),
            liveness_misses_agent: Some("pi".to_string()),
        };
        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(PaneId(30), 80, 24, None, &SpawnContext::default())
            .unwrap();
        pane.set_agent_claim(&full);

        // The wire-and-disk shape: the claim's stringly rendering is the
        // legacy metadata spelling, byte for byte.
        assert_eq!(pane.metadata().len(), AGENT_CLAIM_KEYS.len());
        for key in AGENT_CLAIM_KEYS {
            assert!(
                pane.metadata().contains_key(*key),
                "full claim write must spell {key}"
            );
        }
        assert_eq!(
            pane.metadata().get("agent_seq").map(String::as_str),
            Some("1000"),
            "seq renders under the legacy agent_seq key"
        );
        assert_eq!(
            pane.metadata().get("agent_resume_argv").map(String::as_str),
            Some(r#"["pi","--session"]"#),
            "argv renders as the compact JSON string the report path stores"
        );
        assert_eq!(pane.agent_claim().as_ref(), Some(&full));

        // A cleared field's key is REMOVED, not empty-stringed.
        let mut cleared = full.clone();
        cleared.state = None;
        cleared.message = None;
        cleared.liveness_misses = None;
        pane.set_agent_claim(&cleared);
        assert!(!pane.metadata().contains_key("agent_state"));
        assert!(!pane.metadata().contains_key("agent_message"));
        assert!(!pane.metadata().contains_key("agent_liveness_misses"));
        assert_eq!(
            pane.metadata()
                .get("agent_state_source")
                .map(String::as_str),
            Some("hook"),
            "untouched fields survive the whole-claim write"
        );
    }

    /// The typed view is lenient by design: a pane whose claim carries a
    /// corrupt counter or argv reads those fields as absent instead of
    /// failing the whole claim.
    #[test]
    fn unparseable_numeric_and_argv_values_read_as_absent() {
        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(PaneId(31), 80, 24, None, &SpawnContext::default())
            .unwrap();
        pane.set_metadata("agent", "omp");
        pane.set_metadata("agent_seq", "not-a-number");
        pane.set_metadata("agent_liveness_misses", "99999");
        pane.set_metadata("agent_resume_argv", "[not json");
        let claim = pane.agent_claim().expect("the agent label is present");
        assert_eq!(claim.agent, "omp");
        assert_eq!(claim.seq, None);
        assert_eq!(claim.liveness_misses, None, "99999 overflows u8");
        assert_eq!(claim.resume_argv, None);
    }

    /// The serde form is the migration representation for a future
    /// claim-shaped block in the state file: agent_seq's rename is spelled
    /// once, on the field, and absent Options are skipped.
    #[test]
    fn agent_claim_serializes_to_the_migration_representation() {
        use crate::mux::hooks::AgentClaim;

        let minimal = AgentClaim {
            agent: "claude".to_string(),
            ..AgentClaim::default()
        };
        let json = serde_json::to_value(&minimal).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"agent": "claude"}),
            "absent fields are skipped so the representation stays stable"
        );

        let with_seq = AgentClaim {
            seq: Some(7),
            ..minimal
        };
        let json = serde_json::to_value(&with_seq).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"agent": "claude", "agent_seq": 7}),
            "seq travels under its legacy key name"
        );
    }

    #[test]
    fn agent_factory_tags_the_agent_at_spawn() {
        let factory = AgentPaneFactory {
            agent: "kimi".to_string(),
            cwd: None,
            socket_path: None,
            bin_path: None,
        };
        let pane = factory
            .create_pane(
                PaneId(5),
                80,
                24,
                Some("sleep 30"),
                &SpawnContext::default(),
            )
            .expect("agent pane should spawn");
        assert!(pane.is_running(), "the agent CLI pane runs");
        assert!(pane.child_pid().is_some(), "it has a child pid");
        assert_eq!(
            pane.metadata().get("agent").map(String::as_str),
            Some("kimi"),
            "metadata[agent] is tagged at spawn (seam S1)"
        );
    }

    #[test]
    fn agent_factory_seeds_the_hook_env_contract() {
        // Nothing ever binds or writes to this path — the pane only echoes
        // it back via $PAR_MUX_SOCKET — but a `process::id()`-derived path
        // in the shared temp dir still repeats once the OS recycles a pid,
        // so a `TempDir` keeps this consistent with the sibling fixtures.
        let dir = tempfile::Builder::new()
            .prefix("par-mux-agent-env-")
            .tempdir()
            .expect("create temp dir for socket path");
        let socket = dir.path().join("socket").display().to_string();
        let factory = AgentPaneFactory {
            agent: "kimi".to_string(),
            cwd: None,
            socket_path: Some(socket.clone()),
            bin_path: None,
        };
        // Variable expansion syntax is the shell's: $VAR under POSIX sh,
        // %VAR% under cmd.exe (the pane command runs via the platform's
        // default shell — see ShellPaneFactory::create_pane).
        #[cfg(windows)]
        let echo_env = "echo AGENV=%PAR_MUX_ENV%/%PAR_MUX_PANE_ID%/%PAR_MUX_SOCKET%";
        #[cfg(not(windows))]
        let echo_env = "echo AGENV=$PAR_MUX_ENV/$PAR_MUX_PANE_ID/$PAR_MUX_SOCKET";
        let mut pane = factory
            .create_pane(PaneId(6), 80, 24, Some(echo_env), &SpawnContext::default())
            .expect("agent pane should spawn");

        // Collect the pane's output until the env line lands — the child
        // sees the contract, which is what a ported hook script keys on.
        let seen: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        pane.on_output(move |bytes: &[u8]| sink.lock().extend_from_slice(bytes));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let text = String::from_utf8_lossy(&seen.lock().clone()).to_string();
            if text.contains(&format!("AGENV=1/%6/{socket}")) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child never saw the env contract; output so far: {text}"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    #[test]
    fn resize_updates_the_terminal_dimensions() {
        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(PaneId(4), 80, 24, None, &SpawnContext::default())
            .unwrap();
        pane.resize(100, 30).expect("resize should succeed");
        let terminal = pane.terminal();
        let guard = terminal.read();
        assert_eq!(guard.size(), (100, 30));
    }

    /// Drive the pane terminal's OSC title directly (the same bytes the
    /// pane's program would emit) so the precedence tests do not depend on
    /// PTY scheduling.
    fn set_osc_title(pane: &MuxPane, title: &str) {
        pane.terminal()
            .write()
            .process(format!("\x1b]2;{title}\x07").as_bytes());
    }

    #[test]
    fn without_a_user_title_the_osc_title_is_reported() {
        let factory = ShellPaneFactory::default();
        let pane = factory
            .create_pane(PaneId(7), 80, 24, None, &SpawnContext::default())
            .unwrap();
        set_osc_title(&pane, "prog title");
        assert_eq!(pane.effective_title(), "prog title");
        assert!(
            pane.user_title().is_none(),
            "an OSC title is not a user title"
        );
    }

    #[test]
    fn a_user_title_survives_a_later_osc_title() {
        // The documented divergence from tmux: -T is sticky; the program
        // cannot overwrite it.
        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(PaneId(8), 80, 24, None, &SpawnContext::default())
            .unwrap();
        assert!(pane.set_user_title("user title"));
        set_osc_title(&pane, "later prog title");
        assert_eq!(pane.effective_title(), "user title");

        // And clearing the user title falls back to the OSC title again.
        assert!(pane.set_user_title(""));
        assert_eq!(pane.effective_title(), "later prog title");
        assert!(pane.user_title().is_none());
    }

    #[test]
    fn setting_the_same_title_twice_reports_no_change() {
        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(PaneId(9), 80, 24, None, &SpawnContext::default())
            .unwrap();
        assert!(pane.set_user_title("same"));
        assert!(
            !pane.set_user_title("same"),
            "an identical title is not a change"
        );
        assert!(pane.set_user_title(""), "clearing a set title is a change");
        assert!(
            !pane.set_user_title(""),
            "clearing an already-clear pane is not a change"
        );
    }

    #[test]
    fn spawn_context_cwd_reaches_the_child_process() {
        let dir = tempfile::tempdir().unwrap();
        let factory = ShellPaneFactory::default();
        let context = SpawnContext {
            cwd: Some(dir.path()),
            ..SpawnContext::default()
        };
        // `pwd` under the POSIX shell, bare `cd` under cmd.exe (the spawn
        // wraps pane commands in `cmd.exe /C` on Windows) — each prints the
        // child's cwd, proving the context directory reached the process.
        // cmd.exe may print that path through a junction (C:\WINDOWS\TEMP
        // resolving to C:\Windows\SystemTemp) and with its own casing, so
        // the WITNESS is the tempdir's unique random leaf, not the full
        // string: the probe prints only the cwd, and no other directory
        // carries this run's leaf name.
        #[cfg(unix)]
        let probe = "pwd";
        #[cfg(windows)]
        let probe = "cd";
        let leaf = dir
            .path()
            .file_name()
            .expect("tempdir path has a leaf")
            .to_string_lossy()
            .to_string();
        let mut pane = factory
            .create_pane(PaneId(11), 80, 24, Some(probe), &context)
            .expect("pane should spawn");
        let seen: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        pane.on_output(move |bytes: &[u8]| sink.lock().extend_from_slice(bytes));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let text = String::from_utf8_lossy(&seen.lock().clone()).to_string();
            if text.contains(&leaf) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child never ran in the context cwd ({leaf}); probe said: {text}"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    #[test]
    fn spawn_context_reaches_the_child_environment() {
        let factory = ShellPaneFactory {
            bin_path: Some("/opt/par-mux-bin".to_string()),
            ..ShellPaneFactory::default()
        };
        let context = SpawnContext {
            session: Some((SessionId(4), "work")),
            window: Some(WindowId(7)),
            env: None,
            cwd: None,
            output: None,
        };
        #[cfg(windows)]
        let echo_env =
            "echo IDENT=%PAR_MUX_SESSION_ID%/%PAR_MUX_SESSION%/%PAR_MUX_WINDOW_ID%/%PAR_MUX_BIN%";
        #[cfg(not(windows))]
        let echo_env =
            "echo IDENT=$PAR_MUX_SESSION_ID/$PAR_MUX_SESSION/$PAR_MUX_WINDOW_ID/$PAR_MUX_BIN";
        let mut pane = factory
            .create_pane(PaneId(10), 80, 24, Some(echo_env), &context)
            .expect("pane should spawn");
        let seen: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        pane.on_output(move |bytes: &[u8]| sink.lock().extend_from_slice(bytes));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let text = String::from_utf8_lossy(&seen.lock().clone()).to_string();
            if text.contains("IDENT=$4/work/@7//opt/par-mux-bin") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child never saw the identity vars; output so far: {text}"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    /// SEC-128: an OSC 7 host is local only when it is absent (the parser's
    /// fold of empty/`localhost`) or matches this machine's name, full or
    /// short form, in any case.
    #[test]
    fn osc7_host_is_local_matches_only_this_machine() {
        assert!(osc7_host_is_local(None));
        let local = local_hostname().expect("hostname");
        let short = local.split('.').next().unwrap().to_string();
        assert!(osc7_host_is_local(Some(&local)));
        assert!(osc7_host_is_local(Some(&local.to_uppercase())));
        assert!(osc7_host_is_local(Some(&short)));
        assert!(osc7_host_is_local(Some(&format!(
            "{short}.example.invalid"
        ))));
        assert!(!osc7_host_is_local(Some("remote.invalid")));
        assert!(!osc7_host_is_local(Some(&format!("{short}x"))));
    }

    /// SEC-125: a held (remain-on-exit) pane keeps its frozen screen but
    /// not its reaped child's PID, so `pane-info cmd=`, the host probe,
    /// scrape liveness and respawn's cwd fallback cannot reach a PID the OS
    /// may have handed to another process.
    #[test]
    fn a_held_dead_pane_serves_no_child_pid() {
        let mut pane = ShellPaneFactory::default()
            .create_pane(PaneId(20), 80, 24, Some("exit 3"), &SpawnContext::default())
            .expect("pane should spawn");
        // The reader's EOF can flip `poll_running` before the child is a
        // reapable zombie, so re-run the reaper's step until the code lands.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if !pane.poll_running() {
                pane.mark_dead();
                if pane.exit_code().is_some() {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the pane's child was never reaped"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(pane.dead());
        assert_eq!(pane.exit_code(), Some(3));
        assert!(pane.child_pid().is_none(), "a reaped PID is not served");
        assert!(pane.persistence_cwd().is_none(), "no OSC 7, no live child");
        assert!(pane.snapshot_capture_parts().probe_cwd().is_none());
        assert!(pane.kill().is_ok(), "killing a held pane signals nothing");
    }
}
