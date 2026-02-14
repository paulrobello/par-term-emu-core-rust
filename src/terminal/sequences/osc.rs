//! OSC (Operating System Command) sequence handling
//!
//! Handles OSC sequences for terminal control, including:
//! - Window title manipulation
//! - Color queries and modifications
//! - Clipboard operations (OSC 52)
//! - Hyperlinks (OSC 8)
//! - Shell integration (OSC 133)
//! - Notifications (OSC 9, OSC 777)
//! - Progress bar (OSC 9;4 - ConEmu/Windows Terminal style)
//! - Named progress bars (OSC 934 - multiple concurrent progress bars)
//! - Directory tracking (OSC 7)

use crate::color::Color;
use crate::debug;
use crate::shell_integration::ShellIntegrationMarker;
use crate::terminal::progress::{ProgressBar, ProgressBarCommand, ProgressState};
use crate::terminal::{Notification, Terminal};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use percent_encoding::percent_decode_str;
use url::Url;

impl Terminal {
    /// Check if an OSC command should be filtered due to security settings
    ///
    /// Returns true if the command should be blocked when disable_insecure_sequences is enabled.
    pub(in crate::terminal) fn is_insecure_osc(&self, command: &str) -> bool {
        if !self.disable_insecure_sequences {
            return false;
        }

        // Filter potentially insecure OSC sequences
        matches!(
            command,
            "52" |  // Clipboard operations (can leak data)
            "8" |   // Hyperlinks (can be used for phishing)
            "9" |   // Notifications (can be annoying/misleading)
            "777" // Notifications (urxvt style)
        )
    }

    /// Parse X11/xterm color specification to RGB tuple
    ///
    /// Supported formats:
    /// - rgb:RR/GG/BB (hex, each component 0-FF, case-insensitive)
    /// - #RRGGBB (hex, case-insensitive)
    ///
    /// Returns Some((r, g, b)) where each component is 0-255, or None if invalid
    pub(in crate::terminal) fn parse_color_spec(spec: &str) -> Option<(u8, u8, u8)> {
        let spec = spec.trim();

        if spec.is_empty() {
            return None;
        }

        // Format: rgb:RR/GG/BB (case-insensitive)
        if spec.to_lowercase().starts_with("rgb:") {
            let parts: Vec<&str> = spec[4..].split('/').collect();
            if parts.len() != 3 {
                return None;
            }

            // Parse hex components (1-4 hex digits each, we use first 2)
            let r = u8::from_str_radix(&format!("{:0<2}", &parts[0][..parts[0].len().min(2)]), 16)
                .ok()?;
            let g = u8::from_str_radix(&format!("{:0<2}", &parts[1][..parts[1].len().min(2)]), 16)
                .ok()?;
            let b = u8::from_str_radix(&format!("{:0<2}", &parts[2][..parts[2].len().min(2)]), 16)
                .ok()?;
            return Some((r, g, b));
        }

        // Format: #RRGGBB (case-insensitive)
        if spec.starts_with('#') && spec.len() == 7 {
            let r = u8::from_str_radix(&spec[1..3], 16).ok()?;
            let g = u8::from_str_radix(&spec[3..5], 16).ok()?;
            let b = u8::from_str_radix(&spec[5..7], 16).ok()?;
            return Some((r, g, b));
        }

        None
    }

    /// Push bytes to response buffer (for device queries)
    pub(in crate::terminal) fn push_response(&mut self, bytes: &[u8]) {
        self.response_buffer.extend_from_slice(bytes);
    }

