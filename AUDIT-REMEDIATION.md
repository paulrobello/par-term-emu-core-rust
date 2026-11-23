# Audit Remediation Report

> **Project**: par-term-emu-core-rust
> **Audit Date**: 2026-11-22 (America/Los_Angeles)
> **Remediation Date**: 2026-11-23 (America/Los_Angeles)
> **Severity Filter Applied**: all
> **Plan Source**: AUDIT.md `## Remediation Plan` (no AUDIT-REMEDIATION-PLAN.md playbook — plain /audit run)
> **Implementation Model**: Sonnet (per fix-audit standing policy)
> **Effort**: High
> **Integration branch**: `fix/audit-remediation` (worktree off local `main` 5fbc5bde), final tip `25178067`

---

## Execution Summary

| Phase | Status | Agent(s) | Issues Targeted | Resolved | Partial | Deferred |
|-------|--------|----------|-----------------|----------|---------|----------|
| 1 — Critical Security | ⏭️ Skipped (empty) | — | 0 | — | — | — |
| 2 — Critical Architecture | ✅ | fix-architecture | 1 (ARC-006) | 1 | 0 | 0 |
| 3a — Security (remaining) | ✅ | fix-security | 4 targeted | 4 | 0 | 1 (SEC-002, blocked) |
| 3b — Architecture (remaining) | ✅ | 2 fix-architecture + orchestrator | 8 targeted | 8 | 0 | 1 (ARC-002, dependsOn external card) |
| 3c — Code Quality | ✅ | 2 fix-code-quality + derive follow-up + perf fix-forward | 9 | 9 | 1→0 (QA-003's remainder completed in-run as card 01a20717) | 0 |
| 3d — Documentation | ✅ | fix-documentation | 6 + SEC-006 | 7 | 0 | 1 (DOC-005, blocked) |
| 4 — Verification | ✅ | orchestrator | — | checkall 15/15 green; checkall-ci green twice (incl. re-run after perf fix); write-path bench A/B both directions | — | — |

**Overall**: 30 of 31 issues resolved and verified. 1 landed partially with its remainder completed in-run (QA-003 → derive card done). 3 deferred by the audit's own sequencing rules (ARC-002, SEC-002, DOC-005 — all on the board with recorded reasons). Plus 1 follow-up card created and completed in-run (01a20717, derive variant_names) and 3 gate-script fixes made forward during verification.

---

## Resolved Issues ✅

### Security
- **[SEC-001]** Release-artifact authenticity — `self_update.rs`, `frontend_download.rs`, `install.sh`, `install.ps1`, new `scripts/sign-release.sh`, `docs/SECURITY.md` (commit c09a5a22): minisign detached-signature verification against the pinned key `7D5A457F1AB85B55` (the owner's existing keypair) after the SHA-256 gate and before staging, on every update path. Bad signature aborts; absent `.minisig` warns loudly until the first signed release activates the contract. Four signature test suites + full update-path test. **CI-side recommendations reported, deliberately not committed** (signing step in deployment.yml, `MINISIGN_SECRET_KEY` as approval-gated GitHub environment secret, `permissions:` tightening) — see Requires Manual Intervention.
- **[SEC-003]** HTTPS-only bounded redirect policy in `frontend_download.rs` (1d080dd5) — refuses non-HTTPS redirect targets, max 5 hops, matching the curl path.
- **[SEC-004]** Nonce-based `script-src` (517a8242) — per-response 256-bit nonce on complete HTML responses; `<script>` tags rewritten at serve time; shipped Next.js export verified (both inline-script shapes nonced, no inline handlers); `style-src` keeps `unsafe-inline` (xterm/React inline styles, documented).
- **[SEC-005]** Windows state-file `LockFileEx` claim (ad677bb7) — exclusive non-blocking lock with the Unix path's retry/holder-pid/drop-release semantics; one-byte lock past the pid line (mandatory byte-range locks would block the refused daemon from reading the holder pid). Compiled clean for x86_64 + aarch64 windows-msvc; refusal test now runs on every platform. Runtime verification rides CI.
- **[SEC-006]** System-stats disclosure note in CONFIG_REFERENCE (6080b7a0).
- **[SEC-002]** Deferred → `blocked` on the board: sequenced behind the pyclass-migration watch decision per the audit's blocking relationships.

### Architecture
- **[ARC-001]** Terminal-freeze blast radius recorded as standing policy in DESIGN.md + `check_terminal_fields.py` header (8fea46ab) — in-degree 174, ~1,120 upstream refs at depth 2; `terminal/mod.rs` untouched (conflict-avoided).
- **[ARC-003]** Render decomposition slice (12e3b904): ENH-050's named extractions were already landed in-tree; this run moved four self-contained blocks out of `render/mod.rs` (2,174 → 1,657 lines) as pure moves — `glyphs.rs`, `emulator.rs`, `trace.rs`, `entry.rs` — with re-exports at old paths; 598 attach lib tests byte-identical green. Remainder (session.rs/modal_settings.rs/modal.rs/mouse.rs) stays on the standing ENH-050/052 plans.
- **[ARC-004]** Dev-deps into `[workspace.dependencies]` (0b4789c6): portable-pty + image (core's feature set), `workspace = true` references.
- **[ARC-005]** Rust installer dedup (3f039ccd): `hook_entries.rs` (`HookEntry`, `merge_entries`/`unmerge_entries`); codex.rs + claude.rs now data-driven. **TS half of the audit finding was stale**: `scripts/gen_extensions.py` already renders both extension files from one template (extensions-check green; vitest wire tests green in checkall) — a shared imported module would break single-file installs, so the generator design is correct.
- **[ARC-006]** Deprecated `Broadcaster` removed (f40d778b, Phase 2): module + re-export + allows gone (322 lines); the audit's sessions.rs reference did not exist (only live per-session loop methods, untouched); CHANGELOG `### Removed` notes the embedder break.
- **[ARC-007]** tmux_control parser split (21562d0c): parser.rs 743 → 175 lines (buffer/state machine); new `tmux_control/line/` per-family files; 17 inline `parse_line` tests (was zero); public API stable; fuzz target compiles (verified in Phase 4).
- **[ARC-008]** Dangling parsight worktree registration pruned (orchestrator): watch hard-removed, orphaned dir deleted, registration `unreachable:true` for the reaper. Branch `eval-cell-32` (2 throwaway prototype commits) deleted 2026-11-23 on the owner's instruction.
- **[ARC-009]** Proto missing_docs closed (beccac61): root cause was uncommented proto items (prost already emits docs); 258 comments added to `terminal.proto`, pb.rs regenerated, `#[allow(missing_docs)]` removed entirely (0 warnings remain). Bonus: fixed the `proto-rust` Makefile glob that never matched.

### Code Quality
- **[QA-001]** Write-path classification helpers (f14d09e7) + bench-driven perf fix (25178067): pure helpers (`attach_role`, `cell_should_widen`, `wide_char_needs_wrap`, …) + 5 unit tests; `filter_apc_and_advance` split (`handle_kitty_apc_payload`). First landing measured **+8% plain-ASCII regression** (two-directional A/B pairs, criterion); fix-forward added `#[inline]` on the six helpers + `#[cold] #[inline(never)]` on the kitty payload handler (attributes only). Final bench on quiet host (load 5, guard satisfied) vs pre-audit base 5fbc5bde: **plain_ascii −6.8%, unicode_wide −18.3%, scroll −5.8%, sgr_heavy/cursor_addressing unchanged** — no regressions, net measured win. 2,098 core tests green throughout.
- **[QA-002]** 30 of 241 ignored integration tests un-ignored into the fast gate (43db6a16): mux_cli 23 + mux_daemon 7, chosen after running all 241 serially; selection = deterministic protocol/config/installer assertions (<1s, no timing/scheduling deps); ~16 s added to the fast gate; 211 genuinely timing-sensitive tests keep the tier.
- **[QA-003]** Dispatchers consolidated 3 of 4 (f9120ff5): csi erase + color-stack handlers, `drop_oversized_apc`, `agent_state_tail`. `variant_name` was completed in-run via the follow-up card: new `VariantNames` proc-macro (derive 0.49.0 → 0.50.0, 092e6d45/7f90cb4c) now generates the variant→name registries for ServerMessage (48) and ClientMessage (18); wire goldens byte-identical; 13 derive + 258 streaming tests green.
- **[QA-004]** Module-map doc for `python_bindings/common.rs` (2b03bef4) — 19 `impl_*` macros inventoried with the Python surface each drives.
- **[QA-005]** `unreachable!()` enum-default arms → `debug_error!` + skip (bb0287d8).
- **[QA-006]** Roster large-variant: measured, allow kept (bb0287d8 + notes) — RosterDelta/AgentEntry both 224 B; production churn bound ~112 KB/s worst case; boxing would add heap indirection to optimize an unmeasurable copy. Size facts documented in-code.
- **[QA-007]** `test_pty.py` fixed sleep → `wait_for` poller (f070750e).
- **[QA-008]** Grid scroll test pair parameterized over both directions (feedc5d3).
- **[QA-009]** Example dedup (845a247f) — with noted output delta: the two functions were not identical (numbered plain vs colored box); the example now uses the render_utils colored version.

### Documentation
- **[DOC-001]** Contradictory CSP bullet duplicate removed (ad0157f2).
- **[DOC-002]** MSRV 1.99 → 1.100 in README (×2 sites), QUICKSTART, BUILDING; manifest referenced as source of truth (96cd3d27).
- **[DOC-003]** `protocol.rs` → `protocol/` directory references fixed in CONTRIBUTING (×2 sites) + STREAMING (42e51dfd).
- **[DOC-004]** `input_route.rs` → `input_route/` (keyboard/mouse/pointer verified on disk); ARCHITECTURE banner refreshed to v0.87.0 after a 321-path verification sweep — that sweep found the input_route reference was the only broken path (416c362e).
- **[DOC-006]** `PAR_TERM_CORE_BUILD_SHA` documented (6080b7a0) — compile-time only (set by par-mux `build.rs`, baked into `mux::build_stamp()`); the env-docs gate script gained a compile-time-read registration for it (check_env_docs.py, 64877c1a).
- **[DOC-007]** `doc-path` waiver markers added to the 8 broken rows in docs/opus ENH-049/053/061/063 (f94844d8).
- **[DOC-005]** Deferred → `blocked` on the board: sequenced behind the pyclass-migration watch decision (356 docstrings done once, not twice).

---

## Requires Manual Intervention 🔧

### SEC-001 CI-side activation (Paul)
- **Why**: touches release infrastructure and secret material the orchestrator will not modify or provision autonomously.
- **Recommended**: (1) add a signing step to `deployment.yml` that runs `scripts/sign-release.sh` (or signs via the `minisign` CLI) and uploads `.minisig` sidecars with each release; (2) provision the minisign private key as a GitHub **environment** secret on an approval-gated `release` environment; (3) tighten workflow `permissions:` (top-level `contents: read`, `contents: write` only on the publish job); (4) sign the next release cut — until then every consumer sees the loud "unsigned" warning path (by design, no brick). Trust-model note: signatures defeat stolen PATs and compromised workflows/actions, not full GitHub account takeover.
- **Estimated effort**: small (one workflow edit + one secret) once the signing step is written.

### Deferred issues (not manual work — awaiting their sequenced blockers)
- **[ARC-002]** Decompose `spawn_drain`/`should_send` — `dependsOn` the passthrough-fencing card `01a20644275270e8ada5a7fcde1bda35` (both edit the same loop). Take the fencing card first.
- **[SEC-002]** Legacy htpasswd hashes on the library path — recommended plan recorded on the card and in-session (default-refuse + `allow_legacy_password_hashes` opt-in + two-release removal), but it stays sequenced behind the pyclass-migration watch decision per the audit.
- **[DOC-005]** Docstring pass on undocumented pyi defs — same watch decision (do the 356 docstrings once, after any surface split).

---

## Verification Results

- Build/`make checkall`: ✅ Pass (15/15 categories, cold run — includes pyright, which one agent had skipped via `SKIP=pyright`; the doc-only Python change it guarded passes)
- `make checkall-ci`: ✅ Pass **twice** — on the pre-perf-fix tip and re-run on the final tip `25178067` (all nextest include-ignored legs, Windows cross-target checks, streaming CI test)
- Fuzz targets: ✅ compile after the parser split (`cargo +nightly check` in fuzz/)
- Write-path bench (criterion, `terminal_throughput`, two directional pairs + final pair, host load 5): final base-vs-tip **plain_ascii −6.8%, unicode_wide −18.3%, scroll −5.8%, sgr_heavy ±0.2% ns, cursor_addressing −0.02% ns**
- Regressions found and fixed forward during verification: (1) DOC-006's new env-doc row tripped `env-docs-check`/`otel-check` → compile-time read registered in the checker; (2) QA-004's module-map linked crate-private traits → de-linked; (3) QA-001's first landing regressed plain-ASCII +8% → fixed forward to −6.8% (bench-driven, both directions confirmed); (4) SEC-003/SEC-005 changelog entries moved under `### Fixed`.

---

## Files Changed

77 files: +3,813 / −2,258 (full list: `git diff --name-status 5fbc5bde..25178067` on `fix/audit-remediation`). Highlights: 13 new files (tmux_control/line/ ×7, render extraction ×4, hook_entries.rs, variant_names.rs, sign-release.sh), 1 deleted (broadcaster.rs), 2 regenerated (terminal.pb.rs, pb checksum), docs ×10, installers ×2, tests ×4 suites touched.

---

## Next Steps

1. Review `Requires Manual Intervention` (SEC-001 CI-side) and the three deferred cards.
2. Take board card `01a20644` (passthrough fencing), then ARC-002 unlocks.
3. Decide the pyclass-migration watch → unblocks SEC-002 + DOC-005.
4. Sign the next release cut to activate SEC-001's verification contract.
5. Re-run `/audit` for a fresh cycle when convenient.
