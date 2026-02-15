//! Shell integration OSC sequence handling

use crate::debug;
use crate::shell_integration::ShellIntegrationMarker;
use crate::terminal::Terminal;
use percent_encoding::percent_decode_str;
use url::Url;

impl Terminal {
    pub(crate) fn handle_osc_shell(&mut self, command: &str, params: &[&[u8]]) {
        match command {
            "7" => {
                // Set current working directory (OSC 7)
                if self.accept_osc7 && params.len() >= 2 {
                    if let Ok(cwd_url) = std::str::from_utf8(params[1]) {
                        if let Some((path, hostname, username)) = Self::parse_osc7_url(cwd_url) {
                            // record_cwd_change handles setting shell_integration state
                            // and reads old values before updating
                            self.record_cwd_change(crate::terminal::event::CwdChange {
                                old_cwd: self.shell_integration.cwd().map(|s| s.to_string()),
                                new_cwd: path.clone(),
                                hostname: hostname.clone(),
                                username,
                                timestamp: crate::terminal::unix_millis(),
                            });
                            debug::log(
                                debug::DebugLevel::Debug,
                                "OSC7",
                                &format!("Set directory to: {} (hostname: {:?})", path, hostname),
                            );
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
                                if !self.alt_screen_active {
                                    let close_row = if abs_line > 0 { abs_line - 1 } else { 0 };
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
                                if !self.alt_screen_active {
                                    let close_row = if abs_line > 0 { abs_line - 1 } else { 0 };
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

                                // Record output start row in current command execution
                                if let Some(ref mut execution) = self.current_command {
                                    execution.output_start_row = Some(abs_line);
                                }

                                if !self.alt_screen_active {
                                    let close_row = if abs_line > 0 { abs_line - 1 } else { 0 };
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
                                self.in_command_output = true;
                            }
                            Some('D') => {
                                self.shell_integration
                                    .set_marker(ShellIntegrationMarker::CommandFinished);
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
                                if !self.alt_screen_active {
                                    let closed_info = self
                                        .grid
                                        .zones()
                                        .last()
                                        .map(|z| (z.id, z.zone_type, z.abs_row_start));
                                    self.grid.close_current_zone(abs_line);
                                    if let Some(zone) = self.grid.zones_mut().last_mut() {
                                        if zone.zone_type == crate::zone::ZoneType::Output {
                                            zone.exit_code = parsed_code;
                                        }
                                    }
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
            _ => {}
        }
    }

    /// Parse OSC 7 payload and return decoded path, hostname, username
    pub(crate) fn parse_osc7_url(
        url_str: &str,
    ) -> Option<(String, Option<String>, Option<String>)> {
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

        if !url_str.starts_with("file://") {
            return None;
        }

        let mut remainder = &url_str[7..];
        if remainder.is_empty() {
            return None;
        }

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
}
