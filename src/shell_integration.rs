/// Shell integration markers (OSC 133)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellIntegrationMarker {
    /// Start of prompt (A)
    PromptStart,
    /// Start of command input (B)
    CommandStart,
    /// Start of command output (C)
    CommandExecuted,
    /// End of command output, with exit code (D)
    CommandFinished,
}

/// Shell integration state
#[derive(Debug, Clone)]
pub struct ShellIntegration {
    /// Current marker
    current_marker: Option<ShellIntegrationMarker>,
    /// Command that was executed
    current_command: Option<String>,
    /// Exit code of last command
    last_exit_code: Option<i32>,
    /// Current working directory
    cwd: Option<String>,
}

impl Default for ShellIntegration {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellIntegration {
    /// Create a new shell integration state
    pub fn new() -> Self {
        Self {
            current_marker: None,
            current_command: None,
            last_exit_code: None,
            cwd: None,
        }
    }

    /// Set the current marker
    pub fn set_marker(&mut self, marker: ShellIntegrationMarker) {
        self.current_marker = Some(marker);
    }

    /// Get the current marker
    pub fn marker(&self) -> Option<ShellIntegrationMarker> {
        self.current_marker
    }

    /// Set the current command
    pub fn set_command(&mut self, command: String) {
        self.current_command = Some(command);
    }

    /// Get the current command
    pub fn command(&self) -> Option<&str> {
        self.current_command.as_deref()
    }

    /// Set the exit code
    pub fn set_exit_code(&mut self, code: i32) {
        self.last_exit_code = Some(code);
    }

    /// Get the last exit code
    pub fn exit_code(&self) -> Option<i32> {
        self.last_exit_code
    }

    /// Set current working directory
    pub fn set_cwd(&mut self, cwd: String) {
        self.cwd = Some(cwd);
    }

    /// Get current working directory
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// Check if we're in a prompt
    pub fn in_prompt(&self) -> bool {
        matches!(
            self.current_marker,
            Some(ShellIntegrationMarker::PromptStart)
        )
    }

    /// Check if we're in command input
    pub fn in_command_input(&self) -> bool {
        matches!(
            self.current_marker,
            Some(ShellIntegrationMarker::CommandStart)
        )
    }

    /// Check if we're in command output
    pub fn in_command_output(&self) -> bool {
        matches!(
            self.current_marker,
            Some(ShellIntegrationMarker::CommandExecuted)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_integration_markers() {
        let mut si = ShellIntegration::new();

        si.set_marker(ShellIntegrationMarker::PromptStart);
        assert!(si.in_prompt());

        si.set_marker(ShellIntegrationMarker::CommandStart);
        assert!(si.in_command_input());

        si.set_marker(ShellIntegrationMarker::CommandExecuted);
        assert!(si.in_command_output());
    }

    #[test]
    fn test_shell_integration_command() {
        let mut si = ShellIntegration::new();

        si.set_command("ls -la".to_string());
        assert_eq!(si.command(), Some("ls -la"));

        si.set_exit_code(0);
        assert_eq!(si.exit_code(), Some(0));
    }

    #[test]
    fn test_shell_integration_cwd() {
        let mut si = ShellIntegration::new();

        si.set_cwd("/home/user".to_string());
        assert_eq!(si.cwd(), Some("/home/user"));
    }
}
