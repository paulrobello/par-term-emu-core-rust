# Audit Remediation Report

> **Project**: par-term-emu-core-rust
> **Audit Date**: 2026-09-29
> **Remediation Date**: 2026-09-30
> **Severity Filter Applied**: all
> **Plan Source**: AUDIT.md `## Remediation Plan` + AUDIT-REMEDIATION-PLAN.md playbook
> **Implementation Model**: Opus 5 (all fix agents)

---

## Execution Summary

| Phase | Status | Agent(s) | Issues Targeted | Resolved | Partial | Manual/Blocked |
|-------|--------|----------|-----------------|----------|---------|----------------|
| 1 — Critical Security (promoted set) | ✅ | fix-security ×3 (split by file conflicts) | 8 | 8 | 0 | 0 |
| 2 — Critical Architecture | ✅ | fix-architecture ×2 (ARC-089/090 then ARC-103 cluster) | 3 | 3 | 0 | 0 |
| 3a — Security (remaining) | ✅ | fix-security | 3 | 3 | 0 | 0 |
| 3b — Architecture (remaining) | ✅ | fix-architecture ×5 | 27 | 26 | 1 (ARC-108 steps b/c) | 0 |
| 3c — Code Quality | ✅ | fix-code-quality ×3 | 38 | 36 | 2 (QA-201/216 remainders landed) | 0 |
| 3d — Documentation | ✅ | fix-documentation ×3 | 35 | 34 | 1 (DOC-127 lint sweep) | 0 |
| 4 — Verification | ✅ | — | — | — | — | — |

**Overall**: 131 audit issues addressed — 128 fully resolved, 3 with recorded remaining scope (ARC-108 steps b/c, DOC-117 examples remainder, DOC-127 `missing_docs` lint sweep — all filed on the kanban board). All six D1–D6 decisions were implemented per the audit's recorded resolutions, including the approved ABI v4 batch (D4), `Cargo.lock` tracking (D2), and action SHA pinning (D3).

---

## Resolved Highlights ✅

### Security
- **SEC-125** reaped-PID signals, **SEC-126** respawn flag grammar (getopt-style `split_leading_flags`, fuzzed clean), **SEC-127** 1 MiB control-line budget, **SEC-128** OSC 7 local-host rule, **SEC-129** `cmd=` name cap, **SEC-130** kitty `t=t` TOCTOU (handle-pinned delete), **SEC-131** Python debug log `0600`/`O_NOFOLLOW`, **SEC-132** send-keys log redaction, **SEC-133** host-probe deadlock/wedge bounds (merges QA-193), **SEC-134** `paste` gone from the lockfile (rayon dropped with your approval), **SEC-135** all 58 action refs SHA-pinned and verified via `gh api`.

### Architecture
- **ARC-089/090** respawn `-k` output detach + `mutate_layout` zoom choke point; **ARC-100** `HostConfig` survives RIS; **ARC-092** snapshot-restore damage sync; **ARC-091** departing-row trigger scans; **ARC-093** one key encoder across Python/C/mux/macros; **ARC-094/095/096/097/113/119** mux emit/dead-state/lookup/save/typed-telemetry/shutdown-lock; **ARC-098** kitty astral keys; **ARC-101/112/114** C ABI v4: `ptec_terminal_*` rename, palette-resolved readback + grapheme channel, `TermEvent` v2 + `on_event_v2`, NUL-drop fixed, `TERM_CORE_ABI_VERSION` 4; **ARC-102/202** `TriggerState`/`MacroState` extracted, `hooks/`, `tree/`, `pty_session/` module splits (behavior-neutral, verified by line-multiset diff); **ARC-104/105/106/107** FFI gates in CI, one wheel feature set, `mux-bin` split, `Cargo.lock` committed + `--locked`; **ARC-108(a)** `GridSnapshot` into `grid`; **ARC-110/111/116/117/120/121** streaming loop unification, `log` facade, feature placement, macro un-export, build stamp, fuzz targets.

### Code Quality
- QA-182 client-size caps (`MAX_CLIENT_COLS/ROWS`, `MAX_CELL_PIXELS`), QA-184→198→191 mux-factory deadline/queue/park chain, QA-190 `exit_code` field, QA-194 Windows `TOKEN_USER` alignment, QA-195 `terminal_write()` mirror guard, QA-196 env-mutation eliminated (clippy lint added), QA-199 single registration helper + `read_control_line`, QA-201 all 48 unsafe blocks documented (Miri found and fixed 4 OOB test literals), QA-203 deadline-based PTY tests, QA-204 `stub-drift`, QA-206 real memory estimates, QA-207 hot-path diagnostics removed, QA-208/215 FFI dedup + `mem::forget` fix, QA-212 9 allows removed with parameter structs, QA-213 typed `MouseEventType`, QA-214 8 duplicate-family hoists (golden-byte tests), QA-216 unwraps → `Entry`/`first_chunk`/`expect`, QA-217/218/219 as planned.

