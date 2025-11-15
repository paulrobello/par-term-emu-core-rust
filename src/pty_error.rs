use std::fmt;
use std::io;

/// Errors that can occur during PTY operations
#[derive(Debug)]
pub enum PtyError {
    /// Failed to spawn a process
    ProcessSpawnError(String),
    /// Process has already exited
    ProcessExitedError(i32),
    /// I/O error occurred
    IoError(io::Error),
    /// Failed to resize the PTY
    ResizeError(String),
    /// PTY session has not been started
    NotStartedError,
    /// Mutex lock failed (poisoned)
    LockError(String),
}

impl fmt::Display for PtyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PtyError::ProcessSpawnError(msg) => write!(f, "Failed to spawn process: {}", msg),
            PtyError::ProcessExitedError(code) => {
                write!(f, "Process has already exited with code: {}", code)
            }
            PtyError::IoError(err) => write!(f, "I/O error: {}", err),
            PtyError::ResizeError(msg) => write!(f, "Failed to resize PTY: {}", msg),
            PtyError::NotStartedError => write!(f, "PTY session has not been started"),
            PtyError::LockError(msg) => write!(f, "Mutex lock error: {}", msg),
        }
    }
}

impl std::error::Error for PtyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PtyError::IoError(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for PtyError {
    fn from(err: io::Error) -> Self {
        PtyError::IoError(err)
    }
}
