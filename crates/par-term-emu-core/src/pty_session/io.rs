//! PTY I/O: output callbacks, input writes (and the cloneable input
//! handle mux uses), resize, and SIGWINCH delivery.

use super::*;
use crate::terminal::ObserverDispatchBatch;
use std::sync::mpsc::{Receiver, SyncSender};

/// Capacity of the per-session input queue: the number of `write` chunks a
/// caller may run ahead of the tty before backpressure blocks it. 16 chunks
/// of 64 KiB caps the queue at 1 MiB per session.
const INPUT_QUEUE_CHUNKS: usize = 16;

/// Per-session background input writer (card 01a218f2). `PtySession::write`
/// and [`PtyInputHandle::write`] enqueue onto a bounded channel instead of
/// blocking on the tty round trip under the writer mutex; a dedicated
/// thread drains it into the PTY under the same writer mutex the reader
/// thread's query forwarding and `get_writer()` consumers use, so all tty
/// input stays mutually serialized. Input recording happens at enqueue
/// time, synchronous on the caller thread under the recording-flag gate —
/// the exact recording semantics the pre-queue code had (after
/// `stop_recording()` returns, every `write` that returned `Ok` before it
/// is in the recording; byte-exact). A tty write failure latches a sticky
/// error that the next `write` reports; the thread keeps consuming
/// (discarding) so senders never block on a broken tty.
pub(super) struct InputWriterQueue {
    sender: Mutex<Option<SyncSender<Vec<u8>>>>,
    error: Arc<Mutex<Option<String>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    terminal: Arc<RwLock<Terminal>>,
    recording: Arc<AtomicBool>,
}

impl InputWriterQueue {
    /// Create the queue and start its drain thread over the PTY writer.
    /// Called from `spawn`; `writer` is the same handle the reader thread
    /// and `get_writer()` consumers share.
    pub(super) fn start(
        writer: Arc<Mutex<Box<dyn Write + Send>>>,
        terminal: Arc<RwLock<Terminal>>,
        recording: Arc<AtomicBool>,
    ) -> Self {
        let (sender, receiver) = std::sync::mpsc::sync_channel(INPUT_QUEUE_CHUNKS);
        let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let thread_error = Arc::clone(&error);
        let spawned = std::thread::Builder::new()
            .name("pty-input-writer".into())
            .spawn(move || input_writer_thread(receiver, writer, thread_error));
        if let Err(e) = spawned.as_ref() {
            debug::log(
                debug::DebugLevel::Error,
                "PTY_INPUT",
                &format!("input writer thread spawn failed: {e}"),
            );
        }
        Self {
            sender: Mutex::new(Some(sender)),
            error,
            thread: Mutex::new(spawned.ok()),
            terminal,
            recording,
        }
    }

    /// Close the queue's own sender clone and join the drain thread with a
    /// bounded timeout — the drop/restart counterpart of the reader
    /// thread's 2 s join. Channel closure + drain of pending bytes exits
    /// the thread; a thread still blocked in `write_all` on a live tty is
    /// detached (the master close that follows fails it fast).
    pub(super) fn shutdown(&self) {
        *self.sender.lock() = None;
        let handle = self.thread.lock().take();
        if let Some(handle) = handle {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !handle.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                debug::log(
                    debug::DebugLevel::Info,
                    "PTY_INPUT",
                    "input writer thread did not finish within 2s, detaching",
                );
            }
        }
    }

    /// Enqueue `data` for the drain thread. Backpressure: blocks while
    /// [`INPUT_QUEUE_CHUNKS`] chunks are pending. A latched tty failure is
    /// reported sticky; a failed enqueue reports the latched error or, if
    /// the drain thread is gone without one, a generic `IoError`.
    pub(super) fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        if let Some(err) = self.error.lock().clone() {
            return Err(PtyError::IoError(std::io::Error::other(err)));
        }
        // Record at enqueue time, synchronous on the caller — the flag
        // gate means the exclusive Terminal lock is paid only while a
        // recording is actually running (card 01a218f2 criterion 2).
        if self.recording.load(Ordering::SeqCst) {
            self.terminal.write().record_input(data);
        }
        let send_result = self.sender.lock().as_ref().map(|s| s.send(data.to_vec()));
        match send_result {
            None | Some(Err(_)) => {
                let latched = self.error.lock().clone();
                return Err(input_writer_gone_error(latched));
            }
            Some(Ok(())) => {}
        }
        Ok(())
    }
}

