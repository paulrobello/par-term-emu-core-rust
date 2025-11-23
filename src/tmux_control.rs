//! Tmux control protocol support
//!
//! Implements parsing and handling of tmux control mode protocol.
//! Control mode is enabled with `tmux -C` (or `-CC` for no echo).
//!
//! # Protocol Overview
//!
//! - Commands sent to tmux are standard tmux commands
//! - Output from tmux consists of:
//!   - Command output wrapped in `%begin`/`%end` or `%begin`/`%error`
//!   - Asynchronous notifications starting with `%`
//!   - Pane output in format `%output %pane-id data`
//!
//! # References
//!
//! - [Tmux Control Mode Wiki](https://github.com/tmux/tmux/wiki/Control-Mode)

/// Tmux control protocol notification types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxNotification {
    /// Command output block started
    /// Arguments: timestamp, command_number, flags
    Begin {
        timestamp: u64,
        command_number: u32,
        flags: String,
    },

    /// Command output block ended successfully
    /// Arguments: timestamp, command_number, flags
    End {
        timestamp: u64,
        command_number: u32,
        flags: String,
    },

    /// Command output block ended with error
    /// Arguments: timestamp, command_number, flags
    Error {
        timestamp: u64,
        command_number: u32,
        flags: String,
    },

    /// Pane output data
    /// Arguments: pane_id, output_data (octal-escaped for control chars)
    Output { pane_id: String, data: Vec<u8> },

    /// Pane mode changed
    /// Arguments: pane_id
    PaneModeChanged { pane_id: String },

    /// Window's active pane changed
    /// Arguments: window_id, pane_id
    WindowPaneChanged { window_id: String, pane_id: String },

    /// Window closed in attached session
    /// Arguments: window_id
    WindowClose { window_id: String },

    /// Window closed in another session (unlinked)
    /// Arguments: window_id
    UnlinkedWindowClose { window_id: String },

    /// Window added to attached session
    /// Arguments: window_id
    WindowAdd { window_id: String },

    /// Window added to another session (unlinked)
    /// Arguments: window_id
    UnlinkedWindowAdd { window_id: String },

    /// Window renamed in attached session
    /// Arguments: window_id, new_name
    WindowRenamed { window_id: String, name: String },

    /// Window renamed in another session (unlinked)
    /// Arguments: window_id, new_name
    UnlinkedWindowRenamed { window_id: String, name: String },

    /// Attached session changed
    /// Arguments: session_id, session_name
    SessionChanged { session_id: String, name: String },

    /// Another client's session changed
    /// Arguments: client_name, session_id, session_name
    ClientSessionChanged {
        client: String,
        session_id: String,
        name: String,
    },

    /// Session renamed
    /// Arguments: session_id, new_name
    SessionRenamed { session_id: String, name: String },

    /// Sessions changed (created or destroyed)
    SessionsChanged,

    /// Session's current window changed
    /// Arguments: session_id, window_id
    SessionWindowChanged {
        session_id: String,
        window_id: String,
    },

    /// Client detached
    /// Arguments: client_name
    ClientDetached { client: String },

    /// Client exited (only with -CC flag)
    Exit,

    /// Pane output paused (flow control)
    /// Arguments: pane_id
    Pause { pane_id: String },

    /// Extended output notification (flow control)
    /// Arguments: pane_id, milliseconds_behind, output_data
    ExtendedOutput {
        pane_id: String,
        delay_ms: u64,
        data: Vec<u8>,
    },

    /// Continue after pause (flow control)
    Continue,

    /// Subscription value changed
    /// Arguments: subscription_name, value
    SubscriptionChanged { name: String, value: String },

    /// Window layout changed
    /// Arguments: window_id, window_layout, window_visible_layout, window_raw_flags
    LayoutChange {
        window_id: String,
        window_layout: String,
        window_visible_layout: String,
        window_raw_flags: String,
    },

    /// Paste buffer changed
    /// Arguments: buffer_name
    PasteBufferChanged { name: String },

    /// Paste buffer deleted
    /// Arguments: buffer_name
    PasteBufferDeleted { name: String },

    /// Unknown or unrecognized notification
    /// Arguments: notification_line
    Unknown { line: String },

    /// Regular terminal output (non-control mode data)
    /// This is used when we receive data that's not a control protocol message
    TerminalOutput { data: Vec<u8> },
}