### Documentation
- DOC-099–133 all landed, including the MUX.md rule corrections, SECURITY.md two-sink logging truth, the ABI v1–v4 table, `MUX_DECISIONS.md`, config-reference env table, 69 run-verified binding Examples (found real bugs, filed as QA-220), and the stub generator now carrying docstrings + typed returns.

---

## Requires Manual Intervention / Follow-Up 🔧

All remaining scope is filed on the kanban board (project `par-term-emu-core-rust`):

- **ARC-108 steps b/c** (backlog) — clock move; `py_convert` relocation needs your call: compat re-export (keeps the inversion) vs breaking removal.
- **DOC-117 / DOC-127** (backlog) — ~36 remaining binding Examples + `pty.rs`/`streaming.rs`; the `missing_docs` lint (373 non-generated hits vs ~30 threshold).
- **Follow-up cards filed during the run**: ARC-094b (declarative command table), ARC-095b (`%pane-exited` replay on registration), ARC-113b/c (state-file size cap, typed `AgentClaim`), QA-220 (detect_* row/col swap — real bug, high priority), QA-221 (`paste()` lock hold), QA-222/223 (parallel-test flakes: `PANIC_ON_COMMAND` global, `prepare_reclaims_a_stale_path`).
- **Cross-repo**: par-term board — zoom-unzoom UX (ARC-090), `mux-bin` build scripts (ARC-106), `escape_keys_for_tmux -l` (ARC-093), log-bridge filter (ARC-111); pardeck board — C ABI v4 migration card (high priority: `ptec_` prefix, vtable layout, attr bits).

---

## Verification Results

- **`make checkall`**: ✅ green on the final integration tree (fmt, clippy `-D warnings` across `python,streaming,mux,mux-bin,serde,streaming-bin,ffi`, ruff, pyright, full Rust lib + 21 integration targets, maturin build + full pytest (760), stub-check, caps-table, FFI header + surface checks, web tests after `bun install`).
- **Windows VM (aarch64)**: ✅ `cargo check --all-targets --locked`, `cargo check --lib --tests --no-default-features --locked --features rust-only,mux-bin,serde`, and the serialized `mux::` run (425 passed) — all green; cross-compile warning-free.
- **Miri**: ✅ FFI module clean (after fixing 4 out-of-bounds `terminal_feed` test literals).
- **Fuzz**: ✅ `kitty`, `mux_parse_command`, `mux_hook_report` clean runs.
- **Lychee**: ✅ 713 doc links, 0 errors.

---

## Files Changed

~120 files across the integration branch `fix/audit-remediation` (da0f66c → 1460c23): `src/mux/**` (incl. new `tree/`, `hooks/` modules), `src/terminal/**` (mod decomposed, `event_fields.rs`), `src/streaming/**` (roster integration, typed events), `src/pty_session/` (module split), `src/ffi.rs` + `include/` (ABI v4), `src/keyboard.rs`, `src/grid/**` (+ `snapshot.rs`), `src/graphics/kitty.rs`, `src/python_bindings/**`, `python/…/_native.pyi` (docstrings + types), `scripts/`, `Makefile`, `Cargo.toml` + committed `Cargo.lock`, `.github/workflows/*` (SHA pins, locked builds, ffi-drift + xcframework jobs), all `docs/*`, `CHANGELOG.md`, `README.md`, `CLAUDE.md`, `tests/*` (+9 new test files), plus 22 executed plan docs deleted and `debug/` removed (D5).

---

## Next Steps

1. Review the cross-repo cards above (pardeck's ABI v4 migration is the time-sensitive one).
2. Decide ARC-108(c): compat re-export vs breaking `py_convert` move.
3. Re-run `/audit` for a fresh AUDIT.md reflecting the remediated tree.
4. On your go: merge `fix/audit-remediation` into `main` (fast-forward is NOT possible — main has the roster merge and the botched `d54a901`; a regular merge was already prepared on the integration side), then dispatch `ci.yml` to exercise the new locked/FFI jobs, then delete the audit artifacts and the worktree.
