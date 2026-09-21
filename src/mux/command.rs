//! Control-mode command parsing (client → server).

use crate::mux::ids::{PaneId, SessionId, WindowId};
use crate::mux::layout::{ResizeDirection, SplitDirection};

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
    /// Split a pane's area in two, creating and focusing a new pane.
    SplitWindow {
        /// Target pane to split.
        pane: PaneId,
        /// Split orientation after tmux's flag mapping: `-h` puts the new
        /// pane beside the target (side by side), `-v`/default below it.
        direction: SplitDirection,
        /// `-p`: percent of the split area given to the NEW pane, 50 when
        /// absent (tmux semantics — the target keeps the remainder).
        percent: u32,
    },
    /// Make a pane its window's active pane.
    SelectPane {
        /// Target pane.
        pane: PaneId,
    },
    /// Grow or shrink a pane by moving its bordering divider.
    ResizePane {
        /// Target pane.
        pane: PaneId,
        /// Which way the border moves (`-L`/`-R`/`-U`/`-D`).
        direction: ResizeDirection,
        /// Cells to move it by; 5 when the flag carries no number (tmux's
        /// default adjustment).
        cells: u32,
    },
    /// Exchange two panes' positions within their window.
    SwapPanes {
        /// Pane swapped into the source's position (`-t`).
        target: PaneId,
        /// Pane swapped into the target's position (`-s`).
        source: PaneId,
    },
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

    // Presence check for valueless flags (`-h`, `-R`, …) — `flag` cannot
    // distinguish "absent" from "present with no following token".
    let has_flag = |flag: &str| args.contains(&flag);

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
        "split-window" => {
            let pane = target_pane("-t")?;
            // tmux's flags name the arrangement, not the divider: `-h`
            // puts the new pane beside the target (our Vertical
            // orientation), `-v`/default below it (Horizontal).
            let direction = if has_flag("-h") {
                SplitDirection::Vertical
            } else {
                SplitDirection::Horizontal
            };
            let percent = match flag("-p") {
                Some(raw) => {
                    let percent: u32 = raw
                        .parse()
                        .map_err(|_| format!("invalid percentage: {raw}"))?;
                    if !(1..=99).contains(&percent) {
                        return Err(format!("percentage must be 1-99: {raw}"));
                    }
                    percent
                }
                None => 50,
            };
            Ok(MuxCommand::SplitWindow {
                pane,
                direction,
                percent,
            })
        }
        "select-pane" => Ok(MuxCommand::SelectPane {
            pane: target_pane("-t")?,
        }),
        "resize-pane" => {
            let pane = target_pane("-t")?;
            // tmux takes one direction flag; the first of the four wins.
            let Some((flag_name, direction)) = [
                ("-L", ResizeDirection::Left),
                ("-R", ResizeDirection::Right),
                ("-U", ResizeDirection::Up),
                ("-D", ResizeDirection::Down),
            ]
            .into_iter()
            .find(|(flag, _)| has_flag(flag)) else {
                return Err("resize-pane requires one of -L -R -U -D".to_string());
            };
            // The cell count is the flag's value when present and numeric;
            // tmux's default adjustment is 5 cells.
            let cells = flag(flag_name)
                .and_then(|raw| raw.parse::<u32>().ok())
                .unwrap_or(5);
            Ok(MuxCommand::ResizePane {
                pane,
                direction,
                cells,
            })
        }
        "swap-pane" => Ok(MuxCommand::SwapPanes {
            target: target_pane("-t")?,
            source: target_pane("-s")?,
        }),
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
    fn parses_split_window_with_flags_and_defaults() {
        // Default: new pane below the target (-v), 50 percent.
        assert_eq!(
            parse_command("split-window -t %0").unwrap(),
            MuxCommand::SplitWindow {
                pane: PaneId(0),
                direction: SplitDirection::Horizontal,
                percent: 50
            }
        );
        assert_eq!(
            parse_command("split-window -t %0 -v").unwrap(),
            MuxCommand::SplitWindow {
                pane: PaneId(0),
                direction: SplitDirection::Horizontal,
                percent: 50
            }
        );
        // -h: side by side; -p: the NEW pane's share.
        assert_eq!(
            parse_command("split-window -t %0 -h -p 25").unwrap(),
            MuxCommand::SplitWindow {
                pane: PaneId(0),
                direction: SplitDirection::Vertical,
                percent: 25
            }
        );
    }

    #[test]
    fn split_window_rejects_out_of_range_percent() {
        assert!(parse_command("split-window -t %0 -p 0").is_err());
        assert!(parse_command("split-window -t %0 -p 100").is_err());
    }

    #[test]
    fn parses_select_and_swap_pane() {
        assert_eq!(
            parse_command("select-pane -t %2").unwrap(),
            MuxCommand::SelectPane { pane: PaneId(2) }
        );
        assert_eq!(
            parse_command("swap-pane -t %2 -s %5").unwrap(),
            MuxCommand::SwapPanes {
                target: PaneId(2),
                source: PaneId(5)
            }
        );
    }

    #[test]
    fn parses_resize_pane_with_default_and_explicit_cells() {
        assert_eq!(
            parse_command("resize-pane -t %0 -R").unwrap(),
            MuxCommand::ResizePane {
                pane: PaneId(0),
                direction: ResizeDirection::Right,
                cells: 5
            }
        );
        assert_eq!(
            parse_command("resize-pane -t %0 -U 12").unwrap(),
            MuxCommand::ResizePane {
                pane: PaneId(0),
                direction: ResizeDirection::Up,
                cells: 12
            }
        );
        assert!(parse_command("resize-pane -t %0").is_err());
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
