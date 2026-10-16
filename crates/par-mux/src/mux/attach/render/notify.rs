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
    /// The in-app toast card over the panes (herdr's `inside herdr`).
    Inside,
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
            "inside" => Some(Self::Inside),
            "terminal" => Some(Self::Terminal),
            "system" => Some(Self::System),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Inside => "inside",
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
/// (iTerm2, Ghostty, WezTerm, Windows Terminal). Kitty's two chunks keep
/// their ST terminators (the kitty protocol's own form); OSC 9 is
/// BEL-terminated — the spelling iTerm2's OSC 9 support was built around
/// (an ST-terminated OSC 9 never presented there in live testing, while
/// Ghostty accepts both).
pub(crate) fn terminal_notification(title: &str, body: &str, kitty: bool) -> Vec<u8> {
    let (title, body) = (sanitize(title), sanitize(body));
    if kitty {
        format!("\x1b]99;i=1:d=0;{title}\x1b\\\x1b]99;i=1:p=body;{body}\x1b\\").into_bytes()
    } else {
        // iTerm2's OSC 9 never presented an escape whose body carried the
        // multibyte `·` separator (live report: pure-ASCII printf bodies
        // present fine in the same terminal), so the OSC 9 body is folded
        // to ASCII — a non-ASCII workspace or agent name becomes `?`.
        // Kitty's OSC 99 is UTF-8 by spec and keeps its text.
        let ascii = |s: &str| -> String {
            s.chars()
                .map(|c| if c.is_ascii() { c } else { '?' })
                .collect()
        };
        format!("\x1b]9;{}: {}\x07", ascii(&title), ascii(&body)).into_bytes()
    }
}

/// Whether the host is kitty, from the attach's environment.
///
/// `KITTY_WINDOW_ID` alone is not proof: it leaks into every process
/// started under a kitty window — a ghostty or iTerm2 launched from a
/// kitty shell, herdr running on kitty. A real host overwrites
/// `TERM_PROGRAM` (`ghostty`, `iTerm.app`, `herdr`); kitty itself sets
/// none. A par-term/par-mux pane exports `KITTY_WINDOW_ID=1` WITH
/// `TERM_PROGRAM=kitty` (the core's pane spawn), so it still counts as
/// kitty and gets OSC 99, which the core parses. So the
/// window id counts only while `TERM_PROGRAM` is unset or `kitty`, and a
/// leaked `TERM=xterm-kitty` (ssh, nested shells) never decides alone.
/// A misread costs the notification: neither ghostty nor iTerm2 parses
/// OSC 99, while kitty also accepts the OSC 9 fallback.
fn host_is_kitty_in(kitty_window_id: Option<&str>, term_program: Option<&str>) -> bool {
    kitty_window_id.is_some_and(|id| !id.is_empty())
        && term_program.is_none_or(|program| program.is_empty() || program == "kitty")
}

/// [`host_is_kitty_in`] over the process environment.
fn host_is_kitty() -> bool {
    host_is_kitty_in(
        std::env::var("KITTY_WINDOW_ID").ok().as_deref(),
        std::env::var("TERM_PROGRAM").ok().as_deref(),
    )
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
            NotifyDelivery::Inside => {
                let status = crate::mux::attach::panels::AgentStatus::from_state(state);
                let (title, detail) = match body.split_once(": ") {
                    Some((who, what)) => (format!("{agent} {what}"), who.to_string()),
                    None => (body.clone(), String::new()),
                };
                self.notify.toast_until =
                    Some(std::time::Instant::now() + super::toast::toast_lifetime(status));
                self.renderer.set_toast(Some(super::toast::Toast {
                    title: sanitize(&title),
                    body: sanitize(&detail),
                    status,
                    pane,
                }));
                // The card only shows while the user looks at this
                // window; the bell makes the host flag the tab/window
                // unread (its bell setting decides sound or badge).
                self.notify.outbox.push(0x07);
            }
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
    /// When the in-app toast on screen expires.
    pub toast_until: Option<std::time::Instant>,
}

impl Default for NotifyState {
    fn default() -> Self {
        Self {
            delivery: NotifyDelivery::Off,
            states: Vec::new(),
            outbox: Vec::new(),
            toast_until: None,
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
            "\x1b]9;par-mux: main ? claude: blocked\x07".as_bytes(),
            "ESC, BEL, and non-ASCII inside pane-controlled text cannot reach \
             the OSC: control ends the string (iTerm2), multibyte broke its \
             presentation (live report)"
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

    #[test]
    fn host_is_kitty_only_when_the_window_id_is_not_leaked() {
        // Real kitty: the window id, no TERM_PROGRAM of its own.
        assert!(host_is_kitty_in(Some("3"), None));
        // A par-term / par-mux pane exports kitty's marks itself.
        assert!(host_is_kitty_in(Some("1"), Some("kitty")));
        // Leaked through a host started under kitty: the host's own
        // TERM_PROGRAM wins (Paul's ghostty/iTerm2/herdr report).
        for program in ["ghostty", "iTerm.app", "herdr", "WezTerm"] {
            assert!(
                !host_is_kitty_in(Some("7"), Some(program)),
                "{program} with a leaked kitty id"
            );
        }
        // TERM=xterm-kitty without the window id (ssh, nested) is not
        // enough, and an empty id is no id.
        assert!(!host_is_kitty_in(None, Some("xterm-kitty")));
        assert!(!host_is_kitty_in(Some(""), Some("ghostty")));
        assert!(!host_is_kitty_in(None, None));
    }
}
