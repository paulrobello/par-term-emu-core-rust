use crate::debug;
use crate::pty_error::PtyError;
use crate::terminal::Terminal;
use portable_pty::{native_pty_system, Child, CommandBuilder, PtyPair, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};

/// A PTY session that manages a shell process and terminal state
pub struct PtySession {
    terminal: Arc<Mutex<Terminal>>,
    pty_pair: Option<PtyPair>,
    child: Option<Box<dyn Child + Send + Sync>>,
    reader_thread: Option<JoinHandle<()>>,
    writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    running: Arc<AtomicBool>,
    env_vars: Vec<(String, String)>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    update_generation: Arc<std::sync::atomic::AtomicU64>,
    /// Whether to reply to XTWINOPS queries (cached from env var PAR_TERM_REPLY_XTWINOPS)
    reply_xtwinops: Arc<AtomicBool>,
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

        Self {
            terminal: Arc::new(Mutex::new(Terminal::with_scrollback(
                cols,
                rows,
                max_scrollback,
            ))),
            pty_pair: None,
            child: None,
            reader_thread: None,
            writer: None,
            running: Arc::new(AtomicBool::new(false)),
            env_vars: Vec::new(),
            cwd: None,
            cols: cols as u16,
            rows: rows as u16,
            update_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            reply_xtwinops: Arc::new(AtomicBool::new(reply_xtwinops)),
        }
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

    /// Spawn a shell process (auto-detected from environment)
    ///
    /// On Unix: Uses $SHELL or defaults to /bin/sh
    /// On Windows: Uses PowerShell or cmd.exe
    pub fn spawn_shell(&mut self) -> Result<(), PtyError> {
        let shell = Self::get_default_shell();
        let args: Vec<&str> = Vec::new();
        self.spawn(&shell, &args)
    }

    /// Get the default shell for the current platform
    pub fn get_default_shell() -> String {
        if cfg!(windows) {
            // Try PowerShell first, fall back to cmd.exe
            if let Ok(powershell) = std::env::var("COMSPEC") {
                powershell
            } else {
                "cmd.exe".to_string()
            }
        } else {
            // Unix-like: check $SHELL, fall back to /bin/bash
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        }
    }

