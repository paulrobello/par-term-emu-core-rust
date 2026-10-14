//! Agent state-change notifications (herdr's background toasts): a pushed
//! `%agent-state-changed` whose new state is one the user selected
//! (`[client] notify-states`) is announced through the chosen delivery
//! (`[client] notify-delivery`) — the host terminal's notification escape
//! (OSC 9, kitty's OSC 99) or the OS notifier. Only a pushed TRANSITION
//! notifies: the status refresh replaces the roster wholesale and never
//! notifies, so neither the first load nor a reconnect floods the user.
//! Like herdr's, notifications are for the background: an agent in the
//! window the client shows never notifies.

use super::WindowSession;

/// How a notification reaches the user (config `notify-delivery`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotifyDelivery {
    /// No notifications.
    Off,
    /// The host terminal's notification escape, written with the frame.
    Terminal,
    /// The OS notifier (`osascript` on macOS, `notify-send` elsewhere).
    System,
}

impl NotifyDelivery {
    /// The config spelling; `None` for an unknown one.
    pub(crate) fn parse(spec: &str) -> Option<Self> {
        match spec {
            "off" => Some(Self::Off),
            "terminal" => Some(Self::Terminal),
            "system" => Some(Self::System),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Terminal => "terminal",
            Self::System => "system",
        }
    }
}

/// Strip what could end or extend an escape (ESC, BEL, C1 ST) and fold
/// line breaks: agent names and workspace names are pane-controlled, and
/// the text rides inside an OSC string.
fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{1b}' | '\u{7}' | '\u{9c}'))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The host terminal's notification escape for `title` / `body`: kitty's
/// structured OSC 99 under kitty (it ignores OSC 9), OSC 9 elsewhere
/// (iTerm2, Ghostty, WezTerm, Windows Terminal). Terminated by ST.
pub(crate) fn terminal_notification(title: &str, body: &str, kitty: bool) -> Vec<u8> {
    let (title, body) = (sanitize(title), sanitize(body));
    if kitty {
        format!("\x1b]99;i=1:d=0;{title}\x1b\\\x1b]99;i=1:p=body;{body}\x1b\\").into_bytes()
    } else {
        format!("\x1b]9;{title}: {body}\x1b\\").into_bytes()
    }
}

/// Whether the host is kitty (its env marks, the way herdr detects it).
fn host_is_kitty() -> bool {
    std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var("TERM").is_ok_and(|term| term == "xterm-kitty")
}

/// Hand `title` / `body` to the OS notifier, detached: argv only (no
/// shell), output discarded, and a reaper thread so the pump never waits
/// on it and no zombie stays behind. Best-effort — a missing notifier
/// shows nothing.
fn system_notification(title: &str, body: &str) {
    let (title, body) = (sanitize(title), sanitize(body));
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("/usr/bin/osascript");
        command.args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            &title,
            &body,
        ]);
        command
    } else if cfg!(windows) {
        return;
    } else {
        let mut command = std::process::Command::new("notify-send");
        command.args(["--", &title, &body]);
        command
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Ok(mut child) = command.spawn() {
        let _ = std::thread::Builder::new()
            .name("par-mux-notify".to_string())
            .spawn(move || {
                let _ = child.wait();
            });
    }
}

impl WindowSession {
    /// A pushed agent state change: announce it when the state is new for
    /// the pane and selected. `previous` is the roster's state before the
    /// push patched it (`None` for a pane the roster lacked).
    pub(super) fn notify_agent_state(
        &mut self,
        pane: u32,
        agent: &str,
        previous: Option<&str>,
        state: &str,
        ws: &str,
    ) {
        // Background-only (herdr's rule): an agent in the window this
        // client shows is already in view, so it never notifies.
        let in_view = self.renderer.layout().iter().any(|r| r.pane == pane);
        if self.notify.delivery == NotifyDelivery::Off
            || in_view
            || previous == Some(state)
            || !self.notify.states.iter().any(|s| s == state)
        {
            return;
        }
        // A push without `ws=` (an older daemon) falls back to the
        // workspace the roster already placed the pane in.
        let ws = if ws.is_empty() {
            self.status
                .agents()
                .iter()
                .find(|entry| entry.pane == pane)
                .and_then(|entry| entry.workspace.clone())
                .unwrap_or_default()
        } else {
            ws.to_string()
        };
        let workspace = self
            .status
            .workspaces()
            .iter()
            .find(|(id, _)| *id == ws)
            .map(|(_, name)| name.clone());
        let body = match workspace {
            Some(name) => format!("{name} \u{b7} {agent}: {state}"),
            None => format!("{agent} (%{pane}): {state}"),
        };
        match self.notify.delivery {
            NotifyDelivery::Off => {}
            NotifyDelivery::Terminal => {
                let bytes = terminal_notification("par-mux", &body, host_is_kitty());
                self.notify.outbox.extend_from_slice(&bytes);
            }
            NotifyDelivery::System => system_notification("par-mux", &body),
        }
    }
}

/// The session's notification settings and the terminal escapes waiting
/// for the next frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NotifyState {
    pub delivery: NotifyDelivery,
    /// The agent states that notify (config `notify-states`).
    pub states: Vec<String>,
    /// Terminal escapes the next frame writes to the host.
    pub outbox: Vec<u8>,
}

impl Default for NotifyState {
    fn default() -> Self {
        Self {
            delivery: NotifyDelivery::Off,
            states: Vec::new(),
            outbox: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_notification_escapes_are_sanitized() {
        let osc9 = terminal_notification("par-mux", "main \u{b7} cl\x1baude: blocked\x07", false);
        assert_eq!(
            osc9,
            "\x1b]9;par-mux: main \u{b7} claude: blocked\x1b\\".as_bytes(),
            "ESC and BEL inside pane-controlled text cannot end the OSC"
        );
        let kitty = String::from_utf8(terminal_notification("par-mux", "a\nb", true)).unwrap();
        assert!(kitty.contains("]99;i=1:d=0;par-mux\x1b\\"), "{kitty:?}");
        assert!(kitty.contains("]99;i=1:p=body;a b\x1b\\"), "{kitty:?}");
        assert_eq!(
            NotifyDelivery::parse("system"),
            Some(NotifyDelivery::System)
        );
        assert_eq!(NotifyDelivery::parse("toast"), None);
    }
}
