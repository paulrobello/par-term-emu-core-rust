//! PTY session management.
//!
//! `PtySession` owns a shell process running under a pseudoterminal and feeds
//! its output into a [`Terminal`]. All terminal state lives behind an
//! `Arc<RwLock<Terminal>>` (using `parking_lot::RwLock` for performance and to
//! avoid lock poisoning), shared between the public API and a background
//! reader thread: the reader thread takes the write lock to `process()` PTY
//! output, while API queries take read locks.
//!
//! ## Reader thread
//!
//! A dedicated background thread reads raw bytes from the PTY master and calls
//! `term.process(..)` while holding the terminal lock. It also forwards
//! device-query responses (DA/DSR/DECRQM/etc.) back to the child through the
//! shared writer. Raw output callbacks (if registered) fire after each read
//! is applied to the terminal, so callback consumers that also read terminal
//! state never see bytes the state lacks; the bytes themselves are passed
//! unmodified, for recording, logging, or streaming to clients.
//!
//! ## Lifetime signaling
//!
//! The `running` flag is an `Arc<AtomicBool>` that the session sets to `true`
//! on spawn and clears on PTY EOF or when `try_wait()` observes an exited
//! child. It is a **best-effort** indicator: there is a window where the child
//! has exited but the flag has not yet been cleared. For a precise exit
//! status, call `try_wait()` (non-blocking) or `wait()` (blocking) instead.
//!
//! ## Generation counter
//!
//! A monotonically increasing generation counter is bumped whenever observable
//! terminal state changes (new output, resize, mode change, etc.). Callers
//! that poll state can compare the generation to detect whether anything has
//! changed since their last read without diffing the full grid.

use crate::coprocess::{CoprocessConfig, CoprocessId, CoprocessManager};
use crate::debug;
use crate::pty_error::PtyError;
use crate::terminal::Terminal;
use parking_lot::{Mutex, RwLock};
use portable_pty::{native_pty_system, Child, CommandBuilder, PtyPair, PtySize};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc,
};
use std::thread::JoinHandle;

mod reader;

/// Callback function for PTY output
///
/// Called whenever raw data is read from the PTY master, after the bytes have
/// been applied to the terminal. The bytes are passed unmodified, for
/// capturing the raw ANSI stream for logging, recording, or streaming to
/// clients; because the terminal state already includes them, a callback that
/// reads terminal state (e.g. to compose a snapshot for a client) is always
/// consistent with the bytes it observes.
///
/// # Arguments
/// * `data` - The raw bytes read from the PTY
pub type OutputCallback = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// Wait-free geometry mirror (ENH-023). The reader thread and every resize
/// path publish `(cols, rows, cursor)` here while already holding the
/// terminal write lock, so hot polling consumers (`size()`,
/// `cursor_position()`) never contend with output processing for these two
/// queries. Each value is individually consistent; the size/cursor pair can
/// straddle a concurrent resize — callers needing a consistent pair use
/// [`PtySession::snapshot_geometry`].
#[derive(Debug, Default)]
struct GeometryMirror {
    cols: AtomicU32,
    rows: AtomicU32,
    /// Cursor column in the low 32 bits, row in the high 32.
    cursor: AtomicU64,
}

impl GeometryMirror {
    /// Publish the terminal's current geometry. Called while holding the
    /// terminal write lock, so the values a reader Acquires are always ones
    /// the terminal actually held at some point.
    fn publish(&self, term: &Terminal) {
        let (cols, rows) = term.size();
        let cursor = term.cursor();
        self.cols.store(cols as u32, Ordering::Release);
        self.rows.store(rows as u32, Ordering::Release);
        self.cursor.store(
            ((cursor.row as u64) << 32) | cursor.col as u64,
            Ordering::Release,
        );
    }

    fn size(&self) -> (usize, usize) {
        (
            self.cols.load(Ordering::Acquire) as usize,
            self.rows.load(Ordering::Acquire) as usize,
        )
    }

    fn cursor(&self) -> (usize, usize) {
        let packed = self.cursor.load(Ordering::Acquire);
        ((packed & 0xFFFF_FFFF) as usize, (packed >> 32) as usize)
    }
}

/// The shared handles one PTY input write needs — writer, terminal, and
/// liveness flag — cloned out of a session so the write can happen with no
/// broader lock held. [`PtyInputHandle::write`] mirrors
/// [`PtySession::write`] exactly: same liveness check, same input recording,
/// same errors.
#[derive(Clone)]
pub(crate) struct PtyInputHandle {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    terminal: Arc<RwLock<Terminal>>,
    running: Arc<AtomicBool>,
}

impl PtyInputHandle {
    pub(crate) fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(PtyError::NotStartedError);
        }
        write_input(Some(&*self.writer), &self.terminal, data)
    }
}

/// The input write shared by [`PtySession::write`] and
/// [`PtyInputHandle::write`], after each has checked liveness: log, record
/// the input for session recording, then write and flush to the PTY. A
/// missing writer is `NotStartedError`, reported after the input is
/// recorded.
fn write_input(
    writer: Option<&Mutex<Box<dyn Write + Send>>>,
    terminal: &RwLock<Terminal>,
    data: &[u8],
) -> Result<(), PtyError> {
    debug::log_pty_write(data);

    // Record input for session recording
    {
        let mut term = terminal.write();
        term.record_input(data);
    }

    let Some(writer) = writer else {
        return Err(PtyError::NotStartedError);
    };
    let mut w = writer.lock();
    w.write_all(data).map_err(PtyError::IoError)?;
    w.flush().map_err(PtyError::IoError)?;
    Ok(())
}

/// A PTY session that manages a shell process and terminal state
pub struct PtySession {
    terminal: Arc<RwLock<Terminal>>,
    /// Master end of the PTY. The master sees EOF (EIO) once the child's side
    /// of the slave is gone; see `held_slave` for how macOS keeps it readable.
    pty_master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    /// Our copy of the slave, kept open on macOS only. There, when the last
    /// slave descriptor closes, output the master has not read yet is
    /// discarded, so a child that exits before the reader's first `read()`
    /// loses all of its output. Holding a slave fd prevents that, and the
    /// master still reads EOF when the child (the session leader) exits,
    /// because that revokes its controlling tty. Linux keeps unread output
    /// readable after the slave closes and, with a slave held, would never
    /// report EIO, so it drops the slave right after spawn.
    held_slave: Option<Box<dyn portable_pty::SlavePty + Send>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    reader_thread: Option<JoinHandle<()>>,
    writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    running: Arc<AtomicBool>,
    env_vars: Vec<(String, String)>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    /// Cell pixel dimensions used for `TIOCGWINSZ` `pixel_width`/`pixel_height`
    /// reporting. Updated by `resize_with_pixels`; falls back to a coarse 10×20
    /// default for callers that have not yet supplied real cell pixels (e.g.
    /// the initial `PtySession::new` before any layout pass).
    cell_pixel_width: u16,
    cell_pixel_height: u16,
    update_generation: Arc<std::sync::atomic::AtomicU64>,
    /// Signalled by the reader thread once applied content is visible (the
    /// post-write-guard generation bump) and on EOF — what
    /// [`PtySession::wait_for_update`] blocks on instead of sleep-polling.
    update_signal: Arc<(parking_lot::Mutex<()>, parking_lot::Condvar)>,
    /// Whether to reply to XTWINOPS queries (cached from env var PAR_TERM_REPLY_XTWINOPS)
    reply_xtwinops: Arc<AtomicBool>,
    /// Wait-free mirror of `(cols, rows, cursor)` for the polling getters
    /// (ENH-023) — see [`GeometryMirror`].
    geometry: Arc<GeometryMirror>,
    /// Optional callback for raw PTY output (for streaming, logging, etc.)
    /// Wrapped in Arc<Mutex> so it can be updated after the reader thread starts
    output_callback: Arc<Mutex<Option<OutputCallback>>>,
    /// Coprocess manager for piping terminal output to external processes
    coprocess_manager: Arc<Mutex<CoprocessManager>>,
    /// PID of the spawned child process (shell or command), set after spawn
    child_pid: Option<u32>,
    /// Exit code once `try_wait`/`wait`/`kill` (or a respawn's cleanup) has
    /// reaped the child (SEC-125). `Some` means the PID is released to the
    /// OS and may be recycled, so nothing may signal it again. Every reap
    /// records under this lock and every signal is sent while holding it,
    /// so an "unreaped" check cannot go stale before the kill(2). Shared
    /// with the reader thread for its alt-screen pulse; each spawn installs
    /// a fresh record so a still-draining old reader keeps its own child's.
    reaped: Arc<Mutex<Option<i32>>>,
    /// Test seam: the reader thread sleeps this long before its first read,
    /// so a test can make the child exit before any output is read.
    #[cfg(test)]
    first_read_delay: Option<std::time::Duration>,
    /// Test seam: the parent environment a spawn inherits, in place of the
    /// real process env, so env-drop tests need no process-global `set_var`
    /// (QA-196).
    #[cfg(test)]
    parent_env_override: Option<Vec<(std::ffi::OsString, std::ffi::OsString)>>,
    /// Test seam: SIGWINCH deliveries this session attempted.
    #[cfg(all(test, unix))]
    signals_sent: Arc<std::sync::atomic::AtomicUsize>,
}

