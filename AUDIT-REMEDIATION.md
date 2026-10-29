# Audit Remediation Report

> **Project**: par-term-emu-core-rust
> **Audit Date**: 2026-10-29
> **Remediation Date**: 2026-10-29
> **Severity Filter Applied**: all
> **Plan Source**: AUDIT.md `## Remediation Plan` (no AUDIT-REMEDIATION-PLAN.md playbook)
> **Implementation Model**: Opus 5 (all fix agents)

---

## Execution Summary

| Phase | Status | Agent | Issues Targeted | Resolved | Partial | Manual |
|-------|--------|-------|----------------|----------|---------|--------|
| 1 — Critical Security | ⏭️ Skipped | — | 0 (none filed) | — | — | — |
| 2 — Critical Architecture | ✅ | fix-architecture | 1 (ARC-003) | 1 | 0 | 0 |
| 3a — Security (remaining) | ✅ | fix-security | 6 | 6 | 0 | 0 |
| 3b — Architecture (remaining) | ✅ | fix-architecture | 10 | 9 | 1 (ARC-004) | 0 |
| 3c — All Code Quality | ✅ | fix-code-quality | 9 | 8 | 0 | 0 (1 pre-done) |
| 3d — All Documentation | ✅ | fix-documentation | 6 | 6 | 0 | 0 |
| 4 — Verification | ✅ | — | — | — | — | — |

**Overall**: 30 of 32 actionable items landed (1 partial with a filed design follow-up, 1 deliberately not attempted, 1 found already done). The run also absorbed a mid-run upstream card from par-director (doc-error portion) and four orchestrator fixes for drift the remediation itself created.

---

## Resolved Issues ✅

### Security
- **[SEC-001]** Unbounded retained graphics memory — `graphics/{mod.rs,animation.rs,kitty/dispatch.rs}` — `GraphicsLimits::max_total_memory` is now an enforced budget with oldest-first eviction; new count caps `MAX_KITTY_IMAGES` (4,096), `MAX_ANIMATION_FRAMES` (1,024), `MAX_VIRTUAL_PLACEMENTS` (4,096); shared buffers counted once; visible/scrollback placements byte-budgeted (worst case ~2× budget, documented). Caps table regenerated; fuzz seed added. Verified: 1,918 core tests, 60s fuzz run peak 327 MB.
- **[SEC-002]** PNG decompression bomb — `decode_bounded_rgba` rejects declared-size violations before decode and caps decoder allocation at `MAX_IMAGE_PIXELS * 4`; both kitty and iTerm2 paths. Pinned by `png_pixel_bomb_is_rejected_from_header`.
- **[SEC-003]** Legacy htpasswd hashes — BREAKING: `{SHA}`, `$1$`, `$apr1$` refuse at streamer startup by default; `--allow-legacy-hash` / `PAR_TERM_ALLOW_LEGACY_HASH` opt-out; error names user+format, never the hash.
- **[SEC-004]** api_key-in-URL — documented residual in docs/SECURITY.md (browser WebSockets cannot set headers; mitigations: Basic Auth, TLS, key rotation).
- **[SEC-005]** Loopback bind check — exact-literal `localhost` / loopback IP literals only; `127.example.com` no longer suppresses the unauthenticated-bind warning.
- **[SEC-006]** Env values in mux logs — `summarize_line` redacts `set-environment` values and `new-session -e KEY=VALUE` values to `[redacted]`; honors `--`; 120-byte cut retained.

