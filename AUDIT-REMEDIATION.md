# Audit Remediation Report

> **Project**: par-term-emu-core-rust
> **Audit Date**: 2026-11-06
> **Remediation Date**: 2026-11-06
> **Severity Filter Applied**: all
> **Plan Source**: AUDIT.md `## Remediation Plan` (no AUDIT-REMEDIATION-PLAN.md playbook — plain `/audit` run)
> **Implementation Model**: Opus 5 (all fix agents)
> **Integration branch**: `fix/audit-remediation` (worktree `.claude/worktrees/fix-audit-remediation`), base `3b96adf6` = local `main`

---

## Execution Summary

| Phase | Status | Agent | Issues Targeted | Resolved | Partial | Deferred |
|-------|--------|-------|----------------|----------|---------|----------|
| 1 — Critical Security | ⏭️ Skipped (empty) | — | 0 | — | — | — |
| 2 — Critical Architecture | ⏭️ Skipped (empty) | — | 0 | — | — | — |
| 3a — Security (remaining) | ✅ | fix-security | 6 | 4 | 1 (SEC-003, delta landed) | 1 (SEC-005, stale finding — no change needed) |
| 3b — Architecture (remaining) | ✅ | fix-architecture | 8 | 6 | 0 | 2 (ARC-002 blocked external gate; ARC-005 watch item, not a defect) |
| 3c — All Code Quality | ✅ | fix-code-quality | 6 | 5 | 0 | 1 (QA-002, stale finding — already implemented at base) |
| 3d — All Documentation | ✅ | fix-documentation | 4 | 4 | 0 | 0 |
| 4 — Verification | ✅ | orchestrator | — | — | — | — |

**Overall**: 19 issues resolved by code change, 2 closed as stale findings (SEC-005, QA-002 — the audit's premises did not hold at the audited commit), 1 partial (SEC-003: the load-time warning already existed; the remedy's help-text clause landed), 1 deferred (ARC-002, blocked on external card `01a1a556b675`), 1 watch item (ARC-005). Zero issues require manual intervention.

**Follow-up work completed in-run** (agent-reported adjacent defects, fixed by the orchestrator before the final gate): extraction-size cap for the frontend archive (new `MAX_EXTRACTED_BYTES`, bounded reads, test), README auth example switched to the env form, stale `resize_with_cell_pixels` name dropped from `docs/MUX.md`, `docs/ARCHITECTURE.md` synced to the config/ split + shared otel layer + PR-triggered drift gate, SECURITY.md caps table regenerated.

**Follow-up filed for a later session**: `--api-key-file` streamer flag (enhancement card `01a1b43686fb70`, backlog — new CLI surface decision, out of remediation scope).

---

## Resolved Issues ✅

### Security
- **[SEC-001]** Cleartext credential transport on non-loopback no-TLS binds — `src/bin/streaming_server/main.rs`, `cli.rs` — startup now refuses (auth + TLS off + non-loopback) unless `--allow-cleartext-auth` / `PAR_TERM_ALLOW_CLEARTEXT_AUTH` is passed; banner on both refused and opt-in paths; covers API-key and HTTP Basic credentials.
- **[SEC-002]** Unauthenticated non-loopback shell bind warn-only — same file — `warn_if_unauthenticated` became fail-closed `enforce_bind_security`: refuses without `--allow-unauthenticated` / `PAR_TERM_ALLOW_UNAUTHENTICATED`; the SEC-002 banner kept for the explicit case; call site moved after TLS/auth resolution. Commit `c9276e22` (SEC-001+002+004 as one sequenced change).
- **[SEC-004]** Cleartext CLI secrets prominence — `cli.rs`, module docs, `docs/STREAMING.md` — `--http-password-file`/`--http-password-hash` presented before `--http-password`; `PAR_TERM_API_KEY` (with `hide_env_values`) recommended over `--api-key`; ps/history exposure noted.
- **[SEC-003]** Weak htpasswd hashes — **finding partly stale**: the load-time warning (`log_credential_warning`, `src/streaming/config.rs:305`) already existed at the audit base (landed in `65afaa5f`, before the audited HEAD), with emission verified and pinned by tests. The remedy's outstanding clause — documenting it in the streamer help — landed: `--http-password-hash` help recommends bcrypt and names `--allow-legacy-hash` (commit `b362499c`).
- **[SEC-006]** Archive unpack accepts non-regular entries — `frontend_download.rs` — `Archive::unpack` replaced by `extract_regular_files` (regular files+dirs only, `safe_entry_path` containment, link/device/FIFO entries abort loudly); 4 new tar-fixture tests. Follow-up in-run: 256 MiB total extraction cap with `take()`-bounded reads (CWE-409), test added (commit `aa40e980`).