/// Exclusive terminal access that republishes the session's wait-free
/// geometry mirror when dropped, so `size()`/`cursor_position()` never go
/// stale after a mutation (QA-195). Returned by
/// [`PtySession::terminal_write`].
pub struct TerminalWriteGuard<'a> {
    guard: parking_lot::RwLockWriteGuard<'a, Terminal>,
    geometry: &'a GeometryMirror,
}

impl std::ops::Deref for TerminalWriteGuard<'_> {
    type Target = Terminal;
    fn deref(&self) -> &Terminal {
        &self.guard
    }
}

impl std::ops::DerefMut for TerminalWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Terminal {
        &mut self.guard
    }
}

impl Drop for TerminalWriteGuard<'_> {
    fn drop(&mut self) {
        self.geometry.publish(&self.guard);
    }
}

/// Pixel extent of `cells` cells at `cell_px` pixels each, saturated to the
/// `u16` a `winsize`/`PtySize` field can hold (QA-182: `cols * cell_w` in
/// `u16` overflowed at 2000 columns of 40 px cells).
pub(crate) fn pixel_extent(cells: u16, cell_px: u16) -> u16 {
    u16::try_from(u32::from(cells) * u32::from(cell_px)).unwrap_or(u16::MAX)
}

/// Deliver SIGWINCH to the child's process group, falling back to the PID.
///
/// The single delivery path for every resize trigger — the reader thread's
/// alt-screen pulse, [`PtySession::resize`], and
/// [`PtySession::resize_with_pixels`] — so a resize reaches the child the
/// same way regardless of which API the caller used. The group is signalled
/// first so grandchildren (apps launched from the shell) also recalculate;
/// the direct PID is the fallback for a child that left its group.
///
/// Callers hold the session's reap-record lock and have seen it empty
/// (SEC-125); see [`PtySession::signal_winch`].
#[cfg(unix)]
fn send_sigwinch(pid: u32, tag: &str, context: &str) -> std::io::Result<()> {
    // SAFETY: `kill(2)` has no memory-safety preconditions. The caller holds
    // the session's reap-record lock and has seen the child unreaped, and
    // only this session reaps its child (always recording under that lock),
    // so `pid` is still our child — live or a zombie — and cannot have been
    // recycled for an unrelated process. The negative form addresses only
    // that child's process group.
    unsafe {
        if libc::kill(-(pid as libc::pid_t), libc::SIGWINCH) == 0 {
            debug::log(
                debug::DebugLevel::Debug,
                tag,
                &format!("SIGWINCH sent to process group -{pid} ({context})"),
            );
            return Ok(());
        }
        let group_err = std::io::Error::last_os_error();
        if libc::kill(pid as libc::pid_t, libc::SIGWINCH) == 0 {
            debug::log(
                debug::DebugLevel::Debug,
                tag,
                &format!("SIGWINCH sent to PID {pid} (group failed: {group_err}; {context})"),
            );
            return Ok(());
        }
        let pid_err = std::io::Error::last_os_error();
        debug::log(
            debug::DebugLevel::Error,
            tag,
            &format!("SIGWINCH delivery failed for {context}: group {group_err}, pid {pid_err}"),
        );
        Err(pid_err)
    }
}

impl PtySession {
    /// Create a new PTY session with the specified dimensions
    ///
    /// # Arguments
    /// * `cols` - Number of columns (width)
    /// * `rows` - Number of rows (height)
    /// * `max_scrollback` - Maximum number of scrollback lines
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        // Check environment variable once at initialization
        let reply_xtwinops = std::env::var("PAR_TERM_REPLY_XTWINOPS")
            .ok()
            .map(|v| v != "0" && v.to_lowercase() != "false")
            .unwrap_or(true);

        let session = Self {
            terminal: Arc::new(RwLock::new(Terminal::with_scrollback(
                cols,
                rows,
                max_scrollback,
            ))),
            pty_master: None,
            held_slave: None,
            child: None,
            reader_thread: None,
            writer: None,
            running: Arc::new(AtomicBool::new(false)),
            env_vars: Vec::new(),
            cwd: None,
            cols: cols as u16,
            rows: rows as u16,
            cell_pixel_width: 10,
            cell_pixel_height: 20,
            update_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            update_signal: Arc::new((parking_lot::Mutex::new(()), parking_lot::Condvar::new())),
            reply_xtwinops: Arc::new(AtomicBool::new(reply_xtwinops)),
            geometry: Arc::new(GeometryMirror::default()),
            output_callback: Arc::new(Mutex::new(None)),
            coprocess_manager: Arc::new(Mutex::new(CoprocessManager::new())),
            child_pid: None,
            reaped: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            first_read_delay: None,
            #[cfg(test)]
            parent_env_override: None,
            #[cfg(all(test, unix))]
            signals_sent: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        session.geometry.publish(&session.terminal.read());
        session
    }

