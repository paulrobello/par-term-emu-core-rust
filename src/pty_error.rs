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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::io::{Error as IoError, ErrorKind};

    #[test]
    fn test_process_spawn_error_display() {
        let err = PtyError::ProcessSpawnError("Command not found".to_string());
        assert_eq!(
            err.to_string(),
            "Failed to spawn process: Command not found"
        );
    }

    #[test]
    fn test_process_exited_error_display() {
        let err = PtyError::ProcessExitedError(1);
        assert_eq!(err.to_string(), "Process has already exited with code: 1");
    }

    #[test]
    fn test_io_error_display() {
        let io_err = IoError::new(ErrorKind::BrokenPipe, "pipe broken");
        let err = PtyError::IoError(io_err);
        assert!(err.to_string().contains("I/O error"));
    }

    #[test]
    fn test_resize_error_display() {
        let err = PtyError::ResizeError("Invalid dimensions".to_string());
        assert_eq!(err.to_string(), "Failed to resize PTY: Invalid dimensions");
    }

    #[test]
    fn test_not_started_error_display() {
        let err = PtyError::NotStartedError;
        assert_eq!(err.to_string(), "PTY session has not been started");
    }

    #[test]
    fn test_lock_error_display() {
        let err = PtyError::LockError("Mutex poisoned".to_string());
        assert_eq!(err.to_string(), "Mutex lock error: Mutex poisoned");
    }

    #[test]
    fn test_io_error_from_conversion() {
        let io_err = IoError::new(ErrorKind::PermissionDenied, "access denied");
        let pty_err: PtyError = io_err.into();

        match pty_err {
            PtyError::IoError(ref e) => {
                assert_eq!(e.kind(), ErrorKind::PermissionDenied);
            }
            _ => panic!("Expected IoError variant"),
        }
    }

    #[test]
    fn test_error_source() {
        let io_err = IoError::new(ErrorKind::NotFound, "file not found");
        let err = PtyError::IoError(io_err);

        // IoError should have a source
        assert!(err.source().is_some());

        // Other error types should not have a source
        let err = PtyError::NotStartedError;
        assert!(err.source().is_none());

        let err = PtyError::ProcessSpawnError("test".to_string());
        assert!(err.source().is_none());
    }

    #[test]
    fn test_error_debug_format() {
        let err = PtyError::NotStartedError;
        let debug_str = format!("{:?}", err);
        assert_eq!(debug_str, "NotStartedError");

        let err = PtyError::ProcessExitedError(127);
        let debug_str = format!("{:?}", err);
        assert_eq!(debug_str, "ProcessExitedError(127)");
    }

    #[test]
    fn test_all_error_variants() {
        // Test that all error variants can be created and displayed
        let errors = vec![
            PtyError::ProcessSpawnError("spawn".to_string()),
            PtyError::ProcessExitedError(0),
            PtyError::IoError(IoError::other("io")),
            PtyError::ResizeError("resize".to_string()),
            PtyError::NotStartedError,
            PtyError::LockError("lock".to_string()),
        ];

        for err in errors {
            // All errors should be displayable
            let _ = err.to_string();
            let _ = format!("{:?}", err);
        }
    }
}
