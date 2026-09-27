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

use crate::mux::ids::PaneId;
use crate::mux::pane::MuxPane;
use crate::mux::tree::MuxTree;
use crate::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// `pane.report_agent`: state (working/blocked/idle), plus whatever session
/// identity the hook happens to know.
fn handle_state_report(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    tree: &Arc<Mutex<MuxTree>>,
) -> (String, Option<TmuxNotification>) {
    let header = match parse_header(params) {
        Ok(header) => header,
        Err(message) => return (error_reply(id, &message), None),
    };
    let Some(state) = params
        .get("state")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|state| !state.is_empty())
    else {
        return (error_reply(id, "missing state"), None);
    };
    // herdr's fixed set, verbatim: anything else would ride the roster and
    // broadcast lines as a word consumers cannot parse (or, with a
    // newline, forge whole ones).
    if !matches!(state, "working" | "blocked" | "idle" | "unknown") {
        return (
            error_reply(
                id,
                &format!("invalid state: {state} (working, blocked, idle, or unknown)"),
            ),
            None,
        );
    }
    // `unknown` is the absence of a hook claim, not a state — never written,
    // never broadcast (the Phase 5 ruling that keeps an agent without hooks
    // out of the roster rather than misreporting it).
    if state == "unknown" {
        return (ok_reply(id), None);
    }

    // Free-text length validation happens before the tree lock: a rejection
    // must write nothing — not even the sequence stamp — or a rejected
    // report would poison the pane's ordering for its seq (SEC-105).
    for field in ["agent_session_id", "agent_session_path", "message"] {
        if let Some(value) = params.get(field).and_then(serde_json::Value::as_str) {
            if let Err(message) = check_value_len(field, value) {
                return (error_reply(id, &message), None);
            }
        }
    }

    let notification = {
        let mut guard = tree.lock();
        let Some(pane) = guard.pane_mut(header.pane_id) else {
            return (
                error_reply(id, &format!("no such pane: {}", header.pane_id)),
                None,
            );
        };
        // herdr's out-of-order rule: a report at or below the last accepted
        // sequence number from the same source is dropped — no metadata
        // write, no broadcast.
        if is_stale(pane.metadata(), header.source.as_deref(), header.seq) {
            return (ok_reply(id), None);
        }

        pane.set_metadata("agent", &header.agent);
        pane.set_metadata("agent_state", state);
        // A hook state report makes this pane hook-authoritative from now
        // on: the scrape tier skips it forever after (the structural
        // precedence rule — a claim is never overwritten by a guess).
        pane.set_metadata("agent_state_source", "hook");
        record_seq(pane, header.source.as_deref(), header.seq);
        if let Some(source) = &header.source {
            pane.set_metadata("agent_source", source);
        }
        // Identity fields ride state reports too: the kimi script attaches
        // agent_session_id whenever it knows one, state report or not.
        for field in ["agent_session_id", "agent_session_path"] {
            if let Some(value) = params.get(field).and_then(serde_json::Value::as_str) {
                pane.set_metadata(field, value);
            }
        }
        // The blocked reason pi and omp attach to their reports (the
        // scannability field the roster exists for): stored when present,
        // CLEARED when a later report omits it, so a stale reason cannot
        // survive into a new state. Whitespace is collapsed at the door —
        // the roster line is one line.
        let message = params
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(|message| message.split_whitespace().collect::<Vec<_>>().join(" "));
        match message {
            Some(message) => pane.set_metadata("agent_message", &message),
            None => pane.clear_metadata(&["agent_message"]),
        }

        Some(TmuxNotification::AgentStateChanged {
            pane_id: header.pane_id.to_string(),
            agent: header.agent.clone(),
            state: state.to_string(),
            source: "hook".to_string(),
        })
    };
    (ok_reply(id), notification)
}

