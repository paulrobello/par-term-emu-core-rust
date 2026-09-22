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
    /// Send keys to a pane.
    SendKeys {
        /// Target pane.
        pane: PaneId,
        /// Final bytes for the pane's PTY — key names interpreted, quotes
        /// resolved, no terminator added. `Enter` is an expressible key, not
        /// an implicit one (tmux semantics; par-mux.md Phase 4 T4.B).
        keys: Vec<u8>,
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
        /// Target session; `None` means the most-recently-created one (the
        /// bare `new-window` tmux clients issue, which tmux resolves against
        /// the client's attached session — par-mux has no client-session
        /// attachment, so "newest" is the documented stand-in).
        session: Option<SessionId>,
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

/// Split `rest` at its first whitespace-separated `flag` occurrence, into the
/// flag's value and the raw remainder following that value.
///
/// The remainder is a raw slice, not a joined token list: send-keys payloads
/// carry quoting that pre-splitting would destroy. The flag must precede the
/// payload — every sender par-mux targets puts `-t` first, and the bounded
/// grammar here documents that rather than papering over it.
fn split_after_flag<'a>(rest: &'a str, flag: &str) -> Option<(&'a str, &'a str)> {
    let bytes = rest.as_bytes();
    let flag = flag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if start == i {
            break;
        }
        if &bytes[start..i] == flag {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let value_start = j;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if value_start == j {
                return None;
            }
            let mut k = j;
            while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                k += 1;
            }
            return Some((&rest[value_start..j], &rest[k..]));
        }
    }
    None
}

/// Split a payload into shell-style words: single- or double-quoted regions
/// contribute their literal content, a backslash outside quotes escapes the
/// next character, and unquoted whitespace separates words.
///
/// This is the bounded grammar par-term's senders actually emit (the `'\''`
/// idiom for embedded quotes); it is not a full shell parser and does not
/// interpolate anything.
fn shell_split(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut have_token = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                have_token = true;
                let quote = c;
                for inner in chars.by_ref() {
                    if inner == quote {
                        break;
                    }
                    current.push(inner);
                }
            }
            '\\' => {
                have_token = true;
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            c if c.is_whitespace() => {
                if have_token {
                    tokens.push(std::mem::take(&mut current));
                    have_token = false;
                }
            }
            c => {
                have_token = true;
                current.push(c);
            }
        }
    }
    if have_token {
        tokens.push(current);
    }
    tokens
}

/// Map one send-keys token to the bytes a terminal expects for that key.
///
/// The table covers exactly the names par-term's `escape_keys_for_tmux`
/// emits, plus `Enter` (which replaces the removed implicit newline) and the
/// arrow keys. Unknown tokens are NOT errors: they are written literally, so
/// passthrough text works without quoting every word (a deliberate, narrower
/// contract than tmux's, which rejects unknown key names).
fn key_to_bytes(name: &str) -> Option<Vec<u8>> {
    match name {
        "C-Space" => Some(vec![0x00]),
        "Enter" => Some(vec![0x0d]),
        "Escape" | "Esc" => Some(vec![0x1b]),
        "BSpace" => Some(vec![0x7f]),
        "Space" => Some(vec![b' ']),
        "Up" => Some(vec![0x1b, b'[', b'A']),
        "Down" => Some(vec![0x1b, b'[', b'B']),
        "Right" => Some(vec![0x1b, b'[', b'C']),
        "Left" => Some(vec![0x1b, b'[', b'D']),
        _ => {
            let letter = name.strip_prefix("C-")?;
            if letter.len() != 1 {
                return None;
            }
            let c = letter.chars().next()?.to_ascii_lowercase();
            if c.is_ascii_lowercase() {
                Some(vec![c as u8 - b'a' + 1])
            } else {
                None
            }
        }
    }
}

/// A bare `0xNN` token: one raw byte, the form `escape_keys_for_tmux` uses
/// for high bytes.
fn hex_byte_token(token: &str) -> Option<u8> {
    let digits = token.strip_prefix("0x")?;
    if digits.len() != 2 {
        return None;
    }
    u8::from_str_radix(digits, 16).ok()
}

/// Parse a send-keys payload into the final bytes for the pane's PTY.
///
/// Three modes, mirroring tmux's contract:
/// - default: tokens are keys — quoted or bare words resolve through the key
///   table, `0xNN` tokens are raw bytes, anything else is literal text.
///   Tokens join with NOTHING between them; a space must be an explicit
///   `Space` key or live inside a quoted run (exactly how
///   `escape_keys_for_tmux` encodes spaces).
/// - `-l`: everything is literal text (quotes still resolved, no key
///   interpretation).
/// - `-H`: tokens are hex byte pairs, `0x` prefix optional.
///
/// No terminator is appended in any mode.
fn parse_send_keys_payload(raw: &str) -> Result<Vec<u8>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("send-keys requires a payload".to_string());
    }
    let (literal, hex, body) = match raw {
        "-l" | "-H" => return Err("send-keys requires a payload".to_string()),
        _ if let Some(rest) = raw.strip_prefix("-l ") => (true, false, rest),
        _ if let Some(rest) = raw.strip_prefix("-H ") => (false, true, rest),
        _ => (false, false, raw),
    };
    let tokens = shell_split(body);
    if tokens.is_empty() {
        return Err("send-keys requires a payload".to_string());
    }
    let mut out = Vec::new();
    if hex {
        for token in &tokens {
            let digits = token.strip_prefix("0x").unwrap_or(token);
            let byte = u8::from_str_radix(digits, 16)
                .map_err(|_| format!("invalid hex byte: {token}"))?;
            out.push(byte);
        }
    } else if literal {
        for token in tokens {
            out.extend_from_slice(token.as_bytes());
        }
    } else {
        for token in tokens {
            if let Some(bytes) = key_to_bytes(&token) {
                out.extend_from_slice(&bytes);
            } else if let Some(byte) = hex_byte_token(&token) {
                out.push(byte);
            } else {
                out.extend_from_slice(token.as_bytes());
            }
        }
    }
    Ok(out)
}