/// Build the error for a failed enqueue — the latched tty failure when one
/// exists, otherwise a generic "drain thread is gone" `IoError`.
fn input_writer_gone_error(latched: Option<String>) -> PtyError {
    PtyError::IoError(std::io::Error::other(
        latched.unwrap_or_else(|| "PTY input writer thread is gone".into()),
    ))
}

/// The drain loop: FIFO order, tty-only work, sticky error latch with
/// discard-on-error so senders never block on a broken tty.
fn input_writer_thread(
    receiver: Receiver<Vec<u8>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    error: Arc<Mutex<Option<String>>>,
) {
    while let Ok(data) = receiver.recv() {
        if error.lock().is_some() {
            continue;
        }
        let result = {
            let mut w = writer.lock();
            w.write_all(&data).and_then(|()| w.flush())
        };
        if let Err(e) = result {
            *error.lock() = Some(e.to_string());
        }
    }
}

/// The shared handles one PTY input write needs — the input queue and the
/// liveness flag — cloned out of a session so the write can happen with no
/// broader lock held. [`PtyInputHandle::write`] mirrors
/// [`PtySession::write`] exactly: same liveness check, same input
/// recording, same errors.
#[cfg(feature = "mux")]
#[derive(Clone)]
#[doc(hidden)]
pub struct PtyInputHandle {
    queue: Arc<InputWriterQueue>,
    running: Arc<AtomicBool>,
}

#[cfg(feature = "mux")]
impl PtyInputHandle {
    /// Write input bytes to the PTY — [`PtySession::write`] without
    /// `&mut self`, for a caller holding a broader lock (the mux
    /// dispatcher's tree mutex) that must release it before a write.
    ///
    /// # Arguments
    /// * `data` - Bytes to write to the PTY
    ///
    /// # Returns
    /// `Ok(())` once the bytes are enqueued for the drain thread (not yet
    /// necessarily on the tty); the same errors as [`PtySession::write`] —
    /// `NotStartedError` when the session is not running, `IoError` for a
    /// latched tty failure (sticky, may report one call after the failing
    /// write) or a gone drain thread.
    #[doc(hidden)]
    pub fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(PtyError::NotStartedError);
        }
        self.queue.write(data)
    }
}

/// Pixel extent of `cells` cells at `cell_px` pixels each, saturated to the
/// `u16` a `winsize`/`PtySize` field can hold (QA-182: `cols * cell_w` in
/// `u16` overflowed at 2000 columns of 40 px cells).
#[doc(hidden)]
pub fn pixel_extent(cells: u16, cell_px: u16) -> u16 {
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
pub(super) fn send_sigwinch(pid: u32, tag: &str, context: &str) -> std::io::Result<()> {
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
        log::warn!(
            "par-term-emu: SIGWINCH delivery failed for {context}: group {group_err}, pid {pid_err}"
        );
        Err(pid_err)
    }
}

impl PtySession {
    /// Set a callback to be called whenever raw output is received from the PTY
    ///
    /// The callback will be called with the raw bytes before they are processed
    /// by the terminal. This is useful for streaming, logging, or recording.
    ///
    /// # Example
    /// ```no_run
    /// use par_term_emu_core::pty_session::PtySession;
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

    /// Set the damage-aware output callback
    /// ([`OutputDamageCallback`]). When set, the reader thread invokes it
    /// INSTEAD of any plain callback set with `set_output_callback`, with
    /// the same raw bytes plus the damage generation and the inclusive
    /// viewport row ranges the chunk changed since this consumer's previous
    /// chunk (ascending, 0-based, viewport-relative). Wholesale changes
    /// (scrolls, alt-screen switches, RIS, resize, snapshot restore) report
    /// the whole viewport, so a consumer's stale row set never survives
    /// them.
    pub fn set_output_callback_with_damage(&mut self, callback: OutputDamageCallback) {
        *self.output_damage_callback.lock() = Some(callback);
    }