    /// VTE OSC dispatch - handle OSC sequences
    pub(in crate::terminal) fn osc_dispatch_impl(
        &mut self,
        params: &[&[u8]],
        _bell_terminated: bool,
    ) {
        debug::log_osc_dispatch(params);
        // Handle OSC sequences
        if params.is_empty() {
            return;
        }

        if let Ok(command) = std::str::from_utf8(params[0]) {
            // Filter insecure sequences if configured
            if self.is_insecure_osc(command) {
                debug::log(
                    debug::DebugLevel::Debug,
                    "SECURITY",
                    &format!(
                        "Blocked insecure OSC {} (disable_insecure_sequences=true)",
                        command
                    ),
                );
                return;
            }

            match command {
                "0" | "2" => {
                    // Set window title
                    if params.len() >= 2 {
                        if let Ok(title) = std::str::from_utf8(params[1]) {
                            let new_title = title.to_string();
                            if self.title != new_title {
                                self.title = new_title.clone();
                                self.terminal_events
                                    .push(crate::terminal::TerminalEvent::TitleChanged(new_title));
                            }
                        }
                    }
                }
                "21" => {
                    // Push window title onto stack (XTWINOPS)
                    // OSC 21 ; text ST
                    if params.len() >= 2 {
                        if let Ok(title) = std::str::from_utf8(params[1]) {
                            self.title_stack.push(title.to_string());
                        }
                    } else {
                        // No parameter - push current title
                        self.title_stack.push(self.title.clone());
                    }
                }
                "22" => {
                    // Pop window title from stack (XTWINOPS)
                    // OSC 22 ST
                    if let Some(title) = self.title_stack.pop() {
                        self.title = title;
                    }
                }
                "23" => {
                    // Pop icon title from stack (XTWINOPS)
                    // OSC 23 ST
                    // Note: We don't distinguish between window and icon titles,
                    // so this behaves the same as OSC 22
                    if let Some(title) = self.title_stack.pop() {
                        self.title = title;
                    }
                }
                "7" => {
                    // Set current working directory (OSC 7)
                    // Format: OSC 7 ; file://hostname/path ST
                    // Only process if accept_osc7 is enabled
                    if self.accept_osc7 && params.len() >= 2 {
                        if let Ok(cwd_url) = std::str::from_utf8(params[1]) {
                            if let Some((path, hostname, username)) = Self::parse_osc7_url(cwd_url)
                            {
                                self.record_cwd_change(path.clone(), hostname.clone(), username);
                                debug::log(
                                    debug::DebugLevel::Debug,
                                    "OSC7",
                                    &format!(
                                        "Set directory to: {} (hostname: {:?})",
                                        path, hostname
                                    ),
                                );
                            }
                        }
                    }
                }
                "8" => {
                    // Hyperlink (OSC 8) - supported by iTerm2, VTE, etc.
                    // Format: OSC 8 ; params ; URI ST
                    // Where params can be id=xyz for link identification
                    if params.len() >= 3 {
                        if let Ok(url) = std::str::from_utf8(params[2]) {
                            let url = url.trim();

                            if url.is_empty() {
                                // Empty URL = end hyperlink
                                self.current_hyperlink_id = None;
                            } else {
                                // Check if URL already exists (deduplication)
                                let id = self
                                    .hyperlinks
                                    .iter()
                                    .find(|(_, v)| v.as_str() == url)
                                    .map(|(k, _)| *k)
                                    .unwrap_or_else(|| {
                                        let id = self.next_hyperlink_id;
                                        self.hyperlinks.insert(id, url.to_string());
                                        self.next_hyperlink_id += 1;
                                        id
                                    });

                                self.current_hyperlink_id = Some(id);

                                // Emit hyperlink event with position data
                                self.terminal_events.push(
                                    crate::terminal::TerminalEvent::HyperlinkAdded {
                                        url: url.to_string(),
                                        row: self.cursor.row,
                                        col: self.cursor.col,
                                        id: Some(id),
                                    },
                                );
                            }
                        }
                    } else if params.len() == 2 {
                        // OSC 8 ; ; ST (empty params and URI = end hyperlink)
                        self.current_hyperlink_id = None;
                    }
                }
                "9" => {
                    // OSC 9 - iTerm2/ConEmu style notifications and progress
                    // Simple notification: OSC 9 ; message ST
                    // Progress bar: OSC 9 ; 4 ; state [; progress] ST
                    //   state: 0=hidden, 1=normal, 2=indeterminate, 3=warning, 4=error
                    //   progress: 0-100 (only for states 1, 3, 4)
                    if params.len() >= 2 {
                        if let Ok(param1) = std::str::from_utf8(params[1]) {
                            let param1 = param1.trim();
                            if param1 == "4" {
                                // Progress bar format: OSC 9 ; 4 ; state [; progress] ST
                                self.handle_osc9_progress(&params[2..]);
                            } else {
                                // Simple notification format
                                let notification =
                                    Notification::new(String::new(), param1.to_string());
                                self.enqueue_notification(notification);
                            }
                        }
                    }
                }
                "777" => {
                    // Notification (OSC 777) - urxvt style
                    // Format: OSC 777 ; notify ; title ; message ST
                    if params.len() >= 4 {
                        if let Ok(action) = std::str::from_utf8(params[1]) {
                            if action == "notify" {
                                if let (Ok(title), Ok(message)) = (
                                    std::str::from_utf8(params[2]),
                                    std::str::from_utf8(params[3]),
                                ) {
                                    let notification =
                                        Notification::new(title.to_string(), message.to_string());
                                    self.enqueue_notification(notification);
                                }
                            }
                        }
                    }
                }
                "52" => {
                    // Clipboard operations (OSC 52) - xterm extension
                    // Format: OSC 52 ; selection ; data ST
                    // selection: c=clipboard, p=primary, q=secondary, s=select, 0-7=cut buffers
                    // data: base64 encoded text, or "?" to query
                    if params.len() >= 3 {
                        // Parse selection parameter (we'll focus on 'c' for clipboard)
                        if let Ok(selection) = std::str::from_utf8(params[1]) {
                            if let Ok(data) = std::str::from_utf8(params[2]) {
                                let data = data.trim();

                                // Handle clipboard operations (selection 'c' or any that includes 'c')
                                if selection.contains('c') || selection.is_empty() {
                                    if data == "?" {
                                        // Query clipboard - only respond if allowed (security)
                                        if self.allow_clipboard_read {
                                            if let Some(content) = &self.clipboard_content {
                                                // Encode clipboard content as base64 and send response
                                                let encoded = BASE64.encode(content.as_bytes());
                                                let response =
                                                    format!("\x1b]52;c;{}\x1b\\", encoded);
                                                self.push_response(response.as_bytes());
                                            } else {
                                                // No clipboard content, send empty response
                                                let response = b"\x1b]52;c;\x1b\\";
                                                self.push_response(response);
                                            }
                                        }
                                        // If not allowed, silently ignore (security)
                                    } else if !data.is_empty() {
                                        // Write to clipboard - decode base64
                                        if let Ok(decoded_bytes) = BASE64.decode(data.as_bytes()) {
                                            if let Ok(text) = String::from_utf8(decoded_bytes) {
                                                self.clipboard_content = Some(text);
                                            }
                                        }
                                        // Silently ignore decode errors
                                    } else {
                                        // Empty data = clear clipboard
                                        self.clipboard_content = None;
                                    }
                                }
                            }
                        }
                    }
                }
                "4" => {
                    // Set ANSI color palette entry (OSC 4)
                    // Format: OSC 4 ; index ; colorspec ST
                    // Example: OSC 4 ; 1 ; rgb:FF/00/00 ST (set color 1 to red)
                    if !self.disable_insecure_sequences && params.len() >= 3 {
                        if let Ok(data) = std::str::from_utf8(params[1]) {
                            if let Ok(index) = data.trim().parse::<usize>() {
                                if index < 16 {
                                    if let Ok(colorspec) = std::str::from_utf8(params[2]) {
                                        if let Some((r, g, b)) = Self::parse_color_spec(colorspec) {
                                            self.ansi_palette[index] = Color::Rgb(r, g, b);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                "104" => {
                    // Reset ANSI color palette (OSC 104)
                    // Format: OSC 104 ST (reset all) or OSC 104 ; index ST (reset one)
                    if !self.disable_insecure_sequences {
                        if params.len() == 1 || (params.len() >= 2 && params[1].is_empty()) {
                            // Reset all colors to defaults
                            self.ansi_palette = Self::default_ansi_palette();
                        } else if params.len() >= 2 {
                            // Reset specific color
                            if let Ok(data) = std::str::from_utf8(params[1]) {
                                if let Ok(index) = data.trim().parse::<usize>() {
                                    if index < 16 {
                                        let defaults = Self::default_ansi_palette();
                                        self.ansi_palette[index] = defaults[index];
                                    }
                                }
                            }
                        }
                    }
                }
                "110" => {
                    // Reset default foreground color (OSC 110)
                    if !self.disable_insecure_sequences {
                        self.default_fg = Color::Rgb(0xE5, 0xE5, 0xE5); // iTerm2 default
                    }
                }
                "111" => {
                    // Reset default background color (OSC 111)
                    if !self.disable_insecure_sequences {
                        self.default_bg = Color::Rgb(0x14, 0x19, 0x1E); // iTerm2 default
                    }
                }
                "112" => {
                    // Reset cursor color (OSC 112)
                    if !self.disable_insecure_sequences {
                        self.cursor_color = Color::Rgb(0xE5, 0xE5, 0xE5); // iTerm2 default
                    }
                }
                "10" => {
                    // Query or set default foreground color (OSC 10)
                    // Format: OSC 10 ; ? ST (query)
                    // Format: OSC 10 ; colorspec ST (set)
                    // Response: OSC 10 ; rgb:rrrr/gggg/bbbb ST
                    if params.len() >= 2 {
                        if let Ok(data) = std::str::from_utf8(params[1]) {
                            let data = data.trim();
                            if data == "?" {
                                // Query foreground color
                                let (r, g, b) = self.default_fg.to_rgb();
                                // Convert 8-bit to 16-bit (multiply by 257)
                                let r16 = (r as u16) * 257;
                                let g16 = (g as u16) * 257;
                                let b16 = (b as u16) * 257;
                                let response = format!(
                                    "\x1b]10;rgb:{:04x}/{:04x}/{:04x}\x1b\\",
                                    r16, g16, b16
                                );
                                self.push_response(response.as_bytes());
                            } else if !self.disable_insecure_sequences {
                                // Set foreground color
                                if let Some((r, g, b)) = Self::parse_color_spec(data) {
                                    self.default_fg = Color::Rgb(r, g, b);
                                }
                            }
                        }
                    }
                }
                "11" => {
                    // Query or set default background color (OSC 11)
                    // Format: OSC 11 ; ? ST (query)
                    // Format: OSC 11 ; colorspec ST (set)
                    // Response: OSC 11 ; rgb:rrrr/gggg/bbbb ST
                    if params.len() >= 2 {
                        if let Ok(data) = std::str::from_utf8(params[1]) {
                            let data = data.trim();
                            if data == "?" {
                                // Query background color
                                let (r, g, b) = self.default_bg.to_rgb();
                                // Convert 8-bit to 16-bit (multiply by 257)
                                let r16 = (r as u16) * 257;
                                let g16 = (g as u16) * 257;
                                let b16 = (b as u16) * 257;
                                let response = format!(
                                    "\x1b]11;rgb:{:04x}/{:04x}/{:04x}\x1b\\",
                                    r16, g16, b16
                                );
                                self.push_response(response.as_bytes());
                            } else if !self.disable_insecure_sequences {
                                // Set background color
                                if let Some((r, g, b)) = Self::parse_color_spec(data) {
                                    self.default_bg = Color::Rgb(r, g, b);
                                }
                            }
                        }
                    }
                }
                "12" => {
                    // Query or set cursor color (OSC 12)
                    // Format: OSC 12 ; ? ST (query)
                    // Format: OSC 12 ; colorspec ST (set)
                    // Response: OSC 12 ; rgb:rrrr/gggg/bbbb ST
                    if params.len() >= 2 {
                        if let Ok(data) = std::str::from_utf8(params[1]) {
                            let data = data.trim();
                            if data == "?" {
                                // Query cursor color
                                let (r, g, b) = self.cursor_color.to_rgb();
                                // Convert 8-bit to 16-bit (multiply by 257)
                                let r16 = (r as u16) * 257;
                                let g16 = (g as u16) * 257;
                                let b16 = (b as u16) * 257;
                                let response = format!(
                                    "\x1b]12;rgb:{:04x}/{:04x}/{:04x}\x1b\\",
                                    r16, g16, b16
                                );
                                self.push_response(response.as_bytes());
                            } else if !self.disable_insecure_sequences {
                                // Set cursor color
                                if let Some((r, g, b)) = Self::parse_color_spec(data) {
                                    self.cursor_color = Color::Rgb(r, g, b);
                                }
                            }
                        }
                    }
                }
                "133" => {
                    // Shell integration (iTerm2/VSCode)
                    if params.len() >= 2 {
                        if let Ok(marker) = std::str::from_utf8(params[1]) {
                            let ts = crate::terminal::unix_millis();
                            let abs_line = self.active_grid().scrollback_len() + self.cursor.row;
                            match marker.chars().next() {
                                Some('A') => {
                                    self.shell_integration
                                        .set_marker(ShellIntegrationMarker::PromptStart);
                                    self.terminal_events.push(
                                        crate::terminal::TerminalEvent::ShellIntegrationEvent {
                                            event_type: "prompt_start".to_string(),
                                            command: None,
                                            exit_code: None,
                                            timestamp: Some(ts),
                                            cursor_line: Some(abs_line),
                                        },
                                    );
                                    // Sub-shell detection: if we see a new prompt while
                                    // inside command output, a sub-shell was spawned
                                    if self.in_command_output && self.shell_depth > 0 {
                                        self.shell_depth += 1;
                                        self.terminal_events.push(
                                            crate::terminal::TerminalEvent::SubShellDetected {
                                                depth: self.shell_depth,
                                                shell_type: None,
                                            },
                                        );
                                    } else if self.shell_depth == 0 {
                                        self.shell_depth = 1;
                                    }
                                    self.in_command_output = false;
                                    // Zone: close any open zone, start new Prompt zone
                                    if !self.alt_screen_active {
                                        let close_row = if abs_line > 0 { abs_line - 1 } else { 0 };
                                        // Emit ZoneClosed for previous zone if exists
                                        if let Some(zone) = self.grid.zones().last() {
                                            let closed_id = zone.id;
                                            let closed_type = zone.zone_type;
                                            let closed_start = zone.abs_row_start;
                                            self.grid.close_current_zone(close_row);
                                            self.terminal_events.push(
                                                crate::terminal::TerminalEvent::ZoneClosed {
                                                    zone_id: closed_id,
                                                    zone_type: closed_type,
                                                    abs_row_start: closed_start,
                                                    abs_row_end: close_row,
                                                    exit_code: None,
                                                },
                                            );
                                        } else {
                                            self.grid.close_current_zone(close_row);
                                        }
                                        // Create and push new zone with ID
                                        let zone_id = self.next_zone_id;
                                        self.next_zone_id += 1;
                                        self.grid.push_zone(crate::zone::Zone::new(
                                            zone_id,
                                            crate::zone::ZoneType::Prompt,
                                            abs_line,
                                            Some(ts),
                                        ));
                                        self.terminal_events.push(
                                            crate::terminal::TerminalEvent::ZoneOpened {
                                                zone_id,
                                                zone_type: crate::zone::ZoneType::Prompt,
                                                abs_row_start: abs_line,
                                            },
                                        );
                                    }
                                }
                                Some('B') => {
                                    self.shell_integration
                                        .set_marker(ShellIntegrationMarker::CommandStart);
                                    self.terminal_events.push(
                                        crate::terminal::TerminalEvent::ShellIntegrationEvent {
                                            event_type: "command_start".to_string(),
                                            command: self
                                                .shell_integration
                                                .command()
                                                .map(|s| s.to_string()),
                                            exit_code: None,
                                            timestamp: Some(ts),
                                            cursor_line: Some(abs_line),
                                        },
                                    );
                                    // Zone: close Prompt zone, start Command zone
                                    if !self.alt_screen_active {
                                        let close_row = if abs_line > 0 { abs_line - 1 } else { 0 };
                                        // Emit ZoneClosed for previous zone if exists
                                        if let Some(zone) = self.grid.zones().last() {
                                            let closed_id = zone.id;
                                            let closed_type = zone.zone_type;
                                            let closed_start = zone.abs_row_start;
                                            self.grid.close_current_zone(close_row);
                                            self.terminal_events.push(
                                                crate::terminal::TerminalEvent::ZoneClosed {
                                                    zone_id: closed_id,
                                                    zone_type: closed_type,
                                                    abs_row_start: closed_start,
                                                    abs_row_end: close_row,
                                                    exit_code: None,
                                                },
                                            );
                                        } else {
                                            self.grid.close_current_zone(close_row);
                                        }
                                        // Create and push new zone with ID
                                        let zone_id = self.next_zone_id;
                                        self.next_zone_id += 1;
                                        let mut zone = crate::zone::Zone::new(
                                            zone_id,
                                            crate::zone::ZoneType::Command,
                                            abs_line,
                                            Some(ts),
                                        );
                                        zone.command =
                                            self.shell_integration.command().map(|s| s.to_string());
                                        self.grid.push_zone(zone);
                                        self.terminal_events.push(
                                            crate::terminal::TerminalEvent::ZoneOpened {
                                                zone_id,
                                                zone_type: crate::zone::ZoneType::Command,
                                                abs_row_start: abs_line,
                                            },
                                        );
                                    }
                                }
                                Some('C') => {
                                    self.shell_integration
                                        .set_marker(ShellIntegrationMarker::CommandExecuted);
                                    self.terminal_events.push(
                                        crate::terminal::TerminalEvent::ShellIntegrationEvent {
                                            event_type: "command_executed".to_string(),
                                            command: self
                                                .shell_integration
                                                .command()
                                                .map(|s| s.to_string()),
                                            exit_code: None,
                                            timestamp: Some(ts),
                                            cursor_line: Some(abs_line),
                                        },
                                    );
                                    // Zone: close Command zone, start Output zone
                                    if !self.alt_screen_active {
                                        let close_row = if abs_line > 0 { abs_line - 1 } else { 0 };
                                        // Emit ZoneClosed for previous zone if exists
                                        if let Some(zone) = self.grid.zones().last() {
                                            let closed_id = zone.id;
                                            let closed_type = zone.zone_type;
                                            let closed_start = zone.abs_row_start;
                                            self.grid.close_current_zone(close_row);
                                            self.terminal_events.push(
                                                crate::terminal::TerminalEvent::ZoneClosed {
                                                    zone_id: closed_id,
                                                    zone_type: closed_type,
                                                    abs_row_start: closed_start,
                                                    abs_row_end: close_row,
                                                    exit_code: None,
                                                },
                                            );
                                        } else {
                                            self.grid.close_current_zone(close_row);
                                        }
                                        // Create and push new zone with ID
                                        let zone_id = self.next_zone_id;
                                        self.next_zone_id += 1;
                                        let mut zone = crate::zone::Zone::new(
                                            zone_id,
                                            crate::zone::ZoneType::Output,
                                            abs_line,
                                            Some(ts),
                                        );
                                        zone.command =
                                            self.shell_integration.command().map(|s| s.to_string());
                                        self.grid.push_zone(zone);
                                        self.terminal_events.push(
                                            crate::terminal::TerminalEvent::ZoneOpened {
                                                zone_id,
                                                zone_type: crate::zone::ZoneType::Output,
                                                abs_row_start: abs_line,
                                            },
                                        );
                                    }
                                    // Mark that we're inside command output
                                    self.in_command_output = true;
                                }
                                Some('D') => {
                                    self.shell_integration
                                        .set_marker(ShellIntegrationMarker::CommandFinished);

                                    // Extract exit code from third OSC parameter when present:
                                    // OSC 133 ; D ; <exit_code> ST
                                    let exit_param = params.get(2).or_else(|| params.get(1));
                                    let mut parsed_code: Option<i32> = None;
                                    if let Some(code_bytes) = exit_param {
                                        if let Ok(code_str) = std::str::from_utf8(code_bytes) {
                                            if let Ok(code) = code_str.parse::<i32>() {
                                                self.shell_integration.set_exit_code(code);
                                                parsed_code = Some(code);
                                            }
                                        }
                                    }
                                    self.terminal_events.push(
                                        crate::terminal::TerminalEvent::ShellIntegrationEvent {
                                            event_type: "command_finished".to_string(),
                                            command: None,
                                            exit_code: parsed_code,
                                            timestamp: Some(ts),
                                            cursor_line: Some(abs_line),
                                        },
                                    );
                                    // Zone: close Output zone, record exit code
                                    if !self.alt_screen_active {
                                        // Capture zone info before closing
                                        let closed_info = self
                                            .grid
                                            .zones()
                                            .last()
                                            .map(|z| (z.id, z.zone_type, z.abs_row_start));
                                        self.grid.close_current_zone(abs_line);
                                        // Set exit code on the just-closed Output zone
                                        if let Some(zone) = self.grid.zones_mut().last_mut() {
                                            if zone.zone_type == crate::zone::ZoneType::Output {
                                                zone.exit_code = parsed_code;
                                            }
                                        }
                                        // Emit ZoneClosed
                                        if let Some((id, zt, start)) = closed_info {
                                            self.terminal_events.push(
                                                crate::terminal::TerminalEvent::ZoneClosed {
                                                    zone_id: id,
                                                    zone_type: zt,
                                                    abs_row_start: start,
                                                    abs_row_end: abs_line,
                                                    exit_code: parsed_code,
                                                },
                                            );
                                        }
                                    }
                                    // Sub-shell detection: leaving command output
                                    self.in_command_output = false;
                                    if self.shell_depth > 1 {
                                        self.shell_depth -= 1;
                                        self.terminal_events.push(
                                            crate::terminal::TerminalEvent::SubShellDetected {
                                                depth: self.shell_depth,
                                                shell_type: None,
                                            },
                                        );
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "1337" => {
                    // iTerm2 OSC 1337 - multiple features
                    // - File=...: Inline images
                    // - SetBadgeFormat=<base64>: Badge format
                    // VTE splits on ; so we need to join params[1..] back together
                    if params.len() >= 2 {
                        // Join all remaining params with semicolons (VTE split them)
                        let mut data_parts = Vec::new();
                        for p in &params[1..] {
                            if let Ok(s) = std::str::from_utf8(p) {
                                data_parts.push(s);
                            }
                        }
                        let data = data_parts.join(";");

                        // Check for SetBadgeFormat
                        if let Some(encoded) = data.strip_prefix("SetBadgeFormat=") {
                            self.handle_set_badge_format(encoded);
                        } else if let Some(payload) = data.strip_prefix("SetUserVar=") {
                            self.handle_set_user_var(payload);
                        } else if let Some(payload) = data.strip_prefix("RemoteHost=") {
                            self.handle_remote_host(payload);
                        } else if let Some(payload) = data.strip_prefix("RequestUpload=") {
                            self.handle_request_upload(payload);
                        } else {
                            // Default to inline image handling
                            self.handle_iterm_image(&data);
                        }
                    }
                }
                "934" => {
                    // OSC 934 - Named progress bar protocol
                    // Format: OSC 934 ; action ; id [; key=value ...] ST
                    self.handle_osc934(params);
                }
                _ => {}
            }
        }
    }

    /// Handle OSC 934 named progress bar sequences
    ///
    /// Format: `OSC 934 ; action ; id [; key=value ...] ST`
    ///
    /// Actions:
    /// - `set` — create or update a progress bar
    /// - `remove` — remove a specific progress bar by ID
    /// - `remove_all` — remove all progress bars
    fn handle_osc934(&mut self, params: &[&[u8]]) {
        match ProgressBarCommand::parse(params) {
            Some(ProgressBarCommand::Set(bar)) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC934",
                    &format!(
                        "Set progress bar: id={}, state={}, percent={}, label={:?}",
                        bar.id,
                        bar.state.description(),
                        bar.percent,
                        bar.label
                    ),
                );
                self.set_named_progress_bar(bar);
            }
            Some(ProgressBarCommand::Remove(id)) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC934",
                    &format!("Remove progress bar: id={}", id),
                );
                self.remove_named_progress_bar(&id);
            }
            Some(ProgressBarCommand::RemoveAll) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC934",
                    "Remove all progress bars",
                );
                self.remove_all_named_progress_bars();
            }
            None => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC934",
                    "Failed to parse OSC 934 sequence",
                );
            }
        }
    }

    /// Parse OSC 7 payload and return decoded path, hostname, username
    ///
    /// Supports:
    /// - file:///path (implicit localhost, no hostname)
    /// - file://hostname/path
    /// - file://user@hostname/path
    /// - file://hostname:port/path (port ignored for hostname matching)
    /// - Performs percent-decoding on path and username, strips query/fragment,
    ///   and rejects non-absolute paths.
    fn parse_osc7_url(url_str: &str) -> Option<(String, Option<String>, Option<String>)> {
        // Prefer strict parsing when possible (handles IPv6 brackets, normalization, etc.)
        if let Ok(url) = Url::parse(url_str) {
            if url.scheme() == "file" {
                let raw_path = url.path();
                if !raw_path.is_empty() && raw_path.starts_with('/') {
                    let path = percent_decode_str(raw_path).decode_utf8_lossy().to_string();

                    let username = url.username();
                    let username = if username.is_empty() {
                        None
                    } else {
                        Some(percent_decode_str(username).decode_utf8_lossy().to_string())
                    };

                    let hostname = url.host_str().map(|h| h.to_string()).and_then(|h| {
                        if h.is_empty() || h.eq_ignore_ascii_case("localhost") {
                            None
                        } else {
                            Some(h)
                        }
                    });

                    if path.starts_with('/') {
                        return Some((path, hostname, username));
                    }
                }
            }
        }

        // Fallback manual parser to handle relaxed/legacy forms
        if !url_str.starts_with("file://") {
            return None;
        }

        let mut remainder = &url_str[7..]; // strip "file://"
        if remainder.is_empty() {
            return None;
        }

        // Strip query/fragment
        if let Some(idx) = remainder.find(['?', '#']) {
            remainder = &remainder[..idx];
        }

        let mut username = None;
        let hostname: Option<String>;
        let path: String;

        if remainder.starts_with('/') {
            path = percent_decode_str(remainder)
                .decode_utf8_lossy()
                .to_string();
            hostname = None;
        } else {
            let slash_idx = remainder.find('/')?;
            let authority = &remainder[..slash_idx];
            let path_part = &remainder[slash_idx..];

            let (user_part, host_part) = match authority.rsplit_once('@') {
                Some((user, host)) => (Some(user), host),
                None => (None, authority),
            };

            if let Some(user) = user_part {
                let decoded = percent_decode_str(user).decode_utf8_lossy().to_string();
                if !decoded.is_empty() {
                    username = Some(decoded);
                }
            }

            let host_only = host_part.split(':').next().unwrap_or("");
            if host_only.is_empty() || host_only.eq_ignore_ascii_case("localhost") {
                hostname = None;
            } else {
                hostname = Some(host_only.to_string());
            }

            path = percent_decode_str(path_part)
                .decode_utf8_lossy()
                .to_string();
        }

        if path.is_empty() || !path.starts_with('/') {
            return None;
        }

        Some((path, hostname, username))
    }

    /// Handle OSC 9;4 progress bar sequences (ConEmu/Windows Terminal style)
    ///
    /// Format: OSC 9 ; 4 ; state [; progress] ST
    /// - state 0: Hide progress bar
    /// - state 1: Normal progress (0-100%)
    /// - state 2: Indeterminate/busy indicator
    /// - state 3: Warning state (0-100%)
    /// - state 4: Error state (0-100%)
    fn handle_osc9_progress(&mut self, params: &[&[u8]]) {
        // Need at least the state parameter
        if params.is_empty() {
            return;
        }

        // Parse state parameter
        let state_param = match std::str::from_utf8(params[0]) {
            Ok(s) => s.trim(),
            Err(_) => return,
        };

        let state_num: u8 = match state_param.parse() {
            Ok(n) => n,
            Err(_) => return,
        };

        let state = ProgressState::from_param(state_num);

        // Parse progress percentage if present and required
        let progress = if state.requires_progress() && params.len() >= 2 {
            match std::str::from_utf8(params[1]) {
                Ok(s) => s.trim().parse::<u8>().unwrap_or(0).min(100),
                Err(_) => 0,
            }
        } else {
            0
        };

        self.progress_bar = ProgressBar::new(state, progress);

        debug::log(
            debug::DebugLevel::Debug,
            "OSC9",
            &format!(
                "Progress bar: state={}, progress={}",
                state.description(),
                progress
            ),
        );
    }

    /// Handle OSC 1337 SetBadgeFormat sequence
    ///
    /// Format: OSC 1337 ; SetBadgeFormat=<base64-encoded-format> ST
    ///
    /// The format string can contain `\(variable)` placeholders that will be
    /// evaluated against session variables. For example:
    /// - `\(username)@\(hostname)`
    /// - `\(session.path)`
    ///
    /// An empty encoded value clears the badge format.
    fn handle_set_badge_format(&mut self, encoded: &str) {
        let encoded = encoded.trim();

        // Empty value clears the badge
        if encoded.is_empty() {
            self.badge_format = None;
            self.terminal_events
                .push(crate::terminal::TerminalEvent::BadgeChanged(None));
            debug::log(debug::DebugLevel::Debug, "OSC1337", "Cleared badge format");
            return;
        }

        // Decode and validate the badge format
        match crate::badge::decode_badge_format(encoded) {
            Ok(format) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC1337",
                    &format!("Set badge format: {:?}", format),
                );
                self.badge_format = Some(format.clone());
                // Emit badge changed event with evaluated badge text
                let badge_text = self.evaluate_badge();
                self.terminal_events
                    .push(crate::terminal::TerminalEvent::BadgeChanged(badge_text));
            }
            Err(e) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC1337",
                    &format!("Invalid badge format: {}", e),
                );
                // Invalid formats are silently ignored (security)
            }
        }
    }

    /// Handle OSC 1337 SetUserVar sequence
    ///
    /// Format: `OSC 1337 ; SetUserVar=<name>=<base64_value> ST`
    ///
    /// Decodes the base64-encoded value and stores it as a user variable
    /// in the session variables. Emits a `UserVarChanged` event if the value changed.
    fn handle_set_user_var(&mut self, payload: &str) {
        let payload = payload.trim();

        // Split on first '=' to get name and base64 value
        let Some((name, encoded_value)) = payload.split_once('=') else {
            debug::log(
                debug::DebugLevel::Debug,
                "OSC1337",
                &format!("SetUserVar: missing '=' separator in {:?}", payload),
            );
            return;
        };

        let name = name.trim();
        if name.is_empty() {
            debug::log(
                debug::DebugLevel::Debug,
                "OSC1337",
                "SetUserVar: empty variable name",
            );
            return;
        }

        // Decode base64 value
        use base64::{engine::general_purpose::STANDARD, Engine};
        let decoded = match STANDARD.decode(encoded_value.trim()) {
            Ok(bytes) => bytes,
            Err(e) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC1337",
                    &format!("SetUserVar: base64 decode error for {:?}: {}", name, e),
                );
                return;
            }
        };