/// `pane.report_agent_session`: session identity for the resume path
/// (Phase 6). Carries no state, so it broadcasts only what a prior state
/// report already established.
///
/// A session is identified by EITHER its id or its transcript path: herdr's
/// `session_ref_from_report` accepts both, and the shipped pi/omp assets
/// prefer the path and drop the id when one exists — requiring the id
/// error-replies every path-carrying session report those two send.
fn handle_session_report(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    tree: &Arc<Mutex<MuxTree>>,
) -> (String, Option<TmuxNotification>) {
    let header = match parse_header(params) {
        Ok(header) => header,
        Err(message) => return (error_reply(id, &message), None),
    };
    let session_id = params
        .get("agent_session_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let session_path = params
        .get("agent_session_path")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if session_id.is_none() && session_path.is_none() {
        return (
            error_reply(id, "missing agent_session_id or agent_session_path"),
            None,
        );
    }
    if let Some(value) = session_id {
        if let Err(message) = check_value_len("agent_session_id", value) {
            return (error_reply(id, &message), None);
        }
    }
    if let Some(value) = session_path {
        if let Err(message) = check_value_len("agent_session_path", value) {
            return (error_reply(id, &message), None);
        }
    }
    if let Some(start) = params
        .get("session_start_source")
        .and_then(serde_json::Value::as_str)
    {
        if let Err(message) = check_value_len("session_start_source", start) {
            return (error_reply(id, &message), None);
        }
    }
    let resume_argv = match parse_resume_argv(params) {
        Ok(argv) => argv,
        Err(message) => return (error_reply(id, &message), None),
    };

    let notification = {
        let mut guard = tree.lock();
        let Some(pane) = guard.pane_mut(header.pane_id) else {
            return (
                error_reply(id, &format!("no such pane: {}", header.pane_id)),
                None,
            );
        };
        if is_stale(pane.metadata(), header.source.as_deref(), header.seq) {
            return (ok_reply(id), None);
        }

        // Prior identity, captured before the writes below replace it: a
        // report that moves the pane to a DIFFERENT session without a fresh
        // invocation must not leave the old session's argv behind.
        let prior_session_id = pane.metadata().get("agent_session_id").cloned();
        let prior_session_path = pane.metadata().get("agent_session_path").cloned();

        // A report that moves the pane to a DIFFERENT agent ends the
        // previous agent's claim: its state, hook authority, and blocked
        // reason must not survive — and above all must not be rebroadcast
        // under the new label as though the new agent had claimed it (the
        // rebroadcast below reads `agent_state`, which this just removed).
        if pane
            .metadata()
            .get("agent")
            .map(String::as_str)
            .is_some_and(|label| label != header.agent)
        {
            pane.clear_metadata(&["agent_state", "agent_state_source", "agent_message"]);
        }

        pane.set_metadata("agent", &header.agent);
        record_seq(pane, header.source.as_deref(), header.seq);
        if let Some(source) = &header.source {
            pane.set_metadata("agent_source", source);
        }
        if let Some(session_id) = &session_id {
            pane.set_metadata("agent_session_id", session_id);
        }
        if let Some(session_path) = &session_path {
            pane.set_metadata("agent_session_path", session_path);
        }
        // The resume path's provenance (startup vs resume), recorded now
        // because the wire carries it now — Phase 6 persists it.
        if let Some(start) = params
            .get("session_start_source")
            .and_then(serde_json::Value::as_str)
        {
            pane.set_metadata("agent_session_start_source", start);
        }
        // The agent's own resume invocation, stored as a JSON argv string:
        // Phase 6 spawns it verbatim (hook-first; the per-agent table is the
        // fallback for agents that cannot report one). Absent on a report
        // that also moves to a different session means the stored argv is
        // the OLD session's — clear it rather than resume the wrong session.
        match &resume_argv {
            Some(argv) => pane.set_metadata("agent_resume_argv", argv),
            None => {
                let changed = session_id
                    .is_some_and(|value| prior_session_id.as_deref() != Some(value))
                    || session_path
                        .is_some_and(|value| prior_session_path.as_deref() != Some(value));
                if changed {
                    pane.clear_metadata(&["agent_resume_argv"]);
                }
            }
        }

        // The rebroadcast keeps the state's OWN provenance: a claude-shaped
        // pane (identity by hook, state by scrape) must not relabel a
        // scrape guess as a hook claim on its session reports.
        let state_source = pane
            .metadata()
            .get("agent_state_source")
            .cloned()
            .unwrap_or_else(|| "hook".to_string());
        pane.metadata()
            .get("agent_state")
            .map(|state| TmuxNotification::AgentStateChanged {
                pane_id: header.pane_id.to_string(),
                agent: header.agent.clone(),
                state: state.clone(),
                source: state_source,
            })
    };
    (ok_reply(id), notification)
}

/// Every metadata key that constitutes an agent claim, cleared as one unit
/// by `pane.release_agent` and by the scrape tick's liveness sweep
/// (`scrape.rs`) alike — roster label, state, hook authority, sequence
/// stamps, session identity, the ephemeral telemetry blob, and the
/// liveness miss counter the sweep keeps.
pub(crate) const AGENT_CLAIM_KEYS: &[&str] = &[
    "agent",
    "agent_state",
    "agent_state_source",
    "agent_message",
    "agent_source",
    "agent_seq",
    SEQ_STAMPS_KEY,
    "agent_session_id",
    "agent_session_path",
    "agent_session_start_source",
    "agent_resume_argv",
    "agent_telemetry",
    "agent_liveness_misses",
    "agent_liveness_misses_agent",
];

/// `pane.release_agent`: the claiming agent announces it is gone (herdr's
/// SessionEnd shape). The claim — label, state, hook authority, blocked
/// reason, sequence stamps, and session identity — is cleared and the
/// removal broadcast, so the roster drops the pane instead of showing a
/// dead agent "working" until the pane dies, and a restart respawns the
/// pane's original command rather than a resume invocation for a session
/// that no longer has a live agent.
///
/// Guards: the releasing agent must match the pane's current label (a
/// stale hook from a different agent cannot wipe a live claim), and the
/// report must clear the same monotonic-`seq` rule as every other report.
/// Both failing guards are silent ok no-ops, exactly like a stale report.
fn handle_release_report(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    tree: &Arc<Mutex<MuxTree>>,
) -> (String, Option<TmuxNotification>) {
    let header = match parse_header(params) {
        Ok(header) => header,
        Err(message) => return (error_reply(id, &message), None),
    };
    let notification = {
        let mut guard = tree.lock();
        let Some(pane) = guard.pane_mut(header.pane_id) else {
            return (
                error_reply(id, &format!("no such pane: {}", header.pane_id)),
                None,
            );
        };
        if is_stale(pane.metadata(), header.source.as_deref(), header.seq) {
            return (ok_reply(id), None);
        }
        if pane.metadata().get("agent").map(String::as_str) != Some(header.agent.as_str()) {
            return (ok_reply(id), None);
        }
        pane.clear_metadata(AGENT_CLAIM_KEYS);
        Some(TmuxNotification::AgentReleased {
            pane_id: header.pane_id.to_string(),
            agent: header.agent.clone(),
        })
    };
    (ok_reply(id), notification)
}

/// The metadata key holding the pane's accepted telemetry as one canonical
/// JSON object (the validated v1 shape below) — display-only, so the
/// persistence format's named-key capture never copies it.
const TELEMETRY_KEY: &str = "agent_telemetry";

/// How old a telemetry sample may be, in milliseconds of wall clock,
/// before the endpoint drops it: the hub's `STATUSLINE_FRESHNESS_SECONDS`
/// (55 min, par-remote-herd status_telemetry.py), mirrored so daemon and
/// hub age data out at the same rate. Absent beats stale — a dropped
/// sample leaves readers serving nothing rather than something expired.
const TELEMETRY_FRESHNESS_MS: u64 = 55 * 60 * 1000;

/// `pane.report_agent_telemetry`: versioned, bounded agent telemetry —
/// model, effort, context and rate-limit percents — for roster display
/// (card 01a0e3f11c367e62873a8ada8833e6f8, the iOS client's HerdDeck
/// parity enabler). The producer is a hook tailing the agent harness's
/// own status file; par-mux never parses transcripts. The object's shape
/// mirrors the hub's normalized telemetry (`_normalize_claude_record`):
/// `version` (currently 1), the data `source` that distinguishes
/// hook-reported from daemon-probed, `sampled_at_unix_ms`, and optional
/// bounded fields — strings capped (`model` 128, `effort` 32), percents
/// 0-100 rounded, no control characters.
///
/// Like every report it carries the common header and clears the
/// per-source `seq` rule. Four silent drops, no write and no broadcast
/// (the roster query owns serving; a notification lands with it, card
/// 01a0e3f11e287f038dadcf332a1af961): a sample past the freshness window,
/// a report at or below the last accepted `seq`, a sample older than
/// the one already stored (a backward step, whatever its `seq`), and a
/// report whose agent is not the pane's current label (telemetry attaches
/// to a claim; it never takes one over).
fn handle_telemetry_report(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    tree: &Arc<Mutex<MuxTree>>,
) -> (String, Option<TmuxNotification>) {
    let header = match parse_header(params) {
        Ok(header) => header,
        Err(message) => return (error_reply(id, &message), None),
    };
    let (telemetry, sampled_at) = match parse_telemetry_object(params) {
        Ok(parsed) => parsed,
        Err(message) => return (error_reply(id, &message), None),
    };

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(u64::MAX);

    let mut guard = tree.lock();
    let Some(pane) = guard.pane_mut(header.pane_id) else {
        return (
            error_reply(id, &format!("no such pane: {}", header.pane_id)),
            None,
        );
    };
    if is_stale(pane.metadata(), header.source.as_deref(), header.seq) {
        return (ok_reply(id), None);
    }
    if now_ms.saturating_sub(sampled_at) > TELEMETRY_FRESHNESS_MS {
        return (ok_reply(id), None);
    }
    if stored_telemetry_sampled_at(pane.metadata()).is_some_and(|stored| sampled_at < stored) {
        return (ok_reply(id), None);
    }
    // Telemetry attaches to a claim; it never takes one over. A pane
    // already labeled for a DIFFERENT agent keeps that label and its
    // telemetry — the release guard's rule, so a stale hook from a
    // previous agent cannot mislabel a live claim's roster row.
    if pane
        .metadata()
        .get("agent")
        .map(String::as_str)
        .is_some_and(|label| label != header.agent)
    {
        return (ok_reply(id), None);
    }

    // The telemetry's own label rides an unclaimed pane like a session
    // report's would; state itself is never touched — telemetry alone
    // never puts a pane on the roster.
    pane.set_metadata("agent", &header.agent);
    record_seq(pane, header.source.as_deref(), header.seq);
    if let Some(source) = &header.source {
        pane.set_metadata("agent_source", source);
    }
    pane.set_metadata(TELEMETRY_KEY, &telemetry);
    (ok_reply(id), None)
}

/// Validate the `telemetry` object into its canonical stored form — the
/// same fields re-serialized from validated values, so nothing unbounded
/// or unknown reaches pane metadata. Mirrors the hub normalizer's bounds:
/// `model` ≤ 128 chars, `effort` ≤ 32, percents finite 0-100 rounded,
/// strings trimmed, non-empty, control-character-free.
fn parse_telemetry_object(params: &serde_json::Value) -> Result<(String, u64), String> {
    let object = match params.get("telemetry") {
        Some(value) => value
            .as_object()
            .ok_or_else(|| "telemetry must be an object".to_string())?,
        None => return Err("missing telemetry".to_string()),
    };

    let version = match object.get("version") {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| "telemetry version must be a non-negative integer".to_string())?,
        None => return Err("telemetry missing version".to_string()),
    };
    if version != 1 {
        return Err(format!(
            "unsupported telemetry version: {version} (expected 1)"
        ));
    }

    let source = match object.get("source") {
        Some(value) => value
            .as_str()
            .ok_or_else(|| "telemetry source must be a string".to_string())?,
        None => return Err("telemetry missing source".to_string()),
    };
    let source = source.trim();
    if source.is_empty() {
        return Err("telemetry source must not be empty".to_string());
    }
    if source.chars().any(char::is_control) {
        return Err("telemetry source must not contain control characters".to_string());
    }
    check_value_len("telemetry source", source)?;

    let sampled_at = match object.get("sampled_at_unix_ms") {
        Some(value) => value.as_u64().ok_or_else(|| {
            "telemetry sampled_at_unix_ms must be a non-negative integer".to_string()
        })?,
        None => return Err("telemetry missing sampled_at_unix_ms".to_string()),
    };

    let model = bounded_string(object, "model", 128)?;
    let effort = bounded_string(object, "effort", 32)?;
    let thinking_enabled = match object.get("thinking_enabled") {
        Some(value) => Some(
            value
                .as_bool()
                .ok_or_else(|| "telemetry thinking_enabled must be a boolean".to_string())?,
        ),
        None => None,
    };
    let context_used = parse_percent(object, "context_used_percent")?;
    let context_remaining = parse_percent(object, "context_remaining_percent")?;
    let five_hour_remaining = parse_percent(object, "five_hour_remaining_percent")?;
    let seven_day_remaining = parse_percent(object, "seven_day_remaining_percent")?;
    let five_hour_resets = unix_ms_field(object, "five_hour_resets_at_unix_ms")?;
    let seven_day_resets = unix_ms_field(object, "seven_day_resets_at_unix_ms")?;

    let mut validated = serde_json::Map::new();
    validated.insert("version".to_string(), serde_json::Value::from(1));
    validated.insert(
        "source".to_string(),
        serde_json::Value::String(source.to_string()),
    );
    validated.insert(
        "sampled_at_unix_ms".to_string(),
        serde_json::Value::from(sampled_at),
    );
    if let Some(model) = model {
        validated.insert("model".to_string(), serde_json::Value::String(model));
    }
    if let Some(effort) = effort {
        validated.insert("effort".to_string(), serde_json::Value::String(effort));
    }
    if let Some(thinking_enabled) = thinking_enabled {
        validated.insert(
            "thinking_enabled".to_string(),
            serde_json::Value::Bool(thinking_enabled),
        );
    }
    for (field, percent) in [
        ("context_used_percent", context_used),
        ("context_remaining_percent", context_remaining),
        ("five_hour_remaining_percent", five_hour_remaining),
        ("seven_day_remaining_percent", seven_day_remaining),
    ] {
        if let Some(percent) = percent {
            validated.insert(field.to_string(), serde_json::Value::from(percent));
        }
    }
    for (field, resets) in [
        ("five_hour_resets_at_unix_ms", five_hour_resets),
        ("seven_day_resets_at_unix_ms", seven_day_resets),
    ] {
        if let Some(resets) = resets {
            validated.insert(field.to_string(), serde_json::Value::from(resets));
        }
    }

    let canonical = serde_json::Value::Object(validated).to_string();
    check_value_len("telemetry", &canonical)?;
    Ok((canonical, sampled_at))
}

