# Audit Remediation Report

> **Project**: par-term-emu-core-rust
> **Audit Date**: 2026-08-27 (AUDIT.md @ e83f415, v0.46.0)
> **Remediation Date**: 2026-08-28
> **Severity Filter Applied**: all
> **Plan Source**: AUDIT.md `## Remediation Plan` + AUDIT-REMEDIATION-PLAN.md playbook
> **Implementation Model**: Opus 5 (all fix agents)

---

## Execution Summary

| Phase | Status | Agent | Issues Targeted | Resolved | Partial | Manual |
|-------|--------|-------|----------------:|---------:|---------|--------|
| 1 — Critical Security | ✅ | fix-security | 2 | 2 | 0 | 0 |
| 2 — Critical Architecture | ✅ | fix-architecture | 4 | 4 | 0 | 0 |
| 3a — Security (remaining) | ✅ | fix-security | 3 | 3 | 0 | 0 |
| 3b — Architecture (remaining) | ✅ | fix-architecture | 13 | 11 | 0 | 0 |
| 3c — All Code Quality | ✅ | fix-code-quality | 14 | 13 | 1 | 0 |
| 3d — All Documentation | ✅ | fix-documentation | 18 | 18 | 0 | 0 |
| 4 — Verification | ✅ | — | — | — | — | — |

