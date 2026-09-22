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
        if is_stale(pane.metadata(), header.seq) {
            return (ok_reply(id), None);
        }

        // Prior identity, captured before the writes below replace it: a
        // report that moves the pane to a DIFFERENT session without a fresh
        // invocation must not leave the old session's argv behind.
        let prior_session_id = pane.metadata().get("agent_session_id").cloned();
        let prior_session_path = pane.metadata().get("agent_session_path").cloned();

        pane.set_metadata("agent", &header.agent);
        pane.set_metadata("agent_seq", &header.seq.to_string());
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
    serde_json::to_string(&argv)
        .map(Some)
        .map_err(|err| format!("session_resume_argv: {err}"))
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
}
