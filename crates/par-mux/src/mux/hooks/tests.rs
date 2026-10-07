use super::telemetry::parse_telemetry_object;
use super::*;
use crate::mux::pane::ShellPaneFactory;
use base64::Engine as _;

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

/// The pane's stored telemetry decoded back to the canonical JSON
/// object the roster token carries.
fn stored_telemetry_json(pane: &MuxPane, expect: &str) -> serde_json::Value {
    let stored = pane.telemetry.as_ref().expect(expect);
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&stored.canonical_b64)
        .expect("the token is standard base64");
    serde_json::from_slice(&raw).expect("the token is canonical JSON")
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
        notification,
        Some(TmuxNotification::AgentTelemetryChanged {
            pane_id: pane_id.to_string(),
            agent: "claude".to_string(),
        }),
        "an accepted write pushes the identity-only telemetry signal"
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
    let stored: serde_json::Value = stored_telemetry_json(pane, "the canonical object is stored");
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
    let stored: serde_json::Value = stored_telemetry_json(pane, "still stored");
    assert_eq!(
        stored["sampled_at_unix_ms"], sampled_at,
        "same-seq replay dropped"
    );
}

#[test]
fn stale_telemetry_sample_is_dropped_absent_beats_stale() {
    let (tree, pane_id) = tree_with_pane();
    // Two hours old — past the 55-minute freshness window.
    let (reply, notification) = handle_report(
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
    assert_eq!(notification, None, "a dropped report broadcasts nothing");
    let guard = tree.lock();
    let pane = guard.pane(pane_id).expect("pane exists");
    assert!(
        pane.telemetry.is_none(),
        "absent is served instead of stale"
    );
    assert!(
        !pane.metadata().contains_key("agent_seq"),
        "a dropped report writes nothing, not even the sequence stamp"
    );
}

#[test]
fn future_dated_telemetry_sample_is_rejected() {
    let (tree, pane_id) = tree_with_pane();
    // A minute into the future — the freshness subtraction would
    // saturate to 0 and read as maximally fresh (QA-156).
    let (reply, notification) = handle_report(
        &telemetry_report(
            pane_id,
            "claude",
            1_000,
            &sample_telemetry(now_unix_ms() + 60_000),
        ),
        &tree,
    );
    assert!(
        reply.contains("sampled_at_unix_ms is in the future"),
        "error-replied as malformed: {reply}"
    );
    assert_eq!(notification, None, "a rejected report broadcasts nothing");
    let guard = tree.lock();
    let pane = guard.pane(pane_id).expect("pane exists");
    assert!(
        pane.telemetry.is_none(),
        "future-dated telemetry must not be stored"
    );
    assert!(
        !pane.metadata().contains_key("agent_seq"),
        "a rejected report writes nothing, not even the sequence stamp"
    );
}

/// ENH-029: the typed schema's canonical blob is byte-identical to
/// the hand-rolled validator's output. The literal was captured from
/// the pre-change code (full-field sample, fixed sampled_at).
#[test]
fn telemetry_canonical_blob_is_byte_identical() {
    let params: serde_json::Value = serde_json::from_str(
        r#"{"telemetry":{"version":1,"source":"claude_code","sampled_at_unix_ms":1700000000000,"model":"GLM-5.3","effort":"high","thinking_enabled":true,"context_used_percent":63,"context_remaining_percent":37,"five_hour_remaining_percent":80,"seven_day_remaining_percent":95,"five_hour_resets_at_unix_ms":1700003600000,"seven_day_resets_at_unix_ms":1700086400000}}"#,
    )
    .unwrap();
    let (canonical, sampled_at) = parse_telemetry_object(&params).unwrap();
    assert_eq!(sampled_at, 1_700_000_000_000);
    assert_eq!(
        canonical,
        r#"{"context_remaining_percent":37,"context_used_percent":63,"effort":"high","five_hour_remaining_percent":80,"five_hour_resets_at_unix_ms":1700003600000,"model":"GLM-5.3","sampled_at_unix_ms":1700000000000,"seven_day_remaining_percent":95,"seven_day_resets_at_unix_ms":1700086400000,"source":"claude_code","thinking_enabled":true,"version":1}"#
    );
}

/// ENH-029: unknown keys are ignored, not rejected — the hub may add
/// fields before a version bump, so the wire contract tolerates them.
#[test]
fn telemetry_unknown_keys_are_ignored() {
    let base = sample_telemetry(now_unix_ms() - 1_000);
    let mut object: serde_json::Value = serde_json::from_str(&base).unwrap();
    object
        .as_object_mut()
        .unwrap()
        .insert("new_hub_field".to_string(), serde_json::Value::from("x"));
    let params = serde_json::json!({ "telemetry": object });
    let (canonical, _) =
        parse_telemetry_object(&params).expect("unknown keys do not reject the report");
    let (without, _) = parse_telemetry_object(&serde_json::json!({
        "telemetry": serde_json::from_str::<serde_json::Value>(&base).unwrap()
    }))
    .unwrap();
    assert_eq!(canonical, without);
    assert!(!canonical.contains("new_hub_field"));
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
    let stored: serde_json::Value = stored_telemetry_json(pane, "still stored");
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
    assert!(pane.telemetry.is_none());
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
    let stored: serde_json::Value = stored_telemetry_json(pane, "stored");
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
        pane.telemetry.is_none(),
        "a dead agent's telemetry left with its claim"
    );
}