/// One optional bounded string field: trimmed, non-empty, printable-ish
/// (control characters rejected — the forging rule every interpolated
/// field follows), and capped at `max_chars`.
fn bounded_string(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    max_chars: usize,
) -> Result<Option<String>, String> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| format!("telemetry {field} must be a string"))?
        .trim();
    if value.is_empty() {
        return Err(format!("telemetry {field} must not be empty"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!(
            "telemetry {field} must not contain control characters"
        ));
    }
    if value.chars().count() > max_chars {
        return Err(format!(
            "telemetry {field} exceeds {max_chars} characters, report rejected"
        ));
    }
    Ok(Some(value.to_string()))
}

/// One optional percent field: any finite JSON number 0-100, rounded to
/// the nearest integer exactly as the hub normalizer rounds.
fn parse_percent(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<u64>, String> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    let number = value
        .as_f64()
        .ok_or_else(|| format!("telemetry {field} must be a number"))?;
    if !(0.0..=100.0).contains(&number) {
        return Err(format!("telemetry {field} must be between 0 and 100"));
    }
    Ok(Some(number.round() as u64))
}

/// One optional unix-milliseconds timestamp field (the rate-limit reset
/// horizons). Future by nature, so no freshness applies — only that it is
/// a non-negative integer.
fn unix_ms_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<u64>, String> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| format!("telemetry {field} must be a non-negative integer"))
}

/// The `sampled_at_unix_ms` inside the pane's stored telemetry blob, for
/// the backward-step drop. A blob that fails to parse reads as absent, so
/// a hand-corrupted entry costs one report's ordering, not the endpoint.
fn stored_telemetry_sampled_at(
    metadata: &std::collections::HashMap<String, String>,
) -> Option<u64> {
    metadata
        .get(TELEMETRY_KEY)
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| {
            value
                .get("sampled_at_unix_ms")
                .and_then(serde_json::Value::as_u64)
        })
}

/// `session_resume_argv`: the agent's own resume invocation as argv —
/// `["pi","--session","<path>"]`, the shapes herdr's per-agent table encodes,
/// reported by the agents that know theirs so par-mux needs no table entry
/// for them (card 01a0c766bc567c30bc429c4e380554d3; the key this function
/// settles is the one Phase 6 task 6.2 keys its override arm on). Absent is
/// fine; present-but-malformed is an error so a broken script hears about
/// it instead of silently losing its resume path.
fn parse_resume_argv(params: &serde_json::Value) -> Result<Option<String>, String> {
    let Some(value) = params.get("session_resume_argv") else {
        return Ok(None);
    };
    let argv: Vec<String> = value
        .as_array()
        .ok_or("session_resume_argv must be an array of strings")?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "session_resume_argv must be an array of strings".to_string())
        })
        .collect::<Result<_, _>>()?;
    if argv.is_empty() {
        return Err("session_resume_argv must not be empty".to_string());
    }
    let encoded =
        serde_json::to_string(&argv).map_err(|err| format!("session_resume_argv: {err}"))?;
    check_value_len("session_resume_argv", &encoded)?;
    Ok(Some(encoded))
}

/// Whether `seq` is at or below the last accepted report from the same
/// source — the stale side of herdr's ordering rule. Freshness is tracked
/// per reporting source because the sources do not share a clock: the
/// claude/codex/grok hooks stamp `time.time_ns()` while the pi/omp
/// extensions stamp `Date.now()*1000`, three orders of magnitude apart —
/// one per-pane stamp would drop every pi/omp report filed after any
/// claude/codex/grok report.
fn is_stale(
    metadata: &std::collections::HashMap<String, String>,
    source: Option<&str>,
    seq: u64,
) -> bool {
    match seq_stamps(metadata).get(source.unwrap_or("")) {
        Some(&stored) => seq <= stored,
        // No accepted report from this source yet (or a hand-corrupted
        // stamp) means nothing to be stale against.
        None => false,
    }
}

/// Metadata key holding the per-source sequence stamps as a JSON object
/// (`{"<source>": <seq>}`) — herdr's `hook_report_sequences` map,
/// flattened into the pane's stringly metadata. Reports without a source
/// share the empty-string bucket.
const SEQ_STAMPS_KEY: &str = "agent_seq_by_source";