    /// Spawn a process with the specified command and arguments
    ///
    /// # Arguments
    /// * `command` - The command to execute
    /// * `args` - Command-line arguments
    pub fn spawn(&mut self, command: &str, args: &[&str]) -> Result<(), PtyError> {
        if self.is_running() {
            return Err(PtyError::ProcessSpawnError(
                "Process is already running".to_string(),
            ));
        }

        // Create the PTY system
        let pty_system = native_pty_system();
        let pty_size = PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
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

        // Inherit parent environment variables, but deliberately drop static
        // size hints that confuse apps after a PTY resize.
        // Many libraries (e.g. Python's shutil.get_terminal_size) and some TUIs
        // prioritize COLUMNS/LINES env over TIOCGWINSZ, which leaves them stuck
        // at the parent terminal's size. We omit these so children query the PTY.
        let mut dropped_cols = false;
        let mut dropped_lines = false;
        for (key, value) in std::env::vars() {
            if key == "COLUMNS" || key == "LINES" {
                if key == "COLUMNS" {
                    dropped_cols = true;
                }
                if key == "LINES" {
                    dropped_lines = true;
                }
                continue; // skip misleading static size vars
            }
            cmd.env(&key, &value);
        }
        if dropped_cols || dropped_lines {
            debug::log(
                debug::DebugLevel::Info,
                "PTY_SPAWN",
                &format!(
                    "Dropped env vars: {}{}",
                    if dropped_cols { "COLUMNS " } else { "" },
                    if dropped_lines { "LINES" } else { "" }
                ),
            );
        }

        // Set terminal-specific environment variables
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // NOTE: Do NOT set COLUMNS/LINES environment variables!
        // They are static and won't update on resize. Applications should
        // query terminal size via ioctl(TIOCGWINSZ), not environment variables.
        // Setting these breaks libraries like Textual that use shutil.get_terminal_size()
        // which prioritizes env vars over ioctl.

        // Override with user-specified environment variables
        for (key, value) in &self.env_vars {
            cmd.env(key, value);
        }

        // Set working directory
        if let Some(ref cwd) = self.cwd {
            cmd.cwd(cwd);
        }

        // Spawn the child process
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))?;

        // Get the master reader
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))?;

        // Get the master writer (wrapped in Arc<Mutex<>> for shared access)
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))?;
        let writer = Arc::new(Mutex::new(writer));

        // Get child PID before storing
        let child_pid = child.process_id();

        // Store the PTY pair and child
        self.pty_pair = Some(pair);
        self.child = Some(child);
        self.writer = Some(Arc::clone(&writer));
        self.running.store(true, Ordering::SeqCst);

        // Spawn the reader thread (shares writer for device query responses)
        self.start_reader_thread(reader, writer, child_pid);

        Ok(())
    }

    /// Start the reader thread that processes PTY output
    fn start_reader_thread(
        &mut self,
        mut reader: Box<dyn Read + Send>,
        writer: Arc<Mutex<Box<dyn Write + Send>>>,
        child_pid: Option<u32>,
    ) {
        let terminal = Arc::clone(&self.terminal);
        let running = Arc::clone(&self.running);
        let update_generation = Arc::clone(&self.update_generation);
        let reply_xtwinops = Arc::clone(&self.reply_xtwinops);

        let handle = thread::spawn(move || {
            let mut buffer = [0u8; 16384];

            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        // EOF - process has exited
                        running.store(false, Ordering::SeqCst);
                        break;
                    }
                    Ok(n) => {
                        debug::log_pty_read(n);
                        // Process the bytes through the terminal
                        if let Ok(mut term) = terminal.lock() {
                            let old_gen = update_generation.load(Ordering::SeqCst);
                            let was_alt_screen = term.is_alt_screen_active();
                            term.process(&buffer[..n]);
                            let is_alt_screen = term.is_alt_screen_active();

                            // Check for device query responses and write them back to the PTY
                            // This enables nested TUI applications (vim, htop, etc.) to work correctly
                            if term.has_pending_responses() {
                                let mut responses = term.drain_responses();

                                // Optional: filter out XTWINOPS (CSI t) replies to avoid shells
                                // echoing them visibly when ECHOCTL is enabled. Controlled by env
                                // PAR_TERM_REPLY_XTWINOPS (default: 1). Set to 0 to suppress.
                                // (Cached from env var at PtySession initialization)
                                if !reply_xtwinops.load(Ordering::Relaxed) {
                                    let mut filtered = Vec::with_capacity(responses.len());
                                    let mut i = 0;
                                    while i < responses.len() {
                                        if responses[i] == 0x1B
                                            && i + 1 < responses.len()
                                            && responses[i + 1] == b'['
                                        {
                                            // Collect until a final byte; drop if final is 't'
                                            let mut j = i + 2;
                                            let mut dropped = false;
                                            while j < responses.len() {
                                                let b = responses[j];
                                                if (b as char).is_ascii_alphabetic() {
                                                    // Alphabetic final byte for CSI
                                                    if b == b't' {
                                                        dropped = true;
                                                    }
                                                    j += 1;
                                                    break;
                                                }
                                                j += 1;
                                            }
                                            if !dropped {
                                                filtered.extend_from_slice(&responses[i..j]);
                                            }
                                            i = j;
                                        } else {
                                            filtered.push(responses[i]);
                                            i += 1;
                                        }
                                    }
                                    responses = filtered;
                                }

                                if !responses.is_empty() {
                                    debug::log_device_query("pending", &responses);
                                    if let Ok(mut w) = writer.lock() {
                                        // Write responses back to PTY master so child can read them
                                        let _ = w.write_all(&responses);
                                        let _ = w.flush();
                                    }
                                }
                            }

                            // Send resize pulse (SIGWINCH) when entering alternate screen
                            // This helps applications like tmux recalculate their layout correctly
                            // (iTerm2 does this, which is why tmux works correctly there)
                            if !was_alt_screen && is_alt_screen {
                                // Get current terminal dimensions (not stale captured values)
                                let (current_cols, current_rows) = term.size();
                                debug::log(
                                    debug::DebugLevel::Info,
                                    "ALT_SCREEN",
                                    "Entered alternate screen - sending SIGWINCH resize pulse",
                                );
                                // Send SIGWINCH to the child process to force layout recalculation
                                #[cfg(unix)]
                                if let Some(pid) = child_pid {
                                    unsafe {
                                        // Send SIGWINCH to the process group
                                        let pgid = -(pid as i32);
                                        let result = libc::kill(pgid, libc::SIGWINCH);
                                        if result == 0 {
                                            debug::log(
                                                debug::DebugLevel::Info,
                                                "ALT_SCREEN",
                                                &format!(
                                                    "SIGWINCH sent to process group -{} ({}x{})",
                                                    pid, current_cols, current_rows
                                                ),
                                            );
                                        } else {
                                            let err = std::io::Error::last_os_error();
                                            debug::log(
                                                debug::DebugLevel::Error,
                                                "ALT_SCREEN",
                                                &format!("Failed to send SIGWINCH: {}", err),
                                            );
                                        }
                                    }
                                }
                            }

                            // Increment update generation to signal content changed
                            let new_gen = update_generation.fetch_add(1, Ordering::SeqCst) + 1;
                            debug::log_generation_change(old_gen, new_gen, "PTY read");
                        }
                    }
                    Err(e) => {
                        // Log error but continue (could be temporary)
                        eprintln!("PTY read error: {}", e);
                        // If it's a fatal error, stop
                        if e.kind() == std::io::ErrorKind::BrokenPipe {
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            }
        });

        self.reader_thread = Some(handle);
    }

    /// Write data to the PTY (send to the child process)
    ///
    /// # Arguments
    /// * `data` - Bytes to write
    pub fn write(&mut self, data: &[u8]) -> Result<(), PtyError> {
        if !self.is_running() {
            return Err(PtyError::NotStartedError);
        }

        debug::log_pty_write(data);
        if let Some(ref writer) = self.writer {
            let mut w = writer
                .lock()
                .map_err(|e| PtyError::LockError(format!("Writer mutex poisoned: {}", e)))?;
            w.write_all(data).map_err(PtyError::IoError)?;
            w.flush().map_err(PtyError::IoError)?;
            Ok(())
        } else {
            Err(PtyError::NotStartedError)
        }
    }

    /// Write a string to the PTY (convenience method)
    ///
    /// # Arguments
    /// * `s` - String to write
    pub fn write_str(&mut self, s: &str) -> Result<(), PtyError> {
        self.write(s.as_bytes())
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
        if let Ok(mut term) = self.terminal.lock() {
            term.resize(cols as usize, rows as usize);
        }

        // Resize the PTY (sends SIGWINCH to child)
        if let Some(ref pair) = self.pty_pair {
            let pty_size = PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            };
            debug::log(
                debug::DebugLevel::Debug,
                "PTY_RESIZE",
                &format!("Calling pair.master.resize({}, {})", cols, rows),
            );
            debug::log(
                debug::DebugLevel::Trace,
                "PTY_RESIZE",
                &format!(
                    "PtySize {{ rows: {}, cols: {}, pixel_width: {}, pixel_height: {} }}",
                    pty_size.rows, pty_size.cols, pty_size.pixel_width, pty_size.pixel_height
                ),
            );
            pair.master.resize(pty_size).map_err(|e| {
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
                "pair.master.resize() completed successfully",
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

        // Manually send SIGWINCH to the child process
        // This ensures the child receives the resize signal, as portable-pty's
        // resize() may not reliably deliver SIGWINCH in all scenarios
        #[cfg(unix)]
        if let Some(ref child) = self.child {
            if let Some(pid) = child.process_id() {
                debug::log(
                    debug::DebugLevel::Debug,
                    "PTY_RESIZE",
                    &format!("Sending SIGWINCH to PID {}", pid),
                );
                unsafe {
                    // Send SIGWINCH to the process group, not just the direct child
                    // This ensures grandchildren (e.g., apps launched from shell) also receive it
                    let result = libc::kill(-(pid as libc::pid_t), libc::SIGWINCH);
                    if result == 0 {
                        debug::log(
                            debug::DebugLevel::Debug,
                            "PTY_RESIZE",
                            &format!("SIGWINCH sent successfully to process group -{}", pid),
                        );
                        debug::log(
                            debug::DebugLevel::Trace,
                            "PTY_RESIZE",
                            &format!("SIGWINCH notified processes of new size: {}x{}", cols, rows),
                        );
                    } else {
                        let errno = std::io::Error::last_os_error();
                        debug::log(
                            debug::DebugLevel::Info,
                            "PTY_RESIZE",
                            &format!("Failed to send SIGWINCH to process group, errno: {}", errno),
                        );
                        // Fallback: send to the process itself
                        libc::kill(pid as libc::pid_t, libc::SIGWINCH);
                        debug::log(
                            debug::DebugLevel::Debug,
                            "PTY_RESIZE",
                            &format!("SIGWINCH sent to PID {} (fallback)", pid),
                        );
                    }
                }
            }
        }

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

        // Resize the terminal and record pixel size
        if let Ok(mut term) = self.terminal.lock() {
            term.resize(cols as usize, rows as usize);
            term.set_pixel_size(pixel_width as usize, pixel_height as usize);
        }

        // Resize the PTY (sends SIGWINCH to child)
        if let Some(ref pair) = self.pty_pair {
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
                    "Calling pair.master.resize({}, {}) with pixels {}x{}",
                    cols, rows, pixel_width, pixel_height
                ),
            );
            pair.master.resize(pty_size).map_err(|e| {
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
                "pair.master.resize() completed successfully",
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

        // Manually send SIGWINCH to the child process (as in resize())
        #[cfg(unix)]
        if let Some(ref child) = self.child {
            if let Some(pid) = child.process_id() {
                debug::log(
                    debug::DebugLevel::Debug,
                    "PTY_RESIZE",
                    &format!("Sending SIGWINCH to PID {}", pid),
                );
                unsafe {
                    let result = libc::kill(-(pid as libc::pid_t), libc::SIGWINCH);
                    if result == 0 {
                        debug::log(
                            debug::DebugLevel::Debug,
                            "PTY_RESIZE",
                            &format!(
                                "SIGWINCH notified processes of new size: {}x{} ({}x{} px)",
                                cols, rows, pixel_width, pixel_height
                            ),
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Check if the process is still running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Try to get the exit status without blocking
    ///
    /// Returns None if the process hasn't exited yet
    pub fn try_wait(&mut self) -> Result<Option<i32>, PtyError> {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.running.store(false, Ordering::SeqCst);
                    Ok(Some(status.exit_code() as i32))
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
    /// This blocks until the process exits
    pub fn wait(&mut self) -> Result<i32, PtyError> {
        if let Some(ref mut child) = self.child {
            let status = child.wait().map_err(PtyError::IoError)?;
            self.running.store(false, Ordering::SeqCst);
            Ok(status.exit_code() as i32)
        } else {
            Err(PtyError::NotStartedError)
        }
    }

    /// Kill the process
    pub fn kill(&mut self) -> Result<(), PtyError> {
        if let Some(ref mut child) = self.child {
            child.kill().map_err(PtyError::IoError)?;
            self.running.store(false, Ordering::SeqCst);
            Ok(())
        } else {
            Err(PtyError::NotStartedError)
        }
    }

    /// Get a reference to the underlying terminal
    pub fn terminal(&self) -> Arc<Mutex<Terminal>> {
        Arc::clone(&self.terminal)
    }

    /// Get the terminal content as a string
    pub fn content(&self) -> String {
        if let Ok(term) = self.terminal.lock() {
            term.content()
        } else {
            String::new()
        }
    }

    /// Export entire buffer (scrollback + current screen) as plain text
    ///
    /// This exports all buffer contents with:
    /// - No styling, colors, or graphics (Sixel, etc.)
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Empty lines preserved
    pub fn export_text(&self) -> String {
        if let Ok(term) = self.terminal.lock() {
            term.export_text()
        } else {
            String::new()
        }
    }

    /// Export entire buffer (scrollback + current screen) with ANSI styling
    ///
    /// This exports all buffer contents with:
    /// - Full ANSI escape sequences for colors and text attributes
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Efficient escape sequence generation (only emits changes)
    pub fn export_styled(&self) -> String {
        if let Ok(term) = self.terminal.lock() {
            term.export_styled()
        } else {
            String::new()
        }
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
    pub fn screenshot(
        &self,
        config: crate::screenshot::ScreenshotConfig,
        scrollback_offset: usize,
    ) -> crate::screenshot::ScreenshotResult<Vec<u8>> {
        if let Ok(term) = self.terminal.lock() {
            term.screenshot(config, scrollback_offset)
        } else {
            Err(crate::screenshot::ScreenshotError::RenderError(
                "Failed to lock terminal".to_string(),
            ))
        }
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
    pub fn screenshot_to_file(
        &self,
        path: &std::path::Path,
        config: crate::screenshot::ScreenshotConfig,
        scrollback_offset: usize,
    ) -> crate::screenshot::ScreenshotResult<()> {
        if let Ok(term) = self.terminal.lock() {
            term.screenshot_to_file(path, config, scrollback_offset)
        } else {
            Err(crate::screenshot::ScreenshotError::RenderError(
                "Failed to lock terminal".to_string(),
            ))
        }
    }

    /// Get the cursor position
    pub fn cursor_position(&self) -> (usize, usize) {
        if let Ok(term) = self.terminal.lock() {
            let cursor = term.cursor();
            (cursor.col, cursor.row)
        } else {
            (0, 0)
        }
    }

    /// Get the terminal size
    pub fn size(&self) -> (usize, usize) {
        if let Ok(term) = self.terminal.lock() {
            term.size()
        } else {
            (self.cols as usize, self.rows as usize)
        }
    }

    /// Get a specific line from the active terminal buffer
    ///
    /// This returns a line from whichever screen buffer is currently active
    /// (primary or alternate).
    pub fn get_line(&self, row: usize) -> Option<String> {
        if let Ok(term) = self.terminal.lock() {
            term.active_grid()
                .row(row)
                .map(|line| line.iter().map(|cell| cell.c).collect())
        } else {
            None
        }
    }

    /// Get scrollback content
    pub fn scrollback(&self) -> Vec<String> {
        if let Ok(term) = self.terminal.lock() {
            term.scrollback()
        } else {
            Vec::new()
        }
    }

    /// Get the number of scrollback lines
    pub fn scrollback_len(&self) -> usize {
        if let Ok(term) = self.terminal.lock() {
            term.grid().scrollback_len()
        } else {
            0
        }
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
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Kill the child process if still running
        if self.is_running() {
            let _ = self.kill();
        }

        // Wait for the reader thread to finish
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_pty_session() {
        let session = PtySession::new(80, 24, 1000);
        assert_eq!(session.size(), (80, 24));
        assert!(!session.is_running());
    }

    #[test]
    fn test_get_default_shell() {
        let shell = PtySession::get_default_shell();
        assert!(!shell.is_empty());
    }

    #[test]
    fn test_spawn_and_exit() {
        let mut session = PtySession::new(80, 24, 1000);

        // Spawn a simple command that exits immediately
        #[cfg(unix)]
        let result = session.spawn("/bin/echo", &["hello"]);
        #[cfg(windows)]
        let result = session.spawn("cmd.exe", &["/C", "echo hello"]);

        assert!(result.is_ok());

        // Give it time to execute
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Process should have exited
        let exit_code = session.try_wait();
        assert!(exit_code.is_ok());
    }

    #[test]
    fn test_write_to_pty() {
        let mut session = PtySession::new(80, 24, 1000);

        // Try writing without spawning - should fail
        let result = session.write(b"test");
        assert!(result.is_err());
    }

    #[test]
    fn test_resize() {
        let mut session = PtySession::new(80, 24, 1000);
        session.resize(100, 30).ok();
        assert_eq!(session.size(), (100, 30));
    }

    #[test]
    fn test_set_env() {
        let mut session = PtySession::new(80, 24, 1000);
        session.set_env("TEST_VAR", "test_value");
        // Just ensure it doesn't panic
    }
}
