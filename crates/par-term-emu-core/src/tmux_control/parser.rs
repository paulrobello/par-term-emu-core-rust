//! [`TmuxControlParser`]: the control-mode line buffer and parse state machine.

use super::TmuxNotification;

/// Parser for tmux control protocol messages
pub struct TmuxControlParser {
    /// Buffer for accumulating incomplete lines
    line_buffer: Vec<u8>,
    /// Whether we're currently in control mode
    control_mode: bool,
    /// Whether to auto-detect control mode when we see %begin
    /// This helps handle race conditions where data arrives before
    /// set_control_mode(true) is called
    auto_detect: bool,
}

impl TmuxControlParser {
    /// Create a new tmux control protocol parser
    pub fn new(control_mode: bool) -> Self {
        Self {
            line_buffer: Vec::new(),
            control_mode,
            auto_detect: false,
        }
    }

    /// Enable or disable control mode
    pub fn set_control_mode(&mut self, enabled: bool) {
        self.control_mode = enabled;
        // When explicitly enabling control mode, also enable auto-detect
        // so we catch the %begin even if there's a race condition
        if enabled {
            self.auto_detect = true;
        }
    }

    /// Check if control mode is enabled
    pub fn is_control_mode(&self) -> bool {
        self.control_mode
    }

    /// Enable auto-detection of control mode
    ///
    /// When enabled, the parser will automatically switch to control mode
    /// when it sees a `%begin` notification. This helps handle race conditions
    /// where tmux output arrives before set_control_mode(true) is called.
    pub fn set_auto_detect(&mut self, enabled: bool) {
        self.auto_detect = enabled;
    }

    /// Check if auto-detection is enabled
    pub fn is_auto_detect(&self) -> bool {
        self.auto_detect
    }

    /// Parse incoming data and extract notifications
    ///
    /// Returns a vector of notifications parsed from the data.
    /// Any unparsed data is buffered for the next call.
    ///
    /// When auto-detect is enabled and control mode is off, the parser will
    /// scan for `%begin` in the data. If found, it will:
    /// 1. Return any data before `%begin` as TerminalOutput
    /// 2. Switch to control mode
    /// 3. Parse the remaining data as tmux control protocol
    pub fn parse(&mut self, data: &[u8]) -> Vec<TmuxNotification> {
        if !self.control_mode {
            // Not in control mode yet
            if data.is_empty() {
                return Vec::new();
            }

            // If auto-detect is enabled, scan for %begin to auto-switch to control mode
            if self.auto_detect {
                // Look for %begin in the data (could be at start of line)
                // We need to find "\n%begin " or data starting with "%begin "
                if let Some(begin_pos) = self.find_control_mode_start(data) {
                    let mut notifications = Vec::new();

                    // Return data before %begin as terminal output
                    if begin_pos > 0 {
                        notifications.push(TmuxNotification::TerminalOutput {
                            data: data[..begin_pos].to_vec(),
                        });
                    }

                    // Switch to control mode
                    self.control_mode = true;

                    // Parse the remaining data as control protocol
                    let remaining = &data[begin_pos..];
                    notifications.extend(self.parse_control_data(remaining));

                    return notifications;
                }
            }

            // No auto-detect or no %begin found, treat all data as terminal output
            return vec![TmuxNotification::TerminalOutput {
                data: data.to_vec(),
            }];
        }

        // Control mode is enabled, parse as tmux protocol
        self.parse_control_data(data)
    }

    /// Find the start position of tmux control mode in the data
    ///
    /// Looks for `%begin` at the start of a line. Returns the byte offset
    /// where the control protocol starts, or None if not found.
    pub(super) fn find_control_mode_start(&self, data: &[u8]) -> Option<usize> {
        // Check if data starts with %begin (handles case where %begin is at very start)
        if data.starts_with(b"%begin ") || data.starts_with(b"%begin\n") {
            return Some(0);
        }

        // Look for \n%begin in the data
        for i in 0..data.len().saturating_sub(7) {
            if data[i] == b'\n'
                && data.get(i + 1..i + 7) == Some(b"%begin")
                && (data.get(i + 7) == Some(&b' ') || data.get(i + 7) == Some(&b'\n'))
            {
                // Return position after the newline (start of %begin line)
                return Some(i + 1);
            }
        }

        None
    }

    /// Parse data as tmux control protocol
    fn parse_control_data(&mut self, data: &[u8]) -> Vec<TmuxNotification> {
        let mut notifications = Vec::new();

        // Append new data to the line buffer
        self.line_buffer.extend_from_slice(data);

        // Process complete lines
        while let Some(newline_pos) = self.line_buffer.iter().position(|&b| b == b'\n') {
            // Extract the line (without the newline)
            let line_bytes = self.line_buffer.drain(..=newline_pos).collect::<Vec<u8>>();

            // Strip trailing \n and also \r if present (tmux sends \r\n line endings)
            let mut end = line_bytes.len() - 1; // Skip \n
            if end > 0 && line_bytes[end - 1] == b'\r' {
                end -= 1; // Also skip \r
            }
            let line = String::from_utf8_lossy(&line_bytes[..end]).to_string();

            // Parse the line
            if let Some(notification) = super::line::parse_line(&line) {
                notifications.push(notification);
            }
        }

        notifications
    }

    /// Unescape tmux output data (octal escape sequences)
    #[cfg(test)]
    pub(super) fn unescape_output(s: &str) -> Vec<u8> {
        super::line::unescape_output(s)
    }

    /// Clear the internal line buffer
    pub fn clear_buffer(&mut self) {
        self.line_buffer.clear();
    }

    /// Get the current size of the internal line buffer
    pub fn buffer_len(&self) -> usize {
        self.line_buffer.len()
    }
}
