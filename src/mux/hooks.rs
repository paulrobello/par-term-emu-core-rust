//! Hook reports: the agent layer's second grammar on the control socket.
//!
//! herdr's integration scripts (measured: `kimi/herdr-agent-state.sh`) open
//! the control socket, send ONE JSON line — `{"id":…,"method":…,
//! "params":{…}}` — read one reply, and close. par-mux accepts herdr's two
//! methods verbatim (`pane.report_agent`, `pane.report_agent_session`) so
//! those scripts port with an env-var rename (`HERDR_*` → `PAR_MUX_*`); the
//! server's client loop routes any line whose first non-whitespace byte is
//! `{` here, and anything else remains a tmux control command.
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
    let seq = params
        .get("seq")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "missing seq".to_string())?;
    let source = params
        .get("source")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
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
    // `unknown` is the absence of a hook claim, not a state — never written,
    // never broadcast (the Phase 5 ruling that keeps an agent without hooks
    // out of the roster rather than misreporting it).
    if state == "unknown" {
        return (ok_reply(id), None);
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
        // sequence number is dropped — no metadata write, no broadcast.
        if is_stale(pane.metadata(), header.seq) {
            return (ok_reply(id), None);
        }

        pane.set_metadata("agent", &header.agent);
        pane.set_metadata("agent_state", state);
        // A hook state report makes this pane hook-authoritative from now
        // on: the scrape tier skips it forever after (the structural
        // precedence rule — a claim is never overwritten by a guess).
        pane.set_metadata("agent_state_source", "hook");
        pane.set_metadata("agent_seq", &header.seq.to_string());
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
fn handle_session_report(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    tree: &Arc<Mutex<MuxTree>>,
) -> (String, Option<TmuxNotification>) {
    let header = match parse_header(params) {
        Ok(header) => header,
        Err(message) => return (error_reply(id, &message), None),
    };
    let Some(session_id) = params
        .get("agent_session_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return (error_reply(id, "missing agent_session_id"), None);
    };

    let notification = {
        let mut guard = tree.lock();
        let Some(pane) = guard.pane_mut(header.pane_id) else {
            return (
                error_reply(id, &format!("no such pane: {}", header.pane_id)),
                None,
            );
        };
        if is_stale(pane.metadata(), header.seq) {
            return (ok_reply(id), None);
        }

        pane.set_metadata("agent", &header.agent);
        pane.set_metadata("agent_session_id", session_id);
        pane.set_metadata("agent_seq", &header.seq.to_string());
        if let Some(source) = &header.source {
            pane.set_metadata("agent_source", source);
        }
        if let Some(path) = params
            .get("agent_session_path")
            .and_then(serde_json::Value::as_str)
        {
            pane.set_metadata("agent_session_path", path);
        }
        // The resume path's provenance (startup vs resume), recorded now
        // because the wire carries it now — Phase 6 persists it.
        if let Some(start) = params
            .get("session_start_source")
            .and_then(serde_json::Value::as_str)
        {
            pane.set_metadata("agent_session_start_source", start);
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

/// Whether `seq` is at or below the pane's last accepted report — the
/// stale side of herdr's ordering rule.
fn is_stale(metadata: &std::collections::HashMap<String, String>, seq: u64) -> bool {
    match metadata.get("agent_seq").map(|stamp| stamp.parse::<u64>()) {
        Some(Ok(stored)) => seq <= stored,
        // No accepted report yet (or a hand-corrupted stamp) means nothing
        // to be stale against.
        _ => false,
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
}
