# Audit Remediation Report

> **Project**: par-term-emu-core-rust
> **Audit Date**: 2026-10-15
> **Remediation Date**: 2026-10-15
> **Severity Filter Applied**: all (no argument given)
> **Plan Source**: AUDIT.md `## Remediation Plan` (no AUDIT-REMEDIATION-PLAN.md playbook present)
> **Implementation Model**: Opus 5 (all fix agents)
> **Integration Branch**: `fix/audit-remediation`, based on local `main` at d4cbbf0, final tip 7076cf5 (+2 follow-up commits: caps-table regen adec3bb, derive bump c0aec4f)

---

## Execution Summary

| Phase | Status | Agent | Issues Targeted | Resolved | Partial | Deferred |
|-------|--------|-------|----------------|----------|---------|----------|
| 1 — SEC-003 (promoted) | ✅ | fix-security | 1 | 1 | 0 | 0 |
| 2 — ARC-003 (promoted) | ✅ | fix-architecture | 1 | 1 | 0 | 0 |
| 3a — Security (remaining) | ✅ | fix-security | 3 | 3 (SEC-002 already fixed in base) | 0 | 0 |
| 3b — Architecture (remaining) | ✅ | fix-architecture | 4 | 4 | 0 | 1 (ARC-002, external blocker) |
| 3c — All Code Quality | ✅ | fix-code-quality | 3 | 3 (two QA-001 seams rejected with reasons) | 0 | 0 |
| 3d — All Documentation | ✅ | fix-documentation | 5 | 5 | 0 | 0 |
| 4 — Verification | ✅ | orchestrator | — | make checkall + checkall-ci green | — | — |

**Overall**: 19 of 20 audit issues resolved (SEC-002 required no change — already fixed by commit 65afaa5 from an earlier cycle); 1 deferred (ARC-002, blocked on in-progress attach-notifications work). Zero manual-intervention blockers. 22 commits, 85 files changed, +8,481/−5,821 lines.

**Wave order deviation (justified)**: the plan's two intra-Phase-3 edges (ARC-004→QA-001, ARC-006→QA-002) plus the File Conflict Map's cross-domain shared files (`common.rs`, `Makefile`, `pty.rs`/`streaming.rs`) forced sequencing beyond the audit's diagram: waves ran 3a∥P1, then P2∥3b, then 3d, then 3c. Phase 1 and 2 also ran ordered (both edit core `terminal/mod.rs`).

---

## Resolved Issues ✅

### Security
- **[SEC-003]** OSC 52 clipboard write gate — `0b87a24`. New `allow_clipboard_write` flag (default **true**, behavior-preserving) with getter/setter mirroring the read flag across core, Python bindings (both classes), pyi, API_REFERENCE, SECURITY.md, tests; the write path in `osc/clipboard.rs` returns early when off; the flag is carried across RIS (`ESC c` cannot re-enable it). **Standalone commit, flagged for Paul's manual review — revert = `git revert 0b87a24`.** Accepted policy choice: the streaming-server's authenticated clipboard set path is not gated (different threat surface); filed as a backlog follow-up card.
- **[SEC-001]** rustls 0.23.45 → 0.23.46 — `63f99e4`, lockfile-only; closes RUSTSEC-2026-0346/0347/0348/0349. `cargo audit` clean (only the pre-allowed unsafe-libyaml warning). Note: the next full `cargo update` will also move tempfile's getrandom 0.4.3 → 0.3.4 (deliberately excluded here to keep the commit minimal).
- **[SEC-002]** Legacy htpasswd startup warning — **no commit needed**: already fixed in base by 65afaa5 (prior audit cycle). Verified on tree: config.rs constructors warn on `{SHA}`/MD5-crypt/cleartext without leaking secrets, `--require-bcrypt` strict mode exists, SECURITY.md deprecation present, tests green. The audit finding was stale.
- **[SEC-004]** Frontend download integrity — `6b74065`: `.sha256` sidecar mandatory for releases ≥ v0.58.1 (`SIDECAR_REQUIRED_SINCE`), fail-closed for newer/unparseable tags; older releases keep the TLS-only warning. Cutoff chosen over digest pinning because embedded digests cannot cover `latest` or future releases. docs/STREAMING.md updated; 9 tests including the new cutoff test.

