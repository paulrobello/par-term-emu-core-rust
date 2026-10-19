//! `pane.report_agent` and `pane.report_agent_session`: agent state and
//! session identity. `pane.report_agent_session_cwd` is the muse arm's
//! variant for agents whose hooks run with a cleared environment.

use super::{check_value_len, error_reply, is_stale, ok_reply, parse_header, record_seq};
use crate::mux::tree::MuxTree;
use par_term_emu_core::tmux_control::TmuxNotification;
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
        // The owning workspace for the push, read before the pane_mut
        // borrow; the same map roster_rows reads, so push and list-agents
        // agree.
        let ws = guard
            .workspace_of_pane(header.pane_id)
            .map(|w| w.to_string())
            .unwrap_or_default();
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

        // The whole claim renders through the typed claim (ARC-113c): the
        // report's fields overwrite, everything it omits survives from the
        // prior claim — except the blocked reason, which is stored when
        // present and CLEARED when a later report omits it, so a stale
        // reason cannot survive into a new state.
        let mut claim = pane.agent_claim().unwrap_or_default();
        claim.agent = header.agent.clone();
        claim.state = Some(state.to_string());
        // A hook state report makes this pane hook-authoritative from now
        // on: the scrape tier skips it forever after (the structural
        // precedence rule — a claim is never overwritten by a guess).
        claim.state_source = Some("hook".to_string());
        claim.seq = Some(header.seq);
        if let Some(source) = &header.source {
            claim.source = Some(source.clone());
        }
        // Identity fields ride state reports too: the kimi script attaches
        // agent_session_id whenever it knows one, state report or not.
        if let Some(value) = params
            .get("agent_session_id")
            .and_then(serde_json::Value::as_str)
        {
            claim.session_id = Some(value.to_string());
        }
        if let Some(value) = params
            .get("agent_session_path")
            .and_then(serde_json::Value::as_str)
        {
            claim.session_path = Some(value.to_string());
        }
        // The blocked reason pi and omp attach to their reports (the
        // scannability field the roster exists for). Whitespace is collapsed
        // at the door — the roster line is one line.
        claim.message = params
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(|message| message.split_whitespace().collect::<Vec<_>>().join(" "));
        pane.set_agent_claim(&claim);
        record_seq(pane, header.source.as_deref(), header.seq);

        Some(TmuxNotification::AgentStateChanged {
            pane_id: header.pane_id.to_string(),
            agent: header.agent.clone(),
            state: state.to_string(),
            source: "hook".to_string(),
            ws,
            msg: claim.message.clone().unwrap_or_default(),
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
        // The owning workspace for the push, read before the pane_mut
        // borrow; the same map roster_rows reads, so push and list-agents
        // agree.
        let ws = guard
            .workspace_of_pane(header.pane_id)
            .map(|w| w.to_string())
            .unwrap_or_default();
        let Some(pane) = guard.pane_mut(header.pane_id) else {
            return (
                error_reply(id, &format!("no such pane: {}", header.pane_id)),
                None,
            );
        };
        if is_stale(pane, header.source.as_deref(), header.seq) {
            return (ok_reply(id), None);
        }

        // The whole claim renders through the typed claim (ARC-113c), read
        // first so the writes below can compare against the prior identity.
        let mut claim = pane.agent_claim().unwrap_or_default();
        let prior_session_id = claim.session_id.clone();
        let prior_session_path = claim.session_path.clone();

        // A report that moves the pane to a DIFFERENT agent ends the
        // previous agent's claim: its state, hook authority, and blocked
        // reason must not survive — and above all must not be rebroadcast
        // under the new label as though the new agent had claimed it (the
        // rebroadcast below reads `claim.state`, which this just cleared).
        if claim.agent != header.agent {
            claim.state = None;
            claim.state_source = None;
            claim.message = None;
        }

        claim.agent = header.agent.clone();
        claim.seq = Some(header.seq);
        if let Some(source) = &header.source {
            claim.source = Some(source.clone());
        }
        if let Some(session_id) = &session_id {
            claim.session_id = Some((*session_id).to_string());
        }
        if let Some(session_path) = &session_path {
            claim.session_path = Some((*session_path).to_string());
        }
        // The resume path's provenance (startup vs resume), recorded now
        // because the wire carries it now — Phase 6 persists it.
        if let Some(start) = params
            .get("session_start_source")
            .and_then(serde_json::Value::as_str)
        {
            claim.session_start_source = Some(start.to_string());
        }
        // The agent's own resume invocation, stored as a JSON argv string:
        // Phase 6 spawns it verbatim (hook-first; the per-agent table is the
        // fallback for agents that cannot report one). Absent on a report
        // that also moves to a different session means the stored argv is
        // the OLD session's — clear it rather than resume the wrong session.
        match &resume_argv {
            Some(argv) => claim.resume_argv = Some(argv.clone()),
            None => {
                let changed = session_id
                    .is_some_and(|value| prior_session_id.as_deref() != Some(value))
                    || session_path
                        .is_some_and(|value| prior_session_path.as_deref() != Some(value));
                if changed {
                    claim.resume_argv = None;
                }
            }
        }
        pane.set_agent_claim(&claim);
        record_seq(pane, header.source.as_deref(), header.seq);

        // The rebroadcast keeps the state's OWN provenance: a claude-shaped
        // pane (identity by hook, state by scrape) must not relabel a
        // scrape guess as a hook claim on its session reports.
        claim
            .state
            .clone()
            .map(|state| TmuxNotification::AgentStateChanged {
                pane_id: header.pane_id.to_string(),
                agent: header.agent.clone(),
                state,
                source: claim
                    .state_source
                    .clone()
                    .unwrap_or_else(|| "hook".to_string()),
                ws,
                msg: claim.message.clone().unwrap_or_default(),
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
fn parse_resume_argv(params: &serde_json::Value) -> Result<Option<Vec<String>>, String> {
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
    // The length budget applies to the form that persists — the compact
    // JSON rendering the claim stores under `agent_resume_argv`.
    let encoded =
        serde_json::to_string(&argv).map_err(|err| format!("session_resume_argv: {err}"))?;
    check_value_len("session_resume_argv", &encoded)?;
    Ok(Some(argv))
}

/// `pane.report_agent_session_cwd` — the muse arm's session report. Muse
/// clears its hook environment (only MUSE_SESSION_ID/MUSE_THREAD_ID
/// survive, verified live against muse 1.4.9), so a hook inside a par-mux
/// pane cannot carry `PAR_MUX_PANE_ID` the way the claude/codex/grok
/// reporters do. The reporter sends the session's working directory
/// instead; the daemon resolves the pane, then applies the report through
/// the pane-addressed path unchanged — same validation, staleness, claim,
/// and broadcast semantics by construction.
///
/// Resolution is deliberately conservative (a wrong pane must never
/// receive a claim): local panes only (a remote OSC 7 cwd never matches),
/// and three tiers in order —
/// 1. a pane already claimed by this agent (re-report/refresh),
/// 2. an unclaimed pane whose spawn command names the agent as a word
///    (`muse`, `muse-code`, but not `mymuse`),
/// 3. an unclaimed pane in that directory — the "typed `muse` inside a
///    shell pane" host; even a mis-pick is harmless because `muse resume`
///    is workspace-scoped.
///
/// Ties resolve to the lowest pane id. No match is an error reply, which
/// is what daemons without a match return and the reporter ignores.
pub(super) fn handle_session_report_cwd(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    tree: &Arc<Mutex<MuxTree>>,
) -> (String, Option<TmuxNotification>) {
    let Some(cwd) = params
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
    else {
        return (error_reply(id, "missing cwd"), None);
    };
    if let Err(message) = check_value_len("cwd", cwd) {
        return (error_reply(id, &message), None);
    }
    let Some(agent) = params
        .get("agent")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
    else {
        return (error_reply(id, "missing agent"), None);
    };
    if let Err(message) = check_value_len("agent label", agent) {
        return (error_reply(id, &message), None);
    }

    let resolved = {
        let guard = tree.lock();
        resolve_pane_for_cwd(&guard, agent, cwd)
    };
    let Some(pane_id) = resolved else {
        return (error_reply(id, &format!("no pane running in {cwd}")), None);
    };

    // The pane-addressed path re-locks and re-validates; injecting the
    // resolved pane_id keeps every downstream rule identical by construction.
    let mut injected = params.clone();
    injected["pane_id"] = serde_json::json!(pane_id.to_string());
    handle_session_report(id, &injected, tree)
}

/// The pane a cwd-keyed report belongs to, or `None`. Three tiers in
/// order — claimed by this agent, then an unclaimed pane whose spawn
/// command names the agent, then any unclaimed pane in the directory —
/// each resolved to the lowest pane id so the pick is deterministic.
fn resolve_pane_for_cwd(tree: &MuxTree, agent: &str, cwd: &str) -> Option<crate::mux::ids::PaneId> {
    let payload = std::path::PathBuf::from(cwd);
    // Tree-wide pane walk: every session's every window, mirroring
    // roster_rows' iteration shape.
    let pane_ids: Vec<_> = tree
        .sessions()
        .iter()
        .filter_map(|sid| tree.session(*sid))
        .flat_map(|session| {
            session
                .windows
                .iter()
                .filter_map(|w| tree.window(*w))
                .flat_map(|window| window.panes())
        })
        .collect();
    let mut claimed: Option<crate::mux::ids::PaneId> = None;
    let mut agent_named: Option<crate::mux::ids::PaneId> = None;
    let mut unclaimed: Option<crate::mux::ids::PaneId> = None;
    for id in pane_ids {
        let Some(pane) = tree.pane(id) else {
            continue;
        };
        // respawn_cwd resolves local panes only — an OSC 7 report naming a
        // remote host (SEC-128) never yields a path, so it never matches.
        let Some(pane_cwd) = pane.respawn_cwd() else {
            continue;
        };
        if !cwd_matches(&pane_cwd, &payload) {
            continue;
        }
        match pane.agent_claim() {
            Some(claim) if claim.agent == agent => take_lowest(&mut claimed, id),
            None => {
                if pane
                    .spawn_command()
                    .is_some_and(|command| names_agent(command, agent))
                {
                    take_lowest(&mut agent_named, id);
                }
                take_lowest(&mut unclaimed, id);
            }
            // A pane claimed by a different agent is never a candidate: no
            // claim stealing, in any tier.
            Some(_) => {}
        }
    }
    claimed.or(agent_named).or(unclaimed)
}

/// Lowest-id tracking for the resolution tiers: iteration order must not
/// decide between same-tier candidates.
fn take_lowest(slot: &mut Option<crate::mux::ids::PaneId>, id: crate::mux::ids::PaneId) {
    if slot.is_none_or(|best| id < best) {
        *slot = Some(id);
    }
}

/// Compare the report's cwd against a pane's cwd: direct spelling first,
/// then resolved paths — macOS spellings like /tmp vs /private/tmp of one
/// directory must match. A pane whose directory no longer resolves can
/// still match by exact spelling.
fn cwd_matches(pane_cwd: &std::path::Path, payload: &std::path::Path) -> bool {
    if pane_cwd == payload {
        return true;
    }
    match (
        std::fs::canonicalize(pane_cwd),
        std::fs::canonicalize(payload),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Whether a spawn command names the agent as a word: "muse" and
/// "muse-code" match, "mymuse" does not — bounded by non-alphanumeric on
/// both sides.
pub(super) fn names_agent(command: &str, agent: &str) -> bool {
    command.match_indices(agent).any(|(index, _)| {
        let before = command[..index].chars().next_back();
        let after = command[index + agent.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric() && c != '_')
            && after.is_none_or(|c| !c.is_alphanumeric() && c != '_')
    })
}
