//! Coprocess management forwarded to the session's `CoprocessManager`.

use super::*;

impl PtySession {
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