    /// Remove the output callback
    pub fn clear_output_callback(&mut self) {
        *self.output_callback.lock() = None;
        *self.output_damage_callback.lock() = None;
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

    /// Write data to the PTY (send to the child process)
    ///
    /// The bytes are enqueued onto the session's bounded input queue; a
    /// dedicated thread writes them to the tty in FIFO order, so a large
    /// upload (or any write the kernel backpressures) never blocks this
    /// call on the tty round trip (card 01a218f2). `write` blocks only
    /// while 16 chunks are pending — the backpressure window — and takes
    /// no Terminal lock when recording is off (when recording is on, the
    /// enqueue-time `record_input` takes the lock, exactly like the
    /// pre-queue code).
    ///
    /// # Arguments
    /// * `data` - Bytes to write
    ///
    /// # Errors
    /// [`PtyError::NotStartedError`] when the session is not running.
    /// [`PtyError::IoError`] for a tty write failure — sticky, and may
    /// report one call after the failing write (the enqueue-time call
    /// returns `Ok` and the failure is latched for the next call).
    pub fn write(&mut self, data: &[u8]) -> Result<(), PtyError> {
        if !self.is_running() {
            return Err(PtyError::NotStartedError);
        }
        let Some(queue) = &self.input_queue else {
            return Err(PtyError::NotStartedError);
        };
        queue.write(data)
    }

    /// Write a string to the PTY (convenience method)
    ///
    /// # Arguments
    /// * `s` - String to write
    pub fn write_str(&mut self, s: &str) -> Result<(), PtyError> {
        self.write(s.as_bytes())
    }

    #[cfg(feature = "mux")]
    /// The input path detached from `&mut self`: the input queue and the
    /// liveness flag as shared handles, so a caller holding a broader lock
    /// (the mux dispatcher's tree mutex) can clone them under the lock,
    /// release it, and only then write — the enqueue blocks only on the
    /// queue's backpressure window, never on the tty round trip (QA-225,
    /// card 01a218f2). `None` when the session has no PTY input queue
    /// (never spawned), the same [`PtyError::NotStartedError`] condition
    /// [`PtySession::write`] reports.
    #[doc(hidden)]
    pub fn input_handle(&self) -> Option<PtyInputHandle> {
        Some(PtyInputHandle {
            queue: self.input_queue.as_ref()?.clone(),
            running: Arc::clone(&self.running),
        })
    }

    /// Resize the PTY and terminal
    ///
    /// The PTY half runs first: a failed ioctl (or SIGWINCH delivery)
    /// returns before the grid reflows, so the grid can never run ahead of
    /// the child's kernel winsize (card 01a1dd15). On success both halves
    /// land at the same size; on failure a caller retry re-runs the pair.
    ///
    /// # Arguments
    /// * `cols` - New number of columns
    /// * `rows` - New number of rows
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
        let result = self.resize_pty(cols, rows);
        if result.is_ok() {
            self.resize_terminal(cols, rows).deliver();
        }
        result
    }

    /// [`Self::resize`] without delivering observer events: the batch is
    /// returned for the caller to deliver once every lock it holds around
    /// this session is released (a mux tree mutex, for one). Same PTY-first
    /// ordering as [`Self::resize`]: on a failed PTY resize the terminal is
    /// left untouched and the batch comes back empty — the old
    /// reflow-first ordering delivered real events for a resize the child
    /// never agreed to (card 01a1dd15).
    #[cfg(feature = "mux")]
    #[doc(hidden)]
    pub fn resize_deferred(
        &mut self,
        cols: u16,
        rows: u16,
    ) -> (Result<(), PtyError>, ObserverDispatchBatch) {
        let result = self.resize_pty(cols, rows);
        if let Ok(()) = result {
            let batch = self.resize_terminal(cols, rows);
            return (result, batch);
        }
        (result, ObserverDispatchBatch::default())
    }

    /// The terminal half of a resize: grid and geometry, with observer
    /// events returned undelivered so delivery waits for the guard to drop
    /// (ARC-001).
    fn resize_terminal(&mut self, cols: u16, rows: u16) -> ObserverDispatchBatch {
        self.cols = cols;
        self.rows = rows;
        let mut term = self.terminal.write();
        let batch = term.resize_deferred(cols as usize, rows as usize);
        self.geometry.publish(&term);
        batch
    }

    /// The PTY half of [`Self::resize_deferred`]: the pixel-aware PTY
    /// resize at the tracked cell pixel size (updated by
    /// `resize_with_pixels`; the construction default until one runs).
    fn resize_pty(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
        let pixel_width = pixel_extent(cols, self.cell_pixel_width);
        let pixel_height = pixel_extent(rows, self.cell_pixel_height);
        self.resize_pty_with_pixels(cols, rows, pixel_width, pixel_height)
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
        let result = self.resize_pty_with_pixels(cols, rows, pixel_width, pixel_height);
        if result.is_ok() {
            self.resize_terminal_with_pixels(cols, rows, pixel_width, pixel_height)
                .deliver();
        }
        result
    }

