//! Shell integration data types
//!
//! Provides types for tracking command execution and shell integration statistics.

/// Information about a command execution
#[derive(Debug, Clone)]
pub struct CommandExecution {
    /// Command that was executed
    pub command: String,
    /// Current working directory when command was run
    pub cwd: Option<String>,
    /// Start timestamp (milliseconds since epoch)
    pub start_time: u64,
    /// End timestamp (milliseconds since epoch)
    pub end_time: Option<u64>,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Command duration in milliseconds
    pub duration_ms: Option<u64>,
    /// Whether command succeeded (exit code 0)
    pub success: Option<bool>,
    /// Absolute start row of the output zone
    pub output_start_row: Option<usize>,
    /// Absolute end row of the output zone
    pub output_end_row: Option<usize>,
}

/// Command output record combining execution metadata with extracted output text
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Command that was executed
    pub command: String,
    /// Current working directory when command was run
    pub cwd: Option<String>,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Extracted output text
    pub output: String,
}

/// Shell integration statistics
#[derive(Debug, Clone)]
pub struct ShellIntegrationStats {
    /// Total commands executed
    pub total_commands: usize,
    /// Successful commands (exit code 0)
    pub successful_commands: usize,
    /// Failed commands (non-zero exit code)
    pub failed_commands: usize,
    /// Average command duration (milliseconds)
    pub avg_duration_ms: f64,
    /// Total execution time (milliseconds)
    pub total_duration_ms: u64,
}

use crate::terminal::Terminal;

impl Terminal {
    // === Feature 31: Shell Integration++ ===

    /// Start tracking a command execution
    pub fn start_command_execution(&mut self, command: String) {
        let execution = CommandExecution {
            command,
            cwd: self.shell_integration.cwd().map(|s| s.to_string()),
            start_time: crate::terminal::unix_millis(),
            end_time: None,
            exit_code: None,
            duration_ms: None,
            success: None,
            output_start_row: None, // Will be set when Output zone opens
            output_end_row: None,
        };
        self.current_command = Some(execution);
    }

    /// End tracking the current command execution
    pub fn end_command_execution(&mut self, exit_code: Option<i32>) {
        if let Some(mut execution) = self.current_command.take() {
            let now = crate::terminal::unix_millis();
            execution.end_time = Some(now);
            execution.duration_ms = Some(now - execution.start_time);
            execution.exit_code = exit_code;
            execution.success = exit_code.map(|c| c == 0);

            // Only set end row if we had a start row (i.e. an Output zone was opened)
            if execution.output_start_row.is_some() {
                execution.output_end_row = Some(self.grid.total_lines_scrolled() + self.cursor.row);
            }

            self.record_command(execution);
        }
    }

    /// Record a command execution in history
    pub fn record_command(&mut self, execution: CommandExecution) {
        self.command_history.push(execution);
        if self.command_history.len() > self.max_command_history {
            self.command_history.remove(0);
        }
    }

    /// Get command history
    pub fn get_command_history(&self) -> &[CommandExecution] {
        &self.command_history
    }

    /// Get the currently executing command
    pub fn get_current_command(&self) -> Option<&CommandExecution> {
        self.current_command.as_ref()
    }

    /// Get shell integration statistics
    pub fn get_shell_stats(&self) -> ShellIntegrationStats {
        let total = self.command_history.len();
        let successful = self
            .command_history
            .iter()
            .filter(|c| c.success == Some(true))
            .count();
        let failed = self
            .command_history
            .iter()
            .filter(|c| c.success == Some(false))
            .count();
        let total_ms: u64 = self
            .command_history
            .iter()
            .filter_map(|c| c.duration_ms)
            .sum();

        ShellIntegrationStats {
            total_commands: total,
            successful_commands: successful,
            failed_commands: failed,
            avg_duration_ms: if total > 0 {
                total_ms as f64 / total as f64
            } else {
                0.0
            },
            total_duration_ms: total_ms,
        }
    }

    /// Alias for get_shell_stats
    pub fn get_shell_integration_stats(&self) -> ShellIntegrationStats {
        self.get_shell_stats()
    }

