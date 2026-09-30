//! Parser for the par-mux `list-agents` roster output.

use super::proto::pb::AgentEntry;
use base64::{engine::general_purpose::STANDARD, Engine as _};

/// Parses `list-agents` output into roster entries sorted by pane id.
///
/// Each line is `%N agent state hook|scrape [key=value ...]`. Lines without four
/// well-formed positional tokens are skipped. Only `reason=` (padded base64) is
/// consumed; an undecodable reason becomes empty and unknown keys are ignored.
pub fn parse_agents_output(raw: &str) -> Vec<AgentEntry> {
    let mut entries: Vec<AgentEntry> = raw.lines().filter_map(parse_line).collect();
    entries.sort_by_key(|e| e.pane_id);
    entries
}

/// A roster change derived from a single par-mux control notification.
#[derive(Debug, Clone, PartialEq)]
pub enum RosterDelta {
    Upsert(AgentEntry),
    Release { pane_id: u32 },
}

/// Translates one control-mode notification line into a roster delta.
///
/// Returns `None` for every notification that does not affect the roster.
pub fn translate_notification(line: &str) -> Option<RosterDelta> {
    let (verb, rest) = line.trim_start().split_once(char::is_whitespace)?;
    match verb {
        "%agent-state-changed" => parse_state_changed(rest).map(RosterDelta::Upsert),
        "%agent-released" | "%pane-exited" => {
            let pane_id = rest
                .split_whitespace()
                .next()?
                .trim_start_matches('%')
                .parse::<u32>()
                .ok()?;
            Some(RosterDelta::Release { pane_id })
        }
        _ => None,
    }
}

/// Parses the notification body `<pane_id> <agent> <state>[ source=<hook|scrape>]`.
///
/// The source rides as an optional `source=` tail token and is absent when the
/// daemon has no provenance. Reasons are never carried on this notification.
fn parse_state_changed(rest: &str) -> Option<AgentEntry> {
    let mut tokens = rest.split_whitespace();
    let pane_id = tokens.next()?.trim_start_matches('%').parse::<u32>().ok()?;
    let agent = tokens.next()?;
    let state = tokens.next()?;
    let mut source = "";
    for token in tokens {
        if let Some(("source", value)) = token.split_once('=') {
            if value != "hook" && value != "scrape" {
                return None;
            }
            source = value;
        }
    }
    Some(AgentEntry {
        pane_id,
        agent: agent.to_owned(),
        state: state.to_owned(),
        source: source.to_owned(),
        reason: String::new(),
    })
}

fn parse_line(line: &str) -> Option<AgentEntry> {
    let mut tokens = line.split_whitespace();
    let pane_id = tokens.next()?.trim_start_matches('%').parse::<u32>().ok()?;
    let agent = tokens.next()?;
    let state = tokens.next()?;
    let source = tokens.next()?;
    if source != "hook" && source != "scrape" {
        return None;
    }
    let mut reason = String::new();
    for token in tokens {
        if let Some(("reason", value)) = token.split_once('=') {
            reason = STANDARD
                .decode(value)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
        }
    }
    Some(AgentEntry {
        pane_id,
        agent: agent.to_owned(),
        state: state.to_owned(),
        source: source.to_owned(),
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_positional_and_keyvalue_fields() {
        let raw = "12 claude-code working hook\n\
                   15 claude-code blocked scrape reason=SGVsbG8=\n\
                   3 zed idle hook\n";
        let got = parse_agents_output(raw);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].pane_id, 3); // pane-id ascending
        assert_eq!(got[0].agent, "zed");
        assert_eq!(got[0].state, "idle");
        assert_eq!(got[0].source, "hook");
        assert_eq!(got[1].pane_id, 12);
        assert_eq!(got[1].reason, "");
        assert_eq!(got[2].pane_id, 15);
        assert_eq!(got[2].reason, "Hello"); // base64-decoded
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let raw = "not-a-number claude working hook\n\
                   12 claude\n\
                   12 claude working\n\
                   12 claude working reason=SGVsbG8=\n\
                   \n\
                   12 claude working hook\n";
        let got = parse_agents_output(raw);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pane_id, 12);
    }

    #[test]
    fn bad_base64_reason_drops_reason_not_the_row() {
        let raw = "12 claude blocked hook reason=!!!not-base64!!!\n";
        let got = parse_agents_output(raw);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].reason, ""); // undecodable reason → empty, row kept
    }

    #[test]
    fn unknown_keyvalue_tokens_are_ignored() {
        let raw = "12 claude working hook host_telemetry=e30= telemetry=e30=\n";
        let got = parse_agents_output(raw);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn state_changed_line_upserts_with_source() {
        let line = "%agent-state-changed %12 claude-code blocked source=hook";
        match translate_notification(line) {
            Some(RosterDelta::Upsert(e)) => {
                assert_eq!(e.pane_id, 12);
                assert_eq!(e.agent, "claude-code");
                assert_eq!(e.state, "blocked");
                assert_eq!(e.source, "hook");
                assert_eq!(e.reason, "");
            }
            other => panic!("expected upsert, got {other:?}"),
        }
    }

    #[test]
    fn state_changed_line_without_source_has_empty_source() {
        match translate_notification("%agent-state-changed %12 claude-code working") {
            Some(RosterDelta::Upsert(e)) => {
                assert_eq!(e.state, "working");
                assert_eq!(e.source, "");
            }
            other => panic!("expected upsert, got {other:?}"),
        }
    }

    #[test]
    fn state_changed_with_bogus_source_or_short_is_none() {
        assert!(
            translate_notification("%agent-state-changed %12 claude blocked source=bogus")
                .is_none()
        );
        assert!(translate_notification("%agent-state-changed %12 claude").is_none());
    }

    #[test]
    fn released_and_exited_lines_release() {
        assert!(matches!(
            translate_notification("%agent-released %12 claude-code"),
            Some(RosterDelta::Release { pane_id: 12 })
        ));
        assert!(matches!(
            translate_notification("%pane-exited %12 0"),
            Some(RosterDelta::Release { pane_id: 12 })
        ));
    }

    #[test]
    fn non_agent_notifications_are_none() {
        assert!(translate_notification("%output %12 AbCd").is_none());
        assert!(translate_notification("%layout-change ...").is_none());
        assert!(translate_notification("%begin 1 1 1").is_none());
    }
}
