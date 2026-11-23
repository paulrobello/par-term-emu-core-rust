//! Per-line parsing: `parse_line` classifies one control-mode line and
//! dispatches `%name args` to the notification-family modules below.

use super::TmuxNotification;

mod agent;
mod command;
mod misc;
mod output;
mod session;
mod window;

#[cfg(test)]
pub(in crate::tmux_control) use output::unescape_output;

/// Parse a single line into a notification
pub(super) fn parse_line(line: &str) -> Option<TmuxNotification> {
    // Only trim leading whitespace - trailing whitespace may be significant
    // for %output notifications where space is the actual output data.
    let line = line.trim_start();

    // Empty lines are ignored
    if line.is_empty() {
        return None;
    }

    // Check if this is a control protocol message (starts with %)
    if !line.starts_with('%') {
        // Regular output (shouldn't happen in control mode, but handle it)
        return Some(TmuxNotification::TerminalOutput {
            data: line.as_bytes().to_vec(),
        });
    }

    // Split into notification type and arguments
    let parts: Vec<&str> = line[1..].splitn(2, ' ').collect();
    let notification_type = parts[0];
    let args = if parts.len() > 1 { parts[1] } else { "" };

    match notification_type {
        "begin" => command::parse_begin(args),
        "end" => command::parse_end(args),
        "error" => command::parse_error(args),
        "output" => output::parse_output(args),
        "pane-damage" => output::parse_output_damage(args),
        "pane-mode-changed" => window::parse_pane_mode_changed(args),
        "window-pane-changed" => window::parse_window_pane_changed(args),
        "window-close" => window::parse_window_close(args),
        "unlinked-window-close" => window::parse_unlinked_window_close(args),
        "window-add" => window::parse_window_add(args),
        "unlinked-window-add" => window::parse_unlinked_window_add(args),
        "window-renamed" => window::parse_window_renamed(args),
        "unlinked-window-renamed" => window::parse_unlinked_window_renamed(args),
        "session-changed" => session::parse_session_changed(args),
        "client-session-changed" => session::parse_client_session_changed(args),
        "session-renamed" => session::parse_session_renamed(args),
        "sessions-changed" => Some(TmuxNotification::SessionsChanged),
        "workspaces-changed" => Some(TmuxNotification::WorkspacesChanged),
        "config-changed" => Some(TmuxNotification::ConfigChanged),
        "session-window-changed" => session::parse_session_window_changed(args),
        "client-detached" => session::parse_client_detached(args),
        "client-attached" => session::parse_client_attached(args),
        "client-left" => session::parse_client_left(args),
        "exit" => Some(TmuxNotification::Exit),
        "pause" => output::parse_pause(args),
        "extended-output" => output::parse_extended_output(args),
        "continue" => output::parse_continue(args),
        "subscription-changed" => misc::parse_subscription_changed(args),
        "layout-change" => window::parse_layout_change(args),
        "paste-buffer-changed" => misc::parse_paste_buffer_changed(args),
        "paste-buffer-deleted" => misc::parse_paste_buffer_deleted(args),
        "agent-state-changed" => agent::parse_agent_state_changed(args),
        "agent-released" => agent::parse_agent_released(args),
        "agent-telemetry-changed" => agent::parse_agent_telemetry_changed(args),
        "pane-title-changed" => window::parse_pane_title_changed(args),
        "pane-exited" => window::parse_pane_exited(args),
        "pane-respawned" => window::parse_pane_id_only(args),
        "notification" => misc::parse_notification(args),
        _ => Some(TmuxNotification::Unknown {
            line: line.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> Option<TmuxNotification> {
        parse_line(s)
    }

    #[test]
    fn blank_lines_are_ignored() {
        assert_eq!(line(""), None);
        assert_eq!(line("   "), None);
    }

    #[test]
    fn non_protocol_line_is_terminal_output() {
        assert_eq!(
            line("plain text"),
            Some(TmuxNotification::TerminalOutput {
                data: b"plain text".to_vec()
            })
        );
    }

    #[test]
    fn unknown_keyword_is_surfaced_verbatim() {
        assert_eq!(
            line("%frobnicate a b"),
            Some(TmuxNotification::Unknown {
                line: "%frobnicate a b".into()
            })
        );
    }

    #[test]
    fn leading_whitespace_is_trimmed_but_trailing_output_space_survives() {
        match line("  %output %1 a ") {
            Some(TmuxNotification::Output { pane_id, data, .. }) => {
                assert_eq!(pane_id, "%1");
                assert_eq!(data, b"a ");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn bare_keywords_without_arguments() {
        assert_eq!(line("%exit"), Some(TmuxNotification::Exit));
        assert_eq!(
            line("%sessions-changed"),
            Some(TmuxNotification::SessionsChanged)
        );
        assert_eq!(
            line("%workspaces-changed"),
            Some(TmuxNotification::WorkspacesChanged)
        );
        assert_eq!(
            line("%config-changed"),
            Some(TmuxNotification::ConfigChanged)
        );
    }

    #[test]
    fn command_framing_lines() {
        assert_eq!(
            line("%begin 1700 5 1"),
            Some(TmuxNotification::Begin {
                timestamp: 1700,
                command_number: 5,
                flags: "1".into()
            })
        );
        assert!(matches!(
            line("%end 1700 5"),
            Some(TmuxNotification::End { flags, .. }) if flags.is_empty()
        ));
        assert!(matches!(
            line("%error 1 2 x y"),
            Some(TmuxNotification::Error { flags, .. }) if flags == "x y"
        ));
        assert_eq!(line("%begin 1"), None);
        assert_eq!(line("%end abc 1"), None);
    }

    #[test]
    fn output_decodes_octal_and_optional_generation() {
        assert_eq!(
            line("%output %2 hi\\012"),
            Some(TmuxNotification::Output {
                pane_id: "%2".into(),
                generation: None,
                data: b"hi\n".to_vec()
            })
        );
        assert_eq!(
            line("%output %2 gen=9 x"),
            Some(TmuxNotification::Output {
                pane_id: "%2".into(),
                generation: Some(9),
                data: b"x".to_vec()
            })
        );
    }

    #[test]
    fn extended_output_value_follows_the_lone_colon() {
        assert_eq!(
            line("%extended-output %1 25 : ab\\033"),
            Some(TmuxNotification::ExtendedOutput {
                pane_id: "%1".into(),
                delay_ms: 25,
                data: b"ab\x1b".to_vec()
            })
        );
        assert_eq!(line("%extended-output %1 x : a"), None);
        assert_eq!(line("%extended-output %1"), None);
    }

    #[test]
    fn pane_damage_ranges() {
        assert_eq!(
            line("%pane-damage %1 4 0-2 5-5"),
            Some(TmuxNotification::OutputDamage {
                pane_id: "%1".into(),
                generation: 4,
                ranges: vec![(0, 2), (5, 5)]
            })
        );
        assert!(matches!(
            line("%pane-damage %1 4"),
            Some(TmuxNotification::OutputDamage { ranges, .. }) if ranges.is_empty()
        ));
        assert_eq!(line("%pane-damage %1 4 3-1"), None);
        assert_eq!(line("%pane-damage %1 4 junk"), None);
    }

    #[test]
    fn pause_and_continue_carry_the_pane() {
        assert_eq!(
            line("%pause %3"),
            Some(TmuxNotification::Pause {
                pane_id: "%3".into()
            })
        );
        assert_eq!(
            line("%continue %3"),
            Some(TmuxNotification::Continue {
                pane_id: "%3".into()
            })
        );
    }

    #[test]
    fn window_lifecycle_lines() {
        assert_eq!(
            line("%window-add @1"),
            Some(TmuxNotification::WindowAdd {
                window_id: "@1".into(),
                window_layout: String::new(),
                window_visible_layout: String::new(),
                window_raw_flags: String::new()
            })
        );
        assert_eq!(
            line("%window-close @1"),
            Some(TmuxNotification::WindowClose {
                window_id: "@1".into()
            })
        );
        assert_eq!(
            line("%window-renamed @1 my win"),
            Some(TmuxNotification::WindowRenamed {
                window_id: "@1".into(),
                name: "my win".into()
            })
        );
        assert_eq!(line("%window-renamed @1"), None);
        assert_eq!(
            line("%window-pane-changed @1 %2"),
            Some(TmuxNotification::WindowPaneChanged {
                window_id: "@1".into(),
                pane_id: "%2".into()
            })
        );
        assert_eq!(line("%window-pane-changed @1"), None);
    }

    #[test]
    fn layout_change_needs_four_fields() {
        assert!(matches!(
            line("%layout-change @1 lay vis *"),
            Some(TmuxNotification::LayoutChange { window_raw_flags, .. }) if window_raw_flags == "*"
        ));
        assert_eq!(line("%layout-change @1 lay vis"), None);
    }

    #[test]
    fn pane_title_and_exit() {
        assert_eq!(
            line("%pane-title-changed %1 a b c"),
            Some(TmuxNotification::PaneTitleChanged {
                pane_id: "%1".into(),
                title: "a b c".into()
            })
        );
        assert!(matches!(
            line("%pane-title-changed %1"),
            Some(TmuxNotification::PaneTitleChanged { title, .. }) if title.is_empty()
        ));
        assert!(matches!(
            line("%pane-exited %1 7"),
            Some(TmuxNotification::PaneExited {
                exit_code: Some(7),
                ..
            })
        ));
        assert!(matches!(
            line("%pane-exited %1"),
            Some(TmuxNotification::PaneExited {
                exit_code: None,
                ..
            })
        ));
        assert_eq!(line("%pane-respawned"), None);
        assert!(matches!(
            line("%pane-respawned %4"),
            Some(TmuxNotification::PaneRespawned { .. })
        ));
    }

    #[test]
    fn session_and_client_lines() {
        assert_eq!(
            line("%session-changed $1 main"),
            Some(TmuxNotification::SessionChanged {
                session_id: "$1".into(),
                name: "main".into()
            })
        );
        assert_eq!(line("%session-changed $1"), None);
        assert!(matches!(
            line("%client-session-changed c1 $1 main one"),
            Some(TmuxNotification::ClientSessionChanged { name, .. }) if name == "main one"
        ));
        assert_eq!(line("%client-session-changed c1 $1"), None);
        assert_eq!(
            line("%session-window-changed $1 @2"),
            Some(TmuxNotification::SessionWindowChanged {
                session_id: "$1".into(),
                window_id: "@2".into()
            })
        );
        assert_eq!(
            line("%client-left c1 $1"),
            Some(TmuxNotification::ClientLeft {
                client: "c1".into(),
                session_id: Some("$1".into()),
                window_id: None
            })
        );
        assert_eq!(line("%client-attached"), None);
        assert_eq!(line("%client-left"), None);
    }

    #[test]
    fn subscription_and_paste_buffer_lines() {
        assert_eq!(
            line("%subscription-changed sub $1 @1 0 %1 : the value"),
            Some(TmuxNotification::SubscriptionChanged {
                name: "sub".into(),
                value: "the value".into()
            })
        );
        assert_eq!(
            line("%paste-buffer-changed buffer0"),
            Some(TmuxNotification::PasteBufferChanged {
                name: "buffer0".into()
            })
        );
        assert_eq!(
            line("%paste-buffer-deleted buffer0"),
            Some(TmuxNotification::PasteBufferDeleted {
                name: "buffer0".into()
            })
        );
    }

    #[test]
    fn desktop_notification_decodes_base64() {
        assert_eq!(
            line("%notification SGk= body=dGhlcmU="),
            Some(TmuxNotification::Notification {
                title: "Hi".into(),
                body: "there".into()
            })
        );
        assert_eq!(line("%notification"), None);
        assert_eq!(line("%notification !!!"), None);
    }

    #[test]
    fn agent_lines() {
        assert!(matches!(
            line("%agent-state-changed %1 claude working source=hook ws=w1"),
            Some(TmuxNotification::AgentStateChanged { state, source, ws, .. })
                if state == "working" && source == "hook" && ws == "w1"
        ));
        assert_eq!(line("%agent-state-changed %1 claude"), None);
        assert!(matches!(
            line("%agent-released %1 claude"),
            Some(TmuxNotification::AgentReleased { .. })
        ));
        assert_eq!(line("%agent-released %1"), None);
        assert!(matches!(
            line("%agent-telemetry-changed %1 claude"),
            Some(TmuxNotification::AgentTelemetryChanged { .. })
        ));
        assert_eq!(line("%agent-telemetry-changed %1 claude extra"), None);
    }
}
