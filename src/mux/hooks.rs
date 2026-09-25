//! Hook reports: the agent layer's second grammar on the control socket.
//!
//! herdr's integration scripts (measured: `kimi/herdr-agent-state.sh`) open
//! the control socket, send ONE JSON line — `{"id":…,"method":…,
//! "params":{…}}` — read one reply, and close. par-mux accepts herdr's two
//! methods verbatim (`pane.report_agent`, `pane.report_agent_session`) so
//! those scripts port with an env-var rename (`HERDR_*` → `PAR_MUX_*`); the
//! server's client loop routes every line
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
        pane.clear_metadata(&[
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
        ]);
        Some(TmuxNotification::AgentReleased {
            pane_id: header.pane_id.to_string(),
            agent: header.agent.clone(),
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