### Architecture
- **[ARC-001]** Terminal monolith — `ApcPrefilterState` + `BellVolumeState` extracted (55→52 fields); new `make terminal-fields-check` ratchet gate blocks future field growth.
- **[ARC-002]** Facade pyclasses — new `TerminalClipboard(term)` pyclass (`clipboard_api.rs`); flat `Terminal` surface unchanged (api-surface 389/206); `try_borrow` re-entrance raises instead of panicking.
- **[ARC-003]** Attach dispatch cluster — 5 commits: `route_bytes` chord/management helpers, `route_mouse` arm-body extraction, `SettingsValues::field` chord table, `pump_status`/`pump_frame` split, `handle_event` on_* helpers. ENH-050 phase 6 + import hygiene closed.
- **[ARC-004]** Snapshot export (partial) — `TextExportSnapshot`; `PtySession::{export_text,export_styled,scrollback}` hold the read lock only for the grid clone; byte-identical output, three parity tests. Remainder (screenshot/graphics) filed as design follow-up.
- **[ARC-005]** mux_factory split — `mux_factory/{lifecycle,stream,mirror,control,upload,tests}.rs`.
- **[ARC-006]** CLI god file — `main()` 372→6 lines; `dispatch.rs`/`runners.rs`/`daemon.rs` (`DaemonLaunch`/`Serving`); no clap changes, completions green.
- **[ARC-007]** persist split — `persist/{format,save,restore,state_file,layouts,tests,serde_tests}.rs`; serde field order unchanged.
- **[ARC-008]** Derive-generated dict conversion — `#[pydict(nested)]`, `nested(keep_empty)`, struct-field `skip`; hand-written `AgentRoster`/`AgentStateChanged` converters retired; dict shape pinned byte-identical. **Derive crate needs a version bump at the next release cut** (filed, release-blocker).
- **[ARC-009]** Version-train writer — `scripts/set_version_train.py` + `make set-version VERSION=X.Y.Z` + `make version-train-check`; atomic, refuses drift, never touches `derive/`.
- **[ARC-010]** Embedder path — "Which Crate Path to Import" in docs/RUST_USAGE.md + src/lib.rs tier docs.

### Code Quality
- **[QA-703]** Settings registry — `SettingSpec` rows carry getter/empty-text/choice-writer in per-section constant tables; string-keyed `field`/`chord_field`/`option_field`/`MANAGEMENT_CHORDS` removed. 2 new tests.
- **[QA-704]** Extension engine — **already done** (3b8cd541, 2026-10-16): `gen_extensions.py` renders both assets from one template; audit finding was stale. Verified `--check` exit 0 on the merged tree.
- **[QA-706]** XTWINOPS filter — extracted verbatim to `filter_xtwinops_replies`, still called inside the write guard (ARC-027 contract held). 2 unit tests. (Found, left: non-alphabetic CSI final byte does not end the scan — pre-existing.)
- **[QA-707]** Duplicate families — shared `find_flat_cut_in`/`any_entry`/`apply_cuts`; `round_trips` generic helper; the five genuinely-different row grammars left hand-written.
- **[QA-708]** Gate boilerplate — `scripts/_gate_common.py` (`ROOT`, `fail()`, `capture()`, `expect_failure()`); 5 scripts converted; 11 before/after gate runs byte-identical.
- **[QA-709]** mux_attach split — 94 tests verbatim into 5 themed files under `tests/mux_attach/`; crate root keeps helpers; `#[cfg(unix)]` on unix-only modules; `--list` output identical; nextest overrides still match.
- **[QA-710]** README "Load-Sensitive Tests" subsection (label, skip behavior, `make test-rust-sensitive`, quiet-host guidance).
- **[QA-711]** Derive trim — per-arm helpers (`variant_info`, `field_info`, `tag_arm`, `to_arm`, `from_arm`, `field_init`, `variant_pattern`); golden expansion byte-identical across 7 inputs.
- **[QA-705]** Stale worktree — verified already removed before the run; card legitimately `done`.