    /// The environment a spawn inherits: the real process env, or the test
    /// override when one is set.
    fn parent_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        #[cfg(test)]
        if let Some(env) = &self.parent_env_override {
            return env.clone();
        }
        std::env::vars_os().collect()
    }

    /// Set an environment variable for the spawned process
    ///
    /// Must be called before `spawn()` or `spawn_shell()`
    pub fn set_env(&mut self, key: &str, value: &str) {
        self.env_vars.push((key.to_string(), value.to_string()));
    }

    /// Set the working directory for the spawned process
    ///
    /// Must be called before `spawn()` or `spawn_shell()`
    pub fn set_cwd(&mut self, path: &Path) {
        self.cwd = Some(path.to_string_lossy().to_string());
    }

    /// Set a callback to be called whenever raw output is received from the PTY
    ///
    /// The callback will be called with the raw bytes before they are processed
    /// by the terminal. This is useful for streaming, logging, or recording.
    ///
    /// # Example
    /// ```no_run
    /// use par_term_emu_core_rust::pty_session::PtySession;
    /// use std::sync::Arc;
    ///
    /// let mut pty = PtySession::new(80, 24, 1000);
    /// pty.set_output_callback(Arc::new(|data| {
    ///     println!("Received {} bytes", data.len());
    /// }));
    /// ```
    pub fn set_output_callback(&mut self, callback: OutputCallback) {
        *self.output_callback.lock() = Some(callback);
    }

    /// Remove the output callback
    pub fn clear_output_callback(&mut self) {
        *self.output_callback.lock() = None;
    }

    /// Fire the output callback with data that did not come from the PTY
    /// reader thread (e.g. tmux/mux mirror output fed by the frontend).
    ///
    /// Mirrors the reader thread's invocation so streaming/logging
    /// consumers see daemon-fed output identically. No in-crate caller:
    /// embedders that feed daemon-sourced bytes into a mirror session call
    /// it (par-term's mux mirror, `process_mux_output`), so it is not dead.
    pub fn fire_output_callback(&self, data: &[u8]) {
        let guard = self.output_callback.lock();
        if let Some(ref callback) = *guard {
            callback(data);
        }
    }

    /// Get a clone of the PTY writer for external use (e.g., streaming server)
    ///
    /// This allows external code to write input to the PTY in a thread-safe way.
    /// Returns None if the PTY is not running.
    pub fn get_writer(&self) -> Option<Arc<Mutex<Box<dyn Write + Send>>>> {
        self.writer.clone()
    }

    /// Spawn a shell process (auto-detected from environment)
    ///
    /// On Unix: Uses $SHELL or defaults to /bin/bash
    /// On Windows: Uses %COMSPEC% or defaults to cmd.exe
    pub fn spawn_shell(&mut self) -> Result<(), PtyError> {
        self.spawn_shell_with_env(None, None)
    }

    /// Spawn a shell process with environment variables and/or working directory
    ///
    /// This method allows passing environment variables directly without modifying
    /// the parent process environment, making it safe for multi-threaded applications.
    ///
    /// # Arguments
    /// * `env` - Optional environment variables to set for the spawned process.
    ///   These are applied after any variables set via `set_env()`.
    /// * `cwd` - Optional working directory for the spawned process.
    ///   If provided, overrides any directory set via `set_cwd()`.
    ///
    /// # Example
    /// ```no_run
    /// use par_term_emu_core_rust::pty_session::PtySession;
    /// use std::collections::HashMap;
    ///
    /// let mut session = PtySession::new(80, 24, 1000);
    /// let mut env = HashMap::new();
    /// env.insert("MY_VAR".to_string(), "hello".to_string());
    /// session.spawn_shell_with_env(Some(&env), Some("/tmp")).unwrap();
    /// ```
    pub fn spawn_shell_with_env(
        &mut self,
        env: Option<&HashMap<String, String>>,
        cwd: Option<&str>,
    ) -> Result<(), PtyError> {
        let shell = Self::get_default_shell();
        let args: Vec<&str> = Vec::new();
        self.spawn_with_env(&shell, &args, env, cwd)
    }

    /// Spawn a process with environment variables and/or working directory
    ///
    /// This method allows passing environment variables directly without modifying
    /// the parent process environment, making it safe for multi-threaded applications.
    ///
    /// # Arguments
    /// * `command` - The command to execute
    /// * `args` - Command-line arguments
    /// * `env` - Optional environment variables to set for the spawned process.
    ///   These are applied after any variables set via `set_env()`.
    /// * `cwd` - Optional working directory for the spawned process.
    ///   If provided, overrides any directory set via `set_cwd()`.
    ///
    /// # Example
    /// ```no_run
    /// use par_term_emu_core_rust::pty_session::PtySession;
    /// use std::collections::HashMap;
    ///
    /// let mut session = PtySession::new(80, 24, 1000);
    /// let mut env = HashMap::new();
    /// env.insert("MY_VAR".to_string(), "hello".to_string());
    /// session.spawn_with_env("/bin/bash", &["-c", "echo $MY_VAR"], Some(&env), None).unwrap();
    /// ```
    pub fn spawn_with_env(
        &mut self,
        command: &str,
        args: &[&str],
        env: Option<&HashMap<String, String>>,
        cwd: Option<&str>,
    ) -> Result<(), PtyError> {
        self.spawn_internal(command, args, env, cwd)
    }

    /// Get the default shell for the current platform
    pub fn get_default_shell() -> String {
        let shell = if cfg!(windows) {
            // Use %COMSPEC% (typically cmd.exe), fall back to cmd.exe
            if let Ok(comspec) = std::env::var("COMSPEC") {
                comspec
            } else {
                "cmd.exe".to_string()
            }
        } else {
            // Unix-like: check $SHELL, fall back to /bin/bash
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        };

        // Validate that the shell exists and is a file (not a directory)
        #[cfg(unix)]
        {
            if let Ok(metadata) = std::fs::metadata(&shell) {
                if metadata.is_file() {
                    return shell;
                }
            }
            // Fallback to /bin/sh if shell doesn't exist
            "/bin/sh".to_string()
        }

        #[cfg(not(unix))]
        shell
    }

    /// Clean up resources from a previous session before spawning a new one
    ///
    /// This ensures the old reader thread is properly finished before we create
    /// a new PTY and reader thread. Called internally by spawn() when restarting.
    fn cleanup_previous_session(&mut self) {
        // Close writer first to unblock any blocked reads in the old reader thread
        if let Some(writer) = self.writer.take() {
            debug::log(
                debug::DebugLevel::Debug,
                "PTY_CLEANUP",
                "Dropping previous PTY writer to unblock reader",
            );
            drop(writer);
        }

        // Close the old PTY master (dropping it closes the master FD)
        if let Some(master) = self.pty_master.take() {
            debug::log(
                debug::DebugLevel::Debug,
                "PTY_CLEANUP",
                "Dropping previous PTY master",
            );
            drop(master);
        }
        self.held_slave = None;

        // Wait for the old reader thread to finish (with timeout)
        if let Some(handle) = self.reader_thread.take() {
            debug::log(
                debug::DebugLevel::Debug,
                "PTY_CLEANUP",
                "Waiting for previous reader thread to finish",
            );

            let timeout = std::time::Duration::from_secs(2);
            let start = std::time::Instant::now();

            // Poll for thread completion
            while !handle.is_finished() && start.elapsed() < timeout {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            if handle.is_finished() {
                let _ = handle.join();
                debug::log(
                    debug::DebugLevel::Debug,
                    "PTY_CLEANUP",
                    "Previous reader thread joined successfully",
                );
            } else {
                debug::log(
                    debug::DebugLevel::Info,
                    "PTY_CLEANUP",
                    &format!(
                        "Previous reader thread did not finish within {}s timeout, detaching",
                        timeout.as_secs()
                    ),
                );
                // Thread will be detached - it should exit soon once it sees the PTY is closed
            }
        }

        // Clean up child process handle (should already be exited). Every
        // reap here is recorded under the old child's reap record, which a
        // detached old reader still holds, and the PID is dropped, so a
        // spawn that fails after this point leaves nothing to signal
        // (SEC-125).
        if let Some(mut child) = self.child.take() {
            let mut record = self.reaped.lock();
            // Try to reap the child if it hasn't been reaped yet
            match child.try_wait() {
                Ok(Some(status)) => {
                    *record = Some(status.exit_code() as i32);
                    debug::log(
                        debug::DebugLevel::Debug,
                        "PTY_CLEANUP",
                        &format!(
                            "Previous child reaped with exit code: {}",
                            status.exit_code()
                        ),
                    );
                }
                Ok(None) => {
                    // Child still running - kill it. `try_wait` just saw it
                    // unreaped on this thread (std caches a prior reap), so
                    // portable-pty's raw SIGHUP cannot reach a released PID.
                    debug::log(
                        debug::DebugLevel::Info,
                        "PTY_CLEANUP",
                        "Previous child still running, killing",
                    );
                    let _ = child.kill();
                    if let Ok(status) = child.wait() {
                        *record = Some(status.exit_code() as i32);
                    }
                }
                Err(e) => {
                    debug::log(
                        debug::DebugLevel::Error,
                        "PTY_CLEANUP",
                        &format!("Error checking child status: {}", e),
                    );
                }
            }
        }
        self.child_pid = None;

        debug::log(
            debug::DebugLevel::Debug,
            "PTY_CLEANUP",
            "Previous session cleanup complete",
        );
    }

    /// Spawn a process with the specified command and arguments
    ///
    /// # Arguments
    /// * `command` - The command to execute
    /// * `args` - Command-line arguments
    pub fn spawn(&mut self, command: &str, args: &[&str]) -> Result<(), PtyError> {
        self.spawn_internal(command, args, None, None)
    }

    /// Internal implementation for spawning a process
    ///
    /// This handles all the PTY setup and process spawning logic.
    ///
    /// # Arguments
    /// * `command` - The command to execute
    /// * `args` - Command-line arguments
    /// * `additional_env` - Additional environment variables to set (applied after `set_env()` vars)
    /// * `override_cwd` - Working directory override (takes precedence over `set_cwd()`)
    fn spawn_internal(
        &mut self,
        command: &str,
        args: &[&str],
        additional_env: Option<&HashMap<String, String>>,
        override_cwd: Option<&str>,
    ) -> Result<(), PtyError> {
        if self.is_running() {
            return Err(PtyError::ProcessSpawnError(
                "Process is already running".to_string(),
            ));
        }

        // Clean up any previous session resources before spawning
        // This ensures the old reader thread is finished and PTY is closed
        self.cleanup_previous_session();

        debug::log(
            debug::DebugLevel::Info,
            "PTY_SPAWN",
            &format!("Spawning process: {} {:?}", command, args),
        );

        // Create the PTY system
        let pty_system = native_pty_system();
        // Use the tracked cell pixel size (defaulted on construction, updated by
        // `resize_with_pixels`). This drives TIOCGWINSZ so client programs see
        // pixel dimensions consistent with how cells are actually rendered.
        let pty_size = PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: pixel_extent(self.cols, self.cell_pixel_width),
            pixel_height: pixel_extent(self.rows, self.cell_pixel_height),
        };

        debug::log(
            debug::DebugLevel::Trace,
            "PTY_SPAWN",
            &format!(
                "Creating PTY with initial size: {{ rows: {}, cols: {}, pixel_width: {}, pixel_height: {} }}",
                pty_size.rows, pty_size.cols, pty_size.pixel_width, pty_size.pixel_height
            ),
        );

        // Create the PTY pair
        let pair = pty_system
            .openpty(pty_size)
            .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))?;

        debug::log(
            debug::DebugLevel::Trace,
            "PTY_SPAWN",
            &format!(
                "PTY opened successfully with size {}x{}",
                pty_size.cols, pty_size.rows
            ),
        );

        // Build the command
        let mut cmd = CommandBuilder::new(command);
        for arg in args {
            cmd.arg(arg);
        }

        // Check if login shell mode is requested (-l or --login flag)
        // For bash to properly recognize login shell via $0 and shopt login_shell,
        // argv[0] must start with '-'. The -l flag alone makes bash read profile
        // files but doesn't set $0 to -bash.
        // We need to modify argv[0] AFTER path resolution but BEFORE exec.
        // Since CommandBuilder uses args[0] for both path resolution AND arg0,
        // we detect login shell mode here and will handle it in the spawn.
        let is_login_shell = args.iter().any(|a| *a == "-l" || *a == "--login");

        // Inherit parent environment variables, but deliberately drop:
        // 1. COLUMNS/LINES — static size hints that confuse apps after a PTY resize.
        //    Many libraries (e.g. Python's shutil.get_terminal_size) and some TUIs
        //    prioritize these over TIOCGWINSZ, staying stuck at the parent size.
        // 2. TMUX/TMUX_PANE — multiplexer session vars from the parent terminal.
        //    The child shell is inside a new PTY, NOT inside tmux. Inheriting these
        //    causes tools like fzf to render in the parent tmux pane instead of here.
        // 3. STY/WINDOW — GNU Screen equivalents of TMUX.
        // 4. PAR_MUX_* — the par-mux pane identity set. A PtySession spawned by a
        //    process inside a mux pane (e.g. par-term's local tabs) would inherit
        //    the outer pane's identity, so hook scripts report agents to the outer
        //    daemon under the wrong pane id. Prefix-matched so identity vars added
        //    later are covered without editing this list; mux panes re-add their
        //    own values via set_env, which runs after this drop.
        // 5. Outer agent session identity (herdr parity, pane.rs
        //    apply_pane_launch_env): a spawned PTY is not a child agent of
        //    whatever started this process. Nested-session detection keyed on
        //    these vars (omp treats OMPCODE=1 as nested and never reports)
        //    would hide the pane's own agents from rosters. set_env opts back
        //    in for an intentional child session.
        // CommandBuilder::new() pre-loads the full parent environment via
        // get_base_env(), so we must explicitly remove unwanted vars with
        // env_remove() — simply skipping them in the loop below is not enough.
        const DROP_VARS: &[&str] = &[
            "COLUMNS",
            "LINES",
            "TMUX",
            "TMUX_PANE",
            "STY",
            "WINDOW",
            "CLAUDECODE",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDE_CODE_MESSAGING_TOKEN",
            "OMPCODE",
            "CODEX_THREAD_ID",
        ];
        fn dropped_by_name(name: &str) -> bool {
            DROP_VARS.contains(&name) || name.starts_with("PAR_MUX_")
        }
        // A test override replaces the preloaded real env wholesale.
        #[cfg(test)]
        if self.parent_env_override.is_some() {
            cmd.env_clear();
        }
        // One pass over the parent env: remove each dropped name, and re-apply
        // the rest (overriding get_base_env values with current ones).
        let mut dropped: Vec<String> = Vec::new();
        for (key, value) in self.parent_env() {
            let name = key.to_string_lossy();
            if !dropped_by_name(&name) {
                cmd.env(&key, &value);
                continue;
            }
            cmd.env_remove(&key);
            let label = if name.starts_with("PAR_MUX_") {
                "PAR_MUX_*".to_string()
            } else {
                name.into_owned()
            };
            if !dropped.contains(&label) {
                dropped.push(label);
            }
        }
        if !dropped.is_empty() {
            debug::log(
                debug::DebugLevel::Info,
                "PTY_SPAWN",
                &format!("Dropped env vars: {}", dropped.join(", ")),
            );
        }

        // Set terminal-specific environment variables
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // Set Kitty-specific environment variables for protocol detection
        cmd.env("TERM_PROGRAM", "kitty");
        cmd.env("KITTY_WINDOW_ID", "1");
        cmd.env("KITTY_PID", std::process::id().to_string());
        // NOTE: Do NOT set COLUMNS/LINES environment variables!
        // They are static and won't update on resize. Applications should
        // query terminal size via ioctl(TIOCGWINSZ), not environment variables.
        // Setting these breaks libraries like Textual that use shutil.get_terminal_size()
        // which prioritizes env vars over ioctl.

        // Override with user-specified environment variables (from set_env())
        for (key, value) in &self.env_vars {
            cmd.env(key, value);
        }

        // Apply additional environment variables passed directly to spawn
        // These take precedence over set_env() vars
        if let Some(env) = additional_env {
            for (key, value) in env {
                cmd.env(key, value);
            }
        }

        // Set working directory
        // Priority: override_cwd > self.cwd
        let effective_cwd = override_cwd
            .map(|s| s.to_string())
            .or_else(|| self.cwd.clone());
        if let Some(ref cwd) = effective_cwd {
            cmd.cwd(cwd);
        }

        // Spawn the child process using the slave side. Off macOS, drop our handle
        // to the slave immediately after spawn so that when the child exits, the
        // master side sees EOF; macOS keeps it (see `held_slave`).
        let PtyPair { master, slave } = pair;

        let child = if is_login_shell {
            // For login shells, we need to set argv[0] to "-bash" for the shell
            // to properly recognize itself as a login shell via $0 and shopt login_shell.
            // The CommandBuilder's as_command() uses args[0] for both path resolution
            // and arg0, which doesn't work for login shells. We spawn manually.
            self.spawn_login_shell(command, args, &cmd, &slave, additional_env.cloned())
                .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))?
        } else {
            slave
                .spawn_command(cmd)
                .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))?
        };
        let held_slave = if cfg!(target_os = "macos") {
            Some(slave)
        } else {
            drop(slave);
            None
        };

        // Get the master reader
        let reader = master
            .try_clone_reader()
            .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))?;

        // Get the master writer (wrapped in Arc<Mutex<>> for shared access)
        let writer = master
            .take_writer()
            .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))?;
        let writer = Arc::new(Mutex::new(writer));

        // Get child PID before storing
        let child_pid = child.process_id();

        // Store the PTY master and child
        self.pty_master = Some(master);
        self.held_slave = held_slave;
        self.child = Some(child);
        self.writer = Some(Arc::clone(&writer));
        self.running.store(true, Ordering::SeqCst);
        self.child_pid = child_pid;
        // A fresh record, not a cleared one: a previous reader still
        // draining keeps the old child's record (SEC-125).
        self.reaped = Arc::new(Mutex::new(None));

        // Spawn the reader thread (shares writer for device query responses)
        self.start_reader_thread(reader, writer, child_pid);

        Ok(())
    }

    /// Spawn a login shell.
    ///
    /// The `-l` flag is passed to the shell, which makes bash:
    /// 1. Read `/etc/profile` and `~/.bash_profile`
    /// 2. Report `shopt login_shell` as ON
    ///
    /// Note: `$0` will show the shell path (not `-bash`) because portable-pty's
    /// CommandBuilder uses args[0] for both path resolution AND arg0. The `-l`
    /// flag provides full login shell behavior regardless.
    #[allow(clippy::borrowed_box)]
    fn spawn_login_shell(
        &self,
        #[cfg_attr(not(unix), allow(unused_variables))] shell_path: &str,
        _args: &[&str],
        cmd_builder: &CommandBuilder,
        slave: &Box<dyn portable_pty::SlavePty + Send>,
        _additional_env: Option<HashMap<String, String>>,
    ) -> Result<Box<dyn portable_pty::Child + Send + Sync>, PtyError> {
        #[cfg(unix)]
        {
            let shell_basename = shell_path.rsplit('/').next().unwrap_or(shell_path);

            debug::log(
                debug::DebugLevel::Info,
                "PTY_SPAWN",
                &format!(
                    "Spawning login shell: {} -l (login_shell via -l flag)",
                    shell_basename
                ),
            );
        }

        slave
            .spawn_command(cmd_builder.clone())
            .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))
    }

    /// Write data to the PTY (send to the child process)
    ///
    /// # Arguments
    /// * `data` - Bytes to write
    pub fn write(&mut self, data: &[u8]) -> Result<(), PtyError> {
        if !self.is_running() {
            return Err(PtyError::NotStartedError);
        }
        write_input(self.writer.as_deref(), &self.terminal, data)
    }

    /// Write a string to the PTY (convenience method)
    ///
    /// # Arguments
    /// * `s` - String to write
    pub fn write_str(&mut self, s: &str) -> Result<(), PtyError> {
        self.write(s.as_bytes())
    }

    /// The input path detached from `&mut self`: writer, terminal, and
    /// liveness flag as shared handles, so a caller holding a broader lock
    /// (the mux dispatcher's tree mutex) can clone them under the lock,
    /// release it, and only then write — `write_all` blocks once the PTY
    /// buffer fills (QA-225). `None` when the session has no PTY writer
    /// (never spawned), the same [`PtyError::NotStartedError`] condition
    /// [`PtySession::write`] reports.
    pub(crate) fn input_handle(&self) -> Option<PtyInputHandle> {
        Some(PtyInputHandle {
            writer: self.writer.as_ref()?.clone(),
            terminal: Arc::clone(&self.terminal),
            running: Arc::clone(&self.running),
        })
    }

    /// Resize the PTY and terminal
    ///
    /// Sends SIGWINCH to the child process
    ///
    /// # Arguments
    /// * `cols` - New number of columns
    /// * `rows` - New number of rows
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.cols = cols;
        self.rows = rows;

        // Resize the terminal
        {
            let mut term = self.terminal.write();
            term.resize(cols as usize, rows as usize);
            // Record resize event for session recording
            term.record_resize(cols as usize, rows as usize);
            self.geometry.publish(&term);
        }

        // Resize the PTY (sends SIGWINCH to child)
        if let Some(ref master) = self.pty_master {
            // Use the tracked cell pixel size (updated by `resize_with_pixels`).
            // Falls back to the construction default if no pixel-aware resize
            // has been called yet.
            let pty_size = PtySize {
                rows,
                cols,
                pixel_width: pixel_extent(cols, self.cell_pixel_width),
                pixel_height: pixel_extent(rows, self.cell_pixel_height),
            };
            debug::log(
                debug::DebugLevel::Debug,
                "PTY_RESIZE",
                &format!("Calling master.resize({}, {})", cols, rows),
            );
            debug::log(
                debug::DebugLevel::Trace,
                "PTY_RESIZE",
                &format!(
                    "PtySize {{ rows: {}, cols: {}, pixel_width: {}, pixel_height: {} }}",
                    pty_size.rows, pty_size.cols, pty_size.pixel_width, pty_size.pixel_height
                ),
            );
            master.resize(pty_size).map_err(|e| {
                debug::log(
                    debug::DebugLevel::Error,
                    "PTY_RESIZE",
                    &format!("Failed to resize PTY: {}", e),
                );
                PtyError::ResizeError(e.to_string())
            })?;
            debug::log(
                debug::DebugLevel::Debug,
                "PTY_RESIZE",
                "master.resize() completed successfully",
            );
            debug::log(
                debug::DebugLevel::Trace,
                "PTY_RESIZE",
                &format!(
                    "PTY resize complete: internal state now cols={}, rows={}",
                    self.cols, self.rows
                ),
            );
        }

        // Manually deliver SIGWINCH after the pty resize: portable-pty's
        // resize() updates the kernel winsize but may not reliably deliver
        // the signal in all scenarios. Group-then-PID delivery lives in
        // [`send_sigwinch`].
        #[cfg(unix)]
        self.signal_winch("PTY_RESIZE", &format!("resize {cols}x{rows}"));

        Ok(())
    }

    /// Resize the PTY and terminal, including pixel dimensions
    ///
    /// This sets both character dimensions and pixel area for XTWINOPS 14 and
    /// updates the PTY's ws_xpixel/ws_ypixel so children can query it.
    pub fn resize_with_pixels(
        &mut self,
        cols: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<(), PtyError> {
        self.cols = cols;
        self.rows = rows;
        // Cache per-cell pixel size so plain `resize()` (without pixels) and
        // future spawns can reuse it without falling back to the 10×20 default.
        if cols > 0 && rows > 0 {
            self.cell_pixel_width = (pixel_width / cols).max(1);
            self.cell_pixel_height = (pixel_height / rows).max(1);
        }

        // Resize the terminal and record pixel size
        {
            let mut term = self.terminal.write();
            term.resize(cols as usize, rows as usize);
            term.set_pixel_size(pixel_width as usize, pixel_height as usize);
            self.geometry.publish(&term);
        }

        // Resize the PTY (sends SIGWINCH to child)
        if let Some(ref master) = self.pty_master {
            let pty_size = PtySize {
                rows,
                cols,
                pixel_width,
                pixel_height,
            };
            debug::log(
                debug::DebugLevel::Debug,
                "PTY_RESIZE",
                &format!(
                    "Calling master.resize({}, {}) with pixels {}x{}",
                    cols, rows, pixel_width, pixel_height
                ),
            );
            master.resize(pty_size).map_err(|e| {
                debug::log(
                    debug::DebugLevel::Error,
                    "PTY_RESIZE",
                    &format!("Failed to resize PTY: {}", e),
                );
                PtyError::ResizeError(e.to_string())
            })?;
            debug::log(
                debug::DebugLevel::Debug,
                "PTY_RESIZE",
                "master.resize() completed successfully",
            );
            debug::log(
                debug::DebugLevel::Trace,
                "PTY_RESIZE",
                &format!(
                    "PTY resize complete: internal state now cols={}, rows={} (pixels {}x{})",
                    self.cols, self.rows, pixel_width, pixel_height
                ),
            );
        }

        // Manually deliver SIGWINCH after the pty resize (as in resize());
        // delivery and failure logging live in [`send_sigwinch`].
        #[cfg(unix)]
        self.signal_winch(
            "PTY_RESIZE",
            &format!("resize {cols}x{rows} ({pixel_width}x{pixel_height} px)"),
        );

        Ok(())
    }

    /// Check if the process is still running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Liveness for periodic pollers: the reader flag first, then the OS
    /// child handle. On Windows ConPTY the pipe read stays blocked after the
    /// child exits (conhost keeps its end open), so the reader thread never
    /// flips `running` — measured: a cmd.exe pane exits in ~250 ms while the
    /// flag stays true for 15+ s. A reaper that only trusts the flag never
    /// reaps on Windows; this poll asks the OS when the flag claims alive.
    pub fn poll_running(&mut self) -> bool {
        if self.reaped.lock().is_some() || !self.running.load(Ordering::SeqCst) {
            return false;
        }
        match self.try_wait() {
            // Still waiting on the OS: alive. A poll error is not evidence
            // of death — report alive and let the next pass retry.
            Ok(Some(_)) => false,
            Ok(None) | Err(_) => true,
        }
    }

    /// Return the PID of the spawned child process (shell or command).
    ///
    /// Returns `None` if no process has been spawned yet, if the platform
    /// does not expose the PID (unusual), or once [`Self::try_wait`],
    /// [`Self::wait`] or [`Self::kill`] has observed the exit: the reaped
    /// PID is released to the OS and may belong to another process
    /// (SEC-125).
    pub fn child_pid(&self) -> Option<u32> {
        if self.reaped.lock().is_some() {
            None
        } else {
            self.child_pid
        }
    }

    /// Deliver SIGWINCH to the child unless it has been reaped (SEC-125):
    /// the reap record stays locked across the delivery, so no reap can
    /// release the PID between the check and the `kill(2)`.
    #[cfg(unix)]
    fn signal_winch(&self, tag: &str, context: &str) {
        let record = self.reaped.lock();
        if record.is_some() {
            return;
        }
        if let Some(pid) = self.child_pid {
            #[cfg(test)]
            self.signals_sent.fetch_add(1, Ordering::SeqCst);
            let _ = send_sigwinch(pid, tag, context);
        }
        drop(record);
    }

    /// Try to get the exit status without blocking
    ///
    /// Returns None if the process hasn't exited yet. Once the exit has
    /// been observed, every later call returns the recorded code.
    pub fn try_wait(&mut self) -> Result<Option<i32>, PtyError> {
        if let Some(ref mut child) = self.child {
            let mut record = self.reaped.lock();
            if let Some(code) = *record {
                self.running.store(false, Ordering::SeqCst);
                return Ok(Some(code));
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let code = status.exit_code() as i32;
                    *record = Some(code);
                    self.running.store(false, Ordering::SeqCst);
                    Ok(Some(code))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(PtyError::IoError(e)),
            }
        } else {
            Err(PtyError::NotStartedError)
        }
    }

    /// Wait for the process to exit and return its exit code
    ///
    /// This blocks until the process exits. The reap record stays locked
    /// through the wait; the reader thread only `try_lock`s it, so a child
    /// blocked on PTY output cannot deadlock against this call.
    pub fn wait(&mut self) -> Result<i32, PtyError> {
        if let Some(ref mut child) = self.child {
            let mut record = self.reaped.lock();
            let code = match *record {
                Some(code) => code,
                None => {
                    let code = child.wait().map_err(PtyError::IoError)?.exit_code() as i32;
                    *record = Some(code);
                    code
                }
            };
            self.running.store(false, Ordering::SeqCst);
            Ok(code)
        } else {
            Err(PtyError::NotStartedError)
        }
    }

    /// Kill the process
    ///
    /// A no-op once the exit has been observed: the reaped PID is released
    /// and may belong to another process (SEC-125).
    pub fn kill(&mut self) -> Result<(), PtyError> {
        if let Some(ref mut child) = self.child {
            let mut record = self.reaped.lock();
            // portable-pty's kill opens with a raw `kill(pid, SIGHUP)` that,
            // unlike std's `Child::kill`, does not check for a prior reap, so
            // it may only run while the record says unreaped.
            if record.is_none() {
                child.kill().map_err(PtyError::IoError)?;
                // portable-pty's kill escalates SIGHUP → SIGKILL but never
                // waits, so a child that ignores SIGHUP (a shell that trapped
                // it) would stay a zombie until this process exits. SIGKILL
                // cannot be trapped, so a bounded poll reaps it here. When
                // portable-pty's own grace loop already reaped it, std's
                // cached status still answers the first poll.
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
                while std::time::Instant::now() < deadline {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            *record = Some(status.exit_code() as i32);
                            break;
                        }
                        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                        Err(_) => break,
                    }
                }
            }
            self.running.store(false, Ordering::SeqCst);
            Ok(())
        } else {
            Err(PtyError::NotStartedError)
        }
    }

    /// Get an owned, shared handle to the underlying terminal.
    ///
    /// This returns an `Arc` clone, so the handle can outlive the current
    /// borrow of `self`. Use it for long-lived subscribers: a background task,
    /// an observer, or a struct field that keeps the terminal reachable after
    /// this call returns. For a single quick read or write, prefer
    /// [`with_terminal`](Self::with_terminal) /
    /// [`with_terminal_mut`](Self::with_terminal_mut), or
    /// [`terminal_ref`](Self::terminal_ref) when a borrowed handle is enough.
    ///
    /// Holding the `Arc` is cheap. Holding a guard taken from it is not:
    ///
    /// - The background PTY reader thread takes the **write** lock while it
    ///   runs [`Terminal::process`] on PTY output. A read guard held for a long
    ///   time stalls output processing, and a write guard held for a long time
    ///   also blocks every reader.
    /// - Never hold the lock (read or write) while calling into Python. A
    ///   Python callback that re-enters the terminal, or another thread that
    ///   holds the GIL while waiting for this lock, deadlocks.
    ///
    /// Take a guard, copy out what you need, and drop it before doing anything
    /// slow or calling back into Python.
    pub fn terminal(&self) -> Arc<RwLock<Terminal>> {
        Arc::clone(&self.terminal)
    }

    /// Run `f` with shared read access to the terminal and return its result.
    ///
    /// The read lock is held only for the duration of `f`, so the guard cannot
    /// escape the closure. Prefer this over [`terminal`](Self::terminal) or
    /// [`terminal_ref`](Self::terminal_ref) in new code that needs a
    /// short-lived read. Keep `f` short: it blocks the PTY reader thread's
    /// write lock while it runs, and it must not call into Python.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use par_term_emu_core_rust::pty_session::PtySession;
    ///
    /// let session = PtySession::new(80, 24, 1000);
    /// let (cols, rows) = session.with_terminal(|term| term.size());
    /// assert_eq!((cols, rows), (80, 24));
    /// ```
    pub fn with_terminal<R>(&self, f: impl FnOnce(&Terminal) -> R) -> R {
        let guard = self.terminal.read();
        f(&guard)
    }

    /// Run `f` with exclusive write access to the terminal and return its result.
    ///
    /// The write lock is held only for the duration of `f`, so the guard cannot
    /// escape the closure. While `f` runs, the PTY reader thread and every
    /// reader are blocked, so keep it short and never call into Python from it.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use par_term_emu_core_rust::pty_session::PtySession;
    ///
    /// let session = PtySession::new(80, 24, 1000);
    /// session.with_terminal_mut(|term| term.process(b"hello"));
    /// ```
    pub fn with_terminal_mut<R>(&self, f: impl FnOnce(&mut Terminal) -> R) -> R {
        let mut guard = self.terminal.write();
        let result = f(&mut guard);
        // A caller may have moved the cursor or resized through the closure;
        // keep the wait-free mirror honest before the guard drops.
        self.geometry.publish(&guard);
        result
    }

    /// Exclusive terminal access as a guard that republishes the wait-free
    /// geometry mirror when dropped (QA-195), for callers that cannot use
    /// the [`with_terminal_mut`](Self::with_terminal_mut) closure form (the
    /// Python bindings' macro layer). Mutating through the raw
    /// [`terminal`](Self::terminal) lock instead leaves `size()` and
    /// `cursor_position()` stale until the next PTY output. The same rules
    /// apply as for any write guard: keep it short and never hold it
    /// across a call into Python.
    pub fn terminal_write(&self) -> TerminalWriteGuard<'_> {
        TerminalWriteGuard {
            guard: self.terminal.write(),
            geometry: &self.geometry,
        }
    }

    /// Get a borrowed reference to the underlying terminal `Arc`.
    ///
    /// Unlike [`terminal`](Self::terminal), this does not clone the `Arc`, so a
    /// locked guard derived from it borrows `self` directly (no temporary `Arc`
    /// to outlive). Used by the shared Python-binding accessors (ARC-003/QA-001).
    pub fn terminal_ref(&self) -> &Arc<RwLock<Terminal>> {
        &self.terminal
    }

    /// Get the terminal content as a string
    pub fn content(&self) -> String {
        let term = self.terminal.read();
        term.content()
    }

    /// Export entire buffer (scrollback + current screen) as plain text
    ///
    /// This exports all buffer contents with:
    /// - No styling, colors, or graphics (Sixel, etc.)
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Empty lines preserved
    pub fn export_text(&self) -> String {
        let term = self.terminal.read();
        term.export_text()
    }

    /// Export entire buffer (scrollback + current screen) with ANSI styling
    ///
    /// This exports all buffer contents with:
    /// - Full ANSI escape sequences for colors and text attributes
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Efficient escape sequence generation (only emits changes)
    pub fn export_styled(&self) -> String {
        let term = self.terminal.read();
        term.export_styled()
    }

    /// Take a screenshot of the current visible buffer
    ///
    /// Renders the terminal's visible screen buffer to an image using the provided configuration.
    ///
    /// # Arguments
    /// * `config` - Screenshot configuration (font, size, format, etc.)
    /// * `scrollback_offset` - Number of lines to scroll back from current position (default: 0)
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` - Image bytes in the configured format
    /// * `Err(ScreenshotError)` - If rendering or encoding fails
    #[cfg(feature = "screenshot")]
    pub fn screenshot(
        &self,
        config: crate::screenshot::ScreenshotConfig,
        scrollback_offset: usize,
    ) -> crate::screenshot::ScreenshotResult<Vec<u8>> {
        let term = self.terminal.read();
        crate::screenshot::render_terminal(&term, config, scrollback_offset)
    }

    /// Take a screenshot and save to file
    ///
    /// Convenience method to render and save a screenshot directly to a file.
    ///
    /// # Arguments
    /// * `path` - Output file path
    /// * `config` - Screenshot configuration
    /// * `scrollback_offset` - Number of lines to scroll back from current position (default: 0)
    ///
    /// # Returns
    /// * `Ok(())` - Success
    /// * `Err(ScreenshotError)` - If rendering, encoding, or writing fails
    #[cfg(feature = "screenshot")]
    pub fn screenshot_to_file(
        &self,
        path: &std::path::Path,
        config: crate::screenshot::ScreenshotConfig,
        scrollback_offset: usize,
    ) -> crate::screenshot::ScreenshotResult<()> {
        let term = self.terminal.read();
        crate::screenshot::save_terminal(&term, path, config, scrollback_offset)
    }

    /// Get the current cursor position `(col, row)`
    ///
    /// Wait-free (ENH-023): served from an atomic mirror the reader thread
    /// and resize paths refresh while holding the terminal write lock. The
    /// value is individually consistent but may briefly lag the terminal by
    /// one in-flight `process()` batch; for a size/cursor pair consistent
    /// with each other use [`snapshot_geometry`](Self::snapshot_geometry).
    pub fn cursor_position(&self) -> (usize, usize) {
        self.geometry.cursor()
    }

    /// Get the terminal size
    ///
    /// Wait-free (ENH-023): served from an atomic mirror refreshed on every
    /// resize and by the reader thread — `resize` is visible immediately,
    /// with no `process()` call needed. See
    /// [`cursor_position`](Self::cursor_position) for the consistency
    /// contract.
    pub fn size(&self) -> (usize, usize) {
        self.geometry.size()
    }

    /// Size and cursor position read under the terminal read lock, so the
    /// pair is mutually consistent — the escape hatch for callers that
    /// cannot tolerate [`size`](Self::size) and
    /// [`cursor_position`](Self::cursor_position) straddling a resize.
    ///
    /// Returns `((cols, rows), (cursor_col, cursor_row))`.
    pub fn snapshot_geometry(&self) -> ((usize, usize), (usize, usize)) {
        let term = self.terminal.read();
        let (cols, rows) = term.size();
        let cursor = term.cursor();
        ((cols, rows), (cursor.col, cursor.row))
    }

    /// Get a specific line from the active terminal buffer
    ///
    /// This returns a line from whichever screen buffer is currently active
    /// (primary or alternate).
    pub fn get_line(&self, row: usize) -> Option<String> {
        let term = self.terminal.read();
        term.active_grid()
            .row(row)
            .map(|line| line.iter().map(|cell| cell.c).collect())
    }

    /// Get scrollback content
    pub fn scrollback(&self) -> Vec<String> {
        let term = self.terminal.read();
        term.scrollback()
    }

    /// Get the number of scrollback lines
    pub fn scrollback_len(&self) -> usize {
        let term = self.terminal.read();
        term.active_grid().scrollback_len()
    }

    /// Get the current update generation number
    ///
    /// This number is incremented every time the terminal content changes.
    /// Useful for detecting when to redraw in event loops.
    ///
    /// # Returns
    /// The current generation number
    pub fn update_generation(&self) -> u64 {
        self.update_generation.load(Ordering::SeqCst)
    }

    /// Mark the session's content as updated from OUTSIDE the PTY reader
    /// thread — the generation bump + waiter wake the reader performs on
    /// every read. For a session with no child (par-mux panes fed via
    /// `process_data`), nothing else ever advances the generation, so a
    /// generation-keyed render cache would serve stale cells forever.
    pub fn mark_updated(&self) {
        self.update_generation.fetch_add(1, Ordering::SeqCst);
        let _guard = self.update_signal.0.lock();
        self.update_signal.1.notify_all();
    }

    /// Check if the terminal has been updated since a given generation
    ///
    /// # Arguments
    /// * `last_generation` - The generation number from a previous call to `update_generation()`
    ///
    /// # Returns
    /// True if updates have occurred since the given generation
    pub fn has_updates_since(&self, last_generation: u64) -> bool {
        self.update_generation() > last_generation
    }

    /// A shareable handle to the session's wait primitives (ENH-011).
    ///
    /// `PtySession` itself is not `Sync` (its master PTY handle is
    /// `Send`-only), so a Python binding that releases the GIL cannot hold
    /// `&PtySession` across the wait. This handle carries only the `Arc`s
    /// the wait needs and is `Send + Sync + Clone`.
    pub fn update_waiter(&self) -> UpdateWaiter {
        UpdateWaiter {
            terminal: Arc::clone(&self.terminal),
            signal: Arc::clone(&self.update_signal),
            generation: Arc::clone(&self.update_generation),
            running: Arc::clone(&self.running),
        }
    }

    /// Block until the update generation advances past `since` or `timeout`
    /// elapses — see [`UpdateWaiter::wait_for_update`].
    pub fn wait_for_update(&self, since: u64, timeout: std::time::Duration) -> Option<u64> {
        self.update_waiter().wait_for_update(since, timeout)
    }

    /// Block until `predicate` holds on the terminal, re-checking after
    /// every applied update, or until `timeout` elapses — see
    /// [`UpdateWaiter::wait_until`].
    pub fn wait_until(
        &self,
        timeout: std::time::Duration,
        predicate: impl Fn(&Terminal) -> bool,
    ) -> bool {
        self.update_waiter().wait_until(timeout, predicate)
    }

    /// Get the current bell event count
    ///
    /// This counter increments each time the terminal receives a bell character (BEL/\x07).
    /// Applications can poll this to detect bell events for visual bell implementations.
    ///
    /// # Returns
    /// The total number of bell events received since terminal creation
    pub fn bell_count(&self) -> u64 {
        self.terminal.read().bell_count()
    }

    // === Coprocess Management ===

    /// Start a new coprocess
    ///
    /// The coprocess receives terminal output on its stdin (if copy_terminal_output is true)
    /// and its stdout is buffered for reading via `read_from_coprocess()`.
    pub fn start_coprocess(&self, config: CoprocessConfig) -> Result<CoprocessId, String> {
        let mut mgr = self.coprocess_manager.lock();
        mgr.start(config)
    }

    /// Stop a coprocess by ID
    pub fn stop_coprocess(&self, id: CoprocessId) -> Result<(), String> {
        let mut mgr = self.coprocess_manager.lock();
        mgr.stop(id)
    }

    /// Write data to a coprocess's stdin
    pub fn write_to_coprocess(&self, id: CoprocessId, data: &[u8]) -> Result<(), String> {
        let mgr = self.coprocess_manager.lock();
        mgr.write(id, data)
    }

    /// Read buffered output from a coprocess (drains the buffer)
    pub fn read_from_coprocess(&self, id: CoprocessId) -> Result<Vec<String>, String> {
        let mgr = self.coprocess_manager.lock();
        mgr.read(id)
    }

    /// List all coprocess IDs
    pub fn list_coprocesses(&self) -> Vec<CoprocessId> {
        let mgr = self.coprocess_manager.lock();
        mgr.list()
    }

    /// Check if a coprocess is still running
    pub fn coprocess_status(&self, id: CoprocessId) -> Option<bool> {
        let mgr = self.coprocess_manager.lock();
        mgr.status(id)
    }

    /// Read buffered stderr output from a coprocess (drains the buffer)
    pub fn read_coprocess_errors(&self, id: CoprocessId) -> Result<Vec<String>, String> {
        let mgr = self.coprocess_manager.lock();
        mgr.read_errors(id)
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Stop all coprocesses
        {
            let mut mgr = self.coprocess_manager.lock();
            mgr.stop_all();
        }

        // Kill the child process if still running
        if self.is_running() {
            let _ = self.kill();
        }

        // Signal the reader thread to stop (best-effort: it is typically blocked
        // in `read()` and only actually unblocks once the master fd / child
        // process closes below).
        self.running.store(false, Ordering::SeqCst);

        // Force-close the writer AND the PTY master fd BEFORE waiting for the
        // reader thread (ARC-018). The reader is blocked on `read()`; combined
        // with the child kill above (which closes the slave side), closing our
        // copy of the master lets the blocked read return EOF so the thread
        // joins promptly instead of being detached at the timeout. This mirrors
        // the already-correct restart path `cleanup_previous_session()`.
        if let Some(writer) = self.writer.take() {
            drop(writer);
        }
        if let Some(master) = self.pty_master.take() {
            drop(master);
        }
        self.held_slave = None;

        // Wait for the reader thread to finish with timeout
        if let Some(handle) = self.reader_thread.take() {
            use std::time::Duration;

            // Give it 2 seconds to finish gracefully
            let timeout = Duration::from_secs(2);
            let start = std::time::Instant::now();

            // Poll for thread completion
            while !handle.is_finished() && start.elapsed() < timeout {
                std::thread::sleep(Duration::from_millis(10));
            }

            if handle.is_finished() {
                let _ = handle.join();
                debug_log!("PTY_SHUTDOWN", "Reader thread joined successfully");
            } else {
                debug_info!(
                    "PTY_SHUTDOWN",
                    "Reader thread did not finish within {}s timeout, abandoning join",
                    timeout.as_secs()
                );
                // Thread will be detached and cleaned up by OS
                // This prevents indefinite hang during shutdown
            }
        }

        debug_log!("PTY_SHUTDOWN", "PtySession dropped");
    }
}

/// The shareable half of a [`PtySession`]'s wait primitives (ENH-011):
/// the update condvar, the generation counter, the running flag, and the
/// terminal — everything [`UpdateWaiter::wait_for_update`] blocks on, none
/// of it needing `&PtySession` (which is not `Sync`).
#[derive(Clone)]
pub struct UpdateWaiter {
    terminal: Arc<RwLock<Terminal>>,
    signal: Arc<(parking_lot::Mutex<()>, parking_lot::Condvar)>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    running: Arc<AtomicBool>,
}

impl UpdateWaiter {
    /// Block until the update generation advances past `since` or `timeout`
    /// elapses.
    ///
    /// Signalled by the reader thread's content-applied generation bump, so
    /// wakeups are immediate against a sleep poll — and race-free: the
    /// signal fires only after the terminal write guard has dropped, so a
    /// woken caller sees applied content, not in-flight bytes.
    ///
    /// # Arguments
    /// * `since` - The generation to wait past (from `update_generation()`)
    /// * `timeout` - Maximum time to block
    ///
    /// # Returns
    /// The new generation, or `None` on timeout or child exit with no new
    /// content (nothing further can arrive, so waiting would just hang).
    pub fn wait_for_update(&self, since: u64, timeout: std::time::Duration) -> Option<u64> {
        let deadline = std::time::Instant::now() + timeout;
        let (lock, signal) = &*self.signal;
        let mut guard = lock.lock();
        loop {
            let now = self.generation.load(Ordering::SeqCst);
            if now > since {
                return Some(now);
            }
            if !self.running.load(Ordering::SeqCst) && now == since {
                return None;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            signal.wait_for(&mut guard, remaining);
        }
    }

    /// Block until `predicate` holds on the terminal, re-checking after
    /// every applied update, or until `timeout` elapses.
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to block
    /// * `predicate` - Checked against a read-locked terminal; must not block
    ///
    /// # Returns
    /// `true` if the predicate held within the timeout.
    pub fn wait_until(
        &self,
        timeout: std::time::Duration,
        predicate: impl Fn(&Terminal) -> bool,
    ) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut gen = self.generation.load(Ordering::SeqCst);
        loop {
            if predicate(&self.terminal.read()) {
                return true;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match self.wait_for_update(gen, remaining) {
                Some(next) => gen = next,
                // Timed out or the child exited: either way, content that
                // raced the final generation check deserves one last look.
                None => return predicate(&self.terminal.read()),
            }
        }
    }
}

#[cfg(test)]
mod tests;
