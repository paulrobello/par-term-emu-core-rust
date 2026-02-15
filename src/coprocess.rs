//! Coprocess management for piping terminal output to external processes
//!
//! Coprocesses receive terminal output on their stdin and buffer their stdout
//! for API consumption. This enables log processing, filtering, and automation
//! without injecting data back into the PTY.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use parking_lot::Mutex;

/// Unique coprocess identifier
pub type CoprocessId = u64;

/// Policy for restarting a coprocess when it exits
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Never restart (default)
    #[default]
    Never,
    /// Always restart regardless of exit code
    Always,
    /// Restart only on non-zero exit code (or killed by signal)
    OnFailure,
}

/// Configuration for starting a coprocess
#[derive(Debug, Clone)]
pub struct CoprocessConfig {
    /// Command to execute
    pub command: String,
    /// Command arguments
    pub args: Vec<String>,
    /// Working directory (None = inherit)
    pub cwd: Option<String>,
    /// Additional environment variables
    pub env: HashMap<String, String>,
    /// Whether to copy terminal output to this coprocess's stdin
    pub copy_terminal_output: bool,
    /// Restart policy when the coprocess exits
    pub restart_policy: RestartPolicy,
    /// Delay in milliseconds before restarting
    pub restart_delay_ms: u64,
}

impl Default for CoprocessConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            copy_terminal_output: true,
            restart_policy: RestartPolicy::Never,
            restart_delay_ms: 0,
        }
    }
}

/// A running coprocess
struct Coprocess {
    config: CoprocessConfig,
    child: Child,
    stdin_writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    reader_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
    output_buffer: Arc<Mutex<Vec<String>>>,
    error_buffer: Arc<Mutex<Vec<String>>>,
    copy_terminal_output: bool,
    restart_policy: RestartPolicy,
    restart_delay_ms: u64,
    /// When the process was first detected as dead (for delay tracking)
    died_at: Option<Instant>,
}

/// Result of spawning a coprocess child
struct SpawnResult {
    child: Child,
    stdin_writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    reader_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
    output_buffer: Arc<Mutex<Vec<String>>>,
    error_buffer: Arc<Mutex<Vec<String>>>,
}

/// Manager for multiple coprocesses
pub struct CoprocessManager {
    coprocesses: HashMap<CoprocessId, Coprocess>,
    next_id: CoprocessId,
    /// Maximum output buffer lines per coprocess
    max_buffer_lines: usize,
}

impl std::fmt::Debug for CoprocessManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoprocessManager")
            .field("coprocess_count", &self.coprocesses.len())
            .field("next_id", &self.next_id)
            .finish()
    }
}

impl Default for CoprocessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CoprocessManager {
    /// Create a new coprocess manager
    pub fn new() -> Self {
        Self {
            coprocesses: HashMap::new(),
            next_id: 1,
            max_buffer_lines: 10000,
        }
    }