### Architecture
- **[ARC-003]** Hub-file extraction — `4f80cb3`: `terminal/mod.rs` 4,010 → **1,251 lines** (target ~1,500). New `state.rs` (963 lines: all state sub-structs + `Default` + helpers) and 13 themed impl files (`damage.rs`, `events_api.rs`, `host_config.rs`, `input.rs`, `keyboard_api.rs`, `modes_api.rs`, `notify_progress.rs`, `rect_api.rs`, `sixel_api.rs`, `title_badge.rs`, `tmux_api.rs`, `unicode_api.rs`, `zone_api.rs`); observer-dispatch tests moved to `tests/arc001_observer_dispatch.rs`; all `crate::terminal::*` paths preserved via re-exports; all 985 doc-comment lines carried (which is what let DOC-001 land after it). Fallout fixed in-run: caps table paths regenerated (`adec3bb`), stale layout prose fixed under DOC-001.
- **[ARC-004]** tmux_control split — `eb8aa64`: 2,094-line single file → `tmux_control/{mod.rs, notification.rs, parser.rs, tests.rs}`; public paths unchanged; 44 tests kept; `check_mux_docs.py` and docs path references updated.
- **[ARC-006]** Binding-layer duplication — `c3d3fd6`: `impl_terminal_resize_pixels!` + `PixelResize` trait (per-class behavior kept); `MessageCodec` trait folds the 4 codec functions + shared `streaming_disabled()` stubs; derive macro now supports named-field structs + `#[pydict(nested)]`; 5 stats structs derive it; the cyclomatic-51 hand-rolled `system_stats_to_py_dict` deleted after a byte-identical-output check (pinned by a new test). **Follow-up fixed in-run**: `par-term-emu-derive` bumped 0.47.0 → 0.48.0 (`c0aec4f`, with lockfile + CHANGELOG) so the next publish cannot resolve a registry derive without struct support. Second follow-up filed backlog: no gate runs the `python-test,streaming` combination that the new pinned test needs (design call).
- **[ARC-005]** doc(hidden) promotion gate — `b289c0d` + `2ad01f5`: `scripts/check_doc_hidden.py` extracts the member's `#[doc(hidden)]` pub set by module/owner/name and fails on drift from the 34-item baseline recorded in DESIGN.md markers; fails closed on unparseable forms; `make doc-hidden-check` wired into `checkall`; DESIGN.md states the promote-methods-never-fields policy and records 3 previously unlisted promotions.
- **[ARC-001]** Python API surface freeze — `744b0d1`: `scripts/check_api_surface.py` counts unique non-dunder methods per class in `_native.pyi` (baseline Terminal 389 / PtyTerminal 206), fails on growth *and* on unratcheted shrink; `make api-surface-check` wired into `checkall`; policy (new API areas ship as separate pyclasses) documented in ARCHITECTURE.md + CLAUDE.md. The full interface-segregation migration remains a long-term roadmap item, per the audit.

### Code Quality
- **[QA-001]** Parser complexity cluster — `c836a3a`: helpers extracted at natural seams: `handle_csi_style` (SGR param loop, underline 4:n, modify-other-keys), `push_sgr_style` (`push_sgr_color` via a `SgrLayer` enum + `push_sgr_flags`, one named-color table replacing two), `parse_chunk` (`apply_param`, `accumulate_data`), `render_block_element` (`fill_block_rect` replaces 6 copied loops), `encode_legacy` (`encode_legacy_char`, `push_tilde_key`). **Rejected with reasons**: `csi_dispatch_impl` and `parse_line` are arm-count complexity with no seam short of the table-driven rewrite the audit ruled out as churn; `route_mouse` excluded (in-flight attach work). Behavior verified byte-identical by the full member suite.
- **[QA-002]** too_many_arguments allows — `ab31dff`: recount found 13 sites (1 already reasoned, 1 deleted with ARC-006); all 12 bare sites now carry `reason =` in the pane_chrome.rs style; zero bare allows remain. No params structs: 8 sites are Python-visible signatures (off-limits), 4 are public Rust APIs. Flagged: `ansi_utils::generate_sgr` (9 bools + 2 colors) is the one genuine struct candidate if that public API ever changes.
- **[QA-003]** Near-duplicate pairs — `bcb4194`: `try_wait`/`wait` share `observe_exit(query)`; `named_progress_bars`/`get_named_progress_bar` share `named_progress_bar_dict` (stub zero-diff); the three styled-export functions share `push_standalone_row` (byte-identical output).

