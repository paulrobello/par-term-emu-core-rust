use crate::debug;
use crate::pty_error::PtyError;
use crate::terminal::Terminal;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, PtyPair, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};

/// Callback function for PTY output
///
/// Called whenever raw data is read from the PTY master, before it's processed
/// by the terminal. This allows capturing the raw ANSI stream for logging,
/// recording, or streaming to clients.
///
/// # Arguments
/// * `data` - The raw bytes read from the PTY
pub type OutputCallback = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// A PTY session that manages a shell process and terminal state
pub struct PtySession {
    terminal: Arc<Mutex<Terminal>>,
    /// Master end of the PTY. We intentionally drop the slave side after
    /// spawning the child so the master sees EOF when the child exits.
    pty_master: Option<Box<dyn portable_pty::MasterPty + Send>>,
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
    /// Optional callback for raw PTY output (for streaming, logging, etc.)
    /// Wrapped in Arc<Mutex> so it can be updated after the reader thread starts
    output_callback: Arc<Mutex<Option<OutputCallback>>>,
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
            pty_master: None,
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
            output_callback: Arc::new(Mutex::new(None)),
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
        if cfg!(windows) {
            // Use %COMSPEC% (typically cmd.exe), fall back to cmd.exe
            if let Ok(comspec) = std::env::var("COMSPEC") {
                comspec
            } else {
                "cmd.exe".to_string()
            }
        } else {
            // Unix-like: check $SHELL, fall back to /bin/bash
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        }
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

