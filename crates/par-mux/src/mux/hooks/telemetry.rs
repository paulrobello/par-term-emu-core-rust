//! `pane.report_agent_telemetry`: versioned, bounded, display-only agent
//! telemetry, and the freshness rule its roster token is served under.

use super::{check_value_len, error_reply, is_stale, ok_reply, parse_header, record_seq};
use crate::mux::tree::MuxTree;
use base64::Engine as _;
use par_term_emu_core::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A pane's accepted telemetry (ARC-113): the sample time the ordering and
/// freshness checks read, and the roster token — the canonical JSON (the
/// validated v1 shape below), base64-encoded once when the report is
/// accepted. Display-only and never persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredTelemetry {
    pub(crate) sampled_at_unix_ms: u64,
    pub(crate) canonical_b64: String,
}

impl StoredTelemetry {
    pub(super) fn new(canonical: &str, sampled_at_unix_ms: u64) -> Self {
        Self {
            sampled_at_unix_ms,
            canonical_b64: base64::engine::general_purpose::STANDARD.encode(canonical),
        }
    }
}

/// How old a telemetry sample may be, in milliseconds of wall clock,
/// before the endpoint drops it: the hub's `STATUSLINE_FRESHNESS_SECONDS`
/// (55 min, par-remote-herd status_telemetry.py), mirrored so daemon and
/// hub age data out at the same rate. Absent beats stale — a dropped
/// sample leaves readers serving nothing rather than something expired.
pub(crate) const TELEMETRY_FRESHNESS_MS: u64 = 55 * 60 * 1000;

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
/// per-source `seq` rule. An accepted write broadcasts
/// `%agent-telemetry-changed` (card 01a0e3f11e287f038dadcf332a1af961) —
/// identity only, no values: clients re-query the roster. Four silent
/// drops, no write and no broadcast: a sample past the freshness window,
/// a report at or below the last accepted `seq`, a sample older than
/// the one already stored (a backward step, whatever its `seq`), and a
/// report whose agent is not the pane's current label (telemetry attaches
/// to a claim; it never takes one over).
pub(super) fn handle_telemetry_report(
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
    if is_stale(pane, header.source.as_deref(), header.seq) {
        return (ok_reply(id), None);
    }
    // A future stamp would saturate the freshness subtraction to 0 and
    // read as maximally fresh forever (QA-156) — it is malformed, not
    // stale, so it error-replies instead of dropping silently.
    if sampled_at > now_ms {
        return (
            error_reply(id, "telemetry sampled_at_unix_ms is in the future"),
            None,
        );
    }
    if now_ms.saturating_sub(sampled_at) > TELEMETRY_FRESHNESS_MS {
        return (ok_reply(id), None);
    }
    if pane
        .telemetry
        .as_ref()
        .is_some_and(|stored| sampled_at < stored.sampled_at_unix_ms)
    {
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
    pane.telemetry = Some(StoredTelemetry::new(&telemetry, sampled_at));
    (
        ok_reply(id),
        Some(TmuxNotification::AgentTelemetryChanged {
            pane_id: header.pane_id.to_string(),
            agent: header.agent.clone(),
        }),
    )
}

/// The canonical stored form of a v1 telemetry report: exactly the
/// fields [`TelemetryV1::from_wire`] validates, re-serialized from
/// validated values so nothing unbounded or unknown reaches pane
/// metadata. Mirrors the hub normalizer's bounds: `model` ≤ 128 chars,
/// `effort` ≤ 32, percents finite 0-100 rounded, strings trimmed,
/// non-empty, control-character-free.
///
/// Declaration order IS the canonical key order — alphabetical, the
/// shape the pre-ENH-029 `serde_json::Map` (BTreeMap, no
/// `preserve_order` feature) emitted — so stored blobs stay
/// byte-identical, pinned by test. `skip_serializing_if` keeps absent
/// optional fields out of the blob, like the per-field inserts before.
#[derive(Serialize)]
struct TelemetryV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    context_remaining_percent: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_used_percent: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    five_hour_remaining_percent: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    five_hour_resets_at_unix_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    sampled_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    seven_day_remaining_percent: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seven_day_resets_at_unix_ms: Option<u64>,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_enabled: Option<bool>,
    version: u64,
}