### Documentation
- **[DOC-001]** rustdoc links + gate — `7f92280`: 16 intra-doc link errors fixed (audit said 17; 16 measured at base) across cell.rs, progress.rs, terminal docs, debug.rs, graphics, common.rs, screenshot_config.rs; misplaced VT-modes doc line moved to `TerminalModes`; gate renamed `rustdoc-check` covering all three crates (`par-mux` with mux-bin, `par-term-emu-core`, root with python-test), all `-D warnings`, wired into `checkall`; stale layout prose fixed in CLAUDE.md/ARCHITECTURE.md/BUILDING.md/DESIGN.md. Known gap (accepted): docs.rs metadata feature set doesn't cover streaming/FFI docs — widening is a later change.
- **[DOC-002]** Docstring example runner — `2d0efc2`: pty.rs/streaming.rs included; the checker now **fails** on failing Examples, import failure, or zero examples (was always-exit-0); ratchet floor `MISSING_EXAMPLE_FLOOR = 277`; 80 new runnable Examples (PtyTerminal/StreamingConfig/StreamingServer) + 10 broken existing examples rewritten; 282 examples now execute. **Policy accepted**: setter methods no longer counted as missing (PyO3 only surfaces the getter's `__doc__`; mirrors `check_stub_docstrings.py`). Deliberately left without Examples: `spawn_shell` (runs login shell), `StreamingServer.start` (binds socket), `tick_macro`, `recording_to_macro`.
- **[DOC-003]** — `ec627b5`: ENH-059 cites the stable chunk directory with a content-hash note.
- **[DOC-004]** — `a34951d`: `doc-path` waivers added to the three false-positive spans (SKILL.md source copy, REGIONAL_FLAG_LIMITATION.md, ENH-055).
- **[DOC-005]** — `d43ceb9`: opus doc headers pin line citations to their generating docs-only commits; ENH-061's 9 dead tmux_control line citations symbolized at post-split paths.

---

## Deferred / Follow-ups (board-tracked, none blocking)

- **[ARC-002]** Forked agent-state TS extensions — card `blocked`: external blocker "par-mux attach: notifications on agent state change" (in progress) edits the same files. Remediate in a later run after it lands.
- **[backlog]** Streaming-server clipboard set path vs `allow_clipboard_write` — needs a design decision (authenticated client path vs terminal-output gate).
- **[backlog]** No gate runs the `python-test,streaming` feature combination (the new pinned stats test only runs there) — gate-runtime/CI design call.
- **[backlog]** MUX.md:5 cites the removed `src/tmux_control.rs` path — rides the owed Phase B attach-TUI MUX.md update.
- **[noted, no card]** `.venv/.venv` self-referential symlink inside the main checkout's venv (dated 2026-10-12, pre-existing) causes a cargo filesystem-loop warning; harmless but worth deleting manually.

---

## Verification Results

- **make checkall** (fast gate): ✅ Pass — exit 0. Covers fmt, clippy (4 feature configs), ruff, pyright, the full Rust + Python suites, and all doc/drift gates including the three new ones (rustdoc-check, doc-hidden-check, api-surface-check).
- **make checkall-ci** (CI-parity gate): ✅ Pass — exit 0, "All CI-parity checks passed!" (adds nextest + the serial mux suites).
- Per-issue verification: every card was closed only after a grep/read of the landed change on the merged tree plus the agent's scoped test evidence; gate regressions found during Phase 4 (worktree frontend deps; caps-table paths stale after ARC-003) were fixed forward (`adec3bb`) and re-run to green.
- cargo-fuzz targets were not run (not runnable in agent worktrees); the fuzz-covered parsers changed only by helper extraction with the full member suite green — recommend a `make fuzz-all` pass before the next release cut.
- False-alarm note: the first two gate runs failed at `test-web` (missing frontend node_modules in the worktree — bootstrapped) and `caps-table-check` (stale paths after ARC-003 — regenerated); neither was a code regression.

---

## Files Changed

22 commits on `fix/audit-remediation` (d4cbbf0..7076cf5 + adec3bb, c0aec4f): 85 files, +8,481/−5,821.

Created: 13 terminal themed/impl files + `state.rs` + observer test module, `tmux_control/{mod,notification,parser,tests}.rs`, `scripts/check_api_surface.py`, `scripts/check_doc_hidden.py`.
Deleted: `crates/par-term-emu-core/src/tmux_control.rs` (superseded by the split).
Modified: core parser/grid/keyboard/pty_session/graphics sources, binding layer (`python_bindings/**`), streaming (`protocol.rs`, `py_convert.rs`, `server/send.rs`), `frontend_download.rs`, `clipboard.rs`, derive crate, Makefile, `_native.pyi`, CHANGELOG, docs (SECURITY/STREAMING/API_REFERENCE/ARCHITECTURE/BUILDING + 13 opus docs + SKILL.md), root manifests, and 4 Python test files.

Commit list (in order): 63f99e4 (SEC-001), 6b74065 (SEC-004), 0b87a24 (SEC-003), 4f80cb3 (ARC-003), eb8aa64 (ARC-004), c3d3fd6 (ARC-006), b289c0d + 2ad01f5 (ARC-005), 744b0d1 (ARC-001), 7f92280 (DOC-001), ec627b5 (DOC-003), a34951d (DOC-004), d43ceb9 (DOC-005), 2d0efc2 (DOC-002), c0aec4f (derive bump follow-up), c836a3a (QA-001), ab31dff (QA-002), bcb4194 (QA-003), adec3bb (caps table), plus 5 merge commits.

---

## Next Steps

1. **Review SEC-003** (standalone commit `0b87a24`) — the run's one security-relevant behavior surface; flag defaults true so behavior is unchanged.
2. Decide the three backlog design calls listed above (clipboard set path, feature-combo gate, MUX.md ride-along timing).
3. Re-run `/audit` for an updated AUDIT.md; run `make fuzz-all` before the next release cut.
4. Wrap-up (pending Paul's confirmation): CHANGELOG remediation note, delete AUDIT.md, fast-forward `fix/audit-remediation` into local `main`, remove the worktree. No push (pushes happen at release cuts).
