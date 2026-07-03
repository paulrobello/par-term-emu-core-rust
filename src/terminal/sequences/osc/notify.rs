//! Notification and progress OSC sequence handling

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use crate::debug;
use crate::terminal::notification::Urgency;
use crate::terminal::progress::{ProgressBar, ProgressBarCommand, ProgressState};
use crate::terminal::Notification;
use crate::terminal::Terminal;

impl Terminal {
    pub(crate) fn handle_osc_notify(&mut self, command: &str, params: &[&[u8]]) {
        match command {
            "9" if params.len() >= 2 => {
                if let Ok(param1) = std::str::from_utf8(params[1]) {
                    let param1 = param1.trim();
                    if param1 == "4" {
                        self.handle_osc9_progress(&params[2..]);
                    } else {
                        let notification = Notification::new(String::new(), param1.to_string());
                        self.enqueue_notification(notification);
                    }
                }
            }
            "777" if params.len() >= 4 => {
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
            "934" => {
                self.handle_osc934(params);
            }
            "99" => {
                self.handle_osc99(&params[1..]);
            }
            _ => {}
        }
    }

    /// Handle OSC 99 (Kitty desktop notification protocol).
    ///
    /// Format: `OSC 99 ; <metadata> ; <payload> ST` where `<metadata>` is zero
    /// or more colon-separated `key=value` pairs. `params[0]` is the metadata
    /// segment and `params[1..]` is the payload segment(s).
    ///
    /// Supported metadata keys: `i` (id, for grouping/updating), `d` (done:
    /// `0` = more chunks follow, `1` = last chunk, default `1`), `p` (payload
    /// type: `title` default or `body`), `e` (encoding: `0` raw default, `1`
    /// base64), `u` (urgency: `0` low, `1` normal default, `2` critical), `a`
    /// (comma-separated actions, e.g. `focus,report`). Unknown keys are
    /// ignored for forward compatibility.
    pub(crate) fn handle_osc99(&mut self, params: &[&[u8]]) {
        if params.is_empty() {
            return;
        }

        let metadata = match std::str::from_utf8(params[0]) {
            Ok(s) => s,
            Err(_) => return,
        };

        let mut id: Option<String> = None;
        let mut done = true;
        let mut is_body = false;
        let mut base64_encoded = false;
        let mut urgency: Option<Urgency> = None;
        let mut actions: Option<Vec<String>> = None;

        for pair in metadata.split(':') {
            if pair.is_empty() {
                continue;
            }
            let mut kv = pair.splitn(2, '=');
            let key = kv.next().unwrap_or("");
            let value = kv.next().unwrap_or("");
            match key {
                "i" => id = Some(value.to_string()),
                "d" => done = value != "0",
                "p" => is_body = value == "body",
                "e" => base64_encoded = value == "1",
                "u" => urgency = Some(Urgency::from_param(value)),
                "a" => {
                    actions = Some(
                        value
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect(),
                    );
                }
                _ => {
                    // Unknown key: ignore for forward compatibility.
                }
            }
        }

        // Rejoin remaining params with ';' - a raw (non-base64) payload could
        // contain literal ';' bytes that VTE's OSC splitting breaks apart.
        let mut payload_bytes: Vec<u8> = Vec::new();
        for (idx, part) in params[1..].iter().enumerate() {
            if idx > 0 {
                payload_bytes.push(b';');
            }
            payload_bytes.extend_from_slice(part);
        }

        let payload_text = if base64_encoded {
            match BASE64.decode(&payload_bytes) {
                Ok(decoded) => String::from_utf8_lossy(&decoded).into_owned(),
                Err(_) => {
                    debug::log(
                        debug::DebugLevel::Debug,
                        "OSC99",
                        "Failed to base64-decode notification payload",
                    );
                    return;
                }
            }
        } else {
            String::from_utf8_lossy(&payload_bytes).into_owned()
        };

        let key = id.clone().unwrap_or_default();
        let entry = self
            .notifications_state
            .osc99_pending
            .entry(key.clone())
            .or_default();

        if id.is_some() {
            entry.id = id;
        }
        if is_body {
            entry.body.push_str(&payload_text);
        } else {
            entry.title.push_str(&payload_text);
        }
        if let Some(u) = urgency {
            entry.urgency = u;
        }
        if let Some(a) = actions {
            entry.actions = a;
        }

        if !done {
            return;
        }

        if let Some(partial) = self.notifications_state.osc99_pending.remove(&key) {
            // No explicit body chunk arrived: treat the accumulated title text
            // as the message, mirroring OSC 9/777's single-string semantics.
            let (title, message) = if partial.body.is_empty() {
                (String::new(), partial.title)
            } else {
                (partial.title, partial.body)
            };
            debug::log(
                debug::DebugLevel::Debug,
                "OSC99",
                &format!(
                    "Notification: id={:?}, urgency={:?}, actions={:?}",
                    partial.id, partial.urgency, partial.actions
                ),
            );
            let notification = Notification::with_metadata(
                title,
                message,
                partial.id,
                partial.urgency,
                partial.actions,
            );
            self.enqueue_notification(notification);
        }
    }

    pub(crate) fn handle_osc9_progress(&mut self, params: &[&[u8]]) {
        if params.is_empty() {
            return;
        }

        let state_param = match std::str::from_utf8(params[0]) {
            Ok(s) => s.trim(),
            Err(_) => return,
        };

        let state_num: u8 = match state_param.parse() {
            Ok(n) => n,
            Err(_) => return,
        };

        let state = ProgressState::from_param(state_num);

        let progress = if state.requires_progress() && params.len() >= 2 {
            match std::str::from_utf8(params[1]) {
                Ok(s) => s.trim().parse::<u8>().unwrap_or(0).min(100),
                Err(_) => 0,
            }
        } else {
            0
        };

        self.progress_state.progress_bar = ProgressBar::new(state, progress);

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

    pub(crate) fn handle_osc934(&mut self, params: &[&[u8]]) {
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
}

#[cfg(test)]
mod osc99_tests {
    use super::*;
    use crate::terminal::Terminal;

    #[test]
    fn test_osc99_simple_single_chunk() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]99;;Hello\x1b\\");
        let notifications = term.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].title, "");
        assert_eq!(notifications[0].message, "Hello");
        assert_eq!(notifications[0].id, None);
    }

    #[test]
    fn test_osc99_multi_chunk_title_then_body() {
        let mut term = Terminal::new(80, 24);
        // First chunk: title, not done yet.
        term.process(b"\x1b]99;i=x:d=0:p=title;Hi\x1b\\");
        assert_eq!(term.notifications().len(), 0);
        // Second chunk: body, marks the notification done.
        term.process(b"\x1b]99;i=x:d=1:p=body;There\x1b\\");
        let notifications = term.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].title, "Hi");
        assert_eq!(notifications[0].message, "There");
        assert_eq!(notifications[0].id.as_deref(), Some("x"));
    }

    #[test]
    fn test_osc99_base64_payload() {
        let mut term = Terminal::new(80, 24);
        let encoded = BASE64.encode(b"Hello");
        let seq = format!("\x1b]99;e=1;{}\x1b\\", encoded);
        term.process(seq.as_bytes());
        let notifications = term.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].message, "Hello");
    }

    #[test]
    fn test_osc99_urgency_and_actions() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]99;u=2:a=focus,report;Alert\x1b\\");
        let notifications = term.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].urgency, Urgency::Critical);
        assert_eq!(
            notifications[0].actions,
            vec!["focus".to_string(), "report".to_string()]
        );
    }

    #[test]
    fn test_osc99_unknown_key_ignored() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]99;x=weird:p=body;Body text\x1b\\");
        let notifications = term.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].title, "");
        assert_eq!(notifications[0].message, "Body text");
    }

    #[test]
    fn test_osc99_default_urgency_is_normal() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]99;;Plain\x1b\\");
        assert_eq!(term.notifications()[0].urgency, Urgency::Normal);
        assert!(term.notifications()[0].actions.is_empty());
    }
}
