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

    #[test]
    fn test_shell_integration_default() {
        let si = ShellIntegration::default();
        assert!(si.marker().is_none());
        assert!(si.command().is_none());
        assert!(si.exit_code().is_none());
        assert!(si.cwd().is_none());
    }

    #[test]
    fn test_shell_integration_marker_transitions() {
        let mut si = ShellIntegration::new();

        si.set_marker(ShellIntegrationMarker::PromptStart);
        assert_eq!(si.marker(), Some(ShellIntegrationMarker::PromptStart));

        si.set_marker(ShellIntegrationMarker::CommandStart);
        assert_eq!(si.marker(), Some(ShellIntegrationMarker::CommandStart));

        si.set_marker(ShellIntegrationMarker::CommandExecuted);
        assert_eq!(si.marker(), Some(ShellIntegrationMarker::CommandExecuted));

        si.set_marker(ShellIntegrationMarker::CommandFinished);
        assert_eq!(si.marker(), Some(ShellIntegrationMarker::CommandFinished));
    }

    #[test]
    fn test_shell_integration_exit_codes() {
        let mut si = ShellIntegration::new();

        si.set_exit_code(0);
        assert_eq!(si.exit_code(), Some(0));

        si.set_exit_code(1);
        assert_eq!(si.exit_code(), Some(1));

        si.set_exit_code(127);
        assert_eq!(si.exit_code(), Some(127));

        si.set_exit_code(-1);
        assert_eq!(si.exit_code(), Some(-1));
    }

    #[test]
    fn test_shell_integration_command_updates() {
        let mut si = ShellIntegration::new();

        si.set_command("echo hello".to_string());
        assert_eq!(si.command(), Some("echo hello"));

        si.set_command("ls -la".to_string());
        assert_eq!(si.command(), Some("ls -la"));
    }

    #[test]
    fn test_shell_integration_cwd_updates() {
        let mut si = ShellIntegration::new();

        si.set_cwd("/home/user".to_string());
        assert_eq!(si.cwd(), Some("/home/user"));

        si.set_cwd("/tmp".to_string());
        assert_eq!(si.cwd(), Some("/tmp"));
    }

    #[test]
    fn test_shell_integration_in_prompt_states() {
        let mut si = ShellIntegration::new();

        assert!(!si.in_prompt());

        si.set_marker(ShellIntegrationMarker::PromptStart);
        assert!(si.in_prompt());

        si.set_marker(ShellIntegrationMarker::CommandStart);
        assert!(!si.in_prompt());
    }

    #[test]
    fn test_shell_integration_in_command_input_states() {
        let mut si = ShellIntegration::new();

        assert!(!si.in_command_input());

        si.set_marker(ShellIntegrationMarker::CommandStart);
        assert!(si.in_command_input());

        si.set_marker(ShellIntegrationMarker::CommandExecuted);
        assert!(!si.in_command_input());
    }

    #[test]
    fn test_shell_integration_in_command_output_states() {
        let mut si = ShellIntegration::new();

        assert!(!si.in_command_output());

        si.set_marker(ShellIntegrationMarker::CommandExecuted);
        assert!(si.in_command_output());

        si.set_marker(ShellIntegrationMarker::CommandFinished);
        assert!(!si.in_command_output());
    }

    #[test]
    fn test_shell_integration_full_workflow() {
        let mut si = ShellIntegration::new();

        // Start prompt
        si.set_marker(ShellIntegrationMarker::PromptStart);
        assert!(si.in_prompt());

        // User starts typing command
        si.set_marker(ShellIntegrationMarker::CommandStart);
        si.set_command("echo hello".to_string());
        assert!(si.in_command_input());
        assert_eq!(si.command(), Some("echo hello"));

        // Command executes
        si.set_marker(ShellIntegrationMarker::CommandExecuted);
        assert!(si.in_command_output());

        // Command finishes
        si.set_marker(ShellIntegrationMarker::CommandFinished);
        si.set_exit_code(0);
        assert!(!si.in_command_output());
        assert_eq!(si.exit_code(), Some(0));
    }

    #[test]
    fn test_shell_integration_empty_command() {
        let mut si = ShellIntegration::new();
        si.set_command("".to_string());
        assert_eq!(si.command(), Some(""));
    }

    #[test]
    fn test_shell_integration_marker_equality() {
        assert_eq!(
            ShellIntegrationMarker::PromptStart,
            ShellIntegrationMarker::PromptStart
        );
        assert_ne!(
            ShellIntegrationMarker::PromptStart,
            ShellIntegrationMarker::CommandStart
        );
    }

    #[test]
    fn test_shell_integration_clone() {
        let mut si = ShellIntegration::new();
        si.set_marker(ShellIntegrationMarker::PromptStart);
        si.set_command("test".to_string());
        si.set_exit_code(0);
        si.set_cwd("/home".to_string());

        let cloned = si.clone();
        assert_eq!(cloned.marker(), si.marker());
        assert_eq!(cloned.command(), si.command());
        assert_eq!(cloned.exit_code(), si.exit_code());
        assert_eq!(cloned.cwd(), si.cwd());
    }
}
