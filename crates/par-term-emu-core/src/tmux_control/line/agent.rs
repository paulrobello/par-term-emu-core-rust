//! par-mux agent roster notifications.

use crate::tmux_control::TmuxNotification;

pub(super) fn parse_agent_state_changed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    // Trailing `key=value` tokens are metadata, never state. Index 2 is
    // always state (a three-token line keeps `source=hook` as its
    // state), and unknown keys are ignored so a future token never
    // folds into it.
    let mut end = parts.len();
    let (mut source, mut ws, mut msg) = (String::new(), String::new(), String::new());
    while end > 3 {
        let Some((key, value)) = parts[end - 1].split_once('=') else {
            break;
        };
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            break;
        }
        match key {
            "source" => source = value.to_string(),
            // The blocked reason rides the standard base64 of the
            // whitespace-collapsed text (any text, one token);
            // malformed base64 or non-UTF-8 stays empty — the
            // consumer renders no message, never garbage.
            "msg" => {
                use base64::Engine as _;
                msg = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .unwrap_or_default()
            }
            "ws" => ws = value.to_string(),
            _ => {}
        }
        end -= 1;
    }
    Some(TmuxNotification::AgentStateChanged {
        pane_id: parts[0].to_string(),
        agent: parts[1].to_string(),
        // The state is the rest of the line, so a multi-word state
        // survives the round trip.
        state: parts[2..end].join(" "),
        source,
        ws,
        msg,
    })
}

pub(super) fn parse_agent_released(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }
    Some(TmuxNotification::AgentReleased {
        pane_id: parts[0].to_string(),
        agent: parts[1].to_string(),
    })
}

/// `%agent-telemetry-changed {pane_id} {agent}` — the same two-token
/// identity shape as `%agent-released`; the values live on the roster,
/// not the push.
pub(super) fn parse_agent_telemetry_changed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }
    Some(TmuxNotification::AgentTelemetryChanged {
        pane_id: parts[0].to_string(),
        agent: parts[1].to_string(),
    })
}