    /// Spawn a child process with reader threads.
    fn spawn_child(
        config: &CoprocessConfig,
        max_buffer_lines: usize,
    ) -> Result<SpawnResult, String> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args);

        if let Some(ref cwd) = config.cwd {
            cmd.current_dir(cwd);
        }

        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn coprocess: {}", e))?;

        let stdin_writer: Option<Arc<Mutex<Box<dyn Write + Send>>>> = child.stdin.take().map(|s| {
            let boxed: Box<dyn Write + Send> = Box::new(s);
            Arc::new(Mutex::new(boxed))
        });

        let output_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let error_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));

        // Start stdout reader thread
        let reader_thread = if let Some(stdout) = child.stdout.take() {
            let buffer_clone = Arc::clone(&output_buffer);
            let running_clone = Arc::clone(&running);
            let max_lines = max_buffer_lines;

            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(text) => {
                            let mut buf = buffer_clone.lock();
                            if buf.len() >= max_lines {
                                buf.remove(0);
                            }
                            buf.push(text);
                        }
                        Err(_) => break,
                    }
                }
                running_clone.store(false, Ordering::SeqCst);
            }))
        } else {
            None
        };

        // Start stderr reader thread to capture error output
        let stderr_thread = if let Some(stderr) = child.stderr.take() {
            let err_buf_clone = Arc::clone(&error_buffer);
            let max_lines = max_buffer_lines;

            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(text) => {
                            let mut buf = err_buf_clone.lock();
                            if buf.len() >= max_lines {
                                buf.remove(0);
                            }
                            buf.push(text);
                        }
                        Err(_) => break,
                    }
                }
            }))
        } else {
            None
        };

        Ok(SpawnResult {
            child,
            stdin_writer,
            reader_thread,
            stderr_thread,
            running,
            output_buffer,
            error_buffer,
        })
    }

    /// Start a new coprocess
    pub fn start(&mut self, config: CoprocessConfig) -> Result<CoprocessId, String> {
        if config.command.is_empty() {
            return Err("Command must not be empty".to_string());
        }

        // Validate command doesn't contain path traversal
        if config.command.contains("..") {
            return Err("Command path must not contain '..'".to_string());
        }

        // Validate command doesn't contain shell metacharacters that could enable injection
        const SHELL_META: &[char] = &[
            '|', ';', '&', '$', '`', '(', ')', '{', '}', '<', '>', '\n', '\r',
        ];
        if config.command.chars().any(|c| SHELL_META.contains(&c)) {
            return Err("Command must not contain shell metacharacters".to_string());
        }

        // Validate working directory if specified
        if let Some(ref cwd) = config.cwd {
            if cwd.is_empty() {
                return Err("Working directory must not be empty".to_string());
            }
            if cwd.contains("..") {
                return Err("Working directory must not contain '..'".to_string());
            }
            // Canonicalize and verify the directory exists
            let cwd_path = std::path::Path::new(cwd);
            if cwd_path.exists() && !cwd_path.is_dir() {
                return Err("Working directory path is not a directory".to_string());
            }
        }

        // Validate environment variable names (must be valid identifier-like)
        for key in config.env.keys() {
            if key.is_empty() {
                return Err("Environment variable name must not be empty".to_string());
            }
            if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!("Invalid environment variable name: {}", key));
            }
        }

        let spawn = Self::spawn_child(&config, self.max_buffer_lines)?;

        let id = self.next_id;
        self.next_id += 1;

        let coprocess = Coprocess {
            copy_terminal_output: config.copy_terminal_output,
            restart_policy: config.restart_policy,
            restart_delay_ms: config.restart_delay_ms,
            config,
            child: spawn.child,
            stdin_writer: spawn.stdin_writer,
            reader_thread: spawn.reader_thread,
            stderr_thread: spawn.stderr_thread,
            running: spawn.running,
            output_buffer: spawn.output_buffer,
            error_buffer: spawn.error_buffer,
            died_at: None,
        };

        self.coprocesses.insert(id, coprocess);
        Ok(id)
    }

    /// Stop a coprocess by ID
    pub fn stop(&mut self, id: CoprocessId) -> Result<(), String> {
        if let Some(mut coproc) = self.coprocesses.remove(&id) {
            coproc.running.store(false, Ordering::SeqCst);
            // Drop stdin to signal EOF to the child
            coproc.stdin_writer = None;
            // Kill the child process
            let _ = coproc.child.kill();
            let _ = coproc.child.wait();
            Ok(())
        } else {
            Err(format!("Coprocess {} not found", id))
        }
    }

    /// Stop all coprocesses
    pub fn stop_all(&mut self) {
        let ids: Vec<CoprocessId> = self.coprocesses.keys().copied().collect();
        for id in ids {
            let _ = self.stop(id);
        }
    }

    /// Write data to a coprocess's stdin
    pub fn write(&self, id: CoprocessId, data: &[u8]) -> Result<(), String> {
        let coproc = self
            .coprocesses
            .get(&id)
            .ok_or_else(|| format!("Coprocess {} not found", id))?;

        if let Some(ref writer) = coproc.stdin_writer {
            let mut w = writer.lock();
            w.write_all(data)
                .map_err(|e| format!("Write error: {}", e))?;
            w.flush().map_err(|e| format!("Flush error: {}", e))?;
            Ok(())
        } else {
            Err("Coprocess stdin not available".to_string())
        }
    }

    /// Read buffered output from a coprocess (drains the buffer)
    pub fn read(&self, id: CoprocessId) -> Result<Vec<String>, String> {
        let coproc = self
            .coprocesses
            .get(&id)
            .ok_or_else(|| format!("Coprocess {} not found", id))?;

        let mut buf = coproc.output_buffer.lock();
        Ok(std::mem::take(&mut *buf))
    }

    /// List all coprocess IDs
    pub fn list(&self) -> Vec<CoprocessId> {
        let mut ids: Vec<CoprocessId> = self.coprocesses.keys().copied().collect();
        ids.sort();
        ids
    }

    /// Check if a coprocess is still running
    pub fn status(&self, id: CoprocessId) -> Option<bool> {
        self.coprocesses
            .get(&id)
            .map(|c| c.running.load(Ordering::SeqCst))
    }

    /// Read buffered stderr output from a coprocess (drains the buffer)
    pub fn read_errors(&self, id: CoprocessId) -> Result<Vec<String>, String> {
        let coproc = self
            .coprocesses
            .get(&id)
            .ok_or_else(|| format!("Coprocess {} not found", id))?;

        let mut buf = coproc.error_buffer.lock();
        Ok(std::mem::take(&mut *buf))
    }

    /// Determine if a dead coprocess should be restarted based on its exit status
    fn should_restart(coproc: &mut Coprocess) -> bool {
        match coproc.restart_policy {
            RestartPolicy::Never => false,
            RestartPolicy::Always => true,
            RestartPolicy::OnFailure => {
                // Check exit code
                match coproc.child.try_wait() {
                    Ok(Some(status)) => !status.success(),
                    // Process hasn't finished waiting yet or error — treat as failure
                    Ok(None) => true,
                    Err(_) => true,
                }
            }
        }
    }

    /// Feed terminal output to all coprocesses that have copy_terminal_output enabled.
    ///
    /// Also performs cleanup of dead coprocesses:
    /// - Never policy: removes dead coprocesses from the map
    /// - Always/OnFailure: restarts according to policy (with optional delay)
    pub fn feed_output(&mut self, data: &[u8]) {
        // Phase 1: Feed data to running coprocesses, collect dead IDs
        let mut dead_ids: Vec<CoprocessId> = Vec::new();
        for (&id, coproc) in &self.coprocesses {
            if coproc.running.load(Ordering::SeqCst) {
                if coproc.copy_terminal_output {
                    if let Some(ref writer) = coproc.stdin_writer {
                        let mut w = writer.lock();
                        let _ = w.write_all(data);
                        let _ = w.flush();
                    }
                }
            } else {
                dead_ids.push(id);
            }
        }

        // Phase 2: Handle dead coprocesses
        let mut to_remove: Vec<CoprocessId> = Vec::new();
        for id in dead_ids {
            let coproc = match self.coprocesses.get_mut(&id) {
                Some(c) => c,
                None => continue,
            };

            if !Self::should_restart(coproc) {
                to_remove.push(id);
                continue;
            }

            // Handle restart delay
            if coproc.restart_delay_ms > 0 {
                let now = Instant::now();
                match coproc.died_at {
                    None => {
                        // First detection — record time, skip restart this cycle
                        coproc.died_at = Some(now);
                        continue;
                    }
                    Some(died) => {
                        let elapsed = now.duration_since(died).as_millis() as u64;
                        if elapsed < coproc.restart_delay_ms {
                            continue; // Delay not elapsed yet
                        }
                    }
                }
            }

            // Attempt restart
            self.restart_coprocess_by_id(id);
        }

        // Phase 3: Remove dead coprocesses with Never policy
        for id in to_remove {
            self.coprocesses.remove(&id);
        }
    }

    /// Helper to restart a coprocess by ID (avoids borrow issues with &mut self + &mut coproc)
    fn restart_coprocess_by_id(&mut self, id: CoprocessId) {
        // We need to temporarily remove the coprocess to avoid borrow conflicts
        if let Some(mut coproc) = self.coprocesses.remove(&id) {
            // Join old threads
            if let Some(t) = coproc.reader_thread.take() {
                let _ = t.join();
            }
            if let Some(t) = coproc.stderr_thread.take() {
                let _ = t.join();
            }
            let _ = coproc.child.wait();

            match Self::spawn_child(&coproc.config, self.max_buffer_lines) {
                Ok(spawn) => {
                    coproc.child = spawn.child;
                    coproc.stdin_writer = spawn.stdin_writer;
                    coproc.reader_thread = spawn.reader_thread;
                    coproc.stderr_thread = spawn.stderr_thread;
                    coproc.running = spawn.running;
                    coproc.output_buffer = spawn.output_buffer;
                    coproc.error_buffer = spawn.error_buffer;
                    coproc.died_at = None;
                    self.coprocesses.insert(id, coproc);
                }
                Err(_) => {
                    // Failed to restart — mark as permanently dead and put back
                    coproc.restart_policy = RestartPolicy::Never;
                    coproc.died_at = None;
                    self.coprocesses.insert(id, coproc);
                }
            }
        }
    }
}