**Overall**: 50 of 51 issues resolved (QA-005 included in 3c's count above), 1 deliberate partial with scope note (QA-004), 1 deferred by design (ARC-014), 0 requiring manual intervention. Two pre-existing bugs outside the audit were also found and fixed (see Out-of-Audit Fixes).

**Footprint**: 57 commits on `fix/audit-remediation` (base 42ffde7 → HEAD), 115 files changed, +11,485 / −15,151 lines. Working tree clean; every agent branch merged and cleaned up.

---

## Resolved Issues ✅

### Security
- **[SEC-001]** CORS permissive fallback — `src/streaming/server.rs` — no-allowlist branch now mirrors the WS local-origin default via `AllowOrigin::predicate` over the shared host classification; `very_permissive` fully removed; explicit-allowlist path untouched.
- **[SEC-002]** `/sessions` Origin check — `src/streaming/server.rs` — same `check_ws_origin` guard as the WS handlers, 403 on disallowed browser origins, no-origin (curl/native) still succeeds. 4 new axum oneshot tests.
- **[SEC-003]** Kitty path check — `src/graphics/kitty.rs` — component-wise `Component::ParentDir` rejection (`my..notes.png` now loads, `a/../b` still rejected); SECURITY.md no longer implies the check is a sandbox.
- **[SEC-004]** Debug log hardening — `src/debug.rs` — PID-suffixed name, `O_NOFOLLOW` + 0600 on Unix, fails closed on symlink; verified on disk.
- **[SEC-005]** api_key query docs — `docs/STREAMING.md` — documents the `--allow-api-key-in-query` gate and log/history leak risk. **Deviation**: recommends header auth rather than the playbook's "first-message auth", which does not exist in the code.

### Architecture
- **[ARC-001]** web_term/ drift gate — `.github/workflows/ci.yml` — Next build ID pinned to package version (determinism proven by byte-identical consecutive builds), CI job fails on `git diff --exit-code web_term/` naming `make web-build-static`.
- **[ARC-002]** py.typed + stubs — `python/par_term_emu_core_rust/{py.typed,_native.pyi}` + `scripts/generate_stubs.py` — runtime-introspection generator; 69 classes, 1,335 methods, 342 properties (setters as properties); `make stub-check` wired into checkall + CI; wheel shipping verified.
- **[ARC-003]** Streaming codec macro — `derive/src/lib.rs` `PyDictConvert` + `src/streaming/py_convert.rs` — four codec functions are now thin wrappers (2,077 → 1,082 lines); 75 characterization tests passed identically before and after; all protocol matches exhaustive (`_ =>` removed).
- **[ARC-004]** server.rs decomposition — `src/streaming/{config,session,rate_limit}.rs` — 4,224 → 2,741 lines, all types re-exported at old paths (zero call-site churn).
- **[ARC-005]** Binary split — `src/bin/streaming_server/{cli,frontend_download,bootstrap}.rs` — main.rs 1,790 → 572 lines; the three `.expect` panics eliminated structurally (PTY created before server wiring).
- **[ARC-006]** serde_yaml replaced — **Deviation**: `serde_yaml_ng` 0.10 instead of the playbook's `serde_yml`, which was itself deprecated (May 2026); `cargo tree -i serde_yaml` empty.
- **[ARC-007]** Dependency trims — tokio `full` → 8 named features; `exr`/`hdr`/`dds` dropped from image; `cargo tree -i paste` empty; stale Cargo.toml comment corrected.
- **[ARC-008]** Threading docs — CLAUDE.md + `src/pty_session.rs` now state `Arc<RwLock<Terminal>>` / `parking_lot::RwLock` with the read/write split.
- **[ARC-009]** Canonical lockfile — `package-lock.json` deleted (−6,917 lines), bun.lock canonical, gitignored against regeneration.
- **[ARC-010]** SessionState rename — folded into ARC-004's move; `StreamSessionState` throughout, `PySessionState` (multiplexing) untouched.
- **[ARC-011]** sim marker honest — comment + `compile_error!` guard for sim+python.
- **[ARC-012]** Derive publish story — conditional derive-publish step in publish-crates.yml + CLAUDE.md "Derive crate exception".
- **[ARC-013]** No action by design (guidance-only) — closed as such.

### Code Quality
- **[QA-001]** get_word_at (Critical) — `src/text_utils.rs`/`screen.rs` — display-column-correct cell walk; spacer columns resolve to their wide char; 20 Rust + 9 Python tests; documented behavior alignments (iTerm2 default word set; non-word chars return None) match API_REFERENCE.
- **[QA-002]** decode_server_message — verified moot after ARC-003: complexity 240 → 2; the one 86-line escape-hatch helper is per-family by design. Characterization suite 75/75.
- **[QA-003]** Connected builder — `ConnectedBuilder` + `connected_builder()`; `connected_full` via builder; four partial constructors deprecated; call sites migrated; CLAUDE.md checklist names the builder.
- **[QA-004]** Screenshot dedupe — `impl_terminal_screenshot_methods!` in common.rs serves both classes; 17-param signature byte-identical; `inspect.signature` parity test. **Partial by design**: `resize_pixels` NOT deduped — PtyTerminal's variant drives a real PTY resize (SIGWINCH) while PyTerminal's only touches the model; unifying would regress PTY resizing.
- **[QA-005]** Frontend harness — vitest + happy-dom; 35 tests over `lib/protocol.ts` and `TerminalConnection` (backoff schedule, heartbeat/stale-pong, shutdown classification, idempotent dispose); `make test-web` wired into checkall.
- **[QA-006]** write_char decomposition — three extracted helpers; ~510 → ~120 lines; Unicode suites green between each extraction.
- **[QA-007]** Dead code removal — 9 zero-caller rlib fns deleted (−581 lines) after grep-verifying `../par-term` and `../par-term-emu-tui-rust` (TUI imports `debug.py`, so it stays); `MouseMode::X10` kept for ENH-006; CHANGELOG "Removed" section added.
- **[QA-008]** Frontend extraction — `lib/terminal-connection.ts` (framework-free) + `lib/keyboard-layouts.ts`; Terminal.tsx 1,020 → ~870; `web_term/` regenerated with the pinned build ID.
- **[QA-009]** Color/pixel delegation — bindings delegate to core `pub(crate)` helpers; `color_utils` canonical with a 0–1 adapter for screen.rs. Found and fixed a real pre-existing bug: achromatic `rgb_to_hsl` from Python returned lightness 1.0 instead of 100.0.
- **[QA-010]** `CSI * x` — consumed as parsed-but-unimplemented no-op; bare `x` still reaches DECREQTPARM; tests for both.
- **[QA-011]** pytest.raises narrowed to OSError/ValueError per observed types; noqa removed.
- **[QA-012]** `_native` registration split into 5 grouped helpers; `len(dir())` 106 == 106.
- **[QA-013]** Production `eprintln!` → `log::error!` (3 sites); no logger init.
- **[QA-014]** No action by design (monitor-only) — closed as such.

### Documentation
- **[DOC-001]** QUICKSTART.md — MSRV 1.98, real clone URL, releases-page frontend instruction, port 8099, `streaming-bin` features (build command verified to exit 0), 39 examples, API link.
- **[DOC-002]** RUST_USAGE.md — `pty_session` in all PTY recipes, `sim` variant, 0.46 pins, parking_lot streaming example. **Compile-verified in three scratch crates** — caught and fixed three real doc bugs (wrong error type, move-after-builder, silent async send).
- **[DOC-003]** VT_TECHNICAL_REFERENCE — XTPUSHCOLORS/XTPOPCOLORS ❌, alt-screen 47/1047/1048 ❌, DECSACE "parsed and ignored" (matches QA-010), mode 9 ❌, charset G0/G1 flipped to ✅, limitations renumbered.
- **[DOC-004]** API_REFERENCE signatures — `diff_snapshots` removed, `debug_log_snapshot(label)` documented, color conversions as instance methods with real types (verified against color_api.rs), the two streaming settings as properties. Every corrected snippet executed.
- **[DOC-005]** ARCHITECTURE.md — v0.46.0 banner, pty_session/sim feature rows, 37 message variants (counted), post-split module layout, Kitty pre-filter + observer/streaming/FFI consumers in the diagram; all cited paths resolve.
- **[DOC-006]** README install refs — 0.46 pins, releases-page links, live web README link.
- **[DOC-007]** CONTRIBUTING/CLAUDE.md binding paths — `terminal/` and `types/` directories.
- **[DOC-008]** STREAMING.md — three TOC anchors fixed, `--max-clients` 0-rejects-all, `allowed_origins` row, `--preset`; 45/45 anchors resolve, 38/38 CLI args covered.
- **[DOC-009]** API_REFERENCE omissions — ProgressBar, SelectionMode, progress functions, width functions, client codec functions, TOC rows; also added the missing `SelectionMode` re-export in `python/__init__.py`.
- **[DOC-010]** VT_SEQUENCES — Character Sets section (designation/ACS/SO-SI, example executed), CurrentDir marked unsupported.
- **[DOC-011]** Docstrings — getter `__doc__` coverage 110 → 323/323; ~220 field/method docs across `types/*.rs`. **Deviation**: no derive-macro change was needed — pyo3 already forwards field docs through `pyo3_get_all` (proven empirically), so the planned derive version bump had no change to justify.
- **[DOC-012]** README What's New — kept 3 latest releases; README 1,614 → 662 lines; all 43 archived sections identifier-diffed against git history — zero content lost (8 missing bullets merged into CHANGELOG first).
- **[DOC-013]** CONFIG_REFERENCE — `max_osc_data_length` documented (128 MiB default); found the advertised Python binding did not exist and added it (`common.rs`, lands on both classes; stub regenerated).
- **[DOC-014]** README doc links — OBSERVERS/INSTANT_REPLAY/FFI_GUIDE rows.
- **[DOC-015]** Stale audit artifacts — `AUDIT-2026-06-15.md` and `AUDIT-REMEDIATION-2026-06-15.md` removed; current-cycle files and `docs/fable/` untouched.
- **[DOC-016]** Stale comments — ffi.rs Debug-format fix + `event_json`→`event_text` (FFI_GUIDE synced); idle-timeout 300→900 (verified against Default impl).
- **[DOC-017]** proto vs serde-JSON naming — admonition with a three-row example, verified against protocol.rs serde attributes.
- **[DOC-018]** Research note — absolute path removed, `sequences/osc/` layout, plus a broken sixle cross-anchor fixed.

---

## Deferred

- **[ARC-014] Public module surface curation** — deferred to a semver-major release-planning session per the audit's own remedy; requires surveying `../par-term` imports. Card remains on the backlog.

---

## Out-of-Audit Fixes (pre-existing bugs found during remediation)

1. **Kitty zlib empty-input test failure** (`src/graphics/kitty.rs`) — dependency bump 63502d5 changed flate2 so zero-length input returned Err, breaking the documented empty-in/empty-out contract and every gate. Fixed by restoring the contract explicitly; card closed.
2. **`--all-features` vs sim guard** — the Makefile lint/typecheck/clippy targets and the pre-commit clippy hook combined `sim`+`python` and tripped ARC-011's new `compile_error!`. All four sites aligned on CI's explicit `python,streaming` set.
3. **test_streaming.py API drift** — `stop()`/`is_running()`/`address()` never existed on `StreamingServer`; repaired during Phase 2's stub work.
4. **Achromatic HSL scale bug** — fixed under QA-009 (see above).

## Known Remaining (carded, not fixed — out of scope)

- **test_streaming.py failures under streaming build** — 4 async-fixture-misuse tests + 1 timing assertion; skipped under the default build so checkall is green; carded as backlog.
- **`UnderlineStyle.None` naming quirk** — cannot be represented in Python stubs; renaming is a breaking API change (noted on the same card).
- **`bun run lint` eslint plugin crash** — pre-existing toolchain issue, unrelated to remediation.

---

## Verification Results

- Build: ✅ (maturin dev, default + streaming + streaming-bin + sim configurations all checked)
- Tests: ✅ Rust 2,021 lib + 99 streaming integration; Python 512 passed / 146 skipped / 0 failed; Web 35/35
- Lint: ✅ clippy `-D warnings` clean; ruff clean
- Format: ✅ cargo fmt + ruff format clean
- Type Check: ✅ pyright 0 errors (repo-wide, including the new stub)
- Full gate: ✅ **`make checkall` exit 0** — all ten stages (Rust tests, Rust streaming tests, fmt, clippy, Python format, ruff, pyright, stub-check, pytest, vitest)
- Per-issue validation: every issue's fix confirmed present in the merged tree (structural grep/read verification across three batches), in addition to agent-reported test evidence

---

## Incidents During the Run

1. **Phase 3c rate-limit interruption** — the code-quality agent hit a 5-hour API usage limit mid-QA-003; resumed via message with an updated brief; no work lost (completed issues were already committed).
2. **kanban `item batch done` scoping bug** — `--project` + `--ids` closed 62 cards when 50 ids were passed, sweeping the ENH backlog; 9 cards reopened by id; filed upstream on the kanban project (card 01a048f1bcea7e42b33eff283e3a6b83).
3. **`make checkall` first run failed at test-web (exit 127)** — environmental: this worktree's node_modules predated QA-005's vitest dependency; `bun install` re-sync fixed it. Full re-run green.

---

## Files Changed

115 files, +11,485 / −15,151 (base 42ffde7 → `fix/audit-remediation` HEAD). Highlights: `src/streaming/` decomposed and macro-converted; `src/bin/streaming_server/` split; `src/python_bindings/` deduplicated and documented; `python/par_term_emu_core_rust/` gained `py.typed` + `_native.pyi`; `web-terminal-frontend/` gained `lib/terminal-connection.ts`, `lib/keyboard-layouts.ts`, and a vitest harness; `web_term/` regenerated deterministically; 16 documentation files corrected; `package-lock.json` and both 2026-06-15 audit artifacts deleted.

---

## Next Steps

1. Review the ENH-001..007 enhancement backlog (plans under `docs/fable/`) — several reference code states this remediation changed (e.g. ENH-005 supersedes QA-010's no-op).
2. Fix the carded `test_streaming.py` async-fixture failures.
3. Plan ARC-014 (public surface curation) into the next semver-major release.
4. Re-run `/audit` to confirm the findings hold as fixed.