/// Parse one command line from a client.
///
/// Deliberately minimal: whitespace-split with a `-t`/`-s` flag scan. tmux's
/// real argument grammar (quoting, `--`, per-command option tables) is not a
/// goal here, and pretending to implement it would hide that. The one
/// exception is `send-keys`, which carries its own bounded quoting grammar
/// (see [`parse_send_keys_payload`]) because key names, `-l` and `-H` cannot
/// survive a whitespace split.
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
            let rest = line
                .strip_prefix(name)
                .expect("the command name prefixes the line");
            let (target_value, payload_raw) =
                split_after_flag(rest, "-t").ok_or_else(|| format!("{name} requires -t"))?;
            let pane: PaneId = target_value
                .parse()
                .map_err(|_| format!("invalid pane target: {target_value}"))?;
            let keys = parse_send_keys_payload(payload_raw)?;
            Ok(MuxCommand::SendKeys { pane, keys })
        }
        "new-window" => Ok(MuxCommand::NewWindow {
            session: match flag("-t") {
                Some(raw) => Some(
                    raw.parse()
                        .map_err(|_| format!("invalid session target: {raw}"))?,
                ),
                None => None,
            },
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
                keys: b"hello".to_vec()
            }
        );
    }

    #[test]
    fn send_keys_interprets_key_names() {
        let cmd = parse_command("send-keys -t %3 C-c").expect("parses");
        assert_eq!(
            cmd,
            MuxCommand::SendKeys {
                pane: PaneId(3),
                keys: vec![0x03]
            }
        );
        let cmd = parse_command("send-keys -t %3 C-Space Escape BSpace Space Enter").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, vec![0x00, 0x1b, 0x7f, b' ', 0x0d]);
    }

    #[test]
    fn send_keys_maps_arrows_to_csi_sequences() {
        let cmd = parse_command("send-keys -t %3 Up Down Left Right").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"\x1b[A\x1b[B\x1b[D\x1b[C".to_vec());
    }

    #[test]
    fn send_keys_resolves_quoted_runs_and_the_quote_idiom() {
        // Quoted runs are literal, spaces inside them survive, and the
        // '\'' idiom yields a real single quote — the escape_keys_for_tmux
        // round trip.
        let cmd = parse_command("send-keys -t %3 'hello world'").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"hello world".to_vec());

        let cmd = parse_command("send-keys -t %3 'it'\\''s'").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"it's".to_vec());
    }

    #[test]
    fn send_keys_space_between_bare_words_is_explicit_not_implicit() {
        // tmux semantics: tokens join with nothing between them; a space is
        // the Space key. escape_keys_for_tmux encodes exactly this.
        let cmd = parse_command("send-keys -t %3 'hello' Space 'world'").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"hello world".to_vec());

        let cmd = parse_command("send-keys -t %3 hello world").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"helloworld".to_vec());
    }

    #[test]
    fn send_keys_literal_flag_disables_interpretation() {
        let cmd = parse_command("send-keys -t %3 -l C-c").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"C-c".to_vec());
    }

    #[test]
    fn send_keys_hex_flag_takes_byte_pairs() {
        // The form format_send_hex_keys emits for CSI-u sequences.
        let cmd = parse_command("send-keys -t %3 -H 1b 5b 41").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, vec![0x1b, 0x5b, 0x41]);

        assert!(parse_command("send-keys -t %3 -H zz").is_err());
    }

    #[test]
    fn send_keys_bare_hex_token_is_one_byte() {
        let cmd = parse_command("send-keys -t %3 0x1b 'prompt> '").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"\x1bprompt> ".to_vec());
    }

    #[test]
    fn send_keys_round_trips_an_escape_keys_for_tmux_stream() {
        // Representative output of par-term's escape_keys_for_tmux for the
        // bytes b"hi \xe2\x82\xacC-c": printable run quoted, high bytes as
        // 0xNN tokens, the control key by name.
        let cmd = parse_command("send-keys -t %3 'hi ' 0xe2 0x82 0xac C-c").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"hi \xe2\x82\xac\x03".to_vec());
    }

    #[test]
    fn send_keys_requires_a_payload() {
        assert!(parse_command("send-keys -t %3").is_err());
        assert!(parse_command("send-keys -t %3 -l").is_err());
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
                session: Some(SessionId(0)),
                name: Some("build".into())
            }
        );
        assert_eq!(
            parse_command("new-window -t $0").unwrap(),
            MuxCommand::NewWindow {
                session: Some(SessionId(0)),
                name: None
            }
        );
        // Bare new-window — the form tmux clients issue — targets the
        // most-recently-created session, resolved server-side.
        assert_eq!(
            parse_command("new-window").unwrap(),
            MuxCommand::NewWindow {
                session: None,
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
