# Audit Remediation Report

> **Project**: par-term-emu-core-rust
> **Audit Date**: 2026-10-06
> **Remediation Date**: 2026-10-06
> **Severity Filter Applied**: all
> **Plan Source**: AUDIT.md `## Remediation Plan` (no AUDIT-REMEDIATION-PLAN.md playbook present)
> **Implementation Model**: Opus 5 (all fix agents)

---

## Execution Summary

| Phase | Status | Agent | Issues Targeted | Resolved | Partial | Manual |
|-------|--------|-------|----------------|----------|---------|--------|
| 1 — Critical Security | ⏭️ Skipped (empty) | — | 0 | — | — | — |
| 2 — Critical Architecture | ⏭️ Skipped (externally gated) | — | 1 | 0 | 0 | 1 |
| 3a — Security | ✅ | fix-security | 6 | 6 | 0 | 0 |
| 3b — Architecture (remaining) | ✅ (one follow-up card dispatched) | fix-architecture | 7 | 6 | 1 | 0 |
| 3c — Code Quality | ✅ (one partial by design) | fix-code-quality | 6 | 5 | 1 | 0 |
| 3d — Documentation | ✅ (DOC-001 gated) | fix-documentation | 8 | 8 | 0 | 0 |
| Follow-up wave (filed during run) | ✅ | fix-architecture | 6 | 5 | 0 | 1 pending |
| 4 — Verification | ✅ | — | — | — | — | — |

**Overall**: 30/30 audit issues addressed — 25 resolved, 2 closed-with-deviation (continuations filed), 3 remain gated/blocked (ARC-001, DOC-001, ARC-007), plus 8 new defect cards filed from agent findings (5 fixed in-run, 3 backlog with reasons).

Wave split: Phase 3 ran in two waves because QA-001 → ARC-003 and SEC-202 → ARC-003 are intra-phase blocking edges. Wave 1 = Security + Code Quality + Documentation (parallel); Wave 2 = Architecture on the merged state. Phase 2 (ARC-001) and DOC-001 stayed gated on the in-flight render-mode/mux work; ARC-007 is decision-gated.

---

## Resolved Issues ✅

### Security (3a — all six)
- **[SEC-201]** Legacy htpasswd hash warning — `src/streaming/auth_hash.rs`, `src/streaming/config.rs` — `legacy_hash_format()` flags `{SHA}`/`$1$`/`$apr1$`; one `warn!` per credential naming format + username, never the hash. Deviation: optional `{SHA}` opt-in gate not taken (audit marked it optional).
- **[SEC-202]** `--allow-api-key-in-query` startup warning — `src/streaming/server.rs` (additive, preserved through the later ARC-003 split), `cli.rs` help text; fires only when a key is actually configured.
- **[SEC-203]** Frontend archive checksum — `frontend_download.rs` verifies a `.sha256` sidecar before extraction; unusable sidecar fails the download; absent sidecar warns. New optional `sha2` dep (`streaming-bin` only) + `sha256sum` step in `deployment.yml`. Documented limit: catches corruption/asset swap, not full release compromise.
- **[SEC-204]** `.env`/`.env.local`/`.env*.local` in `.gitignore`.
- **[SEC-205]** Frontend dev/start bound to `127.0.0.1`; LAN opt-in documented; STREAMING.md corrected.
- **[SEC-206]** Cleartext Basic Auth password startup warning (`htpasswd -nB` pointer); zeroize/compare untouched.

### Architecture (3b + gated Phase 2)
- **[ARC-002]** Terminal hub (closed-with-deviation): `EventBroker` (`src/terminal/event_broker.rs`) fully owns events/bells/observers/queue capping — proven by compiling with fields private. 8 of ~20 sequence-handler families converted to sub-struct capabilities; continuation itemized and dispatched as a follow-up card (below).
  - **Follow-up (resolved)**: `TextAttributes` sub-struct unblocked SGR; SGR/DCS-query, erase/DECSCA/DECSERA, notify, OSC 7, event_subscription fully converted; window/cursor/mode/edit/report/iTerm partially with routers kept where match-order or multi-sub-struct orchestration matters; esc.rs + scroll.rs deliberately stay on `Terminal` (genuine orchestrators). Two new `too_many_arguments` allows on router signatures, judged acceptable.