impl TmuxNotification {
    /// Get a human-readable description of this notification type
    pub fn notification_type(&self) -> &'static str {
        match self {
            Self::Begin { .. } => "begin",
            Self::End { .. } => "end",
            Self::Error { .. } => "error",
            Self::Output { .. } => "output",
            Self::PaneModeChanged { .. } => "pane-mode-changed",
            Self::WindowPaneChanged { .. } => "window-pane-changed",
            Self::WindowClose { .. } => "window-close",
            Self::UnlinkedWindowClose { .. } => "unlinked-window-close",
            Self::WindowAdd { .. } => "window-add",
            Self::UnlinkedWindowAdd { .. } => "unlinked-window-add",
            Self::WindowRenamed { .. } => "window-renamed",
            Self::UnlinkedWindowRenamed { .. } => "unlinked-window-renamed",
            Self::SessionChanged { .. } => "session-changed",
            Self::ClientSessionChanged { .. } => "client-session-changed",
            Self::SessionRenamed { .. } => "session-renamed",
            Self::SessionsChanged => "sessions-changed",
            Self::SessionWindowChanged { .. } => "session-window-changed",
            Self::ClientDetached { .. } => "client-detached",
            Self::Exit => "exit",
            Self::Pause { .. } => "pause",
            Self::ExtendedOutput { .. } => "extended-output",
            Self::Continue => "continue",
            Self::SubscriptionChanged { .. } => "subscription-changed",
            Self::LayoutChange { .. } => "layout-change",
            Self::PasteBufferChanged { .. } => "paste-buffer-changed",
            Self::PasteBufferDeleted { .. } => "paste-buffer-deleted",
            Self::Unknown { .. } => "unknown",
            Self::TerminalOutput { .. } => "terminal-output",
        }
    }
}

/// Parser for tmux control protocol messages
pub struct TmuxControlParser {
    /// Buffer for accumulating incomplete lines
    line_buffer: Vec<u8>,
    /// Whether we're currently in control mode
    control_mode: bool,
}

impl TmuxControlParser {
    /// Create a new tmux control protocol parser
    pub fn new(control_mode: bool) -> Self {
        Self {
            line_buffer: Vec::new(),
            control_mode,
        }
    }

    /// Enable or disable control mode
    pub fn set_control_mode(&mut self, enabled: bool) {
        self.control_mode = enabled;
    }

    /// Check if control mode is enabled
    pub fn is_control_mode(&self) -> bool {
        self.control_mode
    }

    /// Parse incoming data and extract notifications
    ///
    /// Returns a vector of notifications parsed from the data.
    /// Any unparsed data is buffered for the next call.
    pub fn parse(&mut self, data: &[u8]) -> Vec<TmuxNotification> {
        if !self.control_mode {
            // Not in control mode, treat all data as terminal output
            if data.is_empty() {
                return Vec::new();
            }
            return vec![TmuxNotification::TerminalOutput {
                data: data.to_vec(),
            }];
        }

        let mut notifications = Vec::new();

        // Append new data to the line buffer
        self.line_buffer.extend_from_slice(data);

        // Process complete lines
        while let Some(newline_pos) = self.line_buffer.iter().position(|&b| b == b'\n') {
            // Extract the line (without the newline)
            let line_bytes = self.line_buffer.drain(..=newline_pos).collect::<Vec<u8>>();
            let line = String::from_utf8_lossy(&line_bytes[..line_bytes.len() - 1]).to_string();

            // Parse the line
            if let Some(notification) = Self::parse_line(&line) {
                notifications.push(notification);
            }
        }

        notifications
    }

