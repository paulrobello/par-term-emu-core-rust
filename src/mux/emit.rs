//! Control-mode emitter: `TmuxNotification` values onto the wire.

use crate::tmux_control::TmuxNotification;

/// Current time as epoch seconds, used for notification timestamps.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Escape raw PTY bytes for a `%output` line.
///
/// tmux renders any byte that is not printable ASCII as a three-digit octal
/// escape, and escapes the backslash itself so decoding is unambiguous. The
/// parser in [`crate::tmux_control`] decodes exactly this form.
pub fn escape_output(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        match byte {
            b'\\' => out.push_str("\\134"),
            0x20..=0x7e => out.push(byte as char),
            other => {
                out.push('\\');
                out.push_str(&format!("{other:03o}"));
            }
        }
    }
    out
}

/// Render one notification as a control-mode line, newline-terminated.
///
/// Control mode is line-based: every line must carry its terminator or the
/// client blocks waiting for it.
pub fn emit(notification: &TmuxNotification) -> String {
    match notification {
        TmuxNotification::Output { pane_id, data } => {
            format!("%output {} {}\n", pane_id, escape_output(data))
        }
        TmuxNotification::WindowAdd { window_id } => {
            format!("%window-add {window_id}\n")
        }
        TmuxNotification::WindowClose { window_id } => {
            format!("%window-close {window_id}\n")
        }
        TmuxNotification::UnlinkedWindowClose { window_id } => {
            format!("%unlinked-window-close {window_id}\n")
        }
        TmuxNotification::WindowPaneChanged { window_id, pane_id } => {
            format!("%window-pane-changed {window_id} {pane_id}\n")
        }
        TmuxNotification::PaneModeChanged { pane_id } => {
            format!("%pane-mode-changed {pane_id}\n")
        }
        TmuxNotification::WindowRenamed { window_id, name } => {
            format!("%window-renamed {window_id} {name}\n")
        }
        TmuxNotification::SessionChanged { session_id, name } => {
            format!("%session-changed {session_id} {name}\n")
        }
        TmuxNotification::LayoutChange {
            window_id,
            window_layout,
            window_visible_layout,
            window_raw_flags,
        } => {
            format!(
                "%layout-change {window_id} {window_layout} {window_visible_layout} {window_raw_flags}\n"
            )
        }
        TmuxNotification::Begin {
            timestamp,
            command_number,
            flags,
        } => format!("%begin {timestamp} {command_number} {flags}\n"),
        TmuxNotification::End {
            timestamp,
            command_number,
            flags,
        } => format!("%end {timestamp} {command_number} {flags}\n"),
        TmuxNotification::Error {
            timestamp,
            command_number,
            flags,
        } => format!("%error {timestamp} {command_number} {flags}\n"),
        // Sent to every client on a graceful shutdown, before the sockets
        // close, so a client learns the daemon ended deliberately rather
        // than inferring death from a dropped connection.
        TmuxNotification::Exit => "%exit\n".to_string(),
        TmuxNotification::AgentStateChanged {
            pane_id,
            agent,
            state,
            source,
        } => {
            // The source token rides only when set, so a hand-constructed
            // notification with no provenance emits the Phase 5 line shape
            // unchanged.
            let tail = if source.is_empty() {
                String::new()
            } else {
                format!(" source={source}")
            };
            format!("%agent-state-changed {pane_id} {agent} {state}{tail}\n")
        }
        // No provenance token by construction: a release can only come from
        // a hook report, never a scrape guess.
        TmuxNotification::AgentReleased { pane_id, agent } => {
            format!("%agent-released {pane_id} {agent}\n")
        }
        TmuxNotification::PaneTitleChanged { pane_id, title } => {
            // The separator is omitted for an empty title (the clear
            // operation) so the wire never carries a trailing space; the
            // parser reads a missing title token as empty.
            let tail = if title.is_empty() {
                String::new()
            } else {
                format!(" {title}")
            };
            format!("%pane-title-changed {pane_id}{tail}\n")
        }
        // Seam S3: `TmuxNotification` carries 29 variants; Phase 1 emits the
        // 9 the spine needs and the rest fall through here, producing nothing
        // rather than panicking. Adding a notification is therefore one new
        // arm, never a change at the call sites. Real tmux clients ignore `%`
        // lines they do not recognise (this parser maps them to `Unknown`,
        // which still reaches the client), so a custom variant like
        // `%agent-state-changed` is backward compatible by construction.
        _ => String::new(),
    }
}

