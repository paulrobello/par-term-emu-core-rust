//! `pane.report_agent` and `pane.report_agent_session`: agent state and
//! session identity.

use super::{check_value_len, error_reply, is_stale, ok_reply, parse_header, record_seq};
use crate::mux::tree::MuxTree;
use crate::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use std::sync::Arc;

/// `pane.report_agent`: state (working/blocked/idle), plus whatever session
/// identity the hook happens to know.
pub(super) fn handle_state_report(
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
        if is_stale(pane, header.source.as_deref(), header.seq) {
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
pub(super) fn handle_session_report(
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
        if is_stale(pane, header.source.as_deref(), header.seq) {
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
