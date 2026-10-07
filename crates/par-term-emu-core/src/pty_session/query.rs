//! Terminal access and read-only queries: locks, text export,
//! screenshots, geometry, and scrollback.

use super::*;

impl PtySession {
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
    /// use par_term_emu_core::pty_session::PtySession;
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
    /// use par_term_emu_core::pty_session::PtySession;
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
}
