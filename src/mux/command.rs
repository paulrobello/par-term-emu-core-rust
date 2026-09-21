//! Control-mode command parsing (client → server).

use crate::mux::ids::{PaneId, SessionId, WindowId};

/// A command received from a control-mode client.
///
/// Phase 1 implements the four commands that prove the spine end to end.
/// Additional commands are new variants plus new dispatch arms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxCommand {
    /// Create a session, optionally named.
    NewSession {
        /// Session name; a default is chosen when absent.
        name: Option<String>,
    },
    /// List every live pane.
    ListPanes,
    /// Send literal keys to a pane.
    SendKeys {
        /// Target pane.
        pane: PaneId,
        /// Literal text to write to the pane's PTY.
        keys: String,
    },
    /// Kill a pane.
    KillPane {
        /// Target pane.
        pane: PaneId,
    },
    /// Replay a pane's current screen to the requesting client.
    RefreshClient {
        /// Target pane.
        pane: PaneId,
    },
    /// Add a window to a session, optionally named.
    NewWindow {
        /// Target session.
        session: SessionId,
        /// Window name; a default is chosen when absent.
        name: Option<String>,
    },
    /// Set a session's active window.
    SelectWindow {
        /// Target window; its session is derived from it server-side.
        window: WindowId,
    },
    /// Kill a window and every pane it holds.
    KillWindow {
        /// Target window.
        window: WindowId,
    },
    /// Rename a window.
    RenameWindow {
        /// Target window.
        window: WindowId,
        /// New name.
        name: String,
    },
    /// List every window across every session.
    ListWindows,
    /// List every session.
    ListSessions,
    /// Print a pane's screen, optionally including scrollback.
    CapturePane {
        /// Target pane.
        pane: PaneId,
        /// `-S`: first line to capture, tmux offset convention — `0` is the
        /// first line of the visible screen, negative numbers are history
        /// lines counted back from there (`-1` is the line directly above
        /// the screen). `None` keeps tmux's default: the first visible line.
        start_line: Option<i64>,
        /// `-E`: last line to capture, inclusive, same offset convention.
        /// `None` keeps tmux's default: the bottom of the visible screen.
        end_line: Option<i64>,
    },
    /// Store text in the paste buffer.
    SetBuffer {
        /// Buffer content.
        content: String,
    },
    /// Retrieve the paste buffer's content.
    ShowBuffer,
    /// Write the paste buffer's content to a pane, as `send-keys` would.
    PasteBuffer {
        /// Target pane.
        pane: PaneId,
    },
}

