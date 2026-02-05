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

use parking_lot::Mutex;

/// Unique coprocess identifier
pub type CoprocessId = u64;

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
}

impl Default for CoprocessConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            copy_terminal_output: true,
        }
    }
}

/// A running coprocess
struct Coprocess {
    _config: CoprocessConfig,
    child: Child,
    stdin_writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    _reader_thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
    output_buffer: Arc<Mutex<Vec<String>>>,
    copy_terminal_output: bool,
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

    /// Start a new coprocess
    pub fn start(&mut self, config: CoprocessConfig) -> Result<CoprocessId, String> {
        if config.command.is_empty() {
            return Err("Command must not be empty".to_string());
        }

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
        cmd.stderr(Stdio::null()); // Discard stderr to avoid deadlocks

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn coprocess: {}", e))?;

        let stdin_writer: Option<Arc<Mutex<Box<dyn Write + Send>>>> = child.stdin.take().map(|s| {
            let boxed: Box<dyn Write + Send> = Box::new(s);
            Arc::new(Mutex::new(boxed))
        });

        let output_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));

        // Start stdout reader thread
        let reader_thread = if let Some(stdout) = child.stdout.take() {
            let buffer_clone = Arc::clone(&output_buffer);
            let running_clone = Arc::clone(&running);
            let max_lines = self.max_buffer_lines;

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

        let id = self.next_id;
        self.next_id += 1;

        let copy_terminal_output = config.copy_terminal_output;
        let coprocess = Coprocess {
            _config: config,
            child,
            stdin_writer,
            _reader_thread: reader_thread,
            running,
            output_buffer,
            copy_terminal_output,
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

    /// Feed terminal output to all coprocesses that have copy_terminal_output enabled
    pub fn feed_output(&self, data: &[u8]) {
        for coproc in self.coprocesses.values() {
            if coproc.copy_terminal_output {
                if let Some(ref writer) = coproc.stdin_writer {
                    let mut w = writer.lock();
                    let _ = w.write_all(data);
                    let _ = w.flush();
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
}
