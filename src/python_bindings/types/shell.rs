//! Shell-integration (OSC 133) state, commands, and progress.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Shell integration state
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ShellIntegration", from_py_object)]
#[derive(Clone)]
pub struct PyShellIntegration {
    /// Whether the cursor is currently in a shell prompt (OSC 133;A)
    pub in_prompt: bool,
    /// Whether the cursor is in the command-input line (OSC 133;B)
    pub in_command_input: bool,
    /// Whether the cursor is in command output (OSC 133;C)
    pub in_command_output: bool,
    /// The command being executed or most recently finished
    pub current_command: Option<String>,
    /// Exit code of the last finished command
    pub last_exit_code: Option<i32>,
    /// Working directory reported by OSC 7 / OSC 1337;CurrentDir
    pub cwd: Option<String>,
    /// Remote hostname when tracking a remote session
    pub hostname: Option<String>,
    /// Username on the host
    pub username: Option<String>,
}

impl From<&crate::shell_integration::ShellIntegration> for PyShellIntegration {
    fn from(si: &crate::shell_integration::ShellIntegration) -> Self {
        use crate::shell_integration::ShellIntegrationMarker;

        let marker = si.marker();

        PyShellIntegration {
            in_prompt: marker == Some(ShellIntegrationMarker::PromptStart),

            in_command_input: marker == Some(ShellIntegrationMarker::CommandStart),

            in_command_output: marker == Some(ShellIntegrationMarker::CommandExecuted)
                || marker == Some(ShellIntegrationMarker::CommandFinished),

            current_command: si.command().map(|s: &str| s.to_string()),

            last_exit_code: si.exit_code(),

            cwd: si.cwd().map(|s: &str| s.to_string()),

            hostname: si.hostname().map(|s: &str| s.to_string()),

            username: si.username().map(|s: &str| s.to_string()),
        }
    }
}

#[pymethods]
impl PyShellIntegration {
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "ShellIntegration(in_prompt={}, in_command_input={}, in_command_output={}, hostname={:?}, username={:?})",
            self.in_prompt, self.in_command_input, self.in_command_output,
            self.hostname, self.username
        ))
    }
}

/// Progress bar state from OSC 9;4 sequences (ConEmu/Windows Terminal style)
///
/// This struct represents the current progress bar state as set via OSC 9;4 sequences.
/// Terminal emulators like ConEmu and Windows Terminal use this to display progress
/// in the tab bar, taskbar, or window title.
///
/// ## States
/// - Hidden: No progress bar displayed
/// - Normal: Standard progress (0-100%)
/// - Indeterminate: Busy/loading indicator
/// - Warning: Progress with warning (yellow)
/// - Error: Progress with error (red)
///
/// ## Examples
/// ```python
/// term = Terminal(80, 24)
/// term.process(b"\\x1b]9;4;1;50\\x1b\\\\")  # Set progress to 50%
/// pb = term.progress_bar()
/// print(f"Progress: {pb.progress}%")  # Output: Progress: 50%
/// print(f"State: {pb.state}")  # Output: State: ProgressState.NORMAL
/// ```
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ProgressBar", from_py_object)]
#[derive(Clone)]
pub struct PyProgressBar {
    /// Current progress state
    pub state: crate::python_bindings::enums::PyProgressState,
    /// Progress percentage (0-100)
    pub progress: u8,
}

#[pymethods]
impl PyProgressBar {
    /// Create a new progress bar with given state and progress
    #[new]
    #[pyo3(signature = (state=crate::python_bindings::enums::PyProgressState::Hidden, progress=0))]
    fn new(state: crate::python_bindings::enums::PyProgressState, progress: u8) -> Self {
        Self {
            state,
            progress: progress.min(100),
        }
    }

    /// Check if the progress bar is currently active (visible)
    fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// Generate the OSC 9;4 escape sequence for this progress bar
    fn to_escape_sequence(&self) -> String {
        if self.state.requires_progress() {
            format!("\x1b]9;4;{};{}\x1b\\", self.state as u8, self.progress)
        } else {
            format!("\x1b]9;4;{}\x1b\\", self.state as u8)
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ProgressBar(state={:?}, progress={})",
            self.state, self.progress
        )
    }
}

impl From<&crate::terminal::ProgressBar> for PyProgressBar {
    fn from(pb: &crate::terminal::ProgressBar) -> Self {
        Self {
            state: pb.state.into(),
            progress: pb.progress,
        }
    }
}

/// Command execution record
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "CommandExecution", from_py_object)]
#[derive(Clone)]
pub struct PyCommandExecution {
    /// The command line that was executed
    pub command: String,
    /// Working directory the command ran in, if known
    pub cwd: Option<String>,
    /// Unix epoch milliseconds when the command started
    pub start_time: u64,
    /// Unix epoch milliseconds when the command finished (None while running)
    pub end_time: Option<u64>,
    /// Exit code (None while running)
    pub exit_code: Option<i32>,
    /// Wall-clock duration in milliseconds (None while running)
    pub duration_ms: Option<u64>,
    /// Whether the command succeeded (exit code 0; None while running)
    pub success: Option<bool>,
    /// First row of the command's output, if any
    pub output_start_row: Option<usize>,
    /// Last row of the command's output, if any
    pub output_end_row: Option<usize>,
}

