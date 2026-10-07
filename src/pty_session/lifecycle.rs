//! Session lifecycle: environment, spawning, process state, and teardown.

use super::*;

impl PtySession {
    /// The environment a spawn inherits: the real process env, or the test
    /// override when one is set.
    pub(super) fn parent_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
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
    pub(super) fn cleanup_previous_session(&mut self) {
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
    pub(super) fn spawn_internal(
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
    pub(super) fn spawn_login_shell(
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
}
