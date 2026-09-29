# ENH-029: Typed telemetry schema (`TelemetryV1`) replacing the hand-rolled validator

> Filed from the 2026-09-28 /opus-audit enhancement pass (cycle `audit-2026-09-28`). Board card: `[ENH-029]`.
> Sequencing: after AUDIT QA-156 (future-timestamp reject), which gives the typed validator its first new rule. Independent of ARC-086, which types the pane metadata. This card types the wire object.

## Goal

`parse_telemetry_object` (`src/mux/hooks.rs:567-670`, cyclomatic complexity 36) validates the `pane.report_agent_telemetry` object field by field with hand-written branches, then re-serializes a canonical JSON. `hooks.rs` grew by 747 lines this cycle.

Replace the branching with a `#[serde(deny_unknown_fields)]` struct plus small reusable validators. This makes the field list self-documenting, gives future rules (QA-156's future-timestamp check, a version-2 schema) an obvious home, and cuts complexity.

## Current state

- `parse_telemetry_object(params) -> Result<(String /*canonical json*/, u64 /*sampled_at*/), String>`:
  - Reads `telemetry` as an object and requires `version == 1`, a non-empty control-free `source` (length-checked by `check_value_len`) and `sampled_at_unix_ms: u64`.
  - Optional fields: `model` (≤128), `effort` (≤32), `thinking_enabled: bool`, context percents, 5h/7d rate-limit percents (finite, 0-100, rounded), and `*_resets_at_unix_ms`.
  - Builds a `serde_json::Map` in a fixed order and serializes it.
- **Unknown-field behavior today**: unknown keys are silently ignored (the canonical map is rebuilt from known fields). That behavior is part of the wire contract with the hub (par-remote-herd `status_telemetry.py`), so keep it by default.
- Error strings are part of the observable reply. Tests in `hooks.rs` assert many of them (grep `"telemetry` in the test module).
- `serde` and `serde_json` are already dependencies.

## Implementation

1. Define the wire struct in a new `src/mux/hooks/telemetry.rs`, or in `hooks.rs` if QA-165's split has not happened:
   ```rust
   #[derive(Deserialize)]
   struct TelemetryWireV1 {
       version: u64, source: String, sampled_at_unix_ms: u64,
       #[serde(default)] model: Option<String>, #[serde(default)] effort: Option<String>,
       #[serde(default)] thinking_enabled: Option<bool>,
       #[serde(default)] context_used_percent: Option<f64>, /* … every current optional field … */
   }
   ```
   Do **not** use `deny_unknown_fields`, because that would change behavior. Add a comment explaining why it is omitted.
2. Add validators as small fns: `bounded_text(name, value, max) -> Result<String, String>` (trim, non-empty, no control characters, char-count ≤ max), `percent(name, v) -> Result<u8, String>` (finite, 0..=100, rounded), and `not_future(name, ms, now) -> Result<u64, String>` (QA-156's rule).
3. Add `#[derive(Serialize)] struct TelemetryCanonicalV1 { … }` with `#[serde(skip_serializing_if = "Option::is_none")]`. Its field declaration order must equal the current canonical key order, so stored blobs stay byte-identical. Check this with a test that compares the old and new canonical strings for a full-field sample.
4. Rewrite `parse_telemetry_object` in three steps: `serde_json::from_value::<TelemetryWireV1>(params["telemetry"].clone())`, then map serde errors to the **existing** error strings where tests pin them (type-mismatch messages come from serde, so map `invalid type` errors per field name, or accept serde's wording and update the tests), then validate, then build the canonical struct and serialize it.
5. Keep the function signature and the `check_value_len("telemetry", &canonical)` final check.
6. Reuse the validators in `handle_session_report`/`handle_report` where the same trim/control/length logic is duplicated (grep `char::is_control` in `hooks.rs`), but only where behavior is identical.

## Files to touch

- `src/mux/hooks.rs` (or `src/mux/hooks/telemetry.rs` + `mod.rs`)
- Tests in the same module

## Verify

- Every existing telemetry test in `hooks.rs` passes. If error wording changes, update only the asserted substrings, and list each changed message in the PR.
- New test: a full-field sample's canonical JSON is byte-identical to the pre-change output. Capture it from the old code first and store it as a literal.
- New test: an unknown key is ignored (not rejected), as today.
- `parsight calculate_cyclomatic_complexity parse_telemetry_object` is ≤ 15 after a reindex (from 36).
- `cargo test --lib --no-default-features --features rust-only,mux,serde hooks` passes and `make checkall` is green.

## Rollback

Revert the module. The wire contract, the canonical storage format and the error behavior are unchanged, so no data migration is needed.