impl TelemetryV1 {
    /// Validate a wire `telemetry` object into the canonical form.
    /// Unknown keys are ignored, not rejected — the hub may add fields
    /// before a version bump, so `deny_unknown_fields` would break the
    /// wire contract. Field order preserves the historical validation
    /// order (required fields first), which decides which error a
    /// multiply-invalid payload reports.
    fn from_wire(object: &serde_json::Map<String, serde_json::Value>) -> Result<Self, String> {
        let version = required_u64(object, "version")?;
        if version != 1 {
            return Err(format!(
                "unsupported telemetry version: {version} (expected 1)"
            ));
        }
        let source = telemetry_source(object)?;
        let sampled_at_unix_ms = required_u64(object, "sampled_at_unix_ms")?;
        let model = bounded_string(object, "model", 128)?;
        let effort = bounded_string(object, "effort", 32)?;
        let thinking_enabled = optional_bool(object, "thinking_enabled")?;
        let context_used_percent = parse_percent(object, "context_used_percent")?;
        let context_remaining_percent = parse_percent(object, "context_remaining_percent")?;
        let five_hour_remaining_percent = parse_percent(object, "five_hour_remaining_percent")?;
        let seven_day_remaining_percent = parse_percent(object, "seven_day_remaining_percent")?;
        let five_hour_resets_at_unix_ms = unix_ms_field(object, "five_hour_resets_at_unix_ms")?;
        let seven_day_resets_at_unix_ms = unix_ms_field(object, "seven_day_resets_at_unix_ms")?;
        Ok(Self {
            version,
            source,
            sampled_at_unix_ms,
            model,
            effort,
            thinking_enabled,
            context_used_percent,
            context_remaining_percent,
            five_hour_remaining_percent,
            seven_day_remaining_percent,
            five_hour_resets_at_unix_ms,
            seven_day_resets_at_unix_ms,
        })
    }
}

/// The wire report's `telemetry` member as an object.
fn telemetry_wire_object(
    params: &serde_json::Value,
) -> Result<&serde_json::Map<String, serde_json::Value>, String> {
    match params.get("telemetry") {
        Some(value) => value
            .as_object()
            .ok_or_else(|| "telemetry must be an object".to_string()),
        None => Err("missing telemetry".to_string()),
    }
}

/// One required non-negative-integer field (`version`,
/// `sampled_at_unix_ms`).
fn required_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<u64, String> {
    match object.get(field) {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| format!("telemetry {field} must be a non-negative integer")),
        None => Err(format!("telemetry missing {field}")),
    }
}

/// The required `source` string: trimmed, non-empty,
/// control-character-free, length-checked.
fn telemetry_source(object: &serde_json::Map<String, serde_json::Value>) -> Result<String, String> {
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
    Ok(source.to_string())
}

/// One optional boolean field.
fn optional_bool(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<bool>, String> {
    match object.get(field) {
        Some(value) => {
            Ok(Some(value.as_bool().ok_or_else(|| {
                format!("telemetry {field} must be a boolean")
            })?))
        }
        None => Ok(None),
    }
}

/// Validate the `telemetry` object into its canonical stored form (see
/// [`TelemetryV1`]).
pub(super) fn parse_telemetry_object(params: &serde_json::Value) -> Result<(String, u64), String> {
    let telemetry = TelemetryV1::from_wire(telemetry_wire_object(params)?)?;
    let canonical = serde_json::to_string(&telemetry).map_err(|error| error.to_string())?;
    check_value_len("telemetry", &canonical)?;
    Ok((canonical, telemetry.sampled_at_unix_ms))
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

/// The pane's telemetry for the roster row: the stored canonical JSON,
/// base64-encoded so it rides as ONE whitespace-free token
/// (`telemetry=<b64>` — string values carry spaces, the row is
/// space-split). Encoded once when the report was accepted; this only
/// checks freshness. `None` when the pane holds no telemetry or the sample
/// has aged past [`TELEMETRY_FRESHNESS_MS`] — the reader-side half of
/// absent-beats-stale, so a pane whose hook stopped pushing serves the
/// plain four-token row again without any write.
pub(crate) fn fresh_telemetry_b64(stored: Option<&StoredTelemetry>) -> Option<&str> {
    let stored = stored?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(u64::MAX);
    if now_ms.saturating_sub(stored.sampled_at_unix_ms) > TELEMETRY_FRESHNESS_MS {
        return None;
    }
    Some(&stored.canonical_b64)
}
