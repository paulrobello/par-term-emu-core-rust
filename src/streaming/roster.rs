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

fn parse_line(line: &str) -> Option<AgentEntry> {
    let mut tokens = line.split_whitespace();
    let pane_id = tokens.next()?.trim_start_matches('%').parse::<u32>().ok()?;
    let agent = tokens.next()?;
    let state = tokens.next()?;
    let source = tokens.next()?;
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
}