    /// Parse a single line into a notification
    fn parse_line(line: &str) -> Option<TmuxNotification> {
        let line = line.trim();

        // Empty lines are ignored
        if line.is_empty() {
            return None;
        }

        // Check if this is a control protocol message (starts with %)
        if !line.starts_with('%') {
            // Regular output (shouldn't happen in control mode, but handle it)
            return Some(TmuxNotification::TerminalOutput {
                data: line.as_bytes().to_vec(),
            });
        }

        // Split into notification type and arguments
        let parts: Vec<&str> = line[1..].splitn(2, ' ').collect();
        let notification_type = parts[0];
        let args = if parts.len() > 1 { parts[1] } else { "" };

        match notification_type {
            "begin" => Self::parse_begin(args),
            "end" => Self::parse_end(args),
            "error" => Self::parse_error(args),
            "output" => Self::parse_output(args),
            "pane-mode-changed" => Self::parse_pane_mode_changed(args),
            "window-pane-changed" => Self::parse_window_pane_changed(args),
            "window-close" => Self::parse_window_close(args),
            "unlinked-window-close" => Self::parse_unlinked_window_close(args),
            "window-add" => Self::parse_window_add(args),
            "unlinked-window-add" => Self::parse_unlinked_window_add(args),
            "window-renamed" => Self::parse_window_renamed(args),
            "unlinked-window-renamed" => Self::parse_unlinked_window_renamed(args),
            "session-changed" => Self::parse_session_changed(args),
            "client-session-changed" => Self::parse_client_session_changed(args),
            "session-renamed" => Self::parse_session_renamed(args),
            "sessions-changed" => Some(TmuxNotification::SessionsChanged),
            "session-window-changed" => Self::parse_session_window_changed(args),
            "client-detached" => Self::parse_client_detached(args),
            "exit" => Some(TmuxNotification::Exit),
            "pause" => Self::parse_pause(args),
            "extended-output" => Self::parse_extended_output(args),
            "continue" => Some(TmuxNotification::Continue),
            "subscription-changed" => Self::parse_subscription_changed(args),
            "layout-change" => Self::parse_layout_change(args),
            "paste-buffer-changed" => Self::parse_paste_buffer_changed(args),
            "paste-buffer-deleted" => Self::parse_paste_buffer_deleted(args),
            _ => Some(TmuxNotification::Unknown {
                line: line.to_string(),
            }),
        }
    }