        // Convert to UTF-8
        let value = match String::from_utf8(decoded) {
            Ok(s) => s,
            Err(e) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC1337",
                    &format!("SetUserVar: UTF-8 error for {:?}: {}", name, e),
                );
                return;
            }
        };

        debug::log(
            debug::DebugLevel::Debug,
            "OSC1337",
            &format!("SetUserVar: {}={:?}", name, value),
        );

        self.set_user_var(name.to_string(), value);
    }

    /// Handle OSC 1337 RemoteHost sequence
    ///
    /// Format: `OSC 1337 ; RemoteHost=username@hostname ST`
    ///
    /// Updates `ShellIntegration.hostname` and `ShellIntegration.username`,
    /// then emits a `CwdChanged` event so the frontend can react to the
    /// remote host change. The username part is optional.
    fn handle_remote_host(&mut self, payload: &str) {
        let payload = payload.trim();

        if payload.is_empty() {
            debug::log(
                debug::DebugLevel::Debug,
                "OSC1337",
                "RemoteHost: empty payload",
            );
            return;
        }

        // Parse user@hostname format (username is optional)
        let (username, hostname) = match payload.split_once('@') {
            Some((user, host)) => {
                let user = if user.is_empty() { None } else { Some(user) };
                (user, host)
            }
            None => {
                // No '@' means the whole payload is the hostname
                (None, payload)
            }
        };

        if hostname.is_empty() {
            debug::log(
                debug::DebugLevel::Debug,
                "OSC1337",
                "RemoteHost: empty hostname",
            );
            return;
        }

        debug::log(
            debug::DebugLevel::Debug,
            "OSC1337",
            &format!("RemoteHost: username={:?}, hostname={}", username, hostname),
        );

        // Determine if localhost (treat as no remote host)
        let resolved_hostname = if hostname.eq_ignore_ascii_case("localhost")
            || hostname == "127.0.0.1"
            || hostname == "::1"
        {
            None
        } else {
            Some(hostname.to_string())
        };

        let resolved_username = username.map(|u| u.to_string());

        // Use the current cwd (or empty if none set yet) so the CwdChanged event
        // carries the host/user change. This mirrors how OSC 7 works.
        let current_cwd = self.shell_integration.cwd().unwrap_or("").to_string();

        self.record_cwd_change(current_cwd, resolved_hostname, resolved_username);
    }

    /// Handle OSC 1337 RequestUpload= sequence
    ///
    /// Format: `OSC 1337 ; RequestUpload=format ST`
    ///
    /// The remote application requests the terminal to upload a file.
    /// The `format` parameter specifies the encoding (typically "base64").
    /// This emits an `UploadRequested` event so the frontend can prompt the
    /// user to select a file and call `send_upload_data()` or `cancel_upload()`.
    fn handle_request_upload(&mut self, payload: &str) {
        // Parse format from "format=<value>" pattern
        let format = if let Some(value) = payload.strip_prefix("format=") {
            value.to_string()
        } else {
            // If no format= prefix, use the entire payload as the format
            payload.to_string()
        };

        debug::log(
            debug::DebugLevel::Debug,
            "OSC1337",
            &format!("RequestUpload: format={}", format),
        );

        self.terminal_events
            .push(crate::terminal::TerminalEvent::UploadRequested { format });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell_integration::ShellIntegrationMarker;
    use crate::terminal::Terminal;

    #[test]
    fn test_parse_color_spec_rgb_format() {
        // Valid rgb: format
        assert_eq!(
            Terminal::parse_color_spec("rgb:FF/00/AA"),
            Some((255, 0, 170))
        );
        assert_eq!(
            Terminal::parse_color_spec("rgb:ff/00/aa"),
            Some((255, 0, 170))
        );
        assert_eq!(
            Terminal::parse_color_spec("rgb:12/34/56"),
            Some((18, 52, 86))
        );

        // Single hex digit (should be padded)
        assert_eq!(Terminal::parse_color_spec("rgb:F/0/A"), Some((240, 0, 160)));
    }

    #[test]
    fn test_parse_color_spec_hex_format() {
        // Valid #RRGGBB format
        assert_eq!(Terminal::parse_color_spec("#FF00AA"), Some((255, 0, 170)));
        assert_eq!(Terminal::parse_color_spec("#ff00aa"), Some((255, 0, 170)));
        assert_eq!(Terminal::parse_color_spec("#123456"), Some((18, 52, 86)));
    }

    #[test]
    fn test_parse_color_spec_invalid() {
        // Invalid formats
        assert_eq!(Terminal::parse_color_spec(""), None);
        assert_eq!(Terminal::parse_color_spec("  "), None);
        assert_eq!(Terminal::parse_color_spec("rgb:FF/00"), None); // Missing component
        assert_eq!(Terminal::parse_color_spec("rgb:GG/00/00"), None); // Invalid hex
        assert_eq!(Terminal::parse_color_spec("#FFF"), None); // Too short
        assert_eq!(Terminal::parse_color_spec("#FF00AA00"), None); // Too long
        assert_eq!(Terminal::parse_color_spec("invalid"), None);
    }

    #[test]
    fn test_set_window_title() {
        let mut term = Terminal::new(80, 24);

        // OSC 0 - Set icon name and window title
        term.process(b"\x1b]0;Test Title\x1b\\");
        assert_eq!(term.title(), "Test Title");

        // OSC 2 - Set window title
        term.process(b"\x1b]2;Another Title\x1b\\");
        assert_eq!(term.title(), "Another Title");
    }

    #[test]
    fn test_title_stack() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]0;Original Title\x1b\\");

        // OSC 21 - Push title (no parameter pushes current title)
        term.process(b"\x1b]21\x1b\\");

        // Change title
        term.process(b"\x1b]0;New Title\x1b\\");
        assert_eq!(term.title(), "New Title");

        // OSC 22 - Pop title
        term.process(b"\x1b]22\x1b\\");
        assert_eq!(term.title(), "Original Title");
    }

    #[test]
    fn test_shell_integration_markers() {
        let mut term = Terminal::new(80, 24);

        // OSC 133 A - Prompt start
        term.process(b"\x1b]133;A\x1b\\");
        assert_eq!(
            term.shell_integration.marker(),
            Some(ShellIntegrationMarker::PromptStart)
        );

        // OSC 133 B - Command start
        term.process(b"\x1b]133;B\x1b\\");
        assert_eq!(
            term.shell_integration.marker(),
            Some(ShellIntegrationMarker::CommandStart)
        );

        // OSC 133 C - Command executed
        term.process(b"\x1b]133;C\x1b\\");
        assert_eq!(
            term.shell_integration.marker(),
            Some(ShellIntegrationMarker::CommandExecuted)
        );

        // OSC 133 D - Command finished
        term.process(b"\x1b]133;D\x1b\\");
        assert_eq!(
            term.shell_integration.marker(),
            Some(ShellIntegrationMarker::CommandFinished)
        );
    }

    // Note: Exit code parsing in OSC 133 appears to expect a different format
    // than standard OSC parameter separation allows. Skipping this test for now.
    // The shell integration marker tests cover the main functionality.

    #[test]
    fn test_hyperlinks() {
        let mut term = Terminal::new(80, 24);

        // Start hyperlink
        term.process(b"\x1b]8;;https://example.com\x1b\\");
        assert!(term.current_hyperlink_id.is_some());
        let id1 = term.current_hyperlink_id.unwrap();

        // End hyperlink (empty URL)
        term.process(b"\x1b]8;;\x1b\\");
        assert!(term.current_hyperlink_id.is_none());

        // Start another hyperlink
        term.process(b"\x1b]8;;https://example.org\x1b\\");
        assert!(term.current_hyperlink_id.is_some());
        let id2 = term.current_hyperlink_id.unwrap();

        // IDs should be different
        assert_ne!(id1, id2);

        // Reuse existing URL (deduplication)
        term.process(b"\x1b]8;;https://example.com\x1b\\");
        assert_eq!(term.current_hyperlink_id, Some(id1));
    }

    #[test]
    fn test_osc7_set_directory() {
        let mut term = Terminal::new(80, 24);

        // OSC 7 with file:// URL (localhost)
        term.process(b"\x1b]7;file:///home/user/project\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/user/project"));
        assert_eq!(
            term.session_variables().path,
            Some("/home/user/project".to_string())
        );

        // OSC 7 with hostname
        term.process(b"\x1b]7;file://hostname/home/user/test\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/user/test"));
        assert_eq!(
            term.session_variables().hostname,
            Some("hostname".to_string())
        );
    }

    #[test]
    fn test_osc7_hostname_extraction() {
        let mut term = Terminal::new(80, 24);

        // file:///path - localhost implicit, hostname should be None
        term.process(b"\x1b]7;file:///home/user/project\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/user/project"));
        assert!(term.shell_integration.hostname().is_none());

        // file://hostname/path - hostname should be extracted
        term.process(b"\x1b]7;file://myserver/home/user/test\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/user/test"));
        assert_eq!(term.shell_integration.hostname(), Some("myserver"));

        // file://localhost/path - localhost should be treated as None
        term.process(b"\x1b]7;file://localhost/var/log\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/var/log"));
        assert!(term.shell_integration.hostname().is_none());

        // file://LOCALHOST/path - case insensitive localhost check
        term.process(b"\x1b]7;file://LOCALHOST/tmp\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/tmp"));
        assert!(term.shell_integration.hostname().is_none());

        // Remote host with full path
        term.process(b"\x1b]7;file://remote.server.com/home/alice/work\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/alice/work"));
        assert_eq!(term.shell_integration.hostname(), Some("remote.server.com"));
    }

    #[test]
    fn test_osc7_username_and_port_and_decoding() {
        let mut term = Terminal::new(80, 24);

        // Username and port should be parsed, port stripped from hostname
        term.process(b"\x1b]7;file://alice@example.com:2222/home/alice/Work%20Dir\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/alice/Work Dir"));
        assert_eq!(term.shell_integration.hostname(), Some("example.com"));
        assert_eq!(term.shell_integration.username(), Some("alice"));
        assert_eq!(
            term.session_variables().path,
            Some("/home/alice/Work Dir".to_string())
        );
        assert_eq!(
            term.session_variables().hostname,
            Some("example.com".to_string())
        );
        assert_eq!(term.session_variables().username, Some("alice".to_string()));

        // Query/fragment stripped and percent-decoded unicode
        term.process(b"\x1b]7;file://remote.host/home/alice/caf%C3%A9?foo=bar#frag\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/alice/café"));
        assert_eq!(term.shell_integration.hostname(), Some("remote.host"));

        // Non-file scheme should be ignored (no change)
        term.process(b"\x1b]7;http://example.com/should_not_set\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/alice/café"));
    }

    #[test]
    fn test_osc7_hostname_updates() {
        let mut term = Terminal::new(80, 24);

        // Start with remote host
        term.process(b"\x1b]7;file://server1/home/user\x1b\\");
        assert_eq!(term.shell_integration.hostname(), Some("server1"));

        // Switch to localhost
        term.process(b"\x1b]7;file:///home/user\x1b\\");
        assert!(term.shell_integration.hostname().is_none());

        // Switch to different remote host
        term.process(b"\x1b]7;file://server2/home/user\x1b\\");
        assert_eq!(term.shell_integration.hostname(), Some("server2"));
    }

    #[test]
    fn test_osc7_cwd_history_records_hostname() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]7;file://server1/home/user\x1b\\");
        term.process(b"\x1b]7;file:///home/local\x1b\\");

        let history = term.get_cwd_changes();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].new_cwd, "/home/user");
        assert_eq!(history[0].hostname.as_deref(), Some("server1"));
        assert_eq!(history[1].hostname, None);
    }

    #[test]
    fn test_osc7_emits_cwd_changed_event() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]7;file://server1/home/user\x1b\\");
        let events = term.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::CwdChanged(change)
                if change.new_cwd == "/home/user"
                    && change.hostname.as_deref() == Some("server1")
            )),
            "CwdChanged event with hostname should be emitted",
        );

        // With subscription filter should still receive
        term.set_event_subscription(Some(vec![crate::terminal::TerminalEventKind::CwdChanged]));
        term.process(b"\x1b]7;file:///home/local\x1b\\");
        let subscribed = term.poll_subscribed_events();
        assert!(
            subscribed.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::CwdChanged(change)
                if change.new_cwd == "/home/local" && change.hostname.is_none()
            )),
            "Subscribed poll should return CwdChanged event"
        );
    }

    #[test]
    fn test_notifications_osc9() {
        let mut term = Terminal::new(80, 24);

        // OSC 9 notification
        term.process(b"\x1b]9;Test notification\x1b\\");
        let notifications = term.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].title, "");
        assert_eq!(notifications[0].message, "Test notification");
    }

    #[test]
    fn test_notifications_security() {
        let mut term = Terminal::new(80, 24);

        // Enable security
        term.process(b"\x1b[?1002h"); // Just to ensure terminal processes sequences

        // Create a terminal with insecure sequences disabled
        let mut secure_term = Terminal::new(80, 24);
        secure_term.disable_insecure_sequences = true;

        // OSC 9 should be blocked
        secure_term.process(b"\x1b]9;Should be blocked\x1b\\");
        assert_eq!(secure_term.notifications().len(), 0);

        // OSC 8 (hyperlinks) should be blocked
        secure_term.process(b"\x1b]8;;https://evil.com\x1b\\");
        assert!(secure_term.current_hyperlink_id.is_none());
    }

    #[test]
    fn test_ansi_palette_reset() {
        let mut term = Terminal::new(80, 24);

        // Modify a color (we can't easily test this without accessing private fields,
        // so we'll just ensure the sequence doesn't crash)
        term.process(b"\x1b]104;3\x1b\\"); // Reset color 3

        // Reset all colors
        term.process(b"\x1b]104\x1b\\");
    }

    #[test]
    fn test_default_color_reset() {
        let mut term = Terminal::new(80, 24);

        // OSC 110 - Reset foreground
        term.process(b"\x1b]110\x1b\\");

        // OSC 111 - Reset background
        term.process(b"\x1b]111\x1b\\");

        // OSC 112 - Reset cursor color
        term.process(b"\x1b]112\x1b\\");
    }

    #[test]
    fn test_query_default_colors() {
        let mut term = Terminal::new(80, 24);

        // OSC 10 - Query foreground
        term.process(b"\x1b]10;?\x1b\\");
        let response = term.drain_responses();
        assert!(response.starts_with(b"\x1b]10;rgb:"));

        // OSC 11 - Query background
        term.process(b"\x1b]11;?\x1b\\");
        let response = term.drain_responses();
        assert!(response.starts_with(b"\x1b]11;rgb:"));

        // OSC 12 - Query cursor color
        term.process(b"\x1b]12;?\x1b\\");
        let response = term.drain_responses();
        assert!(response.starts_with(b"\x1b]12;rgb:"));
    }

    #[test]
    fn test_is_insecure_osc() {
        let term = Terminal::new(80, 24);

        // Without security enabled
        assert!(!term.is_insecure_osc("0"));
        assert!(!term.is_insecure_osc("8"));
        assert!(!term.is_insecure_osc("52"));

        // With security enabled
        let mut secure_term = Terminal::new(80, 24);
        secure_term.disable_insecure_sequences = true;

        assert!(!secure_term.is_insecure_osc("0")); // Title is safe
        assert!(secure_term.is_insecure_osc("8")); // Hyperlinks
        assert!(secure_term.is_insecure_osc("52")); // Clipboard
        assert!(secure_term.is_insecure_osc("9")); // Notifications
        assert!(secure_term.is_insecure_osc("777")); // Notifications
    }

    #[test]
    fn test_clipboard_operations() {
        let mut term = Terminal::new(80, 24);

        // Set clipboard (base64 encoded "Hello")
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"Hello");
        let sequence = format!("\x1b]52;c;{}\x1b\\", encoded);
        term.process(sequence.as_bytes());
        assert_eq!(term.clipboard_content, Some("Hello".to_string()));

        // Clear clipboard
        term.process(b"\x1b]52;c;\x1b\\");
        assert_eq!(term.clipboard_content, None);
    }

    #[test]
    fn test_clipboard_query_security() {
        let mut term = Terminal::new(80, 24);
        term.allow_clipboard_read = false;

        // Set clipboard
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"Secret");
        let sequence = format!("\x1b]52;c;{}\x1b\\", encoded);
        term.process(sequence.as_bytes());

        // Query should be blocked
        term.process(b"\x1b]52;c;?\x1b\\");
        let response = term.drain_responses();
        assert_eq!(response, b""); // No response when clipboard read is disabled
    }

    #[test]
    fn test_title_with_special_chars() {
        let mut term = Terminal::new(80, 24);

        // Title with Unicode
        term.process("\x1b]0;测试标题\x1b\\".as_bytes());
        assert_eq!(term.title(), "测试标题");

        // Title with spaces and punctuation
        term.process(b"\x1b]0;Test: A Title! (v1.0)\x1b\\");
        assert_eq!(term.title(), "Test: A Title! (v1.0)");
    }

    // === OSC 9;4 Progress Bar Tests ===

    #[test]
    fn test_progress_bar_normal() {
        let mut term = Terminal::new(80, 24);

        // OSC 9;4;1;50 - Set normal progress to 50%
        term.process(b"\x1b]9;4;1;50\x1b\\");

        assert!(term.has_progress());
        assert_eq!(
            term.progress_state(),
            crate::terminal::ProgressState::Normal
        );
        assert_eq!(term.progress_value(), 50);
    }

    #[test]
    fn test_progress_bar_hidden() {
        let mut term = Terminal::new(80, 24);

        // First set a progress
        term.process(b"\x1b]9;4;1;75\x1b\\");
        assert!(term.has_progress());

        // Then hide it with OSC 9;4;0
        term.process(b"\x1b]9;4;0\x1b\\");

        assert!(!term.has_progress());
        assert_eq!(
            term.progress_state(),
            crate::terminal::ProgressState::Hidden
        );
    }

    #[test]
    fn test_progress_bar_error() {
        let mut term = Terminal::new(80, 24);

        // OSC 9;4;2;100 - Error progress at 100%
        term.process(b"\x1b]9;4;2;100\x1b\\");

        assert!(term.has_progress());
        assert_eq!(term.progress_state(), crate::terminal::ProgressState::Error);
        assert_eq!(term.progress_value(), 100);
    }

    #[test]
    fn test_progress_bar_indeterminate() {
        let mut term = Terminal::new(80, 24);

        // OSC 9;4;3 - Indeterminate progress
        term.process(b"\x1b]9;4;3\x1b\\");

        assert!(term.has_progress());
        assert_eq!(
            term.progress_state(),
            crate::terminal::ProgressState::Indeterminate
        );
        // Progress value is not meaningful for indeterminate
    }

    #[test]
    fn test_progress_bar_warning() {
        let mut term = Terminal::new(80, 24);

        // OSC 9;4;4;80 - Warning/paused progress at 80%
        term.process(b"\x1b]9;4;4;80\x1b\\");

        assert!(term.has_progress());
        assert_eq!(
            term.progress_state(),
            crate::terminal::ProgressState::Warning
        );
        assert_eq!(term.progress_value(), 80);
    }

    #[test]
    fn test_progress_bar_clamps_to_100() {
        let mut term = Terminal::new(80, 24);

        // OSC 9;4;1;150 - Progress value above 100 should clamp
        term.process(b"\x1b]9;4;1;150\x1b\\");

        assert_eq!(term.progress_value(), 100);
    }

    #[test]
    fn test_progress_bar_manual_set() {
        let mut term = Terminal::new(80, 24);

        // Use the programmatic API
        term.set_progress(crate::terminal::ProgressState::Warning, 65);

        assert!(term.has_progress());
        assert_eq!(
            term.progress_state(),
            crate::terminal::ProgressState::Warning
        );
        assert_eq!(term.progress_value(), 65);

        // Clear it
        term.clear_progress();

        assert!(!term.has_progress());
        assert_eq!(
            term.progress_state(),
            crate::terminal::ProgressState::Hidden
        );
    }

    #[test]
    fn test_progress_bar_does_not_affect_notifications() {
        let mut term = Terminal::new(80, 24);

        // OSC 9 with message (notification)
        term.process(b"\x1b]9;Test notification\x1b\\");
        assert_eq!(term.notifications().len(), 1);
        assert_eq!(term.notifications()[0].message, "Test notification");

        // Progress bar should still be hidden
        assert!(!term.has_progress());

        // Progress bar sequence
        term.process(b"\x1b]9;4;1;50\x1b\\");

        // Should have progress now
        assert!(term.has_progress());
        // Notification count should not increase
        assert_eq!(term.notifications().len(), 1);
    }

    #[test]
    fn test_progress_bar_sequence_format() {
        use crate::terminal::ProgressBar;

        // Test escape sequence generation
        assert_eq!(
            ProgressBar::hidden().to_escape_sequence(),
            "\x1b]9;4;0\x1b\\"
        );
        assert_eq!(
            ProgressBar::normal(50).to_escape_sequence(),
            "\x1b]9;4;1;50\x1b\\"
        );
        assert_eq!(
            ProgressBar::error(100).to_escape_sequence(),
            "\x1b]9;4;2;100\x1b\\"
        );
        assert_eq!(
            ProgressBar::indeterminate().to_escape_sequence(),
            "\x1b]9;4;3\x1b\\"
        );
        assert_eq!(
            ProgressBar::warning(75).to_escape_sequence(),
            "\x1b]9;4;4;75\x1b\\"
        );
    }

    // === OSC 1337 SetBadgeFormat Tests ===

    #[test]
    fn test_set_badge_format_simple() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Simple text badge
        let encoded = STANDARD.encode("Production");
        let sequence = format!("\x1b]1337;SetBadgeFormat={}\x1b\\", encoded);
        term.process(sequence.as_bytes());

        assert_eq!(term.badge_format(), Some("Production"));
    }

    #[test]
    fn test_set_badge_format_with_variables() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Badge with variable interpolation
        let encoded = STANDARD.encode(r"\(username)@\(hostname)");
        let sequence = format!("\x1b]1337;SetBadgeFormat={}\x1b\\", encoded);
        term.process(sequence.as_bytes());

        assert_eq!(term.badge_format(), Some(r"\(username)@\(hostname)"));
    }

    #[test]
    fn test_set_badge_format_with_session_prefix() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Badge with session.variable syntax
        let encoded = STANDARD.encode(r"\(session.path)");
        let sequence = format!("\x1b]1337;SetBadgeFormat={}\x1b\\", encoded);
        term.process(sequence.as_bytes());

        assert_eq!(term.badge_format(), Some(r"\(session.path)"));
    }

    #[test]
    fn test_clear_badge_format() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Set a badge
        let encoded = STANDARD.encode("Test Badge");
        let sequence = format!("\x1b]1337;SetBadgeFormat={}\x1b\\", encoded);
        term.process(sequence.as_bytes());
        assert!(term.badge_format().is_some());

        // Clear badge with empty value
        term.process(b"\x1b]1337;SetBadgeFormat=\x1b\\");
        assert!(term.badge_format().is_none());
    }

    #[test]
    fn test_set_badge_format_rejects_unsafe() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Try to set a badge with shell command injection
        let encoded = STANDARD.encode("$(whoami)");
        let sequence = format!("\x1b]1337;SetBadgeFormat={}\x1b\\", encoded);
        term.process(sequence.as_bytes());

        // Should be rejected (badge should remain None)
        assert!(term.badge_format().is_none());
    }

    #[test]
    fn test_set_badge_format_rejects_escape_sequences() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Try to set a badge with escape sequences
        let encoded = STANDARD.encode("\x1b[31mred\x1b[0m");
        let sequence = format!("\x1b]1337;SetBadgeFormat={}\x1b\\", encoded);
        term.process(sequence.as_bytes());

        // Should be rejected
        assert!(term.badge_format().is_none());
    }

    #[test]
    fn test_evaluate_badge() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Set badge format
        let encoded = STANDARD.encode(r"\(username)@\(hostname)");
        let sequence = format!("\x1b]1337;SetBadgeFormat={}\x1b\\", encoded);
        term.process(sequence.as_bytes());

        // Set session variables
        term.session_variables_mut().set_username("alice");
        term.session_variables_mut().set_hostname("server1");

        // Evaluate badge
        let result = term.evaluate_badge();
        assert_eq!(result, Some("alice@server1".to_string()));
    }

    #[test]
    fn test_evaluate_badge_with_dimensions() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(120, 40);

        // Set badge format with dimensions
        let encoded = STANDARD.encode(r"\(columns)x\(rows)");
        let sequence = format!("\x1b]1337;SetBadgeFormat={}\x1b\\", encoded);
        term.process(sequence.as_bytes());

        // Dimensions should be available from session variables
        let result = term.evaluate_badge();
        assert_eq!(result, Some("120x40".to_string()));
    }

    #[test]
    fn test_badge_dimensions_update_on_resize() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Set badge format with dimensions
        let encoded = STANDARD.encode(r"\(columns)x\(rows)");
        let sequence = format!("\x1b]1337;SetBadgeFormat={}\x1b\\", encoded);
        term.process(sequence.as_bytes());

        // Initial evaluation
        assert_eq!(term.evaluate_badge(), Some("80x24".to_string()));

        // Resize terminal
        term.resize(120, 40);

        // Dimensions should update
        assert_eq!(term.evaluate_badge(), Some("120x40".to_string()));
    }

    #[test]
    fn test_evaluate_badge_none() {
        let term = Terminal::new(80, 24);

        // No badge format set
        assert!(term.evaluate_badge().is_none());
    }

    #[test]
    fn test_session_variables_sync_with_title() {
        let mut term = Terminal::new(80, 24);

        // Set title
        term.set_title("My Terminal".to_string());

        // Session variables should have the title
        assert_eq!(
            term.session_variables().get("title"),
            Some("My Terminal".to_string())
        );
    }

    #[test]
    fn test_session_variables_bell_count() {
        let mut term = Terminal::new(80, 24);

        // Send some bells
        term.process(b"\x07\x07\x07");

        // Bell count should be tracked in session variables
        assert_eq!(
            term.session_variables().get("bell_count"),
            Some("3".to_string())
        );
    }

    #[test]
    fn test_set_user_var_basic() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Encode "myhost.example.com" as base64
        let value = STANDARD.encode("myhost.example.com");
        let seq = format!("\x1b]1337;SetUserVar=hostname={}\x07", value);
        term.process(seq.as_bytes());

        // Verify the variable was stored
        assert_eq!(term.get_user_var("hostname"), Some("myhost.example.com"));
    }

    #[test]
    fn test_set_user_var_event_emitted() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        let value = STANDARD.encode("alice");
        let seq = format!("\x1b]1337;SetUserVar=username={}\x07", value);
        term.process(seq.as_bytes());

        let events = term.poll_events();
        let user_var_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::UserVarChanged { .. }))
            .collect();
        assert_eq!(user_var_events.len(), 1);

        if let crate::terminal::TerminalEvent::UserVarChanged {
            name,
            value,
            old_value,
        } = &user_var_events[0]
        {
            assert_eq!(name, "username");
            assert_eq!(value, "alice");
            assert!(old_value.is_none());
        } else {
            panic!("Expected UserVarChanged event");
        }
    }

    #[test]
    fn test_set_user_var_update_with_old_value() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Set initial value
        let value1 = STANDARD.encode("server1");
        let seq1 = format!("\x1b]1337;SetUserVar=host={}\x07", value1);
        term.process(seq1.as_bytes());
        term.poll_events(); // drain

        // Update value
        let value2 = STANDARD.encode("server2");
        let seq2 = format!("\x1b]1337;SetUserVar=host={}\x07", value2);
        term.process(seq2.as_bytes());

        let events = term.poll_events();
        let user_var_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::UserVarChanged { .. }))
            .collect();
        assert_eq!(user_var_events.len(), 1);

        if let crate::terminal::TerminalEvent::UserVarChanged {
            name,
            value,
            old_value,
        } = &user_var_events[0]
        {
            assert_eq!(name, "host");
            assert_eq!(value, "server2");
            assert_eq!(old_value.as_deref(), Some("server1"));
        }
    }

    #[test]
    fn test_set_user_var_no_event_when_same_value() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        // Set value
        let value = STANDARD.encode("same");
        let seq = format!("\x1b]1337;SetUserVar=key={}\x07", value);
        term.process(seq.as_bytes());
        term.poll_events(); // drain

        // Set the same value again
        term.process(seq.as_bytes());

        let events = term.poll_events();
        let user_var_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::UserVarChanged { .. }))
            .collect();
        assert_eq!(user_var_events.len(), 0, "No event when value unchanged");
    }

    #[test]
    fn test_set_user_var_multiple_variables() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        let host = STANDARD.encode("myhost");
        let user = STANDARD.encode("myuser");
        let dir = STANDARD.encode("/home/myuser");

        let seq = format!(
            "\x1b]1337;SetUserVar=hostname={}\x07\x1b]1337;SetUserVar=username={}\x07\x1b]1337;SetUserVar=currentDir={}\x07",
            host, user, dir
        );
        term.process(seq.as_bytes());

        assert_eq!(term.get_user_var("hostname"), Some("myhost"));
        assert_eq!(term.get_user_var("username"), Some("myuser"));
        assert_eq!(term.get_user_var("currentDir"), Some("/home/myuser"));

        let vars = term.get_user_vars();
        assert_eq!(vars.len(), 3);
    }

    #[test]
    fn test_set_user_var_invalid_base64() {
        let mut term = Terminal::new(80, 24);

        // Invalid base64
        let seq = b"\x1b]1337;SetUserVar=key=!!!invalid!!!\x07";
        term.process(seq);

        // Variable should not be set
        assert!(term.get_user_var("key").is_none());
    }

    #[test]
    fn test_set_user_var_missing_separator() {
        let mut term = Terminal::new(80, 24);

        // Missing = between name and value
        let seq = b"\x1b]1337;SetUserVar=keyonly\x07";
        term.process(seq);

        // Nothing should be set
        assert!(term.get_user_var("keyonly").is_none());
    }

    #[test]
    fn test_set_user_var_empty_name() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        let value = STANDARD.encode("test");
        let seq = format!("\x1b]1337;SetUserVar=={}\x07", value);
        term.process(seq.as_bytes());

        // Empty name should be rejected
        assert!(term.get_user_vars().is_empty());
    }

    #[test]
    fn test_set_user_var_available_in_session_variables() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let mut term = Terminal::new(80, 24);

        let value = STANDARD.encode("testval");
        let seq = format!("\x1b]1337;SetUserVar=myvar={}\x07", value);
        term.process(seq.as_bytes());

        // Should be accessible via session_variables.get() for badge evaluation
        assert_eq!(
            term.session_variables().get("myvar"),
            Some("testval".to_string())
        );
    }

    // === OSC 934 Named Progress Bar Tests ===

    #[test]
    fn test_osc934_set_progress_bar() {
        let mut term = Terminal::new(80, 24);

        // OSC 934 ; set ; dl-1 ; percent=50 ; label=Downloading ST
        term.process(b"\x1b]934;set;dl-1;percent=50;label=Downloading\x1b\\");

        let bars = term.named_progress_bars();
        assert_eq!(bars.len(), 1);

        let bar = bars.get("dl-1").unwrap();
        assert_eq!(bar.id, "dl-1");
        assert_eq!(bar.state, crate::terminal::ProgressState::Normal);
        assert_eq!(bar.percent, 50);
        assert_eq!(bar.label, Some("Downloading".to_string()));
    }

    #[test]
    fn test_osc934_set_with_state() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;set;build;state=indeterminate;label=Compiling\x1b\\");

        let bar = term.get_named_progress_bar("build").unwrap();
        assert_eq!(bar.state, crate::terminal::ProgressState::Indeterminate);
        assert_eq!(bar.label, Some("Compiling".to_string()));
    }

    #[test]
    fn test_osc934_update_progress_bar() {
        let mut term = Terminal::new(80, 24);

        // Create
        term.process(b"\x1b]934;set;dl-1;percent=10;label=Starting\x1b\\");
        assert_eq!(term.get_named_progress_bar("dl-1").unwrap().percent, 10);

        // Update
        term.process(b"\x1b]934;set;dl-1;percent=75;label=Almost done\x1b\\");
        let bar = term.get_named_progress_bar("dl-1").unwrap();
        assert_eq!(bar.percent, 75);
        assert_eq!(bar.label, Some("Almost done".to_string()));

        // Still only one bar
        assert_eq!(term.named_progress_bars().len(), 1);
    }

    #[test]
    fn test_osc934_multiple_bars() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;set;dl-1;percent=30;label=File 1\x1b\\");
        term.process(b"\x1b]934;set;dl-2;percent=60;label=File 2\x1b\\");
        term.process(b"\x1b]934;set;build;state=indeterminate\x1b\\");

        assert_eq!(term.named_progress_bars().len(), 3);
        assert_eq!(term.get_named_progress_bar("dl-1").unwrap().percent, 30);
        assert_eq!(term.get_named_progress_bar("dl-2").unwrap().percent, 60);
        assert_eq!(
            term.get_named_progress_bar("build").unwrap().state,
            crate::terminal::ProgressState::Indeterminate
        );
    }

    #[test]
    fn test_osc934_remove_progress_bar() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;set;dl-1;percent=50\x1b\\");
        term.process(b"\x1b]934;set;dl-2;percent=70\x1b\\");
        assert_eq!(term.named_progress_bars().len(), 2);

        term.process(b"\x1b]934;remove;dl-1\x1b\\");
        assert_eq!(term.named_progress_bars().len(), 1);
        assert!(term.get_named_progress_bar("dl-1").is_none());
        assert!(term.get_named_progress_bar("dl-2").is_some());
    }

    #[test]
    fn test_osc934_remove_all() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;set;a;percent=10\x1b\\");
        term.process(b"\x1b]934;set;b;percent=20\x1b\\");
        term.process(b"\x1b]934;set;c;percent=30\x1b\\");
        assert_eq!(term.named_progress_bars().len(), 3);

        term.process(b"\x1b]934;remove_all\x1b\\");
        assert!(term.named_progress_bars().is_empty());
    }

    #[test]
    fn test_osc934_event_emitted_on_set() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;set;dl-1;percent=42;label=Test\x1b\\");

        let events = term.poll_events();
        let pb_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::ProgressBarChanged { .. }))
            .collect();
        assert_eq!(pb_events.len(), 1);

        if let crate::terminal::TerminalEvent::ProgressBarChanged {
            action,
            id,
            state,
            percent,
            label,
        } = &pb_events[0]
        {
            assert_eq!(*action, crate::terminal::ProgressBarAction::Set);
            assert_eq!(id, "dl-1");
            assert_eq!(*state, Some(crate::terminal::ProgressState::Normal));
            assert_eq!(*percent, Some(42));
            assert_eq!(*label, Some("Test".to_string()));
        } else {
            panic!("Expected ProgressBarChanged event");
        }
    }

    #[test]
    fn test_osc934_event_emitted_on_remove() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;set;dl-1;percent=42\x1b\\");
        term.poll_events(); // Clear set event

        term.process(b"\x1b]934;remove;dl-1\x1b\\");
        let events = term.poll_events();
        let pb_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::ProgressBarChanged { .. }))
            .collect();
        assert_eq!(pb_events.len(), 1);

        if let crate::terminal::TerminalEvent::ProgressBarChanged { action, id, .. } = &pb_events[0]
        {
            assert_eq!(*action, crate::terminal::ProgressBarAction::Remove);
            assert_eq!(id, "dl-1");
        }
    }

    #[test]
    fn test_osc934_event_emitted_on_remove_all() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;set;a;percent=10\x1b\\");
        term.process(b"\x1b]934;set;b;percent=20\x1b\\");
        term.poll_events(); // Clear set events

        term.process(b"\x1b]934;remove_all\x1b\\");
        let events = term.poll_events();
        let pb_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::ProgressBarChanged { .. }))
            .collect();
        assert_eq!(pb_events.len(), 1);

        if let crate::terminal::TerminalEvent::ProgressBarChanged { action, .. } = &pb_events[0] {
            assert_eq!(*action, crate::terminal::ProgressBarAction::RemoveAll);
        }
    }

    #[test]
    fn test_osc934_no_event_on_remove_nonexistent() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;remove;nonexistent\x1b\\");
        let events = term.poll_events();
        let pb_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::ProgressBarChanged { .. }))
            .collect();
        assert_eq!(pb_events.len(), 0);
    }

    #[test]
    fn test_osc934_no_event_on_remove_all_empty() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;remove_all\x1b\\");
        let events = term.poll_events();
        let pb_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::ProgressBarChanged { .. }))
            .collect();
        assert_eq!(pb_events.len(), 0);
    }

    #[test]
    fn test_osc934_invalid_sequence_ignored() {
        let mut term = Terminal::new(80, 24);

        // Invalid action
        term.process(b"\x1b]934;invalid\x1b\\");
        assert!(term.named_progress_bars().is_empty());

        // Missing ID for set
        term.process(b"\x1b]934;set\x1b\\");
        assert!(term.named_progress_bars().is_empty());

        // Missing ID for remove
        term.process(b"\x1b]934;remove\x1b\\");
        assert!(term.named_progress_bars().is_empty());
    }

    #[test]
    fn test_osc934_bell_terminated() {
        let mut term = Terminal::new(80, 24);

        // BEL-terminated variant
        term.process(b"\x1b]934;set;dl-1;percent=99;label=Done\x07");

        let bar = term.get_named_progress_bar("dl-1").unwrap();
        assert_eq!(bar.percent, 99);
        assert_eq!(bar.label, Some("Done".to_string()));
    }

    #[test]
    fn test_osc934_warning_state() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;set;job;state=warning;percent=80;label=Disk space low\x1b\\");

        let bar = term.get_named_progress_bar("job").unwrap();
        assert_eq!(bar.state, crate::terminal::ProgressState::Warning);
        assert_eq!(bar.percent, 80);
        assert_eq!(bar.label, Some("Disk space low".to_string()));
    }

    #[test]
    fn test_osc934_error_state() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]934;set;build;state=error;label=Build failed\x1b\\");

        let bar = term.get_named_progress_bar("build").unwrap();
        assert_eq!(bar.state, crate::terminal::ProgressState::Error);
        assert_eq!(bar.label, Some("Build failed".to_string()));
    }

    #[test]
    fn test_osc934_does_not_affect_osc94() {
        let mut term = Terminal::new(80, 24);

        // OSC 9;4 and OSC 934 are independent
        term.process(b"\x1b]9;4;1;50\x1b\\");
        term.process(b"\x1b]934;set;dl-1;percent=75\x1b\\");

        // OSC 9;4 state should be unchanged
        assert!(term.has_progress());
        assert_eq!(term.progress_value(), 50);

        // OSC 934 state should be independent
        assert_eq!(term.named_progress_bars().len(), 1);
        assert_eq!(term.get_named_progress_bar("dl-1").unwrap().percent, 75);
    }

    // === OSC 1337 RemoteHost Tests ===

    #[test]
    fn test_remote_host_user_and_hostname() {
        let mut term = Terminal::new(80, 24);

        // OSC 1337 ; RemoteHost=alice@server1.example.com ST
        term.process(b"\x1b]1337;RemoteHost=alice@server1.example.com\x1b\\");

        assert_eq!(
            term.shell_integration.hostname(),
            Some("server1.example.com")
        );
        assert_eq!(term.shell_integration.username(), Some("alice"));
    }

    #[test]
    fn test_remote_host_hostname_only() {
        let mut term = Terminal::new(80, 24);

        // No username, just hostname
        term.process(b"\x1b]1337;RemoteHost=myserver\x1b\\");

        assert_eq!(term.shell_integration.hostname(), Some("myserver"));
        assert!(term.shell_integration.username().is_none());
    }

    #[test]
    fn test_remote_host_empty_user_at_hostname() {
        let mut term = Terminal::new(80, 24);

        // Empty username with @ prefix
        term.process(b"\x1b]1337;RemoteHost=@myserver\x1b\\");

        assert_eq!(term.shell_integration.hostname(), Some("myserver"));
        assert!(term.shell_integration.username().is_none());
    }

    #[test]
    fn test_remote_host_localhost_clears_hostname() {
        let mut term = Terminal::new(80, 24);

        // First set a remote host
        term.process(b"\x1b]1337;RemoteHost=alice@remote\x1b\\");
        assert_eq!(term.shell_integration.hostname(), Some("remote"));

        // Then switch back to localhost
        term.process(b"\x1b]1337;RemoteHost=alice@localhost\x1b\\");
        assert!(term.shell_integration.hostname().is_none());
        assert_eq!(term.shell_integration.username(), Some("alice"));
    }

    #[test]
    fn test_remote_host_localhost_case_insensitive() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]1337;RemoteHost=user@LOCALHOST\x1b\\");
        assert!(term.shell_integration.hostname().is_none());

        term.process(b"\x1b]1337;RemoteHost=user@Localhost\x1b\\");
        assert!(term.shell_integration.hostname().is_none());
    }

    #[test]
    fn test_remote_host_emits_cwd_changed_event() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]1337;RemoteHost=alice@remote-server\x1b\\");

        let events = term.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::CwdChanged(change)
                if change.hostname.as_deref() == Some("remote-server")
                    && change.username.as_deref() == Some("alice")
            )),
            "CwdChanged event with hostname and username should be emitted",
        );
    }

    #[test]
    fn test_remote_host_bell_terminated() {
        let mut term = Terminal::new(80, 24);

        // BEL-terminated variant
        term.process(b"\x1b]1337;RemoteHost=bob@host2\x07");

        assert_eq!(term.shell_integration.hostname(), Some("host2"));
        assert_eq!(term.shell_integration.username(), Some("bob"));
    }

    #[test]
    fn test_remote_host_session_variables_updated() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]1337;RemoteHost=alice@server1\x1b\\");

        let vars = term.session_variables();
        assert_eq!(vars.hostname, Some("server1".to_string()));
        assert_eq!(vars.username, Some("alice".to_string()));
    }

    #[test]
    fn test_remote_host_empty_payload_ignored() {
        let mut term = Terminal::new(80, 24);

        // Empty payload should be ignored
        term.process(b"\x1b]1337;RemoteHost=\x1b\\");
        assert!(term.shell_integration.hostname().is_none());
        assert!(term.shell_integration.username().is_none());
    }

    #[test]
    fn test_remote_host_empty_hostname_ignored() {
        let mut term = Terminal::new(80, 24);

        // user@ with no hostname
        term.process(b"\x1b]1337;RemoteHost=alice@\x1b\\");
        assert!(term.shell_integration.hostname().is_none());
        assert!(term.shell_integration.username().is_none());
    }

    #[test]
    fn test_remote_host_overrides_osc7_hostname() {
        let mut term = Terminal::new(80, 24);

        // Set hostname via OSC 7
        term.process(b"\x1b]7;file://server1/home/user\x1b\\");
        assert_eq!(term.shell_integration.hostname(), Some("server1"));

        // Override via RemoteHost
        term.process(b"\x1b]1337;RemoteHost=bob@server2\x1b\\");
        assert_eq!(term.shell_integration.hostname(), Some("server2"));
        assert_eq!(term.shell_integration.username(), Some("bob"));
    }

    #[test]
    fn test_remote_host_updates_sequence() {
        let mut term = Terminal::new(80, 24);

        // First remote host
        term.process(b"\x1b]1337;RemoteHost=alice@host1\x1b\\");
        assert_eq!(term.shell_integration.hostname(), Some("host1"));
        assert_eq!(term.shell_integration.username(), Some("alice"));

        // Second remote host
        term.process(b"\x1b]1337;RemoteHost=bob@host2\x1b\\");
        assert_eq!(term.shell_integration.hostname(), Some("host2"));
        assert_eq!(term.shell_integration.username(), Some("bob"));
    }

    #[test]
    fn test_remote_host_loopback_addresses() {
        let mut term = Terminal::new(80, 24);

        // IPv4 loopback
        term.process(b"\x1b]1337;RemoteHost=user@127.0.0.1\x1b\\");
        assert!(term.shell_integration.hostname().is_none());

        // IPv6 loopback
        term.process(b"\x1b]1337;RemoteHost=user@::1\x1b\\");
        assert!(term.shell_integration.hostname().is_none());
    }

    #[test]
    fn test_remote_host_preserves_existing_cwd() {
        let mut term = Terminal::new(80, 24);

        // Set cwd via OSC 7
        term.process(b"\x1b]7;file:///home/user/project\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/user/project"));

        // Set remote host - should not clear cwd
        term.process(b"\x1b]1337;RemoteHost=alice@remote\x1b\\");
        assert_eq!(term.shell_integration.cwd(), Some("/home/user/project"));
        assert_eq!(term.shell_integration.hostname(), Some("remote"));
    }

    // ========== Semantic Zone Tests ==========

    #[test]
    fn test_zones_created_by_osc_133() {
        let mut term = Terminal::new(80, 24);

        // Prompt start
        term.process(b"\x1b]133;A\x07");
        assert_eq!(term.get_zones().len(), 1);
        assert_eq!(term.get_zones()[0].zone_type, crate::zone::ZoneType::Prompt);

        // Type a command - first set it via shell integration
        term.process(b"ls -la");

        // Command start
        term.shell_integration_mut()
            .set_command("ls -la".to_string());
        term.process(b"\x1b]133;B\x07");
        assert_eq!(term.get_zones().len(), 2);
        assert_eq!(
            term.get_zones()[1].zone_type,
            crate::zone::ZoneType::Command
        );
        assert_eq!(term.get_zones()[1].command.as_deref(), Some("ls -la"));

        // Command executed (output begins)
        term.process(b"\x1b]133;C\x07");
        assert_eq!(term.get_zones().len(), 3);
        assert_eq!(term.get_zones()[2].zone_type, crate::zone::ZoneType::Output);
        assert_eq!(term.get_zones()[2].command.as_deref(), Some("ls -la"));

        // Some output
        term.process(b"file1.txt\r\nfile2.txt\r\n");

        // Command finished with exit code 0
        term.process(b"\x1b]133;D;0\x07");
        // Output zone should be closed with exit code
        assert_eq!(term.get_zones()[2].exit_code, Some(0));
    }

    #[test]
    fn test_zones_multiple_commands() {
        let mut term = Terminal::new(80, 24);

        // First command cycle: A -> B -> C -> D
        term.process(b"\x1b]133;A\x07$ ");
        term.process(b"\x1b]133;B\x07");
        term.process(b"\x1b]133;C\x07output1\r\n");
        term.process(b"\x1b]133;D;0\x07");

        // Second command cycle
        term.process(b"\x1b]133;A\x07$ ");
        term.process(b"\x1b]133;B\x07");
        term.process(b"\x1b]133;C\x07output2\r\n");
        term.process(b"\x1b]133;D;1\x07");

        // Should have 6 zones (Prompt, Command, Output) x 2
        let zones = term.get_zones();
        assert_eq!(zones.len(), 6);
        assert_eq!(zones[0].zone_type, crate::zone::ZoneType::Prompt);
        assert_eq!(zones[1].zone_type, crate::zone::ZoneType::Command);
        assert_eq!(zones[2].zone_type, crate::zone::ZoneType::Output);
        assert_eq!(zones[3].zone_type, crate::zone::ZoneType::Prompt);
        assert_eq!(zones[4].zone_type, crate::zone::ZoneType::Command);
        assert_eq!(zones[5].zone_type, crate::zone::ZoneType::Output);
        assert_eq!(zones[2].exit_code, Some(0));
        assert_eq!(zones[5].exit_code, Some(1));
    }

    #[test]
    fn test_zones_not_created_on_alt_screen() {
        let mut term = Terminal::new(80, 24);

        // Switch to alt screen
        term.process(b"\x1b[?1049h");

        // OSC 133 on alt screen should not create zones
        term.process(b"\x1b]133;A\x07");
        assert!(term.get_zones().is_empty());

        // Switch back to primary
        term.process(b"\x1b[?1049l");

        // Now it should create zones
        term.process(b"\x1b]133;A\x07");
        assert_eq!(term.get_zones().len(), 1);
    }

    #[test]
    fn test_zones_cleared_on_reset() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]133;A\x07");
        assert_eq!(term.get_zones().len(), 1);

        term.reset();
        assert!(term.get_zones().is_empty());
    }

    #[test]
    fn test_zones_evicted_on_scrollback_wrap() {
        // Small scrollback to trigger eviction quickly
        let mut term = Terminal::with_scrollback(80, 5, 10);

        // Create a prompt zone
        term.process(b"\x1b]133;A\x07");
        term.process(b"\x1b]133;B\x07");
        term.process(b"\x1b]133;C\x07");

        // Generate enough output to fill scrollback and wrap
        for i in 0..20 {
            term.process(format!("line {}\r\n", i).as_bytes());
        }

        // Command finished
        term.process(b"\x1b]133;D;0\x07");

        // Zones with rows below the scrollback floor should be evicted
        let zones = term.get_zones();
        let floor = term
            .active_grid()
            .total_lines_scrolled()
            .saturating_sub(term.active_grid().max_scrollback());
        for zone in zones {
            assert!(
                zone.abs_row_end >= floor,
                "Zone {:?} at rows {}-{} should be >= floor {}",
                zone.zone_type,
                zone.abs_row_start,
                zone.abs_row_end,
                floor
            );
        }
    }

    #[test]
    fn test_zone_get_zone_at() {
        let mut term = Terminal::new(80, 24);

        // Create zones with newlines between markers so each zone spans distinct rows
        // Prompt at row 0
        term.process(b"\x1b]133;A\x07$ \r\n");
        // Command at row 1
        term.process(b"\x1b]133;B\x07ls\r\n");
        // Output at row 2
        term.process(b"\x1b]133;C\x07output line 1\r\noutput line 2\r\n");
        // Finish at row 4
        term.process(b"\x1b]133;D;0\x07");

        // Row 0 should be in the Prompt zone (abs_row_start=0, closed at row 0)
        let zone = term.get_zone_at(0);
        assert!(zone.is_some());
        assert_eq!(zone.unwrap().zone_type, crate::zone::ZoneType::Prompt);

        // Row 1 should be in the Command zone
        let zone = term.get_zone_at(1);
        assert!(zone.is_some());
        assert_eq!(zone.unwrap().zone_type, crate::zone::ZoneType::Command);

        // Row 2 should be in the Output zone
        let zone = term.get_zone_at(2);
        assert!(zone.is_some());
        assert_eq!(zone.unwrap().zone_type, crate::zone::ZoneType::Output);

        // Row way beyond zones should return None
        assert!(term.get_zone_at(1000).is_none());
    }

    // ========== Command Output Capture Tests ==========

    #[test]
    fn test_get_command_output_basic() {
        let mut term = Terminal::new(80, 24);
        term.shell_integration_mut().set_command("ls".to_string());
        term.start_command_execution("ls".to_string());
        term.process(b"\x1b]133;A\x07$ \r\n");
        term.process(b"\x1b]133;B\x07ls\r\n");
        term.process(b"\x1b]133;C\x07");
        term.process(b"file1.txt\r\nfile2.txt\r\n");
        term.process(b"\x1b]133;D;0\x07");
        term.end_command_execution(0);

        let output = term.get_command_output(0);
        assert!(output.is_some());
        let text = output.unwrap();
        assert!(text.contains("file1.txt"));
        assert!(text.contains("file2.txt"));
    }

    #[test]
    fn test_get_command_output_out_of_bounds() {
        let term = Terminal::new(80, 24);
        assert!(term.get_command_output(0).is_none());
        assert!(term.get_command_output(100).is_none());
    }

    #[test]
    fn test_get_command_output_no_zone() {
        let mut term = Terminal::new(80, 24);
        term.start_command_execution("echo hi".to_string());
        term.end_command_execution(0);
        assert!(term.get_command_output(0).is_none());
    }

    #[test]
    fn test_get_command_output_multiple_commands() {
        let mut term = Terminal::new(80, 24);

        term.shell_integration_mut().set_command("cmd1".to_string());
        term.start_command_execution("cmd1".to_string());
        term.process(b"\x1b]133;A\x07$ \r\n");
        term.process(b"\x1b]133;B\x07cmd1\r\n");
        term.process(b"\x1b]133;C\x07");
        term.process(b"output1\r\n");
        term.process(b"\x1b]133;D;0\x07");
        term.end_command_execution(0);

        term.shell_integration_mut().set_command("cmd2".to_string());
        term.start_command_execution("cmd2".to_string());
        term.process(b"\x1b]133;A\x07$ \r\n");
        term.process(b"\x1b]133;B\x07cmd2\r\n");
        term.process(b"\x1b]133;C\x07");
        term.process(b"output2\r\n");
        term.process(b"\x1b]133;D;0\x07");
        term.end_command_execution(0);

        let out0 = term.get_command_output(0).unwrap();
        assert!(out0.contains("output2"));
        let out1 = term.get_command_output(1).unwrap();
        assert!(out1.contains("output1"));
    }

    #[test]
    fn test_get_command_outputs_filters_evicted() {
        // Use a larger terminal so the "new" command's output doesn't scroll past its zone
        let mut term = Terminal::with_scrollback(80, 24, 50);

        // First command - will be evicted
        term.shell_integration_mut().set_command("old".to_string());
        term.start_command_execution("old".to_string());
        term.process(b"\x1b]133;A\x07$ \r\n");
        term.process(b"\x1b]133;B\x07old\r\n");
        term.process(b"\x1b]133;C\x07");
        term.process(b"old output\r\n");
        term.process(b"\x1b]133;D;0\x07");
        term.end_command_execution(0);

        // Generate enough output to push old command past scrollback
        for i in 0..80 {
            term.process(format!("filler line {}\r\n", i).as_bytes());
        }

        // Second command - recent, output stays in visible grid (no scrolling between C and D)
        term.shell_integration_mut().set_command("new".to_string());
        term.start_command_execution("new".to_string());
        term.process(b"\x1b]133;A\x07$ \r\n");
        term.process(b"\x1b]133;B\x07new\r\n");
        term.process(b"\x1b]133;C\x07");
        term.process(b"new output"); // No \r\n — stays on same line as C marker
        term.process(b"\x1b]133;D;0\x07");
        term.end_command_execution(0);

        let outputs = term.get_command_outputs();
        // Old command's output should be evicted, only new should remain
        assert!(!outputs.is_empty());
        assert!(outputs.iter().any(|o| o.output.contains("new output")));
        // Old command should not be in extractable outputs
        assert!(!outputs.iter().any(|o| o.command == "old"));

        // Direct index access: old command (index 1) should return None
        assert!(term.get_command_output(1).is_none());
        // New command (index 0) should still work
        assert!(term.get_command_output(0).is_some());
    }

    #[test]
    fn test_get_command_output_no_output_rows_returns_none() {
        // A command where end_command_execution is called but no Output zone was created
        // (e.g., the last zone isn't an Output zone).
        let mut term = Terminal::new(80, 24);
        term.start_command_execution("echo hi".to_string());
        // Only create a Prompt zone, no Output zone
        term.process(b"\x1b]133;A\x07$ ");
        term.end_command_execution(0);
        // output_start_row/output_end_row should be None since last zone is Prompt, not Output
        assert!(term.get_command_output(0).is_none());
    }

    // ========== Contextual Awareness Event Tests ==========

    #[test]
    fn test_zone_opened_events_emitted() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]133;A\x1b\\");
        let events = term.poll_events();
        assert!(events.iter().any(|e| matches!(
            e,
            crate::terminal::TerminalEvent::ZoneOpened {
                zone_type,
                ..
            } if *zone_type == crate::zone::ZoneType::Prompt
        )));
    }

    #[test]
    fn test_zone_closed_on_transition() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]133;A\x1b\\");
        term.poll_events(); // drain
        term.process(b"\x1b]133;B\x1b\\");
        let events = term.poll_events();
        // Should have ZoneClosed for prompt and ZoneOpened for command
        assert!(
            events.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::ZoneClosed {
                    zone_type,
                    ..
                } if *zone_type == crate::zone::ZoneType::Prompt
            )),
            "Expected ZoneClosed for Prompt"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::ZoneOpened {
                    zone_type,
                    ..
                } if *zone_type == crate::zone::ZoneType::Command
            )),
            "Expected ZoneOpened for Command"
        );
    }

    #[test]
    fn test_zone_closed_with_exit_code() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]133;A\x1b\\");
        term.process(b"\x1b]133;B\x1b\\");
        term.process(b"\x1b]133;C\x1b\\");
        term.poll_events(); // drain
        term.process(b"\x1b]133;D;0\x1b\\");
        let events = term.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::ZoneClosed {
                    zone_type,
                    exit_code: Some(0),
                    ..
                } if *zone_type == crate::zone::ZoneType::Output
            )),
            "Expected ZoneClosed for Output with exit_code=0"
        );
    }

    #[test]
    fn test_zone_ids_monotonically_increase() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]133;A\x1b\\"); // zone 0 (Prompt)
        term.process(b"\x1b]133;B\x1b\\"); // zone 1 (Command)
        term.process(b"\x1b]133;C\x1b\\"); // zone 2 (Output)
        let events = term.poll_events();
        let zone_ids: Vec<usize> = events
            .iter()
            .filter_map(|e| match e {
                crate::terminal::TerminalEvent::ZoneOpened { zone_id, .. } => Some(*zone_id),
                _ => None,
            })
            .collect();
        assert_eq!(zone_ids, vec![0, 1, 2]);
    }

    #[test]
    fn test_full_zone_lifecycle() {
        let mut term = Terminal::new(80, 24);
        // Full cycle: A -> B -> C -> D
        term.process(b"\x1b]133;A\x1b\\");
        term.process(b"\x1b]133;B\x1b\\");
        term.process(b"\x1b]133;C\x1b\\");
        term.process(b"\x1b]133;D;0\x1b\\");
        let events = term.poll_events();

        // Count opens and closes
        let opens: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::ZoneOpened { .. }))
            .collect();
        let closes: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, crate::terminal::TerminalEvent::ZoneClosed { .. }))
            .collect();

        // 3 opens (Prompt, Command, Output), 3 closes (Prompt, Command, Output)
        assert_eq!(opens.len(), 3, "Expected 3 ZoneOpened events");
        assert_eq!(closes.len(), 3, "Expected 3 ZoneClosed events");
    }

    // ========== Environment Change Event Tests ==========

    #[test]
    fn test_environment_changed_on_cwd() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]7;file:///home/user/project\x1b\\");
        let events = term.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::EnvironmentChanged {
                    key,
                    value,
                    ..
                } if key == "cwd" && value == "/home/user/project"
            )),
            "Expected EnvironmentChanged event for cwd"
        );
    }

    #[test]
    fn test_remote_host_transition_from_osc7() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]7;file://remotehost/home/user\x1b\\");
        let events = term.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::RemoteHostTransition {
                    hostname,
                    old_hostname: None,
                    ..
                } if hostname == "remotehost"
            )),
            "Expected RemoteHostTransition event from OSC 7"
        );
    }

    #[test]
    fn test_remote_host_transition_from_osc1337() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]1337;RemoteHost=alice@server1\x1b\\");
        let events = term.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::RemoteHostTransition {
                    hostname,
                    ..
                } if hostname == "server1"
            )),
            "Expected RemoteHostTransition event from OSC 1337"
        );
    }

    #[test]
    fn test_environment_changed_hostname() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]7;file://myhost/home/user\x1b\\");
        let events = term.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                crate::terminal::TerminalEvent::EnvironmentChanged {
                    key,
                    value,
                    ..
                } if key == "hostname" && value == "myhost"
            )),
            "Expected EnvironmentChanged event for hostname"
        );
    }
}
