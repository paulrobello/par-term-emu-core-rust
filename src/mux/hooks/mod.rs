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
use std::collections::HashMap;
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

/// The agent claim on a pane, typed (ARC-113c): roster label, reported
/// state, hook authority, session identity, and the liveness sweep's miss
/// counter — the twelve metadata keys [`AGENT_CLAIM_KEYS`] lists, read and
/// written through one definition instead of a dozen `set_metadata` calls.
///
/// Storage stays the pane's stringly metadata map, because the map IS the
/// wire-and-disk contract: the roster reader and `agent_session_from_metadata`
/// key off these exact strings, and the save format copies named identity
/// fields from them. The claim is therefore a typed VIEW — [`Self::from_metadata`]
/// parses, [`Self::write_to_metadata`] renders — and unparseable numeric or
/// argv values read as absent (a corrupted counter disappears on the next
/// legitimate claim write rather than poisoning it). The serde form below is
/// the migration-friendly representation for a future claim-shaped block in
/// the state file; nothing writes it yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AgentClaim {
    /// The roster label — the claim's anchor: a pane is claimed iff the
    /// `agent` key is present.
    pub agent: String,
    /// The reported state (working/blocked/idle).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Who authored the state: `"hook"` (permanent once set) or `"scrape"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_source: Option<String>,
    /// The blocked reason, whitespace-collapsed at the report door.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The reporting script's source tag (`par-mux:claude:session-hook`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The most recent accepted report's sequence number, whatever its source.
    #[serde(rename = "agent_seq", skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// The agent's session id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The agent's transcript path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_path: Option<String>,
    /// Startup-vs-resume provenance, recorded when the wire carries it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_start_source: Option<String>,
    /// The agent's own resume invocation, the JSON argv string the report
    /// stored verbatim (`["pi","--session","<path>"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_argv: Option<Vec<String>>,
    /// Unbroken liveness mismatches before the sweep clears the claim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liveness_misses: Option<u8>,
    /// The agent label the miss count belongs to; a relabel starts fresh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liveness_misses_agent: Option<String>,
}

impl AgentClaim {
    /// Parse the claim out of a pane's metadata map, or `None` when the
    /// pane carries no `agent` label. Numeric and argv fields parse
    /// leniently: a value that does not decode reads as absent.
    pub(crate) fn from_metadata(metadata: &HashMap<String, String>) -> Option<Self> {
        let mut claim = Self {
            agent: metadata.get("agent")?.clone(),
            ..Self::default()
        };
        let get = |key: &str| metadata.get(key).map(String::as_str);
        claim.state = get("agent_state").map(str::to_string);
        claim.state_source = get("agent_state_source").map(str::to_string);
        claim.message = get("agent_message").map(str::to_string);
        claim.source = get("agent_source").map(str::to_string);
        claim.seq = get("agent_seq").and_then(|value| value.parse().ok());
        claim.session_id = get("agent_session_id").map(str::to_string);
        claim.session_path = get("agent_session_path").map(str::to_string);
        claim.session_start_source = get("agent_session_start_source").map(str::to_string);
        claim.resume_argv = get("agent_resume_argv").and_then(|value| {
            serde_json::from_str::<Vec<String>>(value)
                .ok()
                .filter(|argv| !argv.is_empty())
        });
        claim.liveness_misses = get("agent_liveness_misses").and_then(|value| value.parse().ok());
        claim.liveness_misses_agent = get("agent_liveness_misses_agent").map(str::to_string);
        Some(claim)
    }

    /// Render the claim back into a pane's metadata map: the label and every
    /// present field are written under their stringly keys, every absent
    /// field's key is REMOVED — a whole-claim write, so a field the caller
    /// cleared cannot survive as a stale string.
    pub(crate) fn write_to_metadata(&self, metadata: &mut HashMap<String, String>) {
        fn put(metadata: &mut HashMap<String, String>, key: &str, value: &Option<String>) {
            match value {
                Some(value) => metadata.insert(key.to_string(), value.clone()),
                None => metadata.remove(key),
            };
        }
        metadata.insert("agent".to_string(), self.agent.clone());
        put(metadata, "agent_state", &self.state);
        put(metadata, "agent_state_source", &self.state_source);
        put(metadata, "agent_message", &self.message);
        put(metadata, "agent_source", &self.source);
        match &self.seq {
            Some(seq) => metadata.insert("agent_seq".to_string(), seq.to_string()),
            None => metadata.remove("agent_seq"),
        };
        put(metadata, "agent_session_id", &self.session_id);
        put(metadata, "agent_session_path", &self.session_path);
        put(
            metadata,
            "agent_session_start_source",
            &self.session_start_source,
        );
        match &self.resume_argv {
            Some(argv) => metadata.insert(
                "agent_resume_argv".to_string(),
                serde_json::to_string(argv).unwrap_or_default(),
            ),
            None => metadata.remove("agent_resume_argv"),
        };
        match &self.liveness_misses {
            Some(misses) => {
                metadata.insert("agent_liveness_misses".to_string(), misses.to_string())
            }
            None => metadata.remove("agent_liveness_misses"),
        };
        put(
            metadata,
            "agent_liveness_misses_agent",
            &self.liveness_misses_agent,
        );
    }
}

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

