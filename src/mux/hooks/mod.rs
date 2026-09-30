//! Hook reports: the agent layer's second grammar on the control socket.
//!
//! herdr's integration scripts (measured: `kimi/herdr-agent-state.sh`) open
//! the control socket, send ONE JSON line — `{"id":…,"method":…,
//! "params":{…}}` — read one reply, and close. par-mux accepts herdr's two
//! methods verbatim (`pane.report_agent`, `pane.report_agent_session`) so
//! those scripts port with an env-var rename (`HERDR_*` → `PAR_MUX_*`), plus
//! `pane.report_agent_telemetry` (display-only telemetry) and
//! `pane.release_agent` (claim teardown); the server's client loop routes
//! every line
//! [`parse_line`](crate::mux::command::parse_line) classifies as a hook
//! report here, and anything else remains a tmux control command.
//!
//! Authorization is the socket's own `0600` owner-only boundary: a hook
//! claiming another pane's id is same-user by construction, the same trust
//! model tmux control mode has (a recorded Phase 5 decision, not an
//! omission). Reports are answered with one JSON line on the same
//! connection; hook connections never join the broadcast set.
//!
//! Accepted reports write agent state into [`crate::mux::pane::MuxPane`'s
//! metadata] (seam S2) and broadcast `%agent-state-changed` (seam S3) —
//! except out-of-order reports, which herdr's monotonic-`seq` rule drops
//! with no write and no broadcast.

mod release;
mod report;
mod telemetry;
#[cfg(test)]
mod tests;

pub(crate) use telemetry::{fresh_telemetry_b64, StoredTelemetry, TELEMETRY_FRESHNESS_MS};

use release::handle_release_report;
use report::{handle_session_report, handle_state_report};
use telemetry::handle_telemetry_report;

use crate::mux::ids::PaneId;
use crate::mux::pane::MuxPane;
use crate::mux::tree::MuxTree;
use crate::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use std::str::FromStr;
use std::sync::Arc;

/// Dispatch one hook-report line.
///
/// Returns the JSON reply for the reporting connection and, when the report
/// was accepted AND carries a state worth announcing, the notification to
/// broadcast to control clients. The caller writes the reply directly and
/// owns the broadcast, so this function takes no client set and holds no
/// locks of its own — the tree lock is taken once, inside, and released
/// before the notification leaves.
pub fn handle_report(line: &str, tree: &Arc<Mutex<MuxTree>>) -> (String, Option<TmuxNotification>) {
    let report: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(err) => return (error_reply(None, &format!("invalid JSON: {err}")), None),
    };
    let id = report.get("id").cloned();
    let method = report
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let Some(params) = report.get("params") else {
        return (error_reply(id, "missing params"), None);
    };

    match method {
        "pane.report_agent" => handle_state_report(id, params, tree),
        "pane.report_agent_session" => handle_session_report(id, params, tree),
        "pane.report_agent_telemetry" => handle_telemetry_report(id, params, tree),
        "pane.release_agent" => handle_release_report(id, params, tree),
        other => (error_reply(id, &format!("unknown method: {other}")), None),
    }
}

/// The fields every report carries: the pane it claims, the agent label,
/// and the monotonic sequence number that orders reports.
struct ReportHeader {
    pane_id: PaneId,
    agent: String,
    seq: u64,
    source: Option<String>,
}

/// Free-text report values are bounded (SEC-105): every one of them
/// persists into pane metadata and the on-disk state file, and the label
/// also rides broadcast lines, so an unvalidated value otherwise pins up
/// to the full SEC-104 line budget per field per pane. 4 KiB clears any
/// legitimate label, path, or blocked reason with orders of magnitude to
/// spare.
/// cap: Bytes accepted for one hook or agent report value sent from a pane.
const MAX_REPORT_VALUE_LEN: usize = 4096;

fn check_value_len(field: &str, value: &str) -> Result<(), String> {
    if value.len() > MAX_REPORT_VALUE_LEN {
        Err(format!(
            "{field} exceeds {MAX_REPORT_VALUE_LEN} bytes, report rejected"
        ))
    } else {
        Ok(())
    }
}