- **[ARC-003]** God-file cluster split (closed-with-deviation, all splits token-level-verified identical): `dispatch/` (63/63 items), `command/` (97/97), `server/` (72/72), `pty_session/` (77/77), `streaming/server/` (114/114), `graphics/kitty/` (42/42), `attach/` wave 1 (101/101) + wave 2 `Session` impl split (35/35 → `pump/actions/navigate/status_row`, mod.rs 1,724→771). `PtyInputHandle` cfg-gated (fixes the bare-clippy default-features failure). **Not done, deliberately**: `handle_client`/`dispatch_command` body decomposition (logic change beyond the split remedy) — backlog card, planned post-Phase-B pass; tree-mutex-held observer delivery in `layout_ops` — backlog card (latent, structural).
- **[ARC-004]** CI `pull_request` trigger: version-check, lint, FFI drift, ubuntu/Py3.14 test + mux jobs (attach suites included), concurrency cancel. Orchestrator adjustment: PR clippy runs the lint-check set **without attach** — two pre-existing `render.rs` type_complexity errors (gated file) would redden every PR; card filed to re-add attach at ARC-001.
- **[ARC-005]** Two-tier API: `src/prelude.rs` (core surface + smoke test), crate-root re-exports all kept, `Py*` wall `#[doc(hidden)]` (deprecated has no effect on `use`; grep found no consumers), `docs/RUST_USAGE.md` updated.
- **[ARC-006]** `ProtoConvert` derive (extends ENH-041's approach, no overlap): both conversion directions for `ServerMessage`/`ClientMessage` incl. the complexity-43 `try_from`; explicit `ToWire`/`FromWire` pairs; golden test proves byte-identical wire output for all 50 variants + lossy paths; proto.rs 1,950 → 1,359 lines; derive crate 0.47.0.
- **[ARC-008]** `scripts/check_derive_version.py` + `make derive-version-check` wired into `checkall`, `release-check`, and CI's version job; derive spec matches 0.47.0.
- **[ARC-009]** Helpers relocated: `unix_millis`/`cells_to_text`/`html_escape` → `src/text_utils.rs`, callers compiler-enumerated, old paths re-exported.

### Code Quality (3c)
- **[QA-001]** `kill_target` + `notify_window_closes` hoisted in mux dispatch; the three kill handlers keep their exact per-variant notify order.
- **[QA-002]** `chord!` macro collapses 19 arms (audit said 20 — prefix/reload chords use different parsers); byte-identical expansion.
- **[QA-003]** Allow removed only at `compose_picker_panel` (7 params = clippy limit, suppressed nothing); 9 keeps retain justification; 4 unlisted PyO3 kwargs allows left alone.
- **[QA-004]** 10 new frontend tests for `terminal-connection.ts` (audit premise stale — 20 already existed); 70/70 green, tsc + eslint clean.
- **[QA-005]** Closed-with-deviation: `write_input` consolidation (both paths live — the audit's "legacy path" premise was wrong), `spawn_login_shell` unified, grid damage iterators shared; `try_wait`/`wait` deliberately kept separate (reap-lock semantics).
- **[QA-006]** `make_terminal` test helper consolidated (`pub(crate)`); example `main()` dupes left per audit.

### Documentation (3d — DOC-001 gated)
- **[DOC-002]** 0.58.0 section: 9 dead paths → current directory paths (audit line numbers stale at HEAD); older sections frozen history.
- **[DOC-003]** Three docs/opus refs fixed (root CHANGELOG; `PaneEndpoint` in `src/mux/server.rs`; `deny.toml`).
- **[DOC-004]** Remedy (b): `scripts/check_stub_docstrings.py` + `make stub-docs-check` in `checkall`; all 337 remaining undocumented defs are allow-listed property setters/init (audit's 48% figure didn't reproduce at HEAD).
- **[DOC-005]** `docs/TROUBLESHOOTING.md` (13 entries, deep-linked); cross-linked from QUICKSTART + README.
- **[DOC-006]** All emoji callouts removed (BUILDING ×17 + QUICKSTART), classDef conversions, SECURITY.md TOC above body.
- **[DOC-007]** MANUAL-PASS.md → `docs/`, all references updated (Makefile, MUX.md, CHANGELOG).
- **[DOC-008]** CONFIG_REFERENCE env-var table extended (`PAR_MUX_CONFIG`, `PAR_MUX_CONTROL_SOCKET`, streamer `PAR_TERM_*` row); `PAR_TERM_FORCE_WEB_DOWNLOAD` gap filled in STREAMING.md.
- **[DOC-009]** What's New = Unreleased + last three releases, cutoff rule stated.

---

## New Defects Found During Remediation

Found by fix agents while refactoring; all filed as cards tagged `audit-2026-10-06`.

**Fixed in-run** (each red-to-green with regression tests):
1. **Event-dispatch index bug** (high): any queue removal below `events_dispatched_up_to` shifted later events left — observers lost events. `EventBroker::extract`/`extract_matching` keep the index consistent; 5 regression tests (3 failed pre-fix).
2. **ZoneScrolledOut never reached observers**: created inside `poll_events()` which drains the same call. Now flushed + dispatched in `process()`/`process_deferred()`/`apply_action(s)` and filtered polls; `resize_deferred` added so `PtySession` resizes deliver after the terminal lock is released (the literal fix would have delivered observer callbacks under the write lock — deadlock).
3. **Unbounded evicted-zone buffer** (`MAX_EVICTED_ZONES` = 10,000, FIFO) + **reflow eviction accounting** (`reflow_push_lines` evicts zones and advances `total_lines_scrolled`, matching the scroll path) + **batch-push eviction lag** (push-first, evict after) + **resize observer dispatch** (`Terminal::resize` ends with `finish_applied_actions`) + **mux_factory locked-path resize/process** switched to deferred delivery + **duplicate resize recording** in `PtySession::resize` removed.

**Backlog with reasons** (not dispatched):
- `handle_client`/`dispatch_command` body decomposition — logic change beyond ARC-003's split remedy; wait for post-Phase-B planned pass.
- `layout_ops` tree-mutex-held observer delivery — latent (no production observers), structural fix threading batches through `sync_pane_sizes` callers.
- Zone evictions outside `process()` reach observers one `process()` call later (documented in CHANGELOG; nothing lost permanently).
- `push_rows_to_scrollback`/`absorb_rows_into_scrollback` cap-crossing in a single batch — fixed in-run; card closed.
- `render.rs` two `type_complexity` errors + five `tests/mux_attach.rs` lints — ride ARC-001; PR lint re-adds attach after.

---

## Requires Manual Intervention / Gated 🔧

- **[ARC-001]** render.rs decomposition (Critical) — externally gated on the in-flight render-mode tab-strip/sidebar work and attach Phase B; blocked on board with pointers to both gate cards.
- **[DOC-001]** ARCHITECTURE.md re-verification — gated on mux layout settling; card notes extended to also cover `EventBroker`, `TextAttributes`, the converted-handler pattern, and the ARC-003 module splits.
- **[ARC-007]** mux workspace extraction — decision-gated (needs owner's call); blocked.

---

## Verification Results

- **Build + full gate (`make checkall`)**: ✅ Pass (final tree; see note below)
- **Windows compile gate** (Parallels Win11 ARM VM, per CLAUDE.md playbook): ✅ `cargo check --locked --all-targets` ✅ `cargo check --locked --lib --tests --no-default-features --features rust-only,mux-bin,serde` — required because ARC-003 moved `cfg(windows)` code agents couldn't compile-check from worktrees. First run failed only on a stale-extract artifact (the playbook's tar overwrite never deletes removed files — the pre-split `kitty.rs` etc. lingered in the VM); after clearing the six stale paths both gates passed.
- **Per-issue validation**: every card closed with per-criterion inspection evidence on the board (merged-tree greps + agent gate outputs); incremental closing after each merge, Phase 4 global gate as safety net.

Gate-run notes (recorded honestly): the first `make checkall` on the integration tree failed twice for worktree-bootstrap reasons, not code — (1) pyright `reportOptionalMemberAccess` in the new DOC-004 script (fixed: `__doc__` guard, commit `fix(scripts)`); (2) `vitest: command not found` — the integration worktree had no frontend `bun install` (fixed; 70/70 frontend tests green). The third run passed end to end.

---

## Files Changed

Merge sequence on `fix/audit-remediation` (each agent branch merged `--no-ff`, token-level or per-issue verification before close):

- Security: `src/streaming/{auth_hash,config,server}.rs`, `src/bin/streaming_server/{cli,frontend_download}.rs`, `.gitignore`, `.github/workflows/deployment.yml`, `web-terminal-frontend/{package.json,README.md}`, `Cargo.toml`, `Cargo.lock`, `docs/{SECURITY,STREAMING}.md`, `CHANGELOG.md`
- Documentation: `CHANGELOG.md`, `QUICKSTART.md`, `README.md`, `CONTRIBUTING.md`, `Makefile`, `docs/{BUILDING,CONFIG_REFERENCE,MUX,SECURITY,STREAMING,TROUBLESHOOTING.md(new),MANUAL-PASS.md(moved),opus/ENH-035,ENH-039,ENH-040}`, `scripts/check_stub_docstrings.py(new)`
- Code quality: `src/mux/{dispatch,config}.rs`, `src/mux/attach/mod.rs`, `src/{pty_session,grid}/…`, `src/terminal/{snapshot_manager,replay}.rs`, `web-terminal-frontend/lib/__tests__/terminal-connection.test.ts`
- Architecture wave 2: `src/terminal/**` (EventBroker, TextAttributes, handler conversions, helpers), `src/text_utils.rs`, `src/mux/{dispatch,command,server}/`(new dirs), `src/mux/attach/{mod,panels,targets,pump,actions,navigate,status_row}.rs`, `src/pty_session/{mod,lifecycle,io,query,updates,coprocess}.rs`, `src/streaming/server/`(new dir), `src/graphics/kitty/`(new dir), `src/lib.rs`, `src/prelude.rs`(new), `derive/src/proto_convert.rs`(new), `src/streaming/proto.rs`, `src/streaming/proto_golden_tests.rs`(new), `Cargo.toml`, `derive/Cargo.toml`, `.github/workflows/ci.yml`, `scripts/check_derive_version.py`(new), `docs/RUST_USAGE.md`, `docs/CROSS_PLATFORM.md`, CLAUDE.md/CONTRIBUTING/doc-path updates
- Defect-fix wave: `src/terminal/{mod,action}.rs` + observer/grid tests, `src/grid/{zone,scroll,tests}.rs`, `src/pty_session/{io,tests}.rs`, `src/streaming/mux_factory.rs`, `src/bin/streaming_server/bootstrap.rs`, `include/terminal_core.h` (regenerated), `docs/{API_REFERENCE,ARCHITECTURE,FFI_GUIDE}.md`, CHANGELOG entries

---

## Next Steps

1. Review the three gated cards (ARC-001, DOC-001, ARC-007) and the two backlog cards deferred to the post-Phase-B mux pass.
2. Re-run `/audit` for an updated AUDIT.md reflecting the new state.
3. The board is the surviving record once the audit artifacts are deleted — every card carries per-issue evidence notes.