        // Clean up child process handle (should already be exited)
        if let Some(mut child) = self.child.take() {
            // Try to reap the child if it hasn't been reaped yet
            match child.try_wait() {
                Ok(Some(status)) => {
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
                    // Child still running - kill it
                    debug::log(
                        debug::DebugLevel::Info,
                        "PTY_CLEANUP",
                        "Previous child still running, killing",
                    );
                    let _ = child.kill();
                    let _ = child.wait();
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
        // Calculate pixel dimensions (10x20 per cell - standard terminal font size)
        // This ensures kitten icat and other tools can query pixel dimensions via TIOCGWINSZ
        let pty_size = PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: self.cols * 10,
            pixel_height: self.rows * 20,
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

        // Spawn the child process using the slave side. Drop our handle to the slave
        // immediately after spawn so that when the child exits, the master side sees EOF.
        let PtyPair { master, slave } = pair;
        let child = slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::ProcessSpawnError(e.to_string()))?;
        drop(slave);

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
        let output_callback = Arc::clone(&self.output_callback);

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

                        // Call output callback if set (for streaming, logging, etc.)
                        {
                            let callback_guard = output_callback.lock();
                            if let Some(ref callback) = *callback_guard {
                                callback(&buffer[..n]);
                            }
                        }

                        // Process the bytes through the terminal
                        {
                            let mut term = terminal.lock();
                            let old_gen = update_generation.load(Ordering::SeqCst);
                            let was_alt_screen = term.is_alt_screen_active();
                            term.process(&buffer[..n]);
                            // Record output for session recording
                            term.record_output(&buffer[..n]);
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
                                    {
                                        let mut w = writer.lock();
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
                        crate::debug_error!("PTY", "PTY read error: {}", e);
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

        // Record input for session recording
        {
            let mut term = self.terminal.lock();
            term.record_input(data);
        }

        if let Some(ref writer) = self.writer {
            let mut w = writer.lock();
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
        {
            let mut term = self.terminal.lock();
            term.resize(cols as usize, rows as usize);
            // Record resize event for session recording
            term.record_resize(cols as usize, rows as usize);
        }

        // Resize the PTY (sends SIGWINCH to child)
        if let Some(ref master) = self.pty_master {
            // Calculate pixel dimensions (10x20 per cell - standard terminal font size)
            // This ensures kitten icat and other tools can query pixel dimensions via TIOCGWINSZ
            let pty_size = PtySize {
                rows,
                cols,
                pixel_width: cols * 10,
                pixel_height: rows * 20,
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
        {
            let mut term = self.terminal.lock();
            term.resize(cols as usize, rows as usize);
            term.set_pixel_size(pixel_width as usize, pixel_height as usize);
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
        let term = self.terminal.lock();
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
        let term = self.terminal.lock();
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
        let term = self.terminal.lock();
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
    pub fn screenshot(
        &self,
        config: crate::screenshot::ScreenshotConfig,
        scrollback_offset: usize,
    ) -> crate::screenshot::ScreenshotResult<Vec<u8>> {
        let term = self.terminal.lock();
        term.screenshot(config, scrollback_offset)
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
        let term = self.terminal.lock();
        term.screenshot_to_file(path, config, scrollback_offset)
    }

    /// Get the cursor position
    pub fn cursor_position(&self) -> (usize, usize) {
        let term = self.terminal.lock();
        let cursor = term.cursor();
        (cursor.col, cursor.row)
    }

    /// Get the terminal size
    pub fn size(&self) -> (usize, usize) {
        let term = self.terminal.lock();
        term.size()
    }

    /// Get a specific line from the active terminal buffer
    ///
    /// This returns a line from whichever screen buffer is currently active
    /// (primary or alternate).
    pub fn get_line(&self, row: usize) -> Option<String> {
        let term = self.terminal.lock();
        term.active_grid()
            .row(row)
            .map(|line| line.iter().map(|cell| cell.c).collect())
    }

    /// Get scrollback content
    pub fn scrollback(&self) -> Vec<String> {
        let term = self.terminal.lock();
        term.scrollback()
    }

    /// Get the number of scrollback lines
    pub fn scrollback_len(&self) -> usize {
        let term = self.terminal.lock();
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

    /// Get the current bell event count
    ///
    /// This counter increments each time the terminal receives a bell character (BEL/\x07).
    /// Applications can poll this to detect bell events for visual bell implementations.
    ///
    /// # Returns
    /// The total number of bell events received since terminal creation
    pub fn bell_count(&self) -> u64 {
        self.terminal.lock().bell_count()
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Kill the child process if still running
        if self.is_running() {
            let _ = self.kill();
        }

        // Close writer to help unblock the reader thread
        // This will cause the PTY to close, which should make the reader's read() return an error
        if let Some(writer) = self.writer.take() {
            drop(writer);
        }

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

    #[test]
    fn test_set_multiple_env_vars() {
        let mut session = PtySession::new(80, 24, 1000);
        session.set_env("VAR1", "value1");
        session.set_env("VAR2", "value2");
        session.set_env("VAR3", "value3");
        // Should allow multiple env vars
    }

    #[test]
    fn test_set_cwd() {
        let mut session = PtySession::new(80, 24, 1000);
        let path = std::path::Path::new("/tmp");
        session.set_cwd(path);
        // Just ensure it doesn't panic
    }

    #[test]
    fn test_size_getters() {
        let session = PtySession::new(100, 50, 2000);
        let (cols, rows) = session.size();
        assert_eq!(cols, 100);
        assert_eq!(rows, 50);
    }

    #[test]
    fn test_terminal_access() {
        let session = PtySession::new(80, 24, 1000);
        let terminal = session.terminal();
        let _guard = terminal.lock();
    }

    #[test]
    fn test_update_generation() {
        let session = PtySession::new(80, 24, 1000);
        let gen1 = session.update_generation();
        let gen2 = session.update_generation();
        assert_eq!(gen1, gen2); // Should be same if no updates
    }

    #[test]
    fn test_is_running_initially_false() {
        let session = PtySession::new(80, 24, 1000);
        assert!(!session.is_running());
    }

    #[test]
    fn test_new_with_different_sizes() {
        let session1 = PtySession::new(40, 20, 500);
        assert_eq!(session1.size(), (40, 20));

        let session2 = PtySession::new(120, 40, 2000);
        assert_eq!(session2.size(), (120, 40));

        let session3 = PtySession::new(200, 60, 5000);
        assert_eq!(session3.size(), (200, 60));
    }

    #[test]
    fn test_resize_multiple_times() {
        let mut session = PtySession::new(80, 24, 1000);

        session.resize(100, 30).ok();
        assert_eq!(session.size(), (100, 30));

        session.resize(120, 40).ok();
        assert_eq!(session.size(), (120, 40));

        session.resize(60, 20).ok();
        assert_eq!(session.size(), (60, 20));
    }

    #[test]
    fn test_resize_to_small_size() {
        let mut session = PtySession::new(80, 24, 1000);
        session.resize(10, 5).ok();
        assert_eq!(session.size(), (10, 5));
    }

    #[test]
    fn test_resize_to_large_size() {
        let mut session = PtySession::new(80, 24, 1000);
        session.resize(500, 200).ok();
        assert_eq!(session.size(), (500, 200));
    }

    #[test]
    fn test_write_empty_data() {
        let mut session = PtySession::new(80, 24, 1000);
        let result = session.write(b"");
        assert!(result.is_err()); // Should fail as not spawned
    }

    #[test]
    fn test_get_default_shell_not_empty() {
        let shell = PtySession::get_default_shell();
        assert!(!shell.is_empty());
        #[cfg(unix)]
        assert!(shell.contains("sh") || shell.contains("bash"));
    }

    #[test]
    fn test_terminal_locked_state() {
        let session = PtySession::new(80, 24, 1000);
        {
            let terminal = session.terminal();
            let _lock1 = terminal.lock();
            // While holding lock, should not be able to get another
        }
        // After releasing, should be able to lock again
        let terminal = session.terminal();
        let _lock2 = terminal.lock();
        drop(_lock2); // Explicitly drop to avoid unused variable warning
    }

    #[test]
    fn test_set_env_with_empty_values() {
        let mut session = PtySession::new(80, 24, 1000);
        session.set_env("EMPTY_VAR", "");
        session.set_env("", "value");
        // Should handle edge cases without panicking
    }

    #[test]
    fn test_set_env_with_unicode() {
        let mut session = PtySession::new(80, 24, 1000);
        session.set_env("UNICODE_VAR", "Hello 世界 🌍");
        // Should handle unicode without panicking
    }

    #[test]
    fn test_spawn_with_env() {
        let mut session = PtySession::new(80, 24, 1000);

        // Create env vars to pass
        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "test_value".to_string());

        // Spawn with env vars
        #[cfg(unix)]
        let result = session.spawn_with_env("/bin/echo", &["hello"], Some(&env), None);
        #[cfg(windows)]
        let result = session.spawn_with_env("cmd.exe", &["/C", "echo hello"], Some(&env), None);

        assert!(result.is_ok());

        // Give it time to execute
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    #[test]
    fn test_spawn_shell_with_env() {
        let mut session = PtySession::new(80, 24, 1000);

        // Create env vars to pass
        let mut env = HashMap::new();
        env.insert("MY_SHELL_VAR".to_string(), "shell_value".to_string());

        // Spawn shell with env vars
        let result = session.spawn_shell_with_env(Some(&env), None);
        assert!(result.is_ok());

        // Shell should be running
        assert!(session.is_running());

        // Clean up
        let _ = session.kill();
    }

    #[test]
    #[ignore]
    fn test_spawn_with_env_cwd() {
        let mut session = PtySession::new(80, 24, 1000);

        // Spawn with cwd set to /tmp
        #[cfg(unix)]
        let result = session.spawn_with_env("/bin/pwd", &[], None, Some("/tmp"));
        #[cfg(windows)]
        let result = session.spawn_with_env("cmd.exe", &["/C", "cd"], None, Some("C:\\"));

        assert!(result.is_ok());

        // Wait for expected output with timeout
        let start_wait = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5);
        let mut content = String::new();
        let mut found = false;

        while start_wait.elapsed() < timeout {
            content = session.export_text();
            #[cfg(unix)]
            if content.contains("/tmp") || content.contains("private/tmp") {
                found = true;
                break;
            }
            #[cfg(windows)]
            if content.contains("C:\\") {
                found = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        assert!(found, "Expected directory path in output, got: {}", content);
    }

    #[test]
    fn test_spawn_shell_with_env_cwd() {
        let mut session = PtySession::new(80, 24, 1000);

        // Spawn shell with cwd
        let result = session.spawn_shell_with_env(None, Some("/tmp"));
        assert!(result.is_ok());

        // Shell should be running
        assert!(session.is_running());

        // Clean up
        let _ = session.kill();
    }

    #[test]
    fn test_env_not_leaked_to_parent() {
        // Set a unique env var name that won't exist in parent
        let unique_var = "PTY_TEST_UNIQUE_VAR_12345";

        // Verify it doesn't exist in parent before spawn
        assert!(
            std::env::var(unique_var).is_err(),
            "Test var should not exist in parent env before spawn"
        );

        let mut session = PtySession::new(80, 24, 1000);

        // Create env vars to pass
        let mut env = HashMap::new();
        env.insert(unique_var.to_string(), "test_value".to_string());

        // Spawn with env vars
        #[cfg(unix)]
        let result = session.spawn_with_env("/bin/echo", &["test"], Some(&env), None);
        #[cfg(windows)]
        let result = session.spawn_with_env("cmd.exe", &["/C", "echo test"], Some(&env), None);

        assert!(result.is_ok());

        // Give it time to execute
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Verify env var was NOT leaked to parent process
        assert!(
            std::env::var(unique_var).is_err(),
            "Test var should NOT exist in parent env after spawn"
        );
    }

    #[test]
    fn test_spawn_with_env_and_set_env_combined() {
        let mut session = PtySession::new(80, 24, 1000);

        // Set env vars via set_env()
        session.set_env("VAR_FROM_SET_ENV", "set_env_value");

        // Create additional env vars to pass
        let mut env = HashMap::new();
        env.insert("VAR_FROM_SPAWN".to_string(), "spawn_value".to_string());

        // Spawn with both set_env vars and additional env vars
        #[cfg(unix)]
        let result = session.spawn_with_env("/bin/echo", &["test"], Some(&env), None);
        #[cfg(windows)]
        let result = session.spawn_with_env("cmd.exe", &["/C", "echo test"], Some(&env), None);

        assert!(result.is_ok());

        // Give it time to execute
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    #[test]
    fn test_spawn_with_empty_env() {
        let mut session = PtySession::new(80, 24, 1000);

        // Pass empty env HashMap
        let env = HashMap::new();

        #[cfg(unix)]
        let result = session.spawn_with_env("/bin/echo", &["hello"], Some(&env), None);
        #[cfg(windows)]
        let result = session.spawn_with_env("cmd.exe", &["/C", "echo hello"], Some(&env), None);

        assert!(result.is_ok());
    }
}
