//! Panes: PTY ownership, output plumbing, and the factory seam.

use crate::mux::agent_resume::render_surviving;
use crate::mux::ids::{PaneId, SessionId, WindowId};
use crate::pty_error::PtyError;
use crate::pty_session::PtySession;
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
    /// The two panes are not in the same window, so their positions cannot
    /// be exchanged.
    PanesInDifferentWindows(PaneId, PaneId),
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
            MuxError::PanesInDifferentWindows(a, b) => {
                write!(f, "panes {a} and {b} are in different windows")
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
    metadata: HashMap<String, String>,
    /// Last persistence snapshot, valid while the terminal has not changed
    /// since it was taken — see [`MuxPane::persisted_snapshot`].
    snapshot_cache: Mutex<Option<SnapshotCacheEntry>>,
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

    /// The terminal emulator backing this pane.
    pub fn terminal(&self) -> Arc<RwLock<Terminal>> {
        self.session.terminal()
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

    /// The pane's persistence snapshot, reusing the cached capture while the
    /// terminal has not changed since it was taken.
    ///
    /// Saves capture every pane on every structural command, but a pane only
    /// changes through PTY output (which bumps the session's update
    /// generation) or a resize (which changes the size) — so an idle pane
    /// costs one `Vec<Cell>` clone instead of a full grid walk. Keyed on
    /// both, per [`SnapshotCacheKey`].
    pub fn persisted_snapshot(&self) -> TerminalSnapshot {
        let terminal = self.session.terminal();
        let generation = self.session.update_generation();
        let (cols, rows) = terminal.read().size();
        let key = SnapshotCacheKey {
            generation,
            cols,
            rows,
        };
        let mut cache = self.snapshot_cache.lock();
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

    /// Whether the pane's child process is still running.
    pub fn is_running(&self) -> bool {
        self.session.is_running()
    }

    /// Liveness for the reaper's periodic pass — the reader flag plus the OS
    /// child handle. On Windows ConPTY the reader never observes EOF after
    /// the child exits, so [`Self::is_running`] alone would leave an exited
    /// pane in the tree forever; see [`PtySession::poll_running`].
    pub fn poll_running(&mut self) -> bool {
        self.session.poll_running()
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

    /// Write client input to the pane's PTY.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), MuxError> {
        self.session.write(bytes).map_err(MuxError::from)
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
            .resize_with_pixels(cols, rows, cols * cell_w, rows * cell_h)
            .map_err(MuxError::from)
    }

    /// Terminate the pane's child process.
    pub fn kill(&mut self) -> Result<(), MuxError> {
        self.session.kill().map_err(MuxError::from)
    }
}

/// Where a new pane lands: the identity and environment its spawn inherits.
///
/// The tree builds one per spawn from the session and window the pane is
/// created in. [`Default`] is a pane outside any session (tests, embedders
/// driving a factory directly): no identity vars, no session environment,
/// no per-spawn cwd.
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
        for (name, value) in context.env.into_iter().flatten() {
            session.set_env(name, value);
        }
        session.set_env("PAR_MUX_PANE_ID", &id.to_string());
        if let Some(socket) = &self.socket_path {
            session.set_env("PAR_MUX_SOCKET", socket);
            session.set_env("PAR_MUX_ENV", "1");
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
        session
    }

    /// The shared back half: wrap a spawned session as a pane.
    fn finish_pane(id: PaneId, session: PtySession, spawn_command: Option<String>) -> MuxPane {
        MuxPane {
            id,
            session,
            spawn_command,
            user_title: None,
            metadata: HashMap::new(),
            snapshot_cache: Mutex::new(None),
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
        let mut session = self.configured_session(id, cols, rows, context);

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

        Ok(Self::finish_pane(id, session, command.map(str::to_string)))
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
        let mut session = self.configured_session(id, cols, rows, context);
        super::win_resume::spawn_resume_argv(&mut session, argv)?;
        Ok(Self::finish_pane(
            id,
            session,
            Some(crate::mux::agent_resume::render_argv(argv)),
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

impl PaneFactory for AgentPaneFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        let shell = ShellPaneFactory {
            cwd: self.cwd.clone(),
            socket_path: self.socket_path.clone(),
            bin_path: self.bin_path.clone(),
        };
        let mut pane = shell.create_pane(id, cols, rows, command, context)?;
        pane.set_metadata("agent", &self.agent);
        Ok(pane)
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
        let shell = ShellPaneFactory {
            cwd: self.cwd.clone(),
            socket_path: self.socket_path.clone(),
            bin_path: self.bin_path.clone(),
        };
        let mut pane = shell.create_argv_pane(id, cols, rows, argv, context)?;
        pane.set_metadata("agent", &self.agent);
        Ok(pane)
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
}