    fn parse_begin(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }
        Some(TmuxNotification::Begin {
            timestamp: parts[0].parse().ok()?,
            command_number: parts[1].parse().ok()?,
            flags: if parts.len() > 2 {
                parts[2..].join(" ")
            } else {
                String::new()
            },
        })
    }

    fn parse_end(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }
        Some(TmuxNotification::End {
            timestamp: parts[0].parse().ok()?,
            command_number: parts[1].parse().ok()?,
            flags: if parts.len() > 2 {
                parts[2..].join(" ")
            } else {
                String::new()
            },
        })
    }

    fn parse_error(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }
        Some(TmuxNotification::Error {
            timestamp: parts[0].parse().ok()?,
            command_number: parts[1].parse().ok()?,
            flags: if parts.len() > 2 {
                parts[2..].join(" ")
            } else {
                String::new()
            },
        })
    }

    fn parse_output(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        if parts.is_empty() {
            return None;
        }

        let pane_id = parts[0].to_string();
        let data = if parts.len() > 1 {
            Self::unescape_output(parts[1])
        } else {
            Vec::new()
        };

        Some(TmuxNotification::Output { pane_id, data })
    }

    /// Unescape tmux output data (octal escape sequences)
    fn unescape_output(s: &str) -> Vec<u8> {
        let mut result = Vec::new();
        let bytes = s.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                // Check for octal escape sequence (\nnn)
                if i + 3 < bytes.len()
                    && bytes[i + 1].is_ascii_digit()
                    && bytes[i + 2].is_ascii_digit()
                    && bytes[i + 3].is_ascii_digit()
                {
                    // Parse three octal digits
                    let octal_str = String::from_utf8_lossy(&bytes[i + 1..i + 4]);
                    if let Ok(byte_val) = u8::from_str_radix(&octal_str, 8) {
                        result.push(byte_val);
                        i += 4;
                        continue;
                    }
                }
                // Not a valid octal sequence, treat as literal backslash
                result.push(b'\\');
                i += 1;
            } else {
                result.push(bytes[i]);
                i += 1;
            }
        }

        result
    }

    fn parse_pane_mode_changed(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::PaneModeChanged {
            pane_id: args.trim().to_string(),
        })
    }

    fn parse_window_pane_changed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }
        Some(TmuxNotification::WindowPaneChanged {
            window_id: parts[0].to_string(),
            pane_id: parts[1].to_string(),
        })
    }

    fn parse_window_close(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::WindowClose {
            window_id: args.trim().to_string(),
        })
    }

    fn parse_unlinked_window_close(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::UnlinkedWindowClose {
            window_id: args.trim().to_string(),
        })
    }

    fn parse_window_add(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::WindowAdd {
            window_id: args.trim().to_string(),
        })
    }

    fn parse_unlinked_window_add(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::UnlinkedWindowAdd {
            window_id: args.trim().to_string(),
        })
    }

    fn parse_window_renamed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        if parts.len() < 2 {
            return None;
        }
        Some(TmuxNotification::WindowRenamed {
            window_id: parts[0].to_string(),
            name: parts[1].to_string(),
        })
    }

    fn parse_unlinked_window_renamed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        if parts.len() < 2 {
            return None;
        }
        Some(TmuxNotification::UnlinkedWindowRenamed {
            window_id: parts[0].to_string(),
            name: parts[1].to_string(),
        })
    }

    fn parse_session_changed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        if parts.len() < 2 {
            return None;
        }
        Some(TmuxNotification::SessionChanged {
            session_id: parts[0].to_string(),
            name: parts[1].to_string(),
        })
    }

    fn parse_client_session_changed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.splitn(3, ' ').collect();
        if parts.len() < 3 {
            return None;
        }
        Some(TmuxNotification::ClientSessionChanged {
            client: parts[0].to_string(),
            session_id: parts[1].to_string(),
            name: parts[2].to_string(),
        })
    }

    fn parse_session_renamed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        if parts.len() < 2 {
            return None;
        }
        Some(TmuxNotification::SessionRenamed {
            session_id: parts[0].to_string(),
            name: parts[1].to_string(),
        })
    }

    fn parse_session_window_changed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }
        Some(TmuxNotification::SessionWindowChanged {
            session_id: parts[0].to_string(),
            window_id: parts[1].to_string(),
        })
    }

    fn parse_client_detached(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::ClientDetached {
            client: args.trim().to_string(),
        })
    }

    fn parse_pause(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::Pause {
            pane_id: args.trim().to_string(),
        })
    }

    fn parse_extended_output(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.splitn(3, ' ').collect();
        if parts.len() < 3 {
            return None;
        }

        let pane_id = parts[0].to_string();
        let delay_ms = parts[1].parse().ok()?;
        // Skip the colon separator in parts[2]
        let data_part = if parts[2].starts_with(':') {
            &parts[2][1..]
        } else {
            parts[2]
        };
        let data = Self::unescape_output(data_part);

        Some(TmuxNotification::ExtendedOutput {
            pane_id,
            delay_ms,
            data,
        })
    }

    fn parse_subscription_changed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        if parts.is_empty() {
            return None;
        }
        Some(TmuxNotification::SubscriptionChanged {
            name: parts[0].to_string(),
            value: if parts.len() > 1 {
                parts[1].to_string()
            } else {
                String::new()
            },
        })
    }

    fn parse_layout_change(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.splitn(4, ' ').collect();
        if parts.len() < 4 {
            return None;
        }
        Some(TmuxNotification::LayoutChange {
            window_id: parts[0].to_string(),
            window_layout: parts[1].to_string(),
            window_visible_layout: parts[2].to_string(),
            window_raw_flags: parts[3].to_string(),
        })
    }

    fn parse_paste_buffer_changed(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::PasteBufferChanged {
            name: args.trim().to_string(),
        })
    }

    fn parse_paste_buffer_deleted(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::PasteBufferDeleted {
            name: args.trim().to_string(),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_new() {
        let parser = TmuxControlParser::new(true);
        assert!(parser.is_control_mode());
        assert_eq!(parser.buffer_len(), 0);

        let parser = TmuxControlParser::new(false);
        assert!(!parser.is_control_mode());
    }

    #[test]
    fn test_set_control_mode() {
        let mut parser = TmuxControlParser::new(false);
        assert!(!parser.is_control_mode());

        parser.set_control_mode(true);
        assert!(parser.is_control_mode());

        parser.set_control_mode(false);
        assert!(!parser.is_control_mode());
    }

    #[test]
    fn test_parse_begin() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%begin 1234567890 42 some-flag\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::Begin {
                timestamp,
                command_number,
                flags,
            } => {
                assert_eq!(*timestamp, 1234567890);
                assert_eq!(*command_number, 42);
                assert_eq!(flags, "some-flag");
            }
            _ => panic!("Expected Begin notification"),
        }
    }

    #[test]
    fn test_parse_end() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%end 1234567890 42\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::End {
                timestamp,
                command_number,
                ..
            } => {
                assert_eq!(*timestamp, 1234567890);
                assert_eq!(*command_number, 42);
            }
            _ => panic!("Expected End notification"),
        }
    }

    #[test]
    fn test_parse_output() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%output %1 Hello World\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::Output { pane_id, data } => {
                assert_eq!(pane_id, "%1");
                assert_eq!(data, b"Hello World");
            }
            _ => panic!("Expected Output notification"),
        }
    }

    #[test]
    fn test_unescape_output() {
        // Test newline escape
        assert_eq!(
            TmuxControlParser::unescape_output("Hello\\012World"),
            b"Hello\nWorld"
        );

        // Test backslash escape
        assert_eq!(
            TmuxControlParser::unescape_output("Path\\134file"),
            b"Path\\file"
        );

        // Test tab escape
        assert_eq!(TmuxControlParser::unescape_output("A\\011B"), b"A\tB");

        // Test multiple escapes
        assert_eq!(
            TmuxControlParser::unescape_output("\\033[32mGreen\\033[0m"),
            b"\x1b[32mGreen\x1b[0m"
        );

        // Test no escapes
        assert_eq!(
            TmuxControlParser::unescape_output("Plain text"),
            b"Plain text"
        );
    }

    #[test]
    fn test_parse_window_renamed() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%window-renamed @1 new-window-name\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::WindowRenamed { window_id, name } => {
                assert_eq!(window_id, "@1");
                assert_eq!(name, "new-window-name");
            }
            _ => panic!("Expected WindowRenamed notification"),
        }
    }

    #[test]
    fn test_parse_session_changed() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%session-changed $1 my-session\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::SessionChanged { session_id, name } => {
                assert_eq!(session_id, "$1");
                assert_eq!(name, "my-session");
            }
            _ => panic!("Expected SessionChanged notification"),
        }
    }

    #[test]
    fn test_parse_sessions_changed() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%sessions-changed\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::SessionsChanged => {}
            _ => panic!("Expected SessionsChanged notification"),
        }
    }

    #[test]
    fn test_parse_exit() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%exit\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::Exit => {}
            _ => panic!("Expected Exit notification"),
        }
    }

    #[test]
    fn test_parse_multiple_lines() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%sessions-changed\n%exit\n");
        assert_eq!(notifications.len(), 2);

        assert!(matches!(
            &notifications[0],
            TmuxNotification::SessionsChanged
        ));
        assert!(matches!(&notifications[1], TmuxNotification::Exit));
    }

    #[test]
    fn test_parse_incomplete_line() {
        let mut parser = TmuxControlParser::new(true);

        // Parse partial data
        let notifications = parser.parse(b"%sessions");
        assert_eq!(notifications.len(), 0);
        assert_eq!(parser.buffer_len(), 9); // "%sessions" buffered

        // Complete the line
        let notifications = parser.parse(b"-changed\n");
        assert_eq!(notifications.len(), 1);
        assert!(matches!(
            &notifications[0],
            TmuxNotification::SessionsChanged
        ));
        assert_eq!(parser.buffer_len(), 0);
    }

    #[test]
    fn test_parse_unknown_notification() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%unknown-notification some args\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::Unknown { line } => {
                assert_eq!(line, "%unknown-notification some args");
            }
            _ => panic!("Expected Unknown notification"),
        }
    }

    #[test]
    fn test_parse_empty_line() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"\n");
        assert_eq!(notifications.len(), 0);
    }

    #[test]
    fn test_parse_terminal_output_in_control_mode() {
        let mut parser = TmuxControlParser::new(true);
        // Non-% prefixed line in control mode (shouldn't normally happen)
        let notifications = parser.parse(b"regular output\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::TerminalOutput { data } => {
                assert_eq!(data, b"regular output");
            }
            _ => panic!("Expected TerminalOutput notification"),
        }
    }

    #[test]
    fn test_parse_non_control_mode() {
        let mut parser = TmuxControlParser::new(false);
        let notifications = parser.parse(b"This is regular terminal output");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::TerminalOutput { data } => {
                assert_eq!(data, b"This is regular terminal output");
            }
            _ => panic!("Expected TerminalOutput notification"),
        }
    }

    #[test]
    fn test_clear_buffer() {
        let mut parser = TmuxControlParser::new(true);
        parser.parse(b"%incomplete");
        assert!(parser.buffer_len() > 0);

        parser.clear_buffer();
        assert_eq!(parser.buffer_len(), 0);
    }

    #[test]
    fn test_parse_pane_mode_changed() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%pane-mode-changed %1\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::PaneModeChanged { pane_id } => {
                assert_eq!(pane_id, "%1");
            }
            _ => panic!("Expected PaneModeChanged notification"),
        }
    }

    #[test]
    fn test_parse_window_pane_changed() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%window-pane-changed @5 %12\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::WindowPaneChanged { window_id, pane_id } => {
                assert_eq!(window_id, "@5");
                assert_eq!(pane_id, "%12");
            }
            _ => panic!("Expected WindowPaneChanged notification"),
        }
    }

    #[test]
    fn test_parse_layout_change() {
        let mut parser = TmuxControlParser::new(true);
        let notifications =
            parser.parse(b"%layout-change @1 abc123,80x24,0,0 def456,80x24,0,0 *Z\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::LayoutChange {
                window_id,
                window_layout,
                window_visible_layout,
                window_raw_flags,
            } => {
                assert_eq!(window_id, "@1");
                assert_eq!(window_layout, "abc123,80x24,0,0");
                assert_eq!(window_visible_layout, "def456,80x24,0,0");
                assert_eq!(window_raw_flags, "*Z");
            }
            _ => panic!("Expected LayoutChange notification"),
        }
    }

    #[test]
    fn test_parse_paste_buffer_changed() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%paste-buffer-changed buffer0\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::PasteBufferChanged { name } => {
                assert_eq!(name, "buffer0");
            }
            _ => panic!("Expected PasteBufferChanged notification"),
        }
    }

    #[test]
    fn test_parse_paste_buffer_deleted() {
        let mut parser = TmuxControlParser::new(true);
        let notifications = parser.parse(b"%paste-buffer-deleted buffer0\n");
        assert_eq!(notifications.len(), 1);

        match &notifications[0] {
            TmuxNotification::PasteBufferDeleted { name } => {
                assert_eq!(name, "buffer0");
            }
            _ => panic!("Expected PasteBufferDeleted notification"),
        }
    }

    #[test]
    fn test_notification_type() {
        assert_eq!(
            TmuxNotification::SessionsChanged.notification_type(),
            "sessions-changed"
        );
        assert_eq!(TmuxNotification::Exit.notification_type(), "exit");
        assert_eq!(
            TmuxNotification::Output {
                pane_id: "%1".to_string(),
                data: vec![]
            }
            .notification_type(),
            "output"
        );
        assert_eq!(
            TmuxNotification::LayoutChange {
                window_id: "@1".to_string(),
                window_layout: "layout".to_string(),
                window_visible_layout: "visible".to_string(),
                window_raw_flags: "flags".to_string(),
            }
            .notification_type(),
            "layout-change"
        );
        assert_eq!(
            TmuxNotification::PasteBufferChanged {
                name: "buffer0".to_string(),
            }
            .notification_type(),
            "paste-buffer-changed"
        );
        assert_eq!(
            TmuxNotification::PasteBufferDeleted {
                name: "buffer0".to_string(),
            }
            .notification_type(),
            "paste-buffer-deleted"
        );
    }
}