### Documentation
- **[DOC-001]** ARCHITECTURE.md stamp refreshed to v0.71.1 + 11 missing-since-v0.63 modules documented; banner refresh added to CONTRIBUTING's pre-tag steps.
- **[DOC-002]** herdr citations marked herdr-tree-relative with waiver comments.
- **[DOC-003]** CONTRIBUTING version-sync rewritten to the five-file lockstep + three pins + gates (merged with ARC-009's `make set-version` flow at the merge).
- **[DOC-004]** Pinned-tests backtick typo fixed.
- **[DOC-005]** ENH-048 plan annotated (pre-ARC-007 paths).
- **[DOC-006]** docs/README.md index (27 docs + archives).

### Orchestrator fixes during the run (drift the remediation created)
- MUX_DECISIONS.md rows D3.2–D3.5/D5/D6.3/D8 and MUX.md persistence rows re-pointed to the split `persist/` and `daemon.rs` modules (`mux-docs-check` green).
- CONTRIBUTING merge conflict resolved by combining DOC-003's gate detail with ARC-009's `make set-version`.
- CHANGELOG merge conflict resolved (all entries kept, BREAKING first under Changed).
- `get_dropped_sixel_graphics` docstring + API_REFERENCE corrected: the counter is the shared Sixel+Kitty store drop count including budget evictions.
- par-director's upstream card (doc-error portion): SKILL.md config path fixed in repo source AND installed copy; `question` added to the states in `references/agent-env-and-hooks.md` and docs/MUX.md. (Finding: `references/` exists only in the installed skill, not the repo — back-port candidate.)

---

## Not Attempted / Requires Decision 🔧

- **[ARC-011]** csi_dispatch table indirection — deliberately not attempted: the audit calls it "optional polish", and rewriting protocol dispatch carries regression risk for no measurable gain. Card remains backlog with the reasoning; revisit only if `csi_dispatch_impl` shows up as a hotspot.
- **[ARC-004 remainder]** Screenshot/graphics export still renders under the lock (`render_terminal` takes `&Terminal`; needs a graphics-store snapshot design). Filed as backlog card.
- **par-director card (features)** — per-tool heartbeat / derived `stalled` state and a durable resumable event stream: feature asks needing design; card stays open.

---

## Verification Results

- Build + fast gate: ✅ `make dev` 0, `make stubs` 0, `make checkall` **exit 0** on the final merged tree (all 15 gate lines ✓, 61 test-result groups ok)
- CI-parity gate: ✅ `make checkall-ci` **exit 0** (covers the 94 relocated load-sensitive mux_attach tests via nextest include-ignored, Windows cross-target checks, and the streaming CI test)
- Per-issue: every card closed with orchestrator-verified evidence (grep-confirmed presence + agent-run suites, per-issue commands and exit codes in each card's notes)
- Known flakes (pre-existing, host-load, not regressions): `a_push_queued_across_the_seed_refresh_still_toasts`, `a_timed_out_pane_info_never_adopts_the_pane_as_a_window` — pass in isolation.

---

## Files Changed

44 commits on `fix/audit-remediation` (base `bdbef990` → tip with this report): 96 files, +22,632/−19,377; 29 files added, 67 modified. Notable new files: `scripts/{check_terminal_fields,set_version_train}.py`, `scripts/_gate_common.py`, `src/streaming/mux_factory/*` (6), `crates/par-mux/src/mux/persist/*` (7), `crates/par-mux/src/bin/par_mux/{dispatch,runners,daemon}.rs`, `crates/par-term-emu-core/src/terminal/export_snapshot.rs`, `src/python_bindings/terminal/clipboard_api.rs`, `tests/test_terminal_clipboard_facade.py`, `crates/par-mux/tests/mux_attach/*` (5), `docs/README.md`, `fuzz/corpus/kitty/budget_ids.seed`.

---

## Next Steps

1. **Release cut**: bump `par-term-emu-derive` + the root's spec together (card filed, high) — ARC-008 changed derive behavior not present in published 0.48.0. Add SEC-003's breaking note to README "What's New" at the cut.
2. **Re-run `/audit`** for a fresh cycle on the remediated tree.
3. **Feature asks** live on the par-director card (heartbeat/stalled state, durable event stream) and the ARC-004 remainder card — both need design before implementation.
4. Backlog cards filed during the run: renderer.rs seam design (ENH-052), rustdoc `-D warnings` failure + cross-target-check root-crate gap.