/// Parse one command line from a client.
///
/// Deliberately minimal: whitespace-split with a `-t`/`-s` flag scan. tmux's
/// real argument grammar (quoting, `--`, per-command option tables) is Phase 2
/// work, and pretending to implement it here would hide that.
pub fn parse_command(line: &str) -> Result<MuxCommand, String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let Some((name, args)) = parts.split_first() else {
        return Err("empty command".to_string());
    };

    let flag = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| *a == flag)
            .and_then(|i| args.get(i + 1))
            .map(|v| (*v).to_string())
    };

    let target_pane = |flag_name: &str| -> Result<PaneId, String> {
        let raw = flag(flag_name).ok_or_else(|| format!("{name} requires {flag_name}"))?;
        raw.parse::<PaneId>()
            .map_err(|_| format!("invalid pane target: {raw}"))
    };

    let target_window = |flag_name: &str| -> Result<WindowId, String> {
        let raw = flag(flag_name).ok_or_else(|| format!("{name} requires {flag_name}"))?;
        raw.parse::<WindowId>()
            .map_err(|_| format!("invalid window target: {raw}"))
    };

    let target_session = |flag_name: &str| -> Result<SessionId, String> {
        let raw = flag(flag_name).ok_or_else(|| format!("{name} requires {flag_name}"))?;
        raw.parse::<SessionId>()
            .map_err(|_| format!("invalid session target: {raw}"))
    };

    // Everything after a `-t <target>` pair, joined back with spaces — the
    // same "trailing free text is the payload" shape `send-keys` already uses.
    let trailing_after_target = || -> String {
        args.iter()
            .skip_while(|a| **a != "-t")
            .skip(2)
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    };

    match *name {
        "new-session" => Ok(MuxCommand::NewSession { name: flag("-s") }),
        "list-panes" => Ok(MuxCommand::ListPanes),
        "kill-pane" => Ok(MuxCommand::KillPane {
            pane: target_pane("-t")?,
        }),
        "refresh-client" => Ok(MuxCommand::RefreshClient {
            pane: target_pane("-t")?,
        }),
        "send-keys" => {
            let pane = target_pane("-t")?;
            let keys = trailing_after_target();
            Ok(MuxCommand::SendKeys { pane, keys })
        }
        "new-window" => Ok(MuxCommand::NewWindow {
            session: target_session("-t")?,
            name: flag("-n"),
        }),
        "select-window" => Ok(MuxCommand::SelectWindow {
            window: target_window("-t")?,
        }),
        "kill-window" => Ok(MuxCommand::KillWindow {
            window: target_window("-t")?,
        }),
        "rename-window" => {
            let window = target_window("-t")?;
            let name = trailing_after_target();
            if name.is_empty() {
                return Err("rename-window requires a new name".to_string());
            }
            Ok(MuxCommand::RenameWindow { window, name })
        }
        "list-windows" => Ok(MuxCommand::ListWindows),
        "list-sessions" => Ok(MuxCommand::ListSessions),
        "capture-pane" => {
            let pane = target_pane("-t")?;
            // tmux's `-S`/`-E` select a start/end line; the raw offsets are
            // kept as-is (negative counts back from the screen top into
            // history) and the server-side adapter resolves them against
            // the combined scrollback+screen buffer.
            let start_line = flag("-S").and_then(|raw| raw.parse::<i64>().ok());
            let end_line = flag("-E").and_then(|raw| raw.parse::<i64>().ok());
            Ok(MuxCommand::CapturePane {
                pane,
                start_line,
                end_line,
            })
        }
        "set-buffer" => {
            let content = args.join(" ");
            if content.is_empty() {
                return Err("set-buffer requires content".to_string());
            }
            Ok(MuxCommand::SetBuffer { content })
        }
        "show-buffer" => Ok(MuxCommand::ShowBuffer),
        "paste-buffer" => Ok(MuxCommand::PasteBuffer {
            pane: target_pane("-t")?,
        }),
        other => Err(format!("unknown command: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_session_with_a_name() {
        let cmd = parse_command("new-session -s work").expect("parses");
        assert_eq!(
            cmd,
            MuxCommand::NewSession {
                name: Some("work".into())
            }
        );
    }

    #[test]
    fn parses_new_session_without_a_name() {
        let cmd = parse_command("new-session").expect("parses");
        assert_eq!(cmd, MuxCommand::NewSession { name: None });
    }

    #[test]
    fn parses_send_keys_with_a_target() {
        let cmd = parse_command("send-keys -t %3 hello").expect("parses");
        assert_eq!(
            cmd,
            MuxCommand::SendKeys {
                pane: PaneId(3),
                keys: "hello".into()
            }
        );
    }

    #[test]
    fn parses_list_panes_and_kill_pane() {
        assert_eq!(parse_command("list-panes").unwrap(), MuxCommand::ListPanes);
        assert_eq!(
            parse_command("kill-pane -t %7").unwrap(),
            MuxCommand::KillPane { pane: PaneId(7) }
        );
    }

    #[test]
    fn rejects_an_unknown_command() {
        assert!(parse_command("frobnicate").is_err());
    }

    #[test]
    fn rejects_a_malformed_target() {
        assert!(parse_command("kill-pane -t notapane").is_err());
        assert!(
            parse_command("kill-pane").is_err(),
            "kill-pane needs a target"
        );
    }

    #[test]
    fn parses_new_window_with_and_without_a_name() {
        assert_eq!(
            parse_command("new-window -t $0 -n build").unwrap(),
            MuxCommand::NewWindow {
                session: SessionId(0),
                name: Some("build".into())
            }
        );
        assert_eq!(
            parse_command("new-window -t $0").unwrap(),
            MuxCommand::NewWindow {
                session: SessionId(0),
                name: None
            }
        );
    }

    #[test]
    fn parses_select_and_kill_window() {
        assert_eq!(
            parse_command("select-window -t @2").unwrap(),
            MuxCommand::SelectWindow {
                window: WindowId(2)
            }
        );
        assert_eq!(
            parse_command("kill-window -t @2").unwrap(),
            MuxCommand::KillWindow {
                window: WindowId(2)
            }
        );
    }

    #[test]
    fn parses_rename_window_and_rejects_a_missing_name() {
        assert_eq!(
            parse_command("rename-window -t @1 scratch").unwrap(),
            MuxCommand::RenameWindow {
                window: WindowId(1),
                name: "scratch".into()
            }
        );
        assert!(
            parse_command("rename-window -t @1").is_err(),
            "rename-window needs a new name"
        );
    }

    #[test]
    fn parses_list_windows_and_list_sessions() {
        assert_eq!(
            parse_command("list-windows").unwrap(),
            MuxCommand::ListWindows
        );
        assert_eq!(
            parse_command("list-sessions").unwrap(),
            MuxCommand::ListSessions
        );
    }

    #[test]
    fn rejects_malformed_window_and_session_targets() {
        assert!(parse_command("new-window -t notasession").is_err());
        assert!(parse_command("select-window -t notawindow").is_err());
        assert!(parse_command("new-window").is_err(), "needs -t");
    }

    #[test]
    fn parses_capture_pane_with_and_without_history() {
        assert_eq!(
            parse_command("capture-pane -t %3 -p").unwrap(),
            MuxCommand::CapturePane {
                pane: PaneId(3),
                start_line: None,
                end_line: None
            }
        );
        assert_eq!(
            parse_command("capture-pane -t %3 -p -S 50 -E -1").unwrap(),
            MuxCommand::CapturePane {
                pane: PaneId(3),
                start_line: Some(50),
                end_line: Some(-1)
            }
        );
        assert_eq!(
            parse_command("capture-pane -t %3 -p -S -20 -E -11").unwrap(),
            MuxCommand::CapturePane {
                pane: PaneId(3),
                start_line: Some(-20),
                end_line: Some(-11)
            }
        );
    }

    #[test]
    fn parses_set_buffer_show_buffer_and_paste_buffer() {
        assert_eq!(
            parse_command("set-buffer hello world").unwrap(),
            MuxCommand::SetBuffer {
                content: "hello world".into()
            }
        );
        assert_eq!(
            parse_command("show-buffer").unwrap(),
            MuxCommand::ShowBuffer
        );
        assert_eq!(
            parse_command("paste-buffer -t %2").unwrap(),
            MuxCommand::PasteBuffer { pane: PaneId(2) }
        );
    }

    #[test]
    fn rejects_a_set_buffer_with_no_content() {
        assert!(parse_command("set-buffer").is_err());
    }
}
