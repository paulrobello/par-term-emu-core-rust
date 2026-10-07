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

// `PtySession` methods by concern (ARC-003): spawn/lifecycle, input and
// resize I/O, terminal queries, update generation, coprocess forwarding.
mod coprocess;
mod io;
mod lifecycle;
mod query;
mod updates;
#[doc(hidden)]
pub use io::pixel_extent;
#[cfg(unix)]
use io::send_sigwinch;
// mux is the only consumer of the cloneable input handle.
#[cfg(feature = "mux")]
#[doc(hidden)]
pub use io::PtyInputHandle;

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

    // === Coprocess Management ===
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
