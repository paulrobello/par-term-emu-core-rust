# Audit Remediation Report

> **Project**: par-term-emu-core-rust
> **Audit Date**: 2026-11-06 (opus evening re-audit, tag `audit-2026-11-06-opus`)
> **Remediation Date**: 2026-11-06 → 2026-11-07
> **Severity Filter Applied**: all
> **Plan Source**: AUDIT.md `## Remediation Plan` + AUDIT-REMEDIATION-PLAN.md playbook
> **Implementation Model**: Opus 5 (all four fix agents + orchestrator regression fixes)

---

## Execution Summary

| Phase | Status | Agent | Issues Targeted | Resolved | Partial | Manual |
|-------|--------|-------|----------------|----------|---------|--------|
| 1 — Critical Security | ⏭️ Skipped (no criticals) | — | 0 | 0 | 0 | 0 |
| 2 — Critical Architecture | ⏭️ Skipped (no criticals) | — | 0 | 0 | 0 | 0 |
| 3a — Security (all) | ✅ | fix-security | 4 | 4 | 0 | 0 |
| 3b — Architecture (all) | ✅ | fix-architecture | 4 | 4 | 0 | 0 |
| 3c — All Code Quality | ✅ | fix-code-quality | 10 | 8 | 0 | 2 (QA-009 dispositions executed by orchestrator; QA-010 deferred by plan) + 1 stale |
| 3d — All Documentation | ✅ | fix-documentation | 22 | 22 | 0 | 0 |
| 4 — Verification | ✅ (after 2 fix-forward iterations) | — | — | — | — | — |

**Overall**: 38 issues resolved (incl. QA-011 fixed pre-run and QA-002 closed as stale), 0 partial, 1 requires a human decision (select-layout-presets worktree), 1 deferred by the playbook's own terms (QA-010).

Pre-run: **QA-011** (tab-strip bounds lost on width change — the user-reported min-tab-width reset) was root-caused during the audit and fixed directly on `main` (`f9a3d1e9`) before remediation launched. **QA-003** extends the pre-existing `[ARC-002]` de-complexify card's scope (notes updated) rather than duplicating work.

---

## Resolved Issues ✅

