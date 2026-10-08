//! PTY I/O: output callbacks, input writes (and the cloneable input
//! handle mux uses), resize, and SIGWINCH delivery.

use super::*;
use crate::terminal::ObserverDispatchBatch;

/// The shared handles one PTY input write needs — writer, terminal, and
/// liveness flag — cloned out of a session so the write can happen with no
/// broader lock held. [`PtyInputHandle::write`] mirrors
/// [`PtySession::write`] exactly: same liveness check, same input recording,
/// same errors.
#[cfg(feature = "mux")]
#[derive(Clone)]
#[doc(hidden)]
pub struct PtyInputHandle {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    terminal: Arc<RwLock<Terminal>>,
    running: Arc<AtomicBool>,
}

#[cfg(feature = "mux")]
impl PtyInputHandle {
    #[doc(hidden)]
    pub fn write(&self, data: &[u8]) -> Result<(), PtyError> {
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
pub(super) fn write_input(
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

    #[cfg(feature = "mux")]
    /// The input path detached from `&mut self`: writer, terminal, and
    /// liveness flag as shared handles, so a caller holding a broader lock
    /// (the mux dispatcher's tree mutex) can clone them under the lock,
    /// release it, and only then write — `write_all` blocks once the PTY
    /// buffer fills (QA-225). `None` when the session has no PTY writer
    /// (never spawned), the same [`PtyError::NotStartedError`] condition
    /// [`PtySession::write`] reports.
    #[doc(hidden)]
    pub fn input_handle(&self) -> Option<PtyInputHandle> {
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
        self.resize_terminal(cols, rows).deliver();
        self.resize_pty(cols, rows)
    }

    /// [`Self::resize`] without delivering observer events: the batch is
    /// returned for the caller to deliver once every lock it holds around
    /// this session is released (a mux tree mutex, for one). The batch
    /// comes back even when the PTY resize fails, because the terminal has
    /// already resized and its events are real.
    #[cfg(feature = "mux")]
    #[doc(hidden)]
    pub fn resize_deferred(
        &mut self,
        cols: u16,
        rows: u16,
    ) -> (Result<(), PtyError>, ObserverDispatchBatch) {
        let dispatch_batch = self.resize_terminal(cols, rows);
        (self.resize_pty(cols, rows), dispatch_batch)
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

    /// The PTY half of [`Self::resize_deferred`]: kernel winsize, then
    /// SIGWINCH.
    fn resize_pty(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
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
        self.resize_terminal_with_pixels(cols, rows, pixel_width, pixel_height)
            .deliver();
        self.resize_pty_with_pixels(cols, rows, pixel_width, pixel_height)
    }

    /// [`Self::resize_with_pixels`] without delivering observer events —
    /// the pixel-aware counterpart of [`Self::resize_deferred`], with the
    /// same batch-on-failure contract.
    #[cfg(feature = "mux")]
    #[doc(hidden)]
    pub fn resize_with_pixels_deferred(
        &mut self,
        cols: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> (Result<(), PtyError>, ObserverDispatchBatch) {
        let dispatch_batch =
            self.resize_terminal_with_pixels(cols, rows, pixel_width, pixel_height);
        (
            self.resize_pty_with_pixels(cols, rows, pixel_width, pixel_height),
            dispatch_batch,
        )
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

    /// Deliver SIGWINCH to the child unless it has been reaped (SEC-125):
    /// the reap record stays locked across the delivery, so no reap can
    /// release the PID between the check and the `kill(2)`.
    #[cfg(unix)]
    pub(super) fn signal_winch(&self, tag: &str, context: &str) {
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
}