fn parse_header(params: &serde_json::Value) -> Result<ReportHeader, String> {
    let pane_id = params
        .get("pane_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing pane_id".to_string())
        .and_then(|raw| PaneId::from_str(raw).map_err(|err| format!("bad pane_id: {err}")))?;
    let agent = params
        .get("agent")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .ok_or_else(|| "missing agent".to_string())?
        .to_string();
    // The label is interpolated verbatim into the space-split
    // `%agent-state-changed` and roster lines: inner whitespace breaks the
    // shape every consumer parses, and a control character (a newline
    // above all) forges a control-mode line delivered to every client.
    // Any process in any pane can reach this endpoint, so the door is here.
    if agent.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("agent label must not contain whitespace or control characters".to_string());
    }
    check_value_len("agent label", &agent)?;
    let seq = params
        .get("seq")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "missing seq".to_string())?;
    let source = params
        .get("source")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    // The source tag shares the metadata-and-emission surface; control
    // characters are rejected for the same forging reason (colons and
    // other printable punctuation are fine — real tags read
    // `par-mux:claude:session-hook`).
    if source
        .as_deref()
        .is_some_and(|source| source.chars().any(char::is_control))
    {
        return Err("source must not contain control characters".to_string());
    }
    if let Some(source) = source.as_deref() {
        check_value_len("source", source)?;
    }
    Ok(ReportHeader {
        pane_id,
        agent,
        seq,
        source,
    })
}

/// Every metadata key that constitutes an agent claim — roster label,
/// state, hook authority, session identity, and the liveness miss counter
/// the sweep keeps. The typed parts of the claim (telemetry, host
/// telemetry, per-source sequence stamps) live on [`MuxPane`] fields;
/// [`clear_agent_claim`] clears both as one unit.
pub(crate) const AGENT_CLAIM_KEYS: &[&str] = &[
    "agent",
    "agent_state",
    "agent_state_source",
    "agent_message",
    "agent_source",
    "agent_seq",
    "agent_session_id",
    "agent_session_path",
    "agent_session_start_source",
    "agent_resume_argv",
    "agent_liveness_misses",
    "agent_liveness_misses_agent",
];

/// Clear a pane's whole agent claim — the metadata keys and the typed
/// telemetry and sequence state — as `pane.release_agent` and the scrape
/// tick's liveness sweep (`scrape.rs`) both do.
pub(crate) fn clear_agent_claim(pane: &mut MuxPane) {
    pane.clear_metadata(AGENT_CLAIM_KEYS);
    pane.telemetry = None;
    pane.host_telemetry = None;
    pane.seq_by_source.clear();
}

/// Whether `seq` is at or below the last accepted report from the same
/// source — the stale side of herdr's ordering rule. Freshness is tracked
/// per reporting source because the sources do not share a clock: the
/// claude/codex/grok hooks stamp `time.time_ns()` while the pi/omp
/// extensions stamp `Date.now()*1000`, three orders of magnitude apart —
/// one per-pane stamp would drop every pi/omp report filed after any
/// claude/codex/grok report.
fn is_stale(pane: &MuxPane, source: Option<&str>, seq: u64) -> bool {
    match pane.seq_by_source.get(source.unwrap_or("")) {
        Some(&stored) => seq <= stored,
        // No accepted report from this source yet means nothing to be
        // stale against.
        None => false,
    }
}

/// Record `seq` as accepted: the plain `agent_seq` (the most recent
/// report, whatever its source) and the reporting source's own bucket in
/// [`MuxPane::seq_by_source`]. Volatile like everything state-shaped — the
/// save format copies named identity fields only, so the buckets never
/// reach disk.
fn record_seq(pane: &mut MuxPane, source: Option<&str>, seq: u64) {
    pane.set_metadata("agent_seq", &seq.to_string());
    pane.seq_by_source
        .insert(source.unwrap_or("").to_string(), seq);
}

/// `{"id":…,"result":"ok"}` — the reply shape herdr's scripts expect (and
/// ignore), echoed id included.
fn ok_reply(id: Option<serde_json::Value>) -> String {
    let mut reply = serde_json::Map::new();
    if let Some(id) = id {
        reply.insert("id".to_string(), id);
    }
    reply.insert(
        "result".to_string(),
        serde_json::Value::String("ok".to_string()),
    );
    format!("{}\n", serde_json::Value::Object(reply))
}

fn error_reply(id: Option<serde_json::Value>, message: &str) -> String {
    let mut reply = serde_json::Map::new();
    if let Some(id) = id {
        reply.insert("id".to_string(), id);
    }
    reply.insert(
        "error".to_string(),
        serde_json::Value::String(message.to_string()),
    );
    format!("{}\n", serde_json::Value::Object(reply))
}
