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
            if let Some(notification) = Self::parse_line(&line) {
                notifications.push(notification);
            }
        }

        notifications
    }

    /// Parse a single line into a notification
    fn parse_line(line: &str) -> Option<TmuxNotification> {
        // Only trim leading whitespace - trailing whitespace may be significant
        // for %output notifications where space is the actual output data.
        let line = line.trim_start();

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
            "workspaces-changed" => Some(TmuxNotification::WorkspacesChanged),
            "session-window-changed" => Self::parse_session_window_changed(args),
            "client-detached" => Self::parse_client_detached(args),
            "client-attached" => Self::parse_client_attached(args),
            "client-left" => Self::parse_client_left(args),
            "exit" => Some(TmuxNotification::Exit),
            "pause" => Self::parse_pause(args),
            "extended-output" => Self::parse_extended_output(args),
            "continue" => Self::parse_continue(args),
            "subscription-changed" => Self::parse_subscription_changed(args),
            "layout-change" => Self::parse_layout_change(args),
            "paste-buffer-changed" => Self::parse_paste_buffer_changed(args),
            "paste-buffer-deleted" => Self::parse_paste_buffer_deleted(args),
            "agent-state-changed" => Self::parse_agent_state_changed(args),
            "agent-released" => Self::parse_agent_released(args),
            "agent-telemetry-changed" => Self::parse_agent_telemetry_changed(args),
            "pane-title-changed" => Self::parse_pane_title_changed(args),
            "pane-exited" => Self::parse_pane_exited(args),
            "pane-respawned" => Self::parse_pane_id_only(args),
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
    pub(super) fn unescape_output(s: &str) -> Vec<u8> {
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
        // `window_id [layout visible-layout raw-flags]` — the triple is
        // optional, so a bare-id line (real tmux's shape, and this daemon's
        // pre-triple emission) parses with empty fields rather than failing.
        let mut parts = args.splitn(4, ' ');
        let window_id = parts.next().unwrap_or_default().trim().to_string();
        Some(TmuxNotification::WindowAdd {
            window_id,
            window_layout: parts.next().unwrap_or_default().to_string(),
            window_visible_layout: parts.next().unwrap_or_default().to_string(),
            window_raw_flags: parts.next().unwrap_or_default().to_string(),
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

    fn parse_pane_title_changed(args: &str) -> Option<TmuxNotification> {
        // The title is the rest of the line — it may contain spaces — and a
        // MISSING second token is the explicit clear (the emitter omits the
        // separator for an empty title rather than trailing-space it).
        let (pane_id, title) = match args.split_once(' ') {
            Some((id, rest)) => (id, rest),
            None => (args, ""),
        };
        if pane_id.is_empty() {
            return None;
        }
        Some(TmuxNotification::PaneTitleChanged {
            pane_id: pane_id.to_string(),
            title: title.to_string(),
        })
    }

    fn parse_pane_exited(args: &str) -> Option<TmuxNotification> {
        // pane_id [exit_code] — a missing code token means the daemon had
        // no exit status to read as it observed the death.
        let (pane_id, code) = match args.split_once(' ') {
            Some((id, rest)) => (id, rest.trim().parse::<i32>().ok()),
            None => (args, None),
        };
        if pane_id.is_empty() {
            return None;
        }
        Some(TmuxNotification::PaneExited {
            pane_id: pane_id.to_string(),
            exit_code: code,
        })
    }

    fn parse_pane_id_only(args: &str) -> Option<TmuxNotification> {
        if args.is_empty() {
            return None;
        }
        Some(TmuxNotification::PaneRespawned {
            pane_id: args.trim().to_string(),
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

    fn parse_client_attached(args: &str) -> Option<TmuxNotification> {
        let client = args.split_whitespace().next()?;
        Some(TmuxNotification::ClientAttached {
            client: client.to_string(),
        })
    }

    fn parse_client_left(args: &str) -> Option<TmuxNotification> {
        let mut parts = args.split_whitespace();
        let client = parts.next()?.to_string();
        let session_id = parts.next().map(str::to_string);
        let window_id = parts.next().map(str::to_string);
        Some(TmuxNotification::ClientLeft {
            client,
            session_id,
            window_id,
        })
    }

    fn parse_pause(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::Pause {
            pane_id: args.trim().to_string(),
        })
    }

    fn parse_continue(args: &str) -> Option<TmuxNotification> {
        Some(TmuxNotification::Continue {
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
        // tmux(1): `pane-id age ... : value` — arguments up to the first
        // lone `:` are for future use and ignored; the value follows the
        // `: ` separator. A line with no lone colon keeps the legacy
        // leading-`:`-stripped reading.
        let rest = parts[2];
        let data_part = if let Some(value) = rest.strip_prefix(": ") {
            value
        } else if let Some(pos) = rest.find(" : ") {
            &rest[pos + 3..]
        } else {
            rest.strip_prefix(':').unwrap_or(rest)
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
        // tmux(1): `name session-id window-id window-index pane-id ... :
        // value` — the value follows the first lone `:`; the ids and any
        // future-use arguments before it are not part of it. A line with no
        // lone colon keeps the whole remainder as the value.
        let rest = parts.get(1).copied().unwrap_or_default();
        let value = match rest.strip_prefix(": ") {
            Some(value) => value,
            None => rest.find(" : ").map_or(rest, |pos| &rest[pos + 3..]),
        };
        Some(TmuxNotification::SubscriptionChanged {
            name: parts[0].to_string(),
            value: value.to_string(),
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

    fn parse_agent_state_changed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() < 3 {
            return None;
        }
        // Trailing `key=value` tokens are metadata, never state. Index 2 is
        // always state (a three-token line keeps `source=hook` as its
        // state), and unknown keys are ignored so a future token never
        // folds into it.
        let mut end = parts.len();
        let (mut source, mut ws) = (String::new(), String::new());
        while end > 3 {
            let Some((key, value)) = parts[end - 1].split_once('=') else {
                break;
            };
            if key.is_empty() || !key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
                break;
            }
            match key {
                "source" => source = value.to_string(),
                "ws" => ws = value.to_string(),
                _ => {}
            }
            end -= 1;
        }
        Some(TmuxNotification::AgentStateChanged {
            pane_id: parts[0].to_string(),
            agent: parts[1].to_string(),
            // The state is the rest of the line, so a multi-word state
            // survives the round trip.
            state: parts[2..end].join(" "),
            source,
            ws,
        })
    }

    fn parse_agent_released(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() != 2 {
            return None;
        }
        Some(TmuxNotification::AgentReleased {
            pane_id: parts[0].to_string(),
            agent: parts[1].to_string(),
        })
    }

    /// `%agent-telemetry-changed {pane_id} {agent}` — the same two-token
    /// identity shape as `%agent-released`; the values live on the roster,
    /// not the push.
    fn parse_agent_telemetry_changed(args: &str) -> Option<TmuxNotification> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() != 2 {
            return None;
        }
        Some(TmuxNotification::AgentTelemetryChanged {
            pane_id: parts[0].to_string(),
            agent: parts[1].to_string(),
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