/// Record `seq` as accepted: the plain `agent_seq` stamp through the typed
/// claim and the reporting source's own bucket in
/// [`MuxPane::seq_by_source`]. Every caller writes the `agent` label before
/// this, so the claim exists (a telemetry-only claim included). Volatile
/// like everything state-shaped — the save format copies named identity
/// fields only, so the buckets never reach disk.
fn record_seq(pane: &mut MuxPane, source: Option<&str>, seq: u64) {
    pane.update_agent_claim(|claim| claim.seq = Some(seq));
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

/// Dispatch one hook-report line against a per-pane endpoint bound to
/// `bound` (ENH-039). Identical to [`handle_report`] except the binding: a
/// report naming another pane is refused (`pane_id does not match this
/// endpoint`) and a report omitting `pane_id` is filed for the bound pane,
/// so a minimal hook script needs no pane id at all. The full control
/// socket keeps [`handle_report`]'s unbound behavior — embedders and
/// par-term's own control connection still report for any pane.
pub fn handle_report_for(
    bound: PaneId,
    line: &str,
    tree: &Arc<Mutex<MuxTree>>,
) -> (String, Option<TmuxNotification>) {
    let mut report: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(err) => return (error_reply(None, &format!("invalid JSON: {err}")), None),
    };
    let id = report.get("id").cloned();
    match report
        .get("params")
        .and_then(|params| params.get("pane_id"))
    {
        // No pane named: the endpoint IS the pane.
        None | Some(serde_json::Value::Null) => {
            if let Some(params) = report.get_mut("params").and_then(|p| p.as_object_mut()) {
                params.insert(
                    "pane_id".to_string(),
                    serde_json::Value::String(bound.to_string()),
                );
            }
        }
        // A pane named that is not this endpoint's: refused before anything
        // is looked up or written.
        Some(value) => {
            let named = value.as_str().and_then(|raw| PaneId::from_str(raw).ok());
            if named != Some(bound) {
                return (
                    error_reply(id, "pane_id does not match this endpoint"),
                    None,
                );
            }
        }
    }
    // The (possibly pane-filled) report goes through the unbound dispatch:
    // same method table, same validation, same per-source seq rule.
    handle_report(&report.to_string(), tree)
}

#[cfg(test)]
mod pane_binding_tests {
    use super::*;
    use crate::mux::pane::ShellPaneFactory;

    /// A tree with one pane (`%0`).
    fn pane_tree() -> Arc<Mutex<MuxTree>> {
        let mut tree = MuxTree::new(Box::new(ShellPaneFactory::default()));
        tree.new_session("t", 80, 24)
            .expect("the test session spawns");
        Arc::new(Mutex::new(tree))
    }

    /// A report naming another pane never passes the binding — checked
    /// before anything is looked up or written, even for a pane that does
    /// not exist.
    #[test]
    fn a_report_for_another_pane_is_refused() {
        let tree = pane_tree();
        let bound: PaneId = "%0".parse().expect("parses");
        let (reply, broadcast) = handle_report_for(
            bound,
            r#"{"id":1,"method":"pane.report_agent","params":{"pane_id":"%9","agent":"a","seq":1,"state":"working"}}"#,
            &tree,
        );
        assert!(
            reply.contains("pane_id does not match this endpoint"),
            "{reply}"
        );
        assert!(broadcast.is_none());
    }

    /// A report that omits pane_id is filed for the bound pane.
    #[test]
    fn an_omitted_pane_id_is_filled_with_the_bound_pane() {
        let tree = pane_tree();
        let bound: PaneId = "%0".parse().expect("parses");
        let (reply, broadcast) = handle_report_for(
            bound,
            r#"{"id":2,"method":"pane.report_agent","params":{"agent":"a","seq":1,"state":"working"}}"#,
            &tree,
        );
        assert!(reply.contains("\"result\":\"ok\""), "{reply}");
        assert!(broadcast.is_some(), "an accepted state report broadcasts");
        assert_eq!(
            tree.lock()
                .pane(bound)
                .expect("pane")
                .metadata()
                .get("agent_state")
                .map(String::as_str),
            Some("working"),
            "the pane-less report landed on the bound pane"
        );
    }
}