/// Render a complete command response block.
///
/// Every control-mode command reply is bracketed: `%begin`, the body, then
/// `%end` on success or `%error` on failure. `command_number` ties the reply to
/// the request and must match across the opening and closing lines.
pub fn emit_block(command_number: u32, body: &str, ok: bool) -> String {
    let timestamp = now_secs();
    let mut out = String::new();
    out.push_str(&emit(&TmuxNotification::Begin {
        timestamp,
        command_number,
        flags: "1".to_string(),
    }));
    if !body.is_empty() {
        out.push_str(body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
    }
    let closing = if ok {
        TmuxNotification::End {
            timestamp,
            command_number,
            flags: "1".to_string(),
        }
    } else {
        TmuxNotification::Error {
            timestamp,
            command_number,
            flags: "1".to_string(),
        }
    };
    out.push_str(&emit(&closing));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux_control::{TmuxControlParser, TmuxNotification};

    /// Feed an emitted line back through the real parser. This is the
    /// conformance oracle: it checks the emitter against the decoder
    /// `par-term-tmux` actually uses, not against a hand-written fixture.
    fn round_trip(notification: &TmuxNotification) -> Vec<TmuxNotification> {
        let line = emit(notification);
        let mut parser = TmuxControlParser::new(true);
        parser.parse(line.as_bytes())
    }

    #[test]
    fn output_notification_round_trips() {
        let original = TmuxNotification::Output {
            pane_id: "%3".to_string(),
            data: b"hello".to_vec(),
        };
        let parsed = round_trip(&original);
        assert_eq!(parsed.len(), 1, "one line in, one notification out");
        match &parsed[0] {
            TmuxNotification::Output { pane_id, data } => {
                assert_eq!(pane_id, "%3");
                assert_eq!(data, b"hello");
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn output_with_control_bytes_round_trips_through_octal_escaping() {
        // Agent TUIs emit escape sequences constantly; if this does not survive
        // the round trip, nothing renders.
        let payload = b"\x1b[31mred\x1b[0m\r\n".to_vec();
        let original = TmuxNotification::Output {
            pane_id: "%0".to_string(),
            data: payload.clone(),
        };
        let parsed = round_trip(&original);
        match &parsed[0] {
            TmuxNotification::Output { data, .. } => assert_eq!(data, &payload),
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn escape_output_uses_three_digit_octal() {
        assert_eq!(escape_output(b"a"), "a");
        assert_eq!(escape_output(b"\x1b"), "\\033");
        assert_eq!(escape_output(b"\r"), "\\015");
        assert_eq!(escape_output(b"\n"), "\\012");
        // Backslash itself must be escaped or decoding is ambiguous.
        assert_eq!(escape_output(b"\\"), "\\134");
    }

    #[test]
    fn window_add_round_trips() {
        let original = TmuxNotification::WindowAdd {
            window_id: "@2".to_string(),
        };
        let parsed = round_trip(&original);
        match &parsed[0] {
            TmuxNotification::WindowAdd { window_id } => assert_eq!(window_id, "@2"),
            other => panic!("expected WindowAdd, got {other:?}"),
        }
    }

    #[test]
    fn every_phase_one_notification_round_trips() {
        // The nine variants Phase 1 emits, each serialized and fed back through
        // the real parser, asserting full equality — not just the two the plan
        // spot-checks. Field order and separator count are load-bearing.
        let flags = || "1".to_string();
        let originals = vec![
            TmuxNotification::Begin {
                timestamp: 1234567890,
                command_number: 4,
                flags: flags(),
            },
            TmuxNotification::End {
                timestamp: 1234567890,
                command_number: 4,
                flags: flags(),
            },
            TmuxNotification::Error {
                timestamp: 1234567890,
                command_number: 4,
                flags: flags(),
            },
            TmuxNotification::Output {
                pane_id: "%1".to_string(),
                data: Vec::new(),
            },
            // A single space of data: separator space plus data space on the wire.
            TmuxNotification::Output {
                pane_id: "%1".to_string(),
                data: b" ".to_vec(),
            },
            TmuxNotification::PaneModeChanged {
                pane_id: "%2".to_string(),
            },
            TmuxNotification::WindowPaneChanged {
                window_id: "@0".to_string(),
                pane_id: "%2".to_string(),
            },
            TmuxNotification::WindowClose {
                window_id: "@0".to_string(),
            },
            TmuxNotification::UnlinkedWindowClose {
                window_id: "@3".to_string(),
            },
        ];
        for original in &originals {
            let parsed = round_trip(original);
            assert_eq!(
                parsed.len(),
                1,
                "one line in, one notification out for {original:?}"
            );
            assert_eq!(&parsed[0], original, "round trip changed the notification");
        }
    }

    #[test]
    fn window_renamed_round_trips() {
        let original = TmuxNotification::WindowRenamed {
            window_id: "@1".to_string(),
            name: "scratch".to_string(),
        };
        let parsed = round_trip(&original);
        assert_eq!(parsed.len(), 1, "one line in, one notification out");
        assert_eq!(&parsed[0], &original, "round trip changed the notification");
    }

    #[test]
    fn session_changed_round_trips() {
        let original = TmuxNotification::SessionChanged {
            session_id: "$0".to_string(),
            name: "main".to_string(),
        };
        let parsed = round_trip(&original);
        assert_eq!(parsed.len(), 1, "one line in, one notification out");
        assert_eq!(&parsed[0], &original, "round trip changed the notification");
    }

    #[test]
    fn layout_change_round_trips() {
        let original = TmuxNotification::LayoutChange {
            window_id: "@0".to_string(),
            window_layout: "0000,89x24,0,0,1".to_string(),
            window_visible_layout: "0000,89x24,0,0,1".to_string(),
            window_raw_flags: "*".to_string(),
        };
        let parsed = round_trip(&original);
        assert_eq!(parsed.len(), 1, "one line in, one notification out");
        assert_eq!(&parsed[0], &original, "round trip changed the notification");
    }

    #[test]
    fn task_2_6_notifications_round_trip_with_full_field_equality() {
        // Same shape as every_phase_one_notification_round_trips: every
        // variant this task adds, serialized and fed back through the real
        // parser, asserting full equality rather than a spot check.
        let originals = vec![
            TmuxNotification::WindowRenamed {
                window_id: "@2".to_string(),
                name: "logs".to_string(),
            },
            TmuxNotification::SessionChanged {
                session_id: "$1".to_string(),
                name: "work".to_string(),
            },
            TmuxNotification::LayoutChange {
                window_id: "@3".to_string(),
                window_layout: "0000,89x24,0,0{44x24,0,0,1,44x24,45,0,2}".to_string(),
                window_visible_layout: "0000,89x24,0,0{44x24,0,0,1,44x24,45,0,2}".to_string(),
                window_raw_flags: "0".to_string(),
            },
        ];
        for original in &originals {
            let parsed = round_trip(original);
            assert_eq!(
                parsed.len(),
                1,
                "one line in, one notification out for {original:?}"
            );
            assert_eq!(&parsed[0], original, "round trip changed the notification");
        }
    }

    #[test]
    fn unemitted_variants_produce_nothing_rather_than_panicking() {
        // Seam S3: the catch-all arm. Adding a notification later is one new
        // arm here, never a change at call sites; until then it emits nothing.
        assert_eq!(emit(&TmuxNotification::SessionsChanged), "");
    }

    #[test]
    fn pane_title_changed_round_trips_with_spaces() {
        // A title legitimately contains spaces; the parser must take the
        // rest of the line rather than one token.
        let original = TmuxNotification::PaneTitleChanged {
            pane_id: "%3".to_string(),
            title: "My build pane".to_string(),
        };
        let parsed = round_trip(&original);
        assert_eq!(parsed.len(), 1, "one line in, one notification out");
        assert_eq!(&parsed[0], &original, "round trip changed the notification");
    }

    #[test]
    fn pane_title_changed_clear_round_trips_as_a_missing_token() {
        // Clear emits no separator; the parser reads the missing token as
        // an empty title, so the round trip is exact.
        let original = TmuxNotification::PaneTitleChanged {
            pane_id: "%3".to_string(),
            title: String::new(),
        };
        let line = emit(&original);
        assert_eq!(
            line, "%pane-title-changed %3\n",
            "no trailing space: {line:?}"
        );
        let parsed = round_trip(&original);
        assert_eq!(parsed.len(), 1, "one line in, one notification out");
        assert_eq!(&parsed[0], &original, "round trip changed the notification");
    }

    #[test]
    fn agent_state_changed_round_trips() {
        let original = TmuxNotification::AgentStateChanged {
            pane_id: "%3".to_string(),
            agent: "kimi".to_string(),
            state: "working".to_string(),
            source: "hook".to_string(),
        };
        let parsed = round_trip(&original);
        assert_eq!(parsed.len(), 1, "one line in, one notification out");
        assert_eq!(&parsed[0], &original, "round trip changed the notification");
    }

    #[test]
    fn agent_state_changed_without_a_source_round_trips() {
        // Both wire shapes must survive: the Phase 5 three-token line (no
        // provenance) and the four-token form. The parser treats a missing
        // `source=` as empty rather than folding the token into the state.
        let mut parser = TmuxControlParser::new(true);
        let parsed = parser.parse(b"%agent-state-changed %4 claude blocked\n");
        match parsed.as_slice() {
            [TmuxNotification::AgentStateChanged {
                pane_id,
                agent,
                state,
                source,
            }] => {
                assert_eq!(
                    (pane_id.as_str(), agent.as_str(), state.as_str()),
                    ("%4", "claude", "blocked")
                );
                assert!(source.is_empty(), "no source token on the wire");
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn agent_state_changed_line_reaches_an_unupgraded_client() {
        // The backward-compatibility mechanism itself: BEFORE the parse arm
        // existed (and for any FUTURE variant), an unrecognized `%` line
        // parses to Unknown { line } and still reaches the client — a client
        // that never learns %agent-state-changed sees the raw line rather
        // than a parse error or a dropped notification.
        let mut parser = TmuxControlParser::new(true);
        let parsed = parser.parse(b"%agent-state-changed %3 kimi working\n");
        match parsed.as_slice() {
            [TmuxNotification::Unknown { line }] => {
                assert!(
                    line.contains("%agent-state-changed %3 kimi working"),
                    "the raw line survives: {line}"
                );
            }
            [TmuxNotification::AgentStateChanged { .. }] => {
                // The parse arm exists now, so this path is taken — the
                // Unknown tolerance is still proven by an unrecognized line:
                let parsed2 = parser.parse(b"%some-future-variant x y\n");
                assert!(
                    matches!(parsed2.as_slice(), [TmuxNotification::Unknown { .. }]),
                    "an unrecognized variant still reaches the client as Unknown"
                );
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn exit_round_trips() {
        let parsed = round_trip(&TmuxNotification::Exit);
        assert_eq!(parsed.len(), 1, "one line in, one notification out");
        assert_eq!(parsed[0], TmuxNotification::Exit);
    }

    #[test]
    fn command_block_emits_begin_and_end() {
        let block = emit_block(7, "pane_one\npane_two", true);
        assert!(block.starts_with("%begin "), "block opens with %begin");
        assert!(
            block.contains("pane_one\npane_two"),
            "body is carried verbatim"
        );
        assert!(block.contains("%end "), "successful block closes with %end");
        assert!(!block.contains("%error"), "successful block has no %error");
    }

    #[test]
    fn failed_command_block_emits_error() {
        let block = emit_block(8, "no such pane", false);
        assert!(block.starts_with("%begin "));
        assert!(block.contains("%error "), "failed block closes with %error");
        assert!(
            !block.contains("%end "),
            "failed block does not also close with %end"
        );
    }

    #[test]
    fn begin_and_end_carry_the_same_command_number() {
        let block = emit_block(42, "body", true);
        let numbers: Vec<&str> = block
            .lines()
            .filter(|l| l.starts_with("%begin") || l.starts_with("%end"))
            .filter_map(|l| l.split_whitespace().nth(2))
            .collect();
        assert_eq!(
            numbers.len(),
            2,
            "both %begin and %end carry a command number"
        );
        assert_eq!(numbers[0], numbers[1], "the numbers must match: {block:?}");
        assert_eq!(numbers[0], "42");
    }

    #[test]
    fn every_emitted_line_ends_with_a_newline() {
        // Control mode is line-based; a missing terminator stalls the client.
        let n = TmuxNotification::WindowClose {
            window_id: "@1".to_string(),
        };
        assert!(emit(&n).ends_with('\n'));
        assert!(emit_block(1, "x", true).ends_with('\n'));
    }

    #[test]
    fn agent_released_round_trips() {
        let original = TmuxNotification::AgentReleased {
            pane_id: "%3".to_string(),
            agent: "pi".to_string(),
        };
        let line = emit(&original);
        assert_eq!(line, "%agent-released %3 pi\n", "the wire shape");
        let parsed = round_trip(&original);
        assert_eq!(parsed.len(), 1, "one line in, one notification out");
        assert_eq!(&parsed[0], &original, "round trip changed the notification");
    }
}