#[pymethods]
impl PyCommandExecution {
    fn __repr__(&self) -> String {
        format!(
            "CommandExecution(command={:?}, exit_code={:?}, duration={:?}ms)",
            self.command, self.exit_code, self.duration_ms
        )
    }
}

impl From<&crate::terminal::CommandExecution> for PyCommandExecution {
    fn from(cmd: &crate::terminal::CommandExecution) -> Self {
        PyCommandExecution {
            command: cmd.command.clone(),
            cwd: cmd.cwd.clone(),
            start_time: cmd.start_time,
            end_time: cmd.end_time,
            exit_code: cmd.exit_code,
            duration_ms: cmd.duration_ms,
            success: cmd.success,
            output_start_row: cmd.output_start_row,
            output_end_row: cmd.output_end_row,
        }
    }
}

/// Shell integration statistics
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ShellIntegrationStats", from_py_object)]
#[derive(Clone)]
pub struct PyShellIntegrationStats {
    /// Total number of tracked commands
    pub total_commands: usize,
    /// Number of commands that exited 0
    pub successful_commands: usize,
    /// Number of commands that exited non-zero
    pub failed_commands: usize,
    /// Average command duration in milliseconds
    pub avg_duration_ms: f64,
    /// Total command time in milliseconds
    pub total_duration_ms: u64,
}

#[pymethods]
impl PyShellIntegrationStats {
    fn __repr__(&self) -> String {
        format!(
            "ShellIntegrationStats(total={}, success={}, failed={}, avg_ms={:.1})",
            self.total_commands,
            self.successful_commands,
            self.failed_commands,
            self.avg_duration_ms
        )
    }
}

impl From<&crate::terminal::ShellIntegrationStats> for PyShellIntegrationStats {
    fn from(stats: &crate::terminal::ShellIntegrationStats) -> Self {
        PyShellIntegrationStats {
            total_commands: stats.total_commands,
            successful_commands: stats.successful_commands,
            failed_commands: stats.failed_commands,
            avg_duration_ms: stats.avg_duration_ms,
            total_duration_ms: stats.total_duration_ms,
        }
    }
}

/// CWD change notification
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "CwdChange", from_py_object)]
#[derive(Clone)]
pub struct PyCwdChange {
    /// Previous working directory (None for the first report)
    pub old_cwd: Option<String>,
    /// New working directory
    pub new_cwd: String,
    /// Host the directory change occurred on, if reported
    pub hostname: Option<String>,
    /// User who changed directory, if reported
    pub username: Option<String>,
    /// Unix epoch milliseconds when the change was observed
    pub timestamp: u64,
}

#[pymethods]
impl PyCwdChange {
    fn __repr__(&self) -> String {
        format!(
            "CwdChange(old={:?}, new={:?}, host={:?}, user={:?})",
            self.old_cwd, self.new_cwd, self.hostname, self.username
        )
    }
}

impl From<&crate::terminal::CwdChange> for PyCwdChange {
    fn from(change: &crate::terminal::CwdChange) -> Self {
        PyCwdChange {
            old_cwd: change.old_cwd.clone(),
            new_cwd: change.new_cwd.clone(),
            hostname: change.hostname.clone(),
            username: change.username.clone(),
            timestamp: change.timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pyshellintegration_repr() {
        let shell_int = PyShellIntegration {
            in_prompt: true,
            in_command_input: false,
            in_command_output: false,
            current_command: Some("ls -la".to_string()),
            last_exit_code: Some(0),
            cwd: Some("/home/user".to_string()),
            hostname: None,
            username: None,
        };

        let repr = shell_int.__repr__().unwrap();
        assert!(repr.contains("in_prompt=true"));
        assert!(repr.contains("in_command_input=false"));
        assert!(repr.contains("in_command_output=false"));
    }

    #[test]
    fn test_pyshellintegration_all_states() {
        let shell_int = PyShellIntegration {
            in_prompt: false,
            in_command_input: true,
            in_command_output: false,
            current_command: None,
            last_exit_code: None,
            cwd: None,
            hostname: None,
            username: None,
        };

        assert!(!shell_int.in_prompt);
        assert!(shell_int.in_command_input);
        assert!(!shell_int.in_command_output);
        assert_eq!(shell_int.current_command, None);
        assert_eq!(shell_int.last_exit_code, None);
        assert_eq!(shell_int.cwd, None);
        assert_eq!(shell_int.hostname, None);
        assert_eq!(shell_int.username, None);
    }

    #[test]
    fn test_pyshellintegration_clone() {
        let shell_int1 = PyShellIntegration {
            in_prompt: true,
            in_command_input: true,
            in_command_output: true,
            current_command: Some("echo test".to_string()),
            last_exit_code: Some(1),
            cwd: Some("/tmp".to_string()),
            hostname: Some("remote-server".to_string()),
            username: Some("alice".to_string()),
        };

        let shell_int2 = shell_int1.clone();

        assert_eq!(shell_int1.in_prompt, shell_int2.in_prompt);
        assert_eq!(shell_int1.current_command, shell_int2.current_command);
        assert_eq!(shell_int1.last_exit_code, shell_int2.last_exit_code);
        assert_eq!(shell_int1.cwd, shell_int2.cwd);
        assert_eq!(shell_int1.hostname, shell_int2.hostname);
        assert_eq!(shell_int1.username, shell_int2.username);
    }
}
