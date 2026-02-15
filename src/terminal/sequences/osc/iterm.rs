//! iTerm2 OSC 1337 sequence handling

use crate::debug;
use crate::terminal::Terminal;

impl Terminal {
    pub(crate) fn handle_osc_iterm(&mut self, _command: &str, params: &[&[u8]]) {
        if params.len() >= 2 {
            let mut data_parts = Vec::new();
            for p in &params[1..] {
                if let Ok(s) = std::str::from_utf8(p) {
                    data_parts.push(s);
                }
            }
            let data = data_parts.join(";");

            if let Some(encoded) = data.strip_prefix("SetBadgeFormat=") {
                self.handle_set_badge_format(encoded);
            } else if let Some(payload) = data.strip_prefix("SetUserVar=") {
                self.handle_set_user_var(payload);
            } else if let Some(payload) = data.strip_prefix("RemoteHost=") {
                self.handle_remote_host(payload);
            } else if let Some(payload) = data.strip_prefix("RequestUpload=") {
                self.handle_request_upload(payload);
            } else {
                self.handle_iterm_image(&data);
            }
        }
    }

    pub(crate) fn handle_set_badge_format(&mut self, encoded: &str) {
        let encoded = encoded.trim();

        if encoded.is_empty() {
            self.badge_format = None;
            self.terminal_events
                .push(crate::terminal::TerminalEvent::BadgeChanged(None));
            debug::log(debug::DebugLevel::Debug, "OSC1337", "Cleared badge format");
            return;
        }

        match crate::badge::decode_badge_format(encoded) {
            Ok(format) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC1337",
                    &format!("Set badge format: {:?}", format),
                );
                self.badge_format = Some(format.clone());
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
            }
        }
    }

    pub(crate) fn handle_set_user_var(&mut self, payload: &str) {
        if let Some((name, encoded_value)) = payload.split_once('=') {
            if name.is_empty() {
                return;
            }
            use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
            if let Ok(decoded_value) = BASE64.decode(encoded_value.trim()) {
                if let Ok(value) = String::from_utf8(decoded_value) {
                    self.set_user_var(name.to_string(), value);
                }
            }
        }
    }

    pub(crate) fn handle_remote_host(&mut self, payload: &str) {
        if payload.is_empty() {
            return;
        }

        let (username, hostname) = if let Some((u, h)) = payload.split_once('@') {
            if h.is_empty() {
                return; // Ignore if hostname part is empty
            }
            (Some(u.to_string()), Some(h.to_string()))
        } else {
            (None, Some(payload.to_string()))
        };

        // Filter out localhost and empty values to match OSC 7 behavior
        let hostname = hostname.and_then(|h| {
            if h.is_empty() || h.eq_ignore_ascii_case("localhost") || h == "127.0.0.1" || h == "::1"
            {
                None
            } else {
                Some(h)
            }
        });

        let username = username.and_then(|u| if u.is_empty() { None } else { Some(u) });

        let current_cwd = self
            .shell_integration
            .cwd()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "/".to_string());

        self.record_cwd_change(crate::terminal::event::CwdChange {
            old_cwd: Some(current_cwd.clone()),
            new_cwd: current_cwd,
            hostname,
            username,
            timestamp: crate::terminal::unix_millis(),
        });
    }

    pub(crate) fn handle_request_upload(&mut self, payload: &str) {
        // payload is e.g. "format=tgz" — extract just the value
        let format = if let Some(val) = payload.strip_prefix("format=") {
            val.to_string()
        } else {
            payload.to_string()
        };
        self.terminal_events
            .push(crate::terminal::TerminalEvent::UploadRequested { format });
    }
}