### Security
- **[SEC-001]** Orphaned iTerm2 multipart transfers retain unbounded memory — `file_transfer.rs`/`graphics.rs` — fail-on-replace + fail-oldest active cap (`DEFAULT_MAX_ACTIVE_TRANSFERS = 8`, caps table 77→78) + 2 regression tests
- **[SEC-002]** Query-param API key not percent-decoded — `http.rs` — `%XX` decode before `ct_eq` on both extraction sites, 3 tests
- **[SEC-003]** Host-header allowlist on loopback static routes — `http.rs`/`listen.rs` — `StaticHostGuard` middleware (403 non-loopback Host on loopback binds; absent Host allowed, matching the module's origin-check convention)
- **[SEC-004]** Failed-auth throttling — `http.rs` — `AuthFailureThrottle` (50ms doubling to 5s, 30s decay) on the async paths; the sync WS handshake callback records but does not sleep (a blocking sleep there would let attackers park every runtime worker — documented at the call site)

### Architecture
- **[ARC-001]** python/python-test feature parity gate — `scripts/check_python_features.py` + early checkall leg, with self-test and scratch-drift proof
- **[ARC-002]** default-session dual ownership — `streaming/server/` — single-owner registry lookup (`default_session` field removed); regression test; resolve-after-close returns a loud `SessionNotFound` (agent rejected silent recreation with PTY-state rationale)
- **[ARC-003]** dead `broadcast_to_session` — deleted; audit's "zero callers" was stale (two internal callers found) — routed through `send_to_session`, restoring metric accounting
- **[ARC-004]** caps-table-check moved into the early deterministic block of checkall

### Code Quality
- **[QA-001 + QA-007]** Upload verbs off the tokio runtime via `spawn_blocking` + scaffolding folded into shared admission helpers; reply strings unchanged
- **[QA-004]** Drain hot-path allocations hoisted (incl. a third `format!` in `adjacent_damage` the audit missed); live-retarget semantics deliberately preserved
- **[QA-005]** Golden variant coverage generated from compile-tied `variant_name()` registries (no-wildcard, dependency-free); golden bytes unchanged
- **[QA-006]** Grid export writes via `std::fmt::Write` (no per-cell `format!`)
- **[QA-008]** `impl_repr!` macro in the common.rs layer; 5/6 sites converted byte-identical — `PtyTerminal`'s repr intentionally unconverted (`PyResult` live-state shape the macro does not cover; documented)
- **[QA-009]** Stale worktrees/branches removed (ws-latency tree+branch, agent-a728 branch, all four remediation agent trees); **select-layout-presets intentionally kept** — 3 unmerged commits, Paul's call
- **[QA-011]** (pre-run, on main) tab-strip `bounds` carried across the resize rebuild + regression test

### Documentation
All 22 (DOC-001…DOC-022) landed as specified; highlights: both runtime-false API claims fixed with runtime verification (DOC-001 `id: str`, DOC-002 badge builtins shadowing), ARCHITECTURE.md inventory completed and stamped v0.79.0 (DOC-005/010/011/012), docstring compliance lifted on 18 streaming methods + Macro YAML quartet with stub regen (DOC-004), web README port and versions corrected (DOC-008/020), doc-path waivers added (DOC-021).

---

## Closed as Stale / Deferred
- **[QA-002]** agent-state TS dedup — **stale finding**: both extensions are generated from one template by `scripts/gen_extensions.py` (commit `3b8bd541`) with `extensions-check` gating drift and a recorded single-file-install contract. No fix warranted; disposition card carries the evidence. ENH-063's plan note updated to target generator-output testing instead of an extracted core.
- **[QA-010]** install.sh / check_mux_docs.py complexity — deferred per the playbook's own "opportunistic only" steps; card remains backlog.
- **[ARC-005]** broadcast fan-out lock — watch card (recheck 180d), no action by design.

---

## Requires Manual Intervention 🔧

### select-layout-presets worktree disposition
- **Why**: `.claude/worktrees/select-layout-presets` (branch `worktree-select-layout-presets` at `d7b71cd1`) holds ≥3 unmerged commits (`d7b71cd1`, `4b718c5c`, `7a58a1d1`). No owning session is visible.
- **Recommended**: Paul decides keep-or-abandon. If abandoned: `git worktree remove --force .claude/worktrees/select-layout-presets && git branch -D worktree-select-layout-presets`.
- **Effort**: one decision + one command.

---

## Verification Results

- `make checkall` (fast gate, full 27-leg run): **PASS** on `fix/audit-remediation` tip — after two orchestrator fix-forwards:
  1. QA-005's registries were dead code under non-test clippy (`-D warnings`) → gated `#[cfg(all(test, feature = "streaming"))]`
  2. `stub-check` drift: `TerminalClipboard` section bullets resolved to Terminal scope and the module-level `char_width` bullet could never match the same-named class method → `check_api_reference.py` gained an H3 class-section override with scope restore + bold module-block scope; instance `char_width` bullet corrected to `(c)` (config is module-function-only)
- Agent-side targeted suites all green before merge (server 43/43 on the combined tree, streaming 51–56 per commit, core graphics/iterm/multipart 54, export 61, protocol 17, golden 2/2, python-test 190+396, mux_factory 41)
- Load-sensitive `#[ignore]` suites: not covered by bare checkall per repo convention — `make checkall-ci` is the pre-push gate (not run here; no push requested)

---

## Files Changed (per agent reports; integration branch `fix/audit-remediation`, base `48eb14c2`)

- Security: `crates/par-term-emu-core/src/terminal/{graphics.rs,file_transfer.rs,sequences/osc/tests.rs}`, `docs/SECURITY.md` (regen), `docs/STREAMING.md`, `src/streaming/server/{http.rs,listen.rs,mod.rs,tests.rs}`
- Architecture: `Makefile`, `scripts/check_python_features.py` (new), `src/streaming/server/{mod,sessions,send,messages,tests}.rs`
- Code Quality: `src/streaming/server/{messages.rs,ws_session.rs,tests.rs}`, `src/streaming/mux_factory/stream.rs`, `src/streaming/{protocol.rs,proto_golden_tests.rs}`, `crates/par-term-emu-core/src/grid/export.rs`, `src/python_bindings/{common.rs,enums.rs,streaming.rs}`
- Documentation: `docs/{API_REFERENCE,ARCHITECTURE,CONFIG_REFERENCE,MUX,VT_SEQUENCES,DOCUMENTATION_STYLE_GUIDE}.md`, `crates/par-term-emu-core/DESIGN.md`, `web-terminal-frontend/README.md`, `src/python_bindings/{streaming.rs,types/recording.rs,terminal/mod.rs}`, `python/par_term_emu_core_rust/_native.pyi` (regen), `src/streaming/session.rs`, `crates/par-mux/src/mux/server/broadcast.rs`, `crates/par-term-emu-core/src/terminal/sequences/csi/cursor.rs`
- Orchestrator: `src/streaming/protocol.rs` (cfg gates), `scripts/check_api_reference.py`, `docs/API_REFERENCE.md`, plus the staged audit artifacts and this report

---

## Next Steps

1. Decide the select-layout-presets worktree disposition (above)
2. Wrap-up (on approval): CHANGELOG `[Unreleased]` entries, delete consumed audit artifacts (`AUDIT.md`, `AUDIT-REMEDIATION.md`, `AUDIT-REMEDIATION-PLAN.md` — `docs/opus/` ENH plans stay), merge is already to main
3. Enhancement backlog remains: ENH-062/063/064 cards + plans (`/enhancement-all`)
4. `make checkall-ci` before the next release cut (per repo convention)