    /// Record a CWD change
    pub fn record_cwd_change(&mut self, change: crate::terminal::CwdChange) {
        let old_hostname = self.last_hostname.clone();
        let old_username = self.last_username.clone();
        let old_cwd = self.shell_integration.cwd().map(|s| s.to_string());

        // Update current state
        self.last_hostname = change.hostname.clone();
        self.last_username = change.username.clone();
        self.shell_integration.set_cwd(change.new_cwd.clone());
        self.shell_integration.set_hostname(change.hostname.clone());
        self.shell_integration.set_username(change.username.clone());

        // Update session variables for badges
        self.session_variables.set_path(change.new_cwd.clone());
        if let Some(ref h) = change.hostname {
            self.session_variables.set_hostname(h.clone());
        } else {
            self.session_variables.hostname = None;
        }
        if let Some(ref u) = change.username {
            self.session_variables.set_username(u.clone());
        } else {
            self.session_variables.username = None;
        }

        // Emit CwdChanged event
        self.terminal_events
            .push(crate::terminal::TerminalEvent::CwdChanged(change.clone()));

        // Emit EnvironmentChanged event for CWD
        self.terminal_events
            .push(crate::terminal::TerminalEvent::EnvironmentChanged {
                key: "cwd".to_string(),
                value: change.new_cwd.clone(),
                old_value: old_cwd,
            });

        // Emit EnvironmentChanged for hostname if changed
        if change.hostname != old_hostname {
            self.terminal_events
                .push(crate::terminal::TerminalEvent::EnvironmentChanged {
                    key: "hostname".to_string(),
                    value: change.hostname.clone().unwrap_or_default(),
                    old_value: old_hostname.clone(),
                });

            // Emit RemoteHostTransition event
            self.terminal_events
                .push(crate::terminal::TerminalEvent::RemoteHostTransition {
                    hostname: change
                        .hostname
                        .clone()
                        .unwrap_or_else(|| "localhost".to_string()),
                    username: change.username.clone(),
                    old_hostname,
                    old_username: old_username.clone(),
                });
        }

        // Emit EnvironmentChanged for username if changed
        if change.username != old_username {
            self.terminal_events
                .push(crate::terminal::TerminalEvent::EnvironmentChanged {
                    key: "username".to_string(),
                    value: change.username.clone().unwrap_or_default(),
                    old_value: old_username,
                });
        }

        self.cwd_changes.push(change);
        if self.cwd_changes.len() > self.max_cwd_history {
            self.cwd_changes.remove(0);
        }
    }

    /// Get CWD change history
    pub fn get_cwd_history(&self) -> &[crate::terminal::event::CwdChange] {
        &self.cwd_changes
    }

    /// Alias for get_cwd_history
    pub fn get_cwd_changes(&self) -> &[crate::terminal::event::CwdChange] {
        self.get_cwd_history()
    }

    /// Set the maximum number of command history entries to retain
    pub fn set_max_command_history(&mut self, max: usize) {
        self.max_command_history = max;
        if self.command_history.len() > max {
            self.command_history
                .drain(0..self.command_history.len() - max);
        }
    }

    /// Set the maximum number of CWD change entries to retain
    pub fn set_max_cwd_history(&mut self, max: usize) {
        self.max_cwd_history = max;
        if self.cwd_changes.len() > max {
            self.cwd_changes.drain(0..self.cwd_changes.len() - max);
        }
    }

    /// Clear command execution history
    pub fn clear_command_history(&mut self) {
        self.command_history.clear();
    }

    /// Clear CWD change history
    pub fn clear_cwd_history(&mut self) {
        self.cwd_changes.clear();
    }

    /// Get output for a command by history index
    /// index 0 is the most recent command
    pub fn get_command_output(&self, index: usize) -> Option<CommandOutput> {
        let len = self.command_history.len();
        if index >= len {
            return None;
        }
        let execution = &self.command_history[len - 1 - index];

        // Use destructuring to ensure we have both rows
        let (start, end) = match (execution.output_start_row, execution.output_end_row) {
            (Some(s), Some(e)) => (s, e),
            _ => return None, // Missing output markers
        };

        if start > end {
            return None; // Invalid range
        }

        // Check if output has been evicted from scrollback
        let floor = self
            .grid
            .total_lines_scrolled()
            .saturating_sub(self.grid.max_scrollback());
        if start < floor {
            return None; // Output partially or fully evicted
        }

        let output = self
            .extract_text_from_row_range(start, end)
            .unwrap_or_default();

        Some(CommandOutput {
            command: execution.command.clone(),
            cwd: execution.cwd.clone(),
            exit_code: execution.exit_code,
            output,
        })
    }

    /// Get all command outputs in history, most recent first
    pub fn get_command_outputs(&self) -> Vec<CommandOutput> {
        (0..self.command_history.len())
            .filter_map(|i| self.get_command_output(i))
            .collect()
    }
}