/// The pane's per-source sequence stamps. A map that fails to parse reads
/// as empty, so a hand-corrupted entry costs staleness ordering, not
/// reports.
fn seq_stamps(
    metadata: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, u64> {
    metadata
        .get(SEQ_STAMPS_KEY)
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default()
}

/// Record `seq` as accepted: the plain `agent_seq` (the most recent
/// report, whatever its source) and the reporting source's own bucket.
/// Volatile like everything state-shaped — the save format copies named
/// identity fields only, so the bucket map never reaches disk.
fn record_seq(pane: &mut MuxPane, source: Option<&str>, seq: u64) {
    pane.set_metadata("agent_seq", &seq.to_string());
    let mut stamps = seq_stamps(pane.metadata());
    stamps.insert(source.unwrap_or("").to_string(), seq);
    if let Ok(encoded) = serde_json::to_string(&stamps) {
        pane.set_metadata(SEQ_STAMPS_KEY, &encoded);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::pane::ShellPaneFactory;

    /// A tree with one session and its first pane, wrapped the way the
    /// server holds one.
    fn tree_with_pane() -> (Arc<Mutex<MuxTree>>, PaneId) {
        let mut tree = MuxTree::new(Box::new(ShellPaneFactory::default()));
        let session = tree
            .new_session("hooks", 80, 24)
            .expect("test session spawns");
        let pane_id = tree
            .session(session)
            .expect("session exists")
            .windows
            .iter()
            .filter_map(|window| tree.window(*window))
            .flat_map(|window| window.panes())
            .next()
            .expect("a new session has a pane");
        (Arc::new(Mutex::new(tree)), pane_id)
    }

    fn state_report(pane: PaneId, agent: &str, state: &str, seq: u64) -> String {
        format!(
            r#"{{"id":"t-{seq}","method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"{agent}","state":"{state}","seq":{seq},"source":"par-mux:test"}}}}"#
        )
    }

    /// A pi/omp-shaped state report: same grammar, plus the optional
    /// blocked-reason `message` field both send.
    fn state_report_with_message(
        pane: PaneId,
        agent: &str,
        state: &str,
        message: &str,
        seq: u64,
    ) -> String {
        format!(
            r#"{{"id":"t-{seq}","method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"{agent}","state":"{state}","message":"{message}","seq":{seq},"source":"par-mux:test"}}}}"#
        )
    }

    #[test]
    fn a_blocked_reason_is_stored_collapsed_and_cleared_by_the_next_bare_report() {
        let (tree, pane_id) = tree_with_pane();

        // The JSON carries the reason with an escaped newline — how the
        // wire form of a multi-line reason arrives — which the endpoint
        // collapses at the door.
        let (reply, _) = handle_report(
            &state_report_with_message(
                pane_id,
                "pi",
                "blocked",
                "permission needed\\n  for  rm -rf build/",
                1_000,
            ),
            &tree,
        );
        assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
        {
            let guard = tree.lock();
            let pane = guard.pane(pane_id).expect("pane exists");
            assert_eq!(
                pane.metadata().get("agent_message").map(String::as_str),
                Some("permission needed for rm -rf build/"),
                "stored, whitespace collapsed to one line"
            );
        }

        // The next report carries no message: the stale reason must not
        // survive into the working state.
        handle_report(&state_report(pane_id, "pi", "working", 2_000), &tree);
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent_state").map(String::as_str),
            Some("working")
        );
        assert!(
            !pane.metadata().contains_key("agent_message"),
            "the blocked reason left with the blocked state: {:?}",
            pane.metadata()
        );
    }

    #[test]
    fn state_report_writes_metadata_and_returns_the_broadcast() {
        let (tree, pane_id) = tree_with_pane();
        let (reply, notification) =
            handle_report(&state_report(pane_id, "kimi", "working", 1_000), &tree);

        assert!(
            reply.contains(r#""id":"t-1000""#) && reply.contains(r#""result":"ok""#),
            "the reply echoes the id and reports ok: {reply}"
        );
        assert_eq!(
            notification,
            Some(TmuxNotification::AgentStateChanged {
                pane_id: pane_id.to_string(),
                agent: "kimi".to_string(),
                state: "working".to_string(),
                source: "hook".to_string()
            })
        );

        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent").map(String::as_str),
            Some("kimi")
        );
        assert_eq!(
            pane.metadata().get("agent_state").map(String::as_str),
            Some("working")
        );
        assert_eq!(
            pane.metadata()
                .get("agent_state_source")
                .map(String::as_str),
            Some("hook"),
            "a hook report marks the pane hook-authoritative"
        );
        assert_eq!(
            pane.metadata().get("agent_seq").map(String::as_str),
            Some("1000")
        );
        assert_eq!(
            pane.metadata().get("agent_source").map(String::as_str),
            Some("par-mux:test")
        );
    }

    #[test]
    fn session_report_records_identity_without_a_broadcast() {
        let (tree, pane_id) = tree_with_pane();
        let report = format!(
            r#"{{"id":"t-1","method":"pane.report_agent_session","params":{{"pane_id":"{pane_id}","agent":"kimi","seq":500,"agent_session_id":"s-1","session_start_source":"startup","source":"par-mux:test"}}}}"#
        );
        let (reply, notification) = handle_report(&report, &tree);

        assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
        assert_eq!(notification, None, "no state to broadcast yet");

        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent_session_id").map(String::as_str),
            Some("s-1")
        );
        assert_eq!(
            pane.metadata()
                .get("agent_session_start_source")
                .map(String::as_str),
            Some("startup")
        );
        assert_eq!(
            pane.metadata().get("agent_seq").map(String::as_str),
            Some("500")
        );
    }

    /// The daemon-side clock the freshness window compares against.
    fn now_unix_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_millis() as u64
    }

    fn telemetry_report(pane: PaneId, agent: &str, seq: u64, telemetry_json: &str) -> String {
        format!(
            r#"{{"id":"t-{seq}","method":"pane.report_agent_telemetry","params":{{"pane_id":"{pane}","agent":"{agent}","seq":{seq},"source":"par-mux:test","telemetry":{telemetry_json}}}}}"#
        )
    }

    /// The hub-normalizer output shape, verbatim: version, data source,
    /// sample time, and the bounded optional fields.
    fn sample_telemetry(sampled_at: u64) -> String {
        format!(
            r#"{{"version":1,"source":"claude_code","sampled_at_unix_ms":{sampled_at},"model":"GLM-5.3","effort":"high","thinking_enabled":true,"context_used_percent":63,"context_remaining_percent":37,"five_hour_remaining_percent":80,"seven_day_remaining_percent":95,"five_hour_resets_at_unix_ms":{},"seven_day_resets_at_unix_ms":{}}}"#,
            sampled_at + 3_600_000,
            sampled_at + 86_400_000
        )
    }

    /// The valid sample with one field's raw JSON swapped for the
    /// malformed variant under test.
    fn sample_with(field: &str, raw_value: &str) -> String {
        let mut sample = sample_telemetry(now_unix_ms() - 1_000);
        let needle = format!("\"{field}\":");
        let start = sample
            .find(&needle)
            .unwrap_or_else(|| panic!("{field} present in the sample"));
        let value_start = start + needle.len();
        let value_end = sample[value_start..]
            .find([',', '}'])
            .map(|offset| value_start + offset)
            .unwrap_or(sample.len());
        sample.replace_range(value_start..value_end, raw_value);
        sample
    }

    #[test]
    fn telemetry_report_stores_the_bounded_object() {
        let (tree, pane_id) = tree_with_pane();
        let sampled_at = now_unix_ms() - 60_000;
        let (reply, notification) = handle_report(
            &telemetry_report(pane_id, "claude", 1_000, &sample_telemetry(sampled_at)),
            &tree,
        );
        assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
        assert_eq!(
            notification, None,
            "telemetry broadcasts nothing yet — the roster-serving card owns the push"
        );

        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent").map(String::as_str),
            Some("claude")
        );
        assert_eq!(
            pane.metadata().get("agent_seq").map(String::as_str),
            Some("1000")
        );
        assert!(
            !pane.metadata().contains_key("agent_state"),
            "telemetry alone never claims roster state"
        );
        let stored: serde_json::Value = serde_json::from_str(
            pane.metadata()
                .get("agent_telemetry")
                .expect("the canonical object is stored"),
        )
        .unwrap();
        assert_eq!(stored["version"], 1);
        assert_eq!(stored["source"], "claude_code");
        assert_eq!(stored["sampled_at_unix_ms"], sampled_at);
        assert_eq!(stored["model"], "GLM-5.3");
        assert_eq!(stored["effort"], "high");
        assert_eq!(stored["thinking_enabled"], true);
        assert_eq!(stored["context_used_percent"], 63);
        assert_eq!(stored["context_remaining_percent"], 37);
        assert_eq!(stored["five_hour_remaining_percent"], 80);
        assert_eq!(stored["seven_day_remaining_percent"], 95);
        assert_eq!(
            stored["five_hour_resets_at_unix_ms"],
            sampled_at + 3_600_000
        );
        assert_eq!(
            stored["seven_day_resets_at_unix_ms"],
            sampled_at + 86_400_000
        );

        // A replay at the same seq is dropped by the ordering rule — the
        // stored sample is unchanged even though the replay's sample time
        // would be newer.
        drop(guard);
        handle_report(
            &telemetry_report(pane_id, "claude", 1_000, &sample_telemetry(now_unix_ms())),
            &tree,
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        let stored: serde_json::Value = serde_json::from_str(
            pane.metadata()
                .get("agent_telemetry")
                .expect("still stored"),
        )
        .unwrap();
        assert_eq!(
            stored["sampled_at_unix_ms"], sampled_at,
            "same-seq replay dropped"
        );
    }

    #[test]
    fn stale_telemetry_sample_is_dropped_absent_beats_stale() {
        let (tree, pane_id) = tree_with_pane();
        // Two hours old — past the 55-minute freshness window.
        let (reply, _) = handle_report(
            &telemetry_report(
                pane_id,
                "claude",
                1_000,
                &sample_telemetry(now_unix_ms() - 2 * 3_600_000),
            ),
            &tree,
        );
        assert!(
            reply.contains(r#""result":"ok""#),
            "dropped silently, like a stale seq: {reply}"
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert!(
            !pane.metadata().contains_key("agent_telemetry"),
            "absent is served instead of stale"
        );
        assert!(
            !pane.metadata().contains_key("agent_seq"),
            "a dropped report writes nothing, not even the sequence stamp"
        );
    }

    #[test]
    fn telemetry_older_than_the_stored_sample_is_dropped() {
        let (tree, pane_id) = tree_with_pane();
        let newer = now_unix_ms() - 30_000;
        let older = now_unix_ms() - 120_000;
        handle_report(
            &telemetry_report(pane_id, "claude", 1_000, &sample_telemetry(newer)),
            &tree,
        );
        let (reply, _) = handle_report(
            &telemetry_report(pane_id, "claude", 2_000, &sample_telemetry(older)),
            &tree,
        );
        assert!(
            reply.contains(r#""result":"ok""#),
            "backward step dropped silently: {reply}"
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        let stored: serde_json::Value = serde_json::from_str(
            pane.metadata()
                .get("agent_telemetry")
                .expect("still stored"),
        )
        .unwrap();
        assert_eq!(stored["sampled_at_unix_ms"], newer, "the newer sample wins");
        assert_eq!(
            pane.metadata().get("agent_seq").map(String::as_str),
            Some("1000"),
            "a dropped report writes nothing, seq included"
        );
    }

    #[test]
    fn malformed_telemetry_is_error_replied_and_writes_nothing() {
        let (tree, pane_id) = tree_with_pane();
        let sampled_at = now_unix_ms() - 1_000;
        let long_model = "m".repeat(129);
        let bad_payloads: Vec<(String, &str)> = vec![
            (
                telemetry_report(pane_id, "claude", 1_000, r#""not an object""#),
                "telemetry must be an object",
            ),
            (
                telemetry_report(
                    pane_id,
                    "claude",
                    1_000,
                    r#"{"source":"claude_code","sampled_at_unix_ms":1}"#,
                ),
                "missing version",
            ),
            (
                telemetry_report(
                    pane_id,
                    "claude",
                    1_000,
                    &format!(
                        r#"{{"version":2,"source":"claude_code","sampled_at_unix_ms":{sampled_at}}}"#
                    ),
                ),
                "unsupported telemetry version: 2",
            ),
            (
                telemetry_report(
                    pane_id,
                    "claude",
                    1_000,
                    &format!(r#"{{"version":1,"sampled_at_unix_ms":{sampled_at}}}"#),
                ),
                "missing source",
            ),
            (
                telemetry_report(
                    pane_id,
                    "claude",
                    1_000,
                    r#"{"version":1,"source":"claude_code"}"#,
                ),
                "missing sampled_at_unix_ms",
            ),
            (
                telemetry_report(
                    pane_id,
                    "claude",
                    1_000,
                    &sample_with("context_used_percent", "150"),
                ),
                "must be between 0 and 100",
            ),
            (
                telemetry_report(
                    pane_id,
                    "claude",
                    1_000,
                    &sample_with("context_used_percent", "-1"),
                ),
                "must be between 0 and 100",
            ),
            (
                telemetry_report(
                    pane_id,
                    "claude",
                    1_000,
                    &sample_with("thinking_enabled", r#""yes""#),
                ),
                "must be a boolean",
            ),
            (
                telemetry_report(
                    pane_id,
                    "claude",
                    1_000,
                    &format!(
                        r#"{{"version":1,"source":"claude_code","sampled_at_unix_ms":{sampled_at},"model":"{long_model}"}}"#
                    ),
                ),
                "exceeds 128",
            ),
            (
                telemetry_report(
                    pane_id,
                    "claude",
                    1_000,
                    // The wire form of a control-carried source: the JSON
                    // escape parses to a real newline at the door.
                    r#"{"version":1,"source":"claude\ncode","sampled_at_unix_ms":1}"#,
                ),
                "must not contain control characters",
            ),
            (
                // No telemetry object at all.
                format!(
                    r#"{{"id":"t-1","method":"pane.report_agent_telemetry","params":{{"pane_id":"{pane_id}","agent":"claude","seq":1000,"source":"par-mux:test"}}}}"#
                ),
                "missing telemetry",
            ),
        ];
        for (line, expected) in bad_payloads {
            let (reply, _) = handle_report(&line, &tree);
            assert!(reply.contains("error"), "error-replied: {reply}");
            assert!(
                reply.contains(expected),
                "expected `{expected}` in: {reply}"
            );
        }
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert!(
            !pane.metadata().contains_key("agent"),
            "rejection writes nothing"
        );
        assert!(!pane.metadata().contains_key("agent_telemetry"));
        assert!(
            !pane.metadata().contains_key("agent_seq"),
            "not even the sequence stamp — a rejection must not poison ordering (SEC-105)"
        );
    }

    #[test]
    fn telemetry_percents_round_to_integers_like_the_hub() {
        let (tree, pane_id) = tree_with_pane();
        let (reply, _) = handle_report(
            &telemetry_report(
                pane_id,
                "claude",
                1_000,
                &sample_with("context_used_percent", "63.6"),
            ),
            &tree,
        );
        assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        let stored: serde_json::Value =
            serde_json::from_str(pane.metadata().get("agent_telemetry").expect("stored")).unwrap();
        assert_eq!(
            stored["context_used_percent"], 64,
            "63.6 rounds away from zero, hub-style"
        );
    }

    #[test]
    fn release_clears_telemetry_with_the_claim() {
        let (tree, pane_id) = tree_with_pane();
        handle_report(&state_report(pane_id, "claude", "working", 1_000), &tree);
        handle_report(
            &telemetry_report(
                pane_id,
                "claude",
                2_000,
                &sample_telemetry(now_unix_ms() - 60_000),
            ),
            &tree,
        );
        let release = format!(
            r#"{{"id":"t-9","method":"pane.release_agent","params":{{"pane_id":"{pane_id}","agent":"claude","seq":3000,"source":"par-mux:test"}}}}"#
        );
        let (reply, _) = handle_report(&release, &tree);
        assert!(reply.contains(r#""result":"ok""#), "released: {reply}");
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert!(
            !pane.metadata().contains_key("agent_telemetry"),
            "a dead agent's telemetry left with its claim"
        );
    }

    #[test]
    fn telemetry_from_a_different_agent_never_takes_over_the_claim() {
        let (tree, pane_id) = tree_with_pane();
        handle_report(&state_report(pane_id, "pi", "working", 1_000), &tree);
        let (reply, _) = handle_report(
            &telemetry_report(
                pane_id,
                "claude",
                2_000,
                &sample_telemetry(now_unix_ms() - 60_000),
            ),
            &tree,
        );
        assert!(
            reply.contains(r#""result":"ok""#),
            "mismatched agent dropped silently, like the release guard: {reply}"
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent").map(String::as_str),
            Some("pi"),
            "the live claim keeps its label"
        );
        assert!(
            !pane.metadata().contains_key("agent_telemetry"),
            "a foreign agent's telemetry never attaches"
        );
    }

    #[test]
    fn telemetry_report_for_a_missing_pane_errors() {
        let (tree, _) = tree_with_pane();
        let (reply, _) = handle_report(
            &telemetry_report(PaneId(99), "claude", 1, &sample_telemetry(now_unix_ms())),
            &tree,
        );
        assert!(reply.contains("no such pane"), "{reply}");
    }

    /// A state report from a named source — the shape every shipped
    /// reporter uses (`par-mux:claude:session-hook`, `par-mux:pi`, …).
    fn sourced_state_report(
        pane: PaneId,
        agent: &str,
        state: &str,
        seq: u64,
        source: &str,
    ) -> String {
        format!(
            r#"{{"id":"t-{seq}","method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"{agent}","state":"{state}","seq":{seq},"source":"{source}"}}}}"#
        )
    }

    #[test]
    fn a_report_from_a_new_source_is_not_stale_against_another_sources_clock() {
        let (tree, pane_id) = tree_with_pane();

        // The two clocks the shipped reporters actually send: the
        // claude/codex/grok session hooks stamp `time.time_ns()`
        // (~1.8e18) while the pi/omp extensions stamp `Date.now()*1000`
        // (~1.8e15) — three orders of magnitude apart. Against one
        // per-pane stamp the pi report is always `<=` stored and dies.
        let claude_seq = 1_790_000_000_000_000_000_u64;
        let (_, first) = handle_report(
            &sourced_state_report(
                pane_id,
                "claude",
                "working",
                claude_seq,
                "par-mux:claude:session-hook",
            ),
            &tree,
        );
        assert!(first.is_some(), "the claude report broadcasts");

        let pi_seq = 1_790_000_000_000_000_u64;
        let (reply, second) = handle_report(
            &sourced_state_report(pane_id, "pi", "blocked", pi_seq, "par-mux:pi"),
            &tree,
        );
        assert!(
            reply.contains(r#""result":"ok""#),
            "accepted, not error-replied: {reply}"
        );
        assert_eq!(
            second,
            Some(TmuxNotification::AgentStateChanged {
                pane_id: pane_id.to_string(),
                agent: "pi".to_string(),
                state: "blocked".to_string(),
                source: "hook".to_string()
            }),
            "the pi report took the pane despite the smaller clock"
        );
        {
            let guard = tree.lock();
            let pane = guard.pane(pane_id).expect("pane exists");
            assert_eq!(
                pane.metadata().get("agent").map(String::as_str),
                Some("pi"),
                "pi owns the pane: {:?}",
                pane.metadata()
            );
        }

        // Staleness still orders reports WITHIN a source: the same pi
        // seq again is a duplicate, and the smaller of two pi seqs is
        // old news.
        for stale_pi in [pi_seq, pi_seq - 1] {
            let (_, duplicate) = handle_report(
                &sourced_state_report(pane_id, "pi", "working", stale_pi, "par-mux:pi"),
                &tree,
            );
            assert_eq!(duplicate, None, "pi seq {stale_pi} must not broadcast");
        }
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent_state").map(String::as_str),
            Some("blocked"),
            "the duplicate pi reports wrote nothing"
        );

        // And the claude bucket survived the pi interlude: a fresh claude
        // report above its own last seq is still accepted.
        drop(guard);
        let (_, third) = handle_report(
            &sourced_state_report(
                pane_id,
                "claude",
                "idle",
                claude_seq + 1,
                "par-mux:claude:session-hook",
            ),
            &tree,
        );
        assert!(third.is_some(), "the claude bucket was not reset by pi");
    }

    #[test]
    fn reports_with_an_invalid_state_or_label_are_rejected_without_broadcast() {
        let (tree, pane_id) = tree_with_pane();

        // A state outside herdr's fixed set — stored today, misparsed by
        // every consumer of the space-split roster line.
        let (reply, notification) =
            handle_report(&state_report(pane_id, "kimi", "waiting", 1_000), &tree);
        assert!(
            reply.contains("invalid state"),
            "a state outside the set is an error: {reply}"
        );
        assert_eq!(notification, None);

        // A label containing whitespace breaks the broadcast shape the
        // same way.
        let (reply, notification) = handle_report(
            &state_report(pane_id, "claude code", "working", 1_000),
            &tree,
        );
        assert!(
            reply.contains("whitespace") && reply.contains("error"),
            "a label with inner whitespace is an error: {reply}"
        );
        assert_eq!(notification, None);

        // The injection shape: agent, state, and source are interpolated
        // verbatim into `%agent-state-changed`, so a newline in any of
        // them forges a control-mode line every attached client parses.
        // The JSON wire form carries the newline escaped (\\n in the raw
        // line, a real \n once decoded).
        let forged = [
            format!(
                r#"{{"id":"f1","method":"pane.report_agent","params":{{"pane_id":"{pane_id}","agent":"x\n%exit","state":"working","seq":1000,"source":"par-mux:test"}}}}"#
            ),
            format!(
                r#"{{"id":"f2","method":"pane.report_agent","params":{{"pane_id":"{pane_id}","agent":"kimi","state":"working\n%exit","seq":1000,"source":"par-mux:test"}}}}"#
            ),
            format!(
                r#"{{"id":"f3","method":"pane.report_agent","params":{{"pane_id":"{pane_id}","agent":"kimi","state":"working","seq":1000,"source":"par-mux:pi\n%exit"}}}}"#
            ),
        ];
        for report in &forged {
            let (reply, notification) = handle_report(report, &tree);
            assert!(
                reply.contains("error") && !reply.contains("\"result\":\"ok\""),
                "a forged field is an error, not a silent ok: {reply}"
            );
            assert_eq!(
                notification, None,
                "no notification — nothing that could carry a forged line to a client"
            );
        }

        {
            let guard = tree.lock();
            let pane = guard.pane(pane_id).expect("pane exists");
            assert!(
                !pane.metadata().contains_key("agent"),
                "the rejected reports wrote nothing: {:?}",
                pane.metadata()
            );
        }

        // The pane is not poisoned: a valid report at the same seq is
        // still accepted afterward.
        let (_, notification) =
            handle_report(&state_report(pane_id, "kimi", "working", 1_000), &tree);
        assert!(notification.is_some(), "a valid report still lands");
    }

    /// SEC-105: free-text report values are bounded — an oversized label,
    /// source, identity field, blocked reason, or resume argv is rejected
    /// with an error and writes nothing, keeping pane metadata and the
    /// persisted state file bounded per pane.
    #[test]
    fn oversized_report_values_are_rejected_without_broadcast() {
        let (tree, pane_id) = tree_with_pane();
        let oversized = "x".repeat(MAX_REPORT_VALUE_LEN + 1);

        // Every persisted free-text field, one report per field.
        let oversized_reports = [
            format!(
                r#"{{"method":"pane.report_agent","params":{{"pane_id":"{pane_id}","agent":"{oversized}","state":"working","seq":1000}}}}"#
            ),
            format!(
                r#"{{"method":"pane.report_agent","params":{{"pane_id":"{pane_id}","agent":"kimi","state":"working","seq":1000,"source":"{oversized}"}}}}"#
            ),
            format!(
                r#"{{"method":"pane.report_agent","params":{{"pane_id":"{pane_id}","agent":"kimi","state":"working","seq":1000,"agent_session_id":"{oversized}"}}}}"#
            ),
            format!(
                r#"{{"method":"pane.report_agent","params":{{"pane_id":"{pane_id}","agent":"kimi","state":"blocked","seq":1000,"message":"{oversized}"}}}}"#
            ),
            format!(
                r#"{{"method":"pane.report_agent_session","params":{{"pane_id":"{pane_id}","agent":"kimi","seq":1000,"agent_session_path":"{oversized}"}}}}"#
            ),
            format!(
                r#"{{"method":"pane.report_agent_session","params":{{"pane_id":"{pane_id}","agent":"kimi","seq":1000,"agent_session_id":"ok","session_resume_argv":["pi","{oversized}"]}}}}"#
            ),
        ];
        for report in &oversized_reports {
            let (reply, notification) = handle_report(report, &tree);
            assert!(
                reply.contains("error") && reply.contains("exceeds"),
                "an oversized value is an error naming the budget: {reply}"
            );
            assert_eq!(notification, None, "nothing is broadcast for a rejection");
        }

        {
            let guard = tree.lock();
            let pane = guard.pane(pane_id).expect("pane exists");
            assert!(
                !pane.metadata().contains_key("agent"),
                "the rejected reports wrote nothing: {:?}",
                pane.metadata()
            );
        }

        // The pane is not poisoned: a valid report at the same seq still
        // lands.
        let (_, notification) =
            handle_report(&state_report(pane_id, "kimi", "working", 1_000), &tree);
        assert!(notification.is_some(), "a valid report still lands");
    }

    #[test]
    fn stale_sequence_reports_are_dropped_without_write_or_broadcast() {
        let (tree, pane_id) = tree_with_pane();
        let (_, first) = handle_report(&state_report(pane_id, "kimi", "working", 1_000), &tree);
        assert!(first.is_some(), "the in-order report broadcasts");

        // An older report and an equal one: both dropped.
        for stale_seq in [999_u64, 1_000] {
            let (reply, notification) =
                handle_report(&state_report(pane_id, "kimi", "blocked", stale_seq), &tree);
            assert!(
                reply.contains(r#""result":"ok""#),
                "dropped is not an error"
            );
            assert_eq!(notification, None, "seq {stale_seq} must not broadcast");
        }

        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent_state").map(String::as_str),
            Some("working"),
            "the stale reports wrote nothing"
        );
        assert_eq!(
            pane.metadata().get("agent_seq").map(String::as_str),
            Some("1000"),
            "the sequence stamp did not move backwards"
        );
    }

    #[test]
    fn unknown_state_is_never_written_or_broadcast() {
        let (tree, pane_id) = tree_with_pane();
        let (reply, notification) =
            handle_report(&state_report(pane_id, "kimi", "unknown", 1_000), &tree);

        assert!(
            reply.contains(r#""result":"ok""#),
            "not an error either: {reply}"
        );
        assert_eq!(notification, None, "'unknown' is the absence of a claim");
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert!(
            !pane.metadata().contains_key("agent_state"),
            "'unknown' must not be written: {:?}",
            pane.metadata()
        );
    }

    #[test]
    fn malformed_reports_and_unknown_methods_error() {
        let (tree, pane_id) = tree_with_pane();

        let (reply, _) = handle_report("this is not json", &tree);
        assert!(reply.contains("invalid JSON"), "{reply}");

        let (reply, _) = handle_report(
            &format!(r#"{{"id":2,"method":"pane.no_such","params":{{"pane_id":"{pane_id}"}}}}"#),
            &tree,
        );
        assert!(reply.contains("unknown method"), "{reply}");

        let (reply, _) = handle_report(
            &state_report(PaneId(9_999), "kimi", "working", 1_000),
            &tree,
        );
        assert!(reply.contains("no such pane"), "{reply}");

        // A pane id herdr-shaped but unparsable, and a missing seq.
        let (reply, _) = handle_report(
            r#"{"id":3,"method":"pane.report_agent","params":{"pane_id":"zero","agent":"kimi","state":"working","seq":1}}"#,
            &tree,
        );
        assert!(reply.contains("bad pane_id"), "{reply}");
        let (reply, _) = handle_report(
            &format!(
                r#"{{"id":4,"method":"pane.report_agent","params":{{"pane_id":"{pane_id}","agent":"kimi","state":"working"}}}}"#
            ),
            &tree,
        );
        assert!(reply.contains("missing seq"), "{reply}");
    }

    #[test]
    fn session_report_after_a_state_report_broadcasts_the_known_state() {
        let (tree, pane_id) = tree_with_pane();
        handle_report(&state_report(pane_id, "kimi", "working", 1_000), &tree);

        let report = format!(
            r#"{{"id":"t-2","method":"pane.report_agent_session","params":{{"pane_id":"{pane_id}","agent":"kimi","seq":2000,"agent_session_id":"s-2"}}}}"#
        );
        let (_, notification) = handle_report(&report, &tree);
        assert_eq!(
            notification,
            Some(TmuxNotification::AgentStateChanged {
                pane_id: pane_id.to_string(),
                agent: "kimi".to_string(),
                state: "working".to_string(),
                source: "hook".to_string()
            })
        );
    }

    /// The pi/omp session-report shape: path-preferred, id dropped — what
    /// the shipped par-term assets actually send (`currentSessionRef`).
    fn session_report(
        pane: PaneId,
        agent: &str,
        session_path: &str,
        resume_argv: Option<&str>,
        seq: u64,
    ) -> String {
        let argv = match resume_argv {
            Some(argv) => format!(r#","session_resume_argv":{argv}"#),
            None => String::new(),
        };
        format!(
            r#"{{"id":"t-{seq}","method":"pane.report_agent_session","params":{{"pane_id":"{pane}","agent":"{agent}","seq":{seq},"source":"par-mux:test","session_start_source":"startup","agent_session_path":"{session_path}"{argv}}}}}"#
        )
    }

    #[test]
    fn a_path_only_session_report_is_accepted() {
        let (tree, pane_id) = tree_with_pane();

        let (reply, _) = handle_report(
            &session_report(pane_id, "pi", "/tmp/pi-session.jsonl", None, 1_000),
            &tree,
        );
        assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata()
                .get("agent_session_path")
                .map(String::as_str),
            Some("/tmp/pi-session.jsonl")
        );
        assert_eq!(
            pane.metadata()
                .get("agent_session_start_source")
                .map(String::as_str),
            Some("startup"),
            "the fields only a session report carries must land"
        );
        assert!(
            !pane.metadata().contains_key("agent_session_id"),
            "a path-only report writes no id: {:?}",
            pane.metadata()
        );
    }

    #[test]
    fn a_session_report_with_neither_ref_errors() {
        let (tree, pane_id) = tree_with_pane();
        let (reply, _) = handle_report(
            &format!(
                r#"{{"id":"t-1","method":"pane.report_agent_session","params":{{"pane_id":"{pane_id}","agent":"pi","seq":1000}}}}"#
            ),
            &tree,
        );
        assert!(
            reply.contains("missing agent_session_id or agent_session_path"),
            "{reply}"
        );
    }

    #[test]
    fn resume_argv_is_stored_verbatim_and_held_while_the_session_holds() {
        let (tree, pane_id) = tree_with_pane();

        let (reply, _) = handle_report(
            &session_report(
                pane_id,
                "pi",
                "/tmp/pi-session.jsonl",
                Some(r#"["pi","--session","/tmp/pi-session.jsonl"]"#),
                1_000,
            ),
            &tree,
        );
        assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
        {
            let guard = tree.lock();
            let pane = guard.pane(pane_id).expect("pane exists");
            assert_eq!(
                pane.metadata().get("agent_resume_argv").map(String::as_str),
                Some(r#"["pi","--session","/tmp/pi-session.jsonl"]"#),
                "stored verbatim as a JSON argv"
            );
        }

        // A later report for the SAME session without one keeps the stored
        // invocation — the asset resent everything it knows, minus this.
        handle_report(
            &session_report(pane_id, "pi", "/tmp/pi-session.jsonl", None, 2_000),
            &tree,
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent_resume_argv").map(String::as_str),
            Some(r#"["pi","--session","/tmp/pi-session.jsonl"]"#),
            "same session, no fresh invocation: the stored one holds"
        );
    }

    #[test]
    fn a_new_session_without_an_invocation_clears_the_stale_one() {
        let (tree, pane_id) = tree_with_pane();
        handle_report(
            &session_report(
                pane_id,
                "pi",
                "/tmp/pi-old.jsonl",
                Some(r#"["pi","--session","/tmp/pi-old.jsonl"]"#),
                1_000,
            ),
            &tree,
        );

        // The pane moved to a different session and reported no invocation
        // for it: resuming the OLD session's argv would resume the wrong
        // session — the failure mode worse than no entry.
        handle_report(
            &session_report(pane_id, "pi", "/tmp/pi-new.jsonl", None, 2_000),
            &tree,
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata()
                .get("agent_session_path")
                .map(String::as_str),
            Some("/tmp/pi-new.jsonl")
        );
        assert!(
            !pane.metadata().contains_key("agent_resume_argv"),
            "the old session's invocation left with the old session: {:?}",
            pane.metadata()
        );
    }

    #[test]
    fn malformed_resume_argv_errors_without_writing() {
        let (tree, pane_id) = tree_with_pane();
        let (reply, _) = handle_report(
            &session_report(
                pane_id,
                "pi",
                "/tmp/pi-session.jsonl",
                Some(r#""pi --session x""#),
                1_000,
            ),
            &tree,
        );
        assert!(
            reply.contains("session_resume_argv must be an array of strings"),
            "{reply}"
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert!(
            !pane.metadata().contains_key("agent_resume_argv"),
            "a rejected report writes nothing: {:?}",
            pane.metadata()
        );
    }

    /// A release report (`pane.release_agent`, herdr's SessionEnd shape):
    /// the agent that claimed the pane announces it is gone. The claim —
    /// state, hook authority, blocked reason, and session identity — is
    /// cleared and the removal is broadcast, so the roster drops the pane
    /// instead of showing a dead agent "working" forever.
    #[test]
    fn a_release_clears_state_identity_and_authority_and_broadcasts_the_removal() {
        let (tree, pane_id) = tree_with_pane();
        handle_report(&state_report(pane_id, "pi", "working", 1_000), &tree);
        handle_report(
            &session_report(pane_id, "pi", "/tmp/pi-session.jsonl", None, 1_100),
            &tree,
        );

        let release = format!(
            r#"{{"id":"t-3","method":"pane.release_agent","params":{{"pane_id":"{pane_id}","agent":"pi","seq":1200,"source":"par-mux:test"}}}}"#
        );
        let (reply, notification) = handle_report(&release, &tree);
        assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
        assert_eq!(
            notification,
            Some(TmuxNotification::AgentReleased {
                pane_id: pane_id.to_string(),
                agent: "pi".to_string(),
            }),
            "the removal is broadcast"
        );

        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        for key in [
            "agent",
            "agent_state",
            "agent_state_source",
            "agent_message",
            "agent_source",
            "agent_seq",
            SEQ_STAMPS_KEY,
            "agent_session_id",
            "agent_session_path",
            "agent_session_start_source",
            "agent_resume_argv",
        ] {
            assert!(
                !pane.metadata().contains_key(key),
                "release cleared {key}: {:?}",
                pane.metadata()
            );
        }
    }

    /// A release naming a different agent than the pane's claim is a no-op:
    /// a stale claude hook must not wipe a live pi claim (herdr's authority
    /// match, on the label).
    #[test]
    fn a_release_from_a_different_agent_is_a_no_op() {
        let (tree, pane_id) = tree_with_pane();
        handle_report(&state_report(pane_id, "pi", "working", 1_000), &tree);

        let release = format!(
            r#"{{"id":"t-2","method":"pane.release_agent","params":{{"pane_id":"{pane_id}","agent":"claude","seq":2000,"source":"par-mux:test"}}}}"#
        );
        let (reply, notification) = handle_report(&release, &tree);
        assert!(
            reply.contains(r#""result":"ok""#),
            "politely accepted: {reply}"
        );
        assert_eq!(notification, None, "nothing was released");
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent_state").map(String::as_str),
            Some("working"),
            "the live claim is intact"
        );
    }

    /// A release below the last accepted sequence number is dropped by the
    /// same monotonic rule as every other report — a late release must not
    /// wipe a newer claim.
    #[test]
    fn a_stale_release_is_dropped() {
        let (tree, pane_id) = tree_with_pane();
        handle_report(&state_report(pane_id, "pi", "working", 1_000), &tree);

        let release = format!(
            r#"{{"id":"t-2","method":"pane.release_agent","params":{{"pane_id":"{pane_id}","agent":"pi","seq":900,"source":"par-mux:test"}}}}"#
        );
        let (reply, notification) = handle_report(&release, &tree);
        assert!(
            reply.contains(r#""result":"ok""#),
            "dropped, not errored: {reply}"
        );
        assert_eq!(notification, None);
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert!(
            pane.metadata().contains_key("agent_state"),
            "the claim survives a stale release"
        );
    }

    /// A session report that moves the pane to a DIFFERENT agent clears the
    /// previous agent's state instead of rebroadcasting it under the new
    /// label — pi's idle must not become claude's hook claim.
    #[test]
    fn a_session_report_that_relabels_the_pane_clears_the_previous_agents_state() {
        let (tree, pane_id) = tree_with_pane();
        handle_report(&state_report(pane_id, "pi", "idle", 1_000), &tree);

        let (reply, notification) = handle_report(
            &session_report(pane_id, "claude", "/tmp/claude-session", None, 1_100),
            &tree,
        );
        assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
        assert_eq!(
            notification, None,
            "the previous agent's state is not rebroadcast under the new label"
        );

        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent").map(String::as_str),
            Some("claude"),
            "the new label is recorded"
        );
        for key in ["agent_state", "agent_state_source", "agent_message"] {
            assert!(
                !pane.metadata().contains_key(key),
                "pi's {key} did not survive the relabel: {:?}",
                pane.metadata()
            );
        }
    }
}
