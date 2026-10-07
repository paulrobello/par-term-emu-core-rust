//! iTerm2 OSC 1337 sequence handling
//!
//! Capability boundary (ARC-002): SetBadgeFormat, SetUserVar, RemoteHost,
//! RequestUpload, and CurrentDir are free functions over the badge state,
//! the event broker, and (for the cwd-changing pair) a [`CwdCapability`].
//! The router stays on `Terminal` because the fallback — inline images
//! (`File=` / `MultipartFile=` / `FilePart=`) — lives in `graphics.rs` and
//! touches the grid, cursor, graphics store, and file-transfer manager.

use crate::debug;
use crate::terminal::shell_integration::record_cwd_change;
use crate::terminal::{
    BadgeState, CommandHistoryState, CwdChange, EventBroker, ShellState, Terminal,
};

/// The state a working-directory change writes: shell integration, badge
/// session variables, events, and the capped CWD history. Shared by OSC 7
/// and the OSC 1337 CurrentDir / RemoteHost handlers.
pub(crate) struct CwdCapability<'a> {
    pub(crate) shell: &'a mut ShellState,
    pub(crate) badge: &'a mut BadgeState,
    pub(crate) events: &'a mut EventBroker,
    pub(crate) history: &'a mut CommandHistoryState,
    pub(crate) max_cwd_history: usize,
}

impl CwdCapability<'_> {
    pub(crate) fn record(&mut self, change: CwdChange) {
        record_cwd_change(
            self.shell,
            self.badge,
            self.events,
            self.history,
            self.max_cwd_history,
            change,
        );
    }
}

impl Terminal {
    /// Borrow the cwd-change capability out of `self`.
    pub(crate) fn cwd_capability(&mut self) -> CwdCapability<'_> {
        CwdCapability {
            shell: &mut self.shell_state,
            badge: &mut self.badge_state,
            events: &mut self.events,
            history: &mut self.command_history_state,
            max_cwd_history: self.host.max_cwd_history,
        }
    }

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
                handle_set_badge_format(&mut self.badge_state, &mut self.events, encoded);
            } else if let Some(payload) = data.strip_prefix("SetUserVar=") {
                handle_set_user_var(&mut self.badge_state, &mut self.events, payload);
            } else if let Some(payload) = data.strip_prefix("RemoteHost=") {
                handle_remote_host(&mut self.cwd_capability(), payload);
            } else if let Some(payload) = data.strip_prefix("RequestUpload=") {
                handle_request_upload(&mut self.events, payload);
            } else if let Some(path) = data.strip_prefix("CurrentDir=") {
                let accept_osc7 = self.security_state.accept_osc7;
                handle_current_dir(&mut self.cwd_capability(), accept_osc7, path);
            } else {
                self.handle_iterm_image(&data);
            }
        }
    }
}

pub(crate) fn handle_set_badge_format(
    badge: &mut BadgeState,
    events: &mut EventBroker,
    encoded: &str,
) {
    let encoded = encoded.trim();

    if encoded.is_empty() {
        badge.badge_format = None;
        events.push(crate::terminal::TerminalEvent::BadgeChanged(None));
        debug::log(debug::DebugLevel::Debug, "OSC1337", "Cleared badge format");
        return;
    }

    match crate::badge::decode_badge_format(encoded) {
        Ok(format) => {
            crate::debug_log!("OSC1337", "Set badge format: {:?}", format);
            badge.badge_format = Some(format.clone());
            let badge_text = badge.evaluate();
            events.push(crate::terminal::TerminalEvent::BadgeChanged(badge_text));
        }
        Err(e) => {
            crate::debug_log!("OSC1337", "Invalid badge format: {}", e);
        }
    }
}

pub(crate) fn handle_set_user_var(badge: &mut BadgeState, events: &mut EventBroker, payload: &str) {
    if let Some((name, encoded_value)) = payload.split_once('=') {
        if name.is_empty() {
            return;
        }
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        if let Ok(decoded_value) = BASE64.decode(encoded_value.trim()) {
            if let Ok(value) = String::from_utf8(decoded_value) {
                badge.set_user_var(name.to_string(), value, events);
            }
        }
    }
}

pub(crate) fn handle_remote_host(cwd: &mut CwdCapability<'_>, payload: &str) {
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
    let hostname = hostname.filter(|h| {
        !(h.is_empty() || h.eq_ignore_ascii_case("localhost") || h == "127.0.0.1" || h == "::1")
    });

    let username = username.filter(|u| !u.is_empty());

    let current_cwd = cwd
        .shell
        .shell_integration
        .cwd()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "/".to_string());

    cwd.record(CwdChange {
        old_cwd: Some(current_cwd.clone()),
        new_cwd: current_cwd,
        hostname,
        username,
        timestamp: crate::text_utils::unix_millis(),
    });
}

pub(crate) fn handle_request_upload(events: &mut EventBroker, payload: &str) {
    // payload is e.g. "format=tgz" — extract just the value
    let format = if let Some(val) = payload.strip_prefix("format=") {
        val.to_string()
    } else {
        payload.to_string()
    };
    events.push(crate::terminal::TerminalEvent::UploadRequested { format });
}

pub(crate) fn handle_current_dir(cwd: &mut CwdCapability<'_>, accept_osc7: bool, path: &str) {
    // iTerm2 CurrentDir is an alias for OSC 7's working-directory update,
    // carrying a raw path; the OSC 7 security gate applies equally here.
    // Unlike OSC 7 it carries no host info, so the previously recorded
    // hostname/username are passed through unchanged.
    // Control characters are rejected like parse_osc7_url (SEC-117):
    // the cwd feeds C-string FFI fields and line-oriented consumers.
    if path.is_empty() || path.chars().any(char::is_control) || !accept_osc7 {
        return;
    }
    let si = &cwd.shell.shell_integration;
    let old_cwd = si.cwd().map(|s| s.to_string());
    let hostname = si.hostname().map(|s| s.to_string());
    let username = si.username().map(|s| s.to_string());

    cwd.record(CwdChange {
        old_cwd,
        new_cwd: path.to_string(),
        hostname,
        username,
        timestamp: crate::text_utils::unix_millis(),
    });
}