    /// [`Self::resize_with_pixels`] without delivering observer events —
    /// the pixel-aware counterpart of [`Self::resize_deferred`], with the
    /// same batch-on-failure contract.
    /// [`Self::resize_with_pixels`] without delivering observer events —
    /// the pixel-aware counterpart of [`Self::resize_deferred`], with the
    /// same PTY-first ordering and the same empty-batch-on-failure
    /// contract.
    #[cfg(feature = "mux")]
    #[doc(hidden)]
    pub fn resize_with_pixels_deferred(
        &mut self,
        cols: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> (Result<(), PtyError>, ObserverDispatchBatch) {
        let result = self.resize_pty_with_pixels(cols, rows, pixel_width, pixel_height);
        if let Ok(()) = result {
            let batch = self.resize_terminal_with_pixels(cols, rows, pixel_width, pixel_height);
            return (result, batch);
        }
        (result, ObserverDispatchBatch::default())
    }

    /// The terminal half of a pixel-aware resize: grid, pixel size, and
    /// geometry, with observer events returned undelivered so delivery
    /// waits for the guard to drop (ARC-001).
    fn resize_terminal_with_pixels(
        &mut self,
        cols: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> ObserverDispatchBatch {
        self.cols = cols;
        self.rows = rows;
        // Cache per-cell pixel size so plain `resize()` (without pixels) and
        // future spawns can reuse it without falling back to the 10×20 default.
        if cols > 0 && rows > 0 {
            self.cell_pixel_width = (pixel_width / cols).max(1);
            self.cell_pixel_height = (pixel_height / rows).max(1);
        }
        let mut term = self.terminal.write();
        let batch = term.resize_deferred(cols as usize, rows as usize);
        term.set_pixel_size(pixel_width as usize, pixel_height as usize);
        self.geometry.publish(&term);
        batch
    }

    /// The PTY half of [`Self::resize_with_pixels_deferred`]: kernel
    /// winsize with pixels, then SIGWINCH.
    fn resize_pty_with_pixels(
        &mut self,
        cols: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<(), PtyError> {
        // Test seam (card 01a1dd15): inject an ioctl failure without a real
        // failing master.
        #[cfg(all(test, unix))]
        if self
            .fail_next_resize_step
            .load(std::sync::atomic::Ordering::Relaxed)
            == 1
        {
            return Err(PtyError::ResizeError(
                "injected master.resize failure".into(),
            ));
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
                log::warn!("par-term-emu: PTY ioctl resize to {cols}x{rows} failed: {e}");
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

        // Manually deliver SIGWINCH after the pty resize: portable-pty's
        // resize() updates the kernel winsize but may not reliably deliver
        // the signal in all scenarios. Group-then-PID delivery and failure
        // logging live in [`send_sigwinch`]. A delivery failure surfaces to
        // the caller (card 01a1dd15): the kernel winsize has already moved,
        // so the caller's reflow (which only runs after this returns Ok)
        // is skipped and the grid cannot run ahead of a child that was
        // never notified.
        #[cfg(unix)]
        if let Err(err) = self.signal_winch(
            "PTY_RESIZE",
            &format!("resize {cols}x{rows} ({pixel_width}x{pixel_height} px)"),
        ) {
            log::warn!(
                "par-term-emu: SIGWINCH delivery failed after PTY resize to {cols}x{rows}: {err}"
            );
            return Err(PtyError::ResizeError(format!(
                "SIGWINCH delivery failed: {err}"
            )));
        }

        Ok(())
    }

    /// Deliver SIGWINCH to the child unless it has been reaped (SEC-125):
    /// the reap record stays locked across the delivery, so no reap can
    /// release the PID between the check and the `kill(2)`. The delivery
    /// result propagates, so a failed resize is observable and the reflow
    /// can be skipped (card 01a1dd15).
    #[cfg(unix)]
    pub(super) fn signal_winch(&self, tag: &str, context: &str) -> std::io::Result<()> {
        let record = self.reaped.lock();
        if record.is_some() {
            return Ok(());
        }
        if let Some(pid) = self.child_pid {
            #[cfg(test)]
            if self
                .fail_next_resize_step
                .load(std::sync::atomic::Ordering::Relaxed)
                == 2
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected SIGWINCH failure",
                ));
            }
            #[cfg(test)]
            self.signals_sent.fetch_add(1, Ordering::SeqCst);
            send_sigwinch(pid, tag, context)
        } else {
            Ok(())
        }
    }
}