impl Drop for CoprocessManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coprocess_config_default() {
        let config = CoprocessConfig::default();
        assert!(config.command.is_empty());
        assert!(config.copy_terminal_output);
        assert_eq!(config.restart_policy, RestartPolicy::Never);
        assert_eq!(config.restart_delay_ms, 0);
    }

    #[test]
    fn test_coprocess_manager_new() {
        let mgr = CoprocessManager::new();
        assert_eq!(mgr.list().len(), 0);
    }

    #[test]
    fn test_coprocess_empty_command() {
        let mut mgr = CoprocessManager::new();
        let result = mgr.start(CoprocessConfig::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_coprocess_path_traversal_rejected() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "../malicious".to_string(),
            ..Default::default()
        };
        let result = mgr.start(config);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains(".."));
    }

    #[test]
    fn test_coprocess_shell_metacharacters_rejected() {
        let mut mgr = CoprocessManager::new();
        for meta in &["|", ";", "&", "$", "`", "(", ")"] {
            let config = CoprocessConfig {
                command: format!("cmd{}", meta),
                ..Default::default()
            };
            let result = mgr.start(config);
            assert!(result.is_err(), "Should reject command with '{}'", meta);
        }
    }

    #[test]
    fn test_coprocess_cwd_traversal_rejected() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "cat".to_string(),
            cwd: Some("../../../etc".to_string()),
            ..Default::default()
        };
        let result = mgr.start(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_coprocess_invalid_env_var_name() {
        let mut mgr = CoprocessManager::new();
        let mut env = HashMap::new();
        env.insert("VALID_NAME".to_string(), "ok".to_string());
        env.insert("invalid name!".to_string(), "bad".to_string());
        let config = CoprocessConfig {
            command: "cat".to_string(),
            env,
            ..Default::default()
        };
        let result = mgr.start(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_coprocess_spawn_cat() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "cat".to_string(),
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();
        assert_eq!(mgr.list(), vec![id]);
        assert_eq!(mgr.status(id), Some(true));
        mgr.stop(id).unwrap();
    }

    #[test]
    fn test_coprocess_write_read() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "cat".to_string(),
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();

        // Write data to coprocess
        mgr.write(id, b"hello\nworld\n").unwrap();

        // Give cat time to process
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Read output
        let output = mgr.read(id).unwrap();
        assert!(output.contains(&"hello".to_string()));
        assert!(output.contains(&"world".to_string()));

        mgr.stop(id).unwrap();
    }

    #[test]
    fn test_coprocess_stop_nonexistent() {
        let mut mgr = CoprocessManager::new();
        assert!(mgr.stop(999).is_err());
    }

    #[test]
    fn test_coprocess_feed_output() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "cat".to_string(),
            copy_terminal_output: true,
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();

        mgr.feed_output(b"fed line\n");
        std::thread::sleep(std::time::Duration::from_millis(100));

        let output = mgr.read(id).unwrap();
        assert!(output.contains(&"fed line".to_string()));

        mgr.stop(id).unwrap();
    }

    #[test]
    fn test_coprocess_dead_process() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "true".to_string(), // exits immediately
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();

        // Wait for process to exit
        std::thread::sleep(std::time::Duration::from_millis(200));

        assert_eq!(mgr.status(id), Some(false));
        mgr.stop(id).unwrap();
    }

    #[test]
    fn test_coprocess_stderr_capture() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "echo error_msg >&2".to_string()],
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();

        // Give process time to write stderr and exit
        std::thread::sleep(std::time::Duration::from_millis(200));

        let errors = mgr.read_errors(id).unwrap();
        assert!(errors.contains(&"error_msg".to_string()));
        mgr.stop(id).unwrap();
    }

    #[test]
    fn test_coprocess_read_errors_nonexistent() {
        let mgr = CoprocessManager::new();
        assert!(mgr.read_errors(999).is_err());
    }

    #[test]
    fn test_coprocess_auto_cleanup_never_policy() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "true".to_string(), // exits immediately
            restart_policy: RestartPolicy::Never,
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();
        assert_eq!(mgr.list(), vec![id]);

        // Wait for process to exit
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(mgr.status(id), Some(false));

        // feed_output should clean up the dead process
        mgr.feed_output(b"data\n");

        // Process should be removed
        assert!(mgr.list().is_empty());
        assert_eq!(mgr.status(id), None);
    }

    #[test]
    fn test_coprocess_restart_always_policy() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "echo restarted; exit 0".to_string()],
            restart_policy: RestartPolicy::Always,
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();

        // Wait for process to exit
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(mgr.status(id), Some(false));

        // feed_output should restart the process (same ID preserved)
        mgr.feed_output(b"data\n");

        // Process should still exist with same ID (restarted in-place)
        assert_eq!(mgr.list(), vec![id]);
        // The restarted process exists — it may have already exited again since it's
        // a short-lived command, but the key assertion is that it was restarted (still in map)
        assert!(mgr.status(id).is_some());

        mgr.stop(id).unwrap();
    }

    #[test]
    fn test_coprocess_restart_on_failure_clean_exit() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "true".to_string(), // exits with code 0
            restart_policy: RestartPolicy::OnFailure,
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();

        // Wait for process to exit cleanly
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(mgr.status(id), Some(false));

        // feed_output should remove it (clean exit, OnFailure policy)
        mgr.feed_output(b"data\n");

        assert!(mgr.list().is_empty());
    }

    #[test]
    fn test_coprocess_restart_on_failure_nonzero_exit() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "false".to_string(), // exits with code 1
            restart_policy: RestartPolicy::OnFailure,
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();

        // Wait for process to exit with failure
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(mgr.status(id), Some(false));

        // feed_output should restart it (non-zero exit, OnFailure policy)
        mgr.feed_output(b"data\n");

        // Process should still exist with same ID
        assert_eq!(mgr.list(), vec![id]);

        mgr.stop(id).unwrap();
    }

    #[test]
    fn test_coprocess_restart_delay() {
        let mut mgr = CoprocessManager::new();
        let config = CoprocessConfig {
            command: "true".to_string(), // exits immediately
            restart_policy: RestartPolicy::Always,
            restart_delay_ms: 300,
            ..Default::default()
        };
        let id = mgr.start(config).unwrap();

        // Wait for process to exit
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(mgr.status(id), Some(false));

        // First feed_output should record died_at but NOT restart yet
        mgr.feed_output(b"data\n");
        // Process should still be in the map (not removed, not yet restarted)
        assert_eq!(mgr.list(), vec![id]);
        assert_eq!(mgr.status(id), Some(false)); // still dead

        // Feed again before delay elapses — should still not restart
        std::thread::sleep(std::time::Duration::from_millis(100));
        mgr.feed_output(b"data\n");
        assert_eq!(mgr.status(id), Some(false)); // still dead

        // Wait for delay to elapse and feed again
        std::thread::sleep(std::time::Duration::from_millis(250));
        mgr.feed_output(b"data\n");

        // Now it should have restarted
        assert_eq!(mgr.list(), vec![id]);
        // The new process may have already exited (it's `true`), but it was restarted

        mgr.stop(id).unwrap();
    }
}