### Architecture
- **[ARC-004]** `mux/config.rs` 3,433-line monolith — replaced by `config/` along the daemon/client/resolve/file/chords/management/reload/parse seams (largest file 496 lines; `mod.rs` is 99 lines of re-export glue); every public path stable, zero call-site edits; all three peer-authored acceptance criteria checked with evidence. Commit `1cce09fd`.
- **[ARC-001]** OTel metrics layer duplicated (~1,100 lines) — shared layer extracted once into `par-term-emu-core` (`otel.rs` + `otel/{provider,stub}.rs`) behind a new core `otel` feature; root and par-mux forward to it; per-binary metrics files are thin config keeping public paths; copy-paste drift fixed (mux guard named the wrong binary); `otel-check`/`mux-otel-check` green incl. single-otel-tree assertions. Commit `aa1c7592`.
- **[ARC-003]** No `[workspace.dependencies]` — hoisted (18 deps named by 2+ members), 48 member refs converted to `workspace = true`; version-train legs green incl. the manifest-rewrite self-test. **Caveat vs the audit's expected outcome**: the base64 0.22/0.23 double-compile is ecosystem-forced (axum 0.8.9 requires ^0.22; bcrypt/reqwest/hyper-util require ^0.23) — the hoist removes the drift class (single spec); the split itself clears when axum moves to base64 0.23, at which point it is a one-line bump. Commit `2f7b80ff`.
- **[ARC-008]** MSRV policy — CONTRIBUTING.md states it: `rust-version` is a release stamp tracking latest stable at each cut, not a support floor. Commit `2f7b80ff`.
- **[ARC-006]** Mixed root `tests/` layout — documented in `docs/BUILDING.md` ("The root tests/ directory mixes two runners"): runner filtering rules and where new tests go. Commit `6df2e45c`.
- **[ARC-007]** web_term drift check dispatch-only — `ci.yml` `web-term-drift` now runs on every `pull_request` (matching the ffi-drift advisory-leg pattern); dispatch carve-outs unchanged. Commit `501d08b8`.

### Code Quality
- **[QA-001]** Watcher backoff duplication — hoisted into `src/streaming/watcher_common.rs` (`pub(super)` constants + `next_backoff`); both watchers import it; misleading tree-watcher doc removed; flap test relocated beside the code. Commit `370a7b6b`.
- **[QA-003]** GIL held during screenshot render — `render_snapshot_detached` wraps render+encode in `py.detach` (the pty.rs idiom); `screenshot`/`screenshot_config` route through it, `screenshot_to_file` variants inherit via pass-through Python tokens; api-surface baselines unmoved. Commit `2c04cde0`.
- **[QA-004]** Uncalled `MuxPane::resize_with_cell_pixels` — par-term consumer grep: zero matches; parsight impact: empty; deleted (commit `518616e2`); `resize_with_cell_pixels_deferred` kept; stale MUX.md prose reworded in-run.
- **[QA-005]** Same predicate name, two semantics — hooks variant renamed `json_reply_is_error` (definition + sole caller); protocol.rs variant unchanged. Commit `17d6164b`.
- **[QA-006]** `MuxClient::send` Ok on error replies — landed as **`send_ok`** (naming deviation: `send_checked` already existed returning `Reply{ok:false}` and par-term calls it); `send_ok` converts `%error` closes into `Err` with command + error text; `send` stays the tmux-compatible default; test-first with the newline-join trap covered. Commit `6d81bafd`.

### Documentation
- **[DOC-001]** Single undocumented public fn — the function is `PtyInputHandle::write` (`pty_session/io.rs`; `PtySession::write` was already documented): 12-line rustdoc in sibling style. Commit `6972819f`.
- **[DOC-002]** Orphaned `MUX_FEATURE_MATRIX.md` — moved to `docs/archive/` with a snapshot status line; indexed in `docs/README.md` Archives; zero dangling references. Commit `1476813f`.
- **[DOC-003]** README What's New scope note — corrected to "the releases from 0.63.0 through 0.79.0"; section content untouched. Commit `b8e0820e`.
- **[DOC-004]** Missing CI badge — GitHub Actions CI badge added at the head of the README badge row. Commit `b8e0820e`.

---

## Closed as Stale Findings (no change needed)

- **[SEC-005]** api-key-in-query per-connection logging — the warning logs exactly once, inside `StreamingServer::start()`, whose only production caller runs once per process: strictly better than the requested once-per-session. The per-connection premise does not hold at the audited base.
- **[QA-002]** Agent-state TS extensions hand-duplicated — `scripts/gen_extensions.py` already renders both files from one template with `make extensions-check` gating drift; landed `3b8cd541` (2026-10-16), before the audit HEAD `b1e970c4`. The audit's "verified NOT template-generated" was wrong at its own base.

---

## Deferred / Watch

- **[ARC-002]** Attach input dispatch de-complexification — **deferred by the plan's own blocking rule**: gated on external card `01a1a556b675` (truncated escape sequences under burst, `blocked`), and coordinated with `01a189b5ca17` (stale-frame ghosts, `blocked`). Card stays `backlog`; work starts after those close.
- **[ARC-005]** Frozen Python surface — the audit itself classifies this as a strategic watch item, not a defect. Card moved to `watch` with verification that the documented per-area-pyclass migration pattern continues.

