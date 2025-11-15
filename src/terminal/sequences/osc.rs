//! OSC (Operating System Command) sequence handling
//!
//! Handles OSC sequences for terminal control, including:
//! - Window title manipulation
//! - Color queries and modifications
//! - Clipboard operations (OSC 52)
//! - Hyperlinks (OSC 8)
//! - Shell integration (OSC 133)
//! - Notifications (OSC 9, OSC 777)
//! - Directory tracking (OSC 7)

use crate::color::Color;
use crate::debug;
use crate::shell_integration::ShellIntegrationMarker;
use crate::terminal::{Notification, Terminal};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

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
                            self.title = title.to_string();
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
                            // Parse file:// URL to extract just the path
                            // Format: file://hostname/path or file:///path (localhost)
                            if let Some(path) = cwd_url.strip_prefix("file://") {
                                // Remove hostname part to get path
                                // Handle both file://hostname/path and file:///path
                                let path = if path.starts_with('/') {
                                    // file:///path (localhost implicit)
                                    path
                                } else {
                                    // file://hostname/path - skip to first /
                                    path.find('/').map(|i| &path[i..]).unwrap_or("")
                                };

                                if !path.is_empty() {
                                    self.shell_integration.set_cwd(path.to_string());
                                    debug::log(
                                        debug::DebugLevel::Debug,
                                        "OSC7",
                                        &format!("Set directory to: {}", path),
                                    );
                                }
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
                            }
                        }
                    } else if params.len() == 2 {
                        // OSC 8 ; ; ST (empty params and URI = end hyperlink)
                        self.current_hyperlink_id = None;
                    }
                }
                "9" => {
                    // Notification (OSC 9) - iTerm2/ConEmu style
                    // Simple format: OSC 9 ; message ST
                    if params.len() >= 2 {
                        if let Ok(message) = std::str::from_utf8(params[1]) {
                            let notification =
                                Notification::new(String::new(), message.to_string());
                            self.notifications.push(notification);
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
                                    self.notifications.push(notification);
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
                            match marker.chars().next() {
                                Some('A') => {
                                    self.shell_integration
                                        .set_marker(ShellIntegrationMarker::PromptStart);
                                }
                                Some('B') => {
                                    self.shell_integration
                                        .set_marker(ShellIntegrationMarker::CommandStart);
                                }
                                Some('C') => {
                                    self.shell_integration
                                        .set_marker(ShellIntegrationMarker::CommandExecuted);
                                }
                                Some('D') => {
                                    self.shell_integration
                                        .set_marker(ShellIntegrationMarker::CommandFinished);
                                    // Extract exit code if present
                                    if let Some(code_str) = marker.split(';').nth(1) {
                                        if let Ok(code) = code_str.parse::<i32>() {
                                            self.shell_integration.set_exit_code(code);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