#[test]
fn telemetry_from_a_different_agent_never_takes_over_the_claim() {
    let (tree, pane_id) = tree_with_pane();
    handle_report(&state_report(pane_id, "pi", "working", 1_000), &tree);
    let (reply, notification) = handle_report(
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
    assert_eq!(notification, None, "a dropped report broadcasts nothing");
    let guard = tree.lock();
    let pane = guard.pane(pane_id).expect("pane exists");
    assert_eq!(
        pane.metadata().get("agent").map(String::as_str),
        Some("pi"),
        "the live claim keeps its label"
    );
    assert!(
        pane.telemetry.is_none(),
        "a foreign agent's telemetry never attaches"
    );
}

/// The roster reader's half of absent-beats-stale: a stored sample
/// aged past the window serves nothing, so the row loses its telemetry
/// token without any write (fabricated state — the write side would
/// never let this sample land).
#[test]
fn fresh_telemetry_serving_drops_aged_samples() {
    let (tree, pane_id) = tree_with_pane();
    {
        let mut guard = tree.lock();
        let pane = guard.pane_mut(pane_id).expect("pane exists");
        pane.set_metadata("agent", "kimi");
        pane.telemetry = Some(StoredTelemetry::new(
            r#"{"version":1,"source":"claude_code"}"#,
            now_unix_ms() - 2 * 3_600_000,
        ));
    }
    let guard = tree.lock();
    let pane = guard.pane(pane_id).expect("pane exists");
    assert_eq!(
        fresh_telemetry_b64(pane.telemetry.as_ref()),
        None,
        "an aged sample is served as absent"
    );
}

/// ARC-113: the roster token is encoded once, when the report is
/// accepted. A fresh read serves the stored string itself — no JSON
/// parse, no re-encode — and it is the canonical blob's base64.
#[test]
fn telemetry_token_is_encoded_once() {
    let (tree, pane_id) = tree_with_pane();
    let (reply, _) = handle_report(
        &telemetry_report(pane_id, "claude", 1_000, &sample_telemetry(now_unix_ms())),
        &tree,
    );
    assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
    let guard = tree.lock();
    let pane = guard.pane(pane_id).expect("pane exists");
    let stored = pane.telemetry.as_ref().expect("stored");
    let served = fresh_telemetry_b64(Some(stored)).expect("fresh");
    assert!(
        std::ptr::eq(served, stored.canonical_b64.as_str()),
        "the roster serves the stored token, not a fresh encoding"
    );
    let params: serde_json::Value = serde_json::from_str(&format!(
        r#"{{"telemetry":{}}}"#,
        sample_telemetry(stored.sampled_at_unix_ms)
    ))
    .unwrap();
    let (canonical, _) = parse_telemetry_object(&params).unwrap();
    assert_eq!(
        served,
        base64::engine::general_purpose::STANDARD.encode(canonical),
        "the token is the canonical blob's base64, byte-identical to before"
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
fn sourced_state_report(pane: PaneId, agent: &str, state: &str, seq: u64, source: &str) -> String {
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
    let (_, notification) = handle_report(&state_report(pane_id, "kimi", "working", 1_000), &tree);
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
    let (_, notification) = handle_report(&state_report(pane_id, "kimi", "working", 1_000), &tree);
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
    assert!(
        pane.seq_by_source.is_empty(),
        "release cleared the per-source sequence stamps"
    );
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