---

## Verification Results

- Build: ✅ Pass
- Rust tests: ✅ Pass (full `make checkall` suite: root lib (ffi/python-test/serde features), par-term-emu-core member, par-mux member incl. integration suites, streaming; 65 `test result: ok` legs)
- Python tests: ✅ Pass (816 passed, 1 skipped)
- Lint: ✅ Pass (fmt --check, clippy -D warnings across all feature sets, ruff, pyright)
- Type check: ✅ Pass (pyright, stub check)
- Web frontend: ✅ Pass (vitest 103/103)
- Doc gates: ✅ Pass (mux-docs, mux-schema, env-docs, doc-links/lychee, rustdoc, stub-docs, caps-table, pinned-tests, completions, ffi-header/surface, api-surface, terminal-fields, version train, otel/mux-otel, extensions)

`make checkall` exit 0 on the merged integration tree at `a3e9602c` (2026-11-06 ~21:00 America/Los_Angeles). Run history: run 1 failed `pinned-tests-check` (orchestrator's own doc prose read as a test citation — reworded); run 2 failed `test-web` (integration worktree `node_modules` symlink had the wrong relative depth — fixed to four ups, memory note updated); run 4 green. `make checkall-ci` (Windows cross-target + nextest include-ignored + streaming CI test) remains to be run at the next release cut or before any push, per the repo's two-tier gate convention — no change in this run touches a labeled (load-sensitive) test.

---

## Files Changed

59 files, +4,906/−4,417, 24 commits since base `3b96adf6`:

- **Deleted**: `crates/par-mux/src/mux/config.rs`
- **Created**: `crates/par-mux/src/mux/config/{mod,client,daemon,resolve,file,chords,management,reload,parse,tests}.rs`, `crates/par-mux/src/mux/config/tests/{chords,file_io,management,reload,resolve}.rs`; `crates/par-term-emu-core/src/otel.rs`, `crates/par-term-emu-core/src/otel/{provider,stub}.rs`; `src/streaming/watcher_common.rs`; `docs/archive/MUX_FEATURE_MATRIX.md` (renamed from repo root, 99% similarity)
- **Modified (Rust)**: `src/bin/streaming_server/{main.rs,cli.rs,frontend_download.rs}`, `src/python_bindings/common.rs`, `src/streaming/{mod.rs,roster_watcher.rs,tree_watcher.rs,metrics.rs,metrics/otel.rs,metrics/stub.rs}`, `crates/par-mux/src/mux/{client.rs,hooks/mod.rs,pane.rs,tree/layout_ops.rs,tree/tests.rs,metrics.rs,metrics/otel.rs,metrics/stub.rs}`, `crates/par-term-emu-core/src/{lib.rs,pty_session/io.rs,terminal/tests/kitty_apc.rs}`
- **Modified (build/CI)**: `Cargo.toml`, `Cargo.lock`, `crates/par-term-emu-core/Cargo.toml`, `crates/par-mux/Cargo.toml`, `Makefile`, `.github/workflows/ci.yml`
- **Modified (docs)**: `README.md`, `CONTRIBUTING.md`, `CLAUDE.md`, `docs/{ARCHITECTURE,BUILDING,CONFIG_REFERENCE,MUX,README,SECURITY,STREAMING}.md`, `scripts/check_mux_docs.py`

Commits: `6972819f`, `1476813f`, `b8e0820e` (docs agent) · `c9276e22`, `b362499c` (security agent) · `370a7b6b`, `2c04cde0`, `518616e2`, `17d6164b`, `6d81bafd` (quality agent) · `1cce09fd`, `aa1c7592`, `2f7b80ff`, `6df2e45c`, `501d08b8` (architecture agent) · merges `e36053bc`, `b686b0b8`, QA merge, architecture merge · orchestrator follow-ups `aa40e980`, MUX.md prose, `793e8637`, `ade65e7f`, `a3e9602c`.

---

## Next Steps

1. Wrap-up (pending Paul's decision): update CHANGELOG.md, delete the consumed audit artifacts (AUDIT.md, this report), fast-forward local `main` to `fix/audit-remediation`. Push remains a separate, explicitly-confirmed step.
2. Re-run `/audit` to get an updated AUDIT.md reflecting the new state.
3. When axum ships a base64-0.23 release, bump the hoisted `base64` one line to clear the double-compile.
4. `--api-key-file` enhancement card (`01a1b43686fb70`) rides the enhancement workflow.
5. Consider `#[ignore = "load-sensitive ..."]` labels for the two non-ignored tests that flaked under host load 39–93 during the run (`render_latency::stdin_wake…`, `integrations::config_hooks::mastracode_asset…`) if fleet sharing keeps loading this box.
6. Minor pre-existing doc nits noticed but not fixed (out of scope): stray double blank line in the README What's New block, `docs/README.md` Archives intro listing live `images/`, the note's "unreleased work on `main`" clause currently having no unreleased paragraph.
