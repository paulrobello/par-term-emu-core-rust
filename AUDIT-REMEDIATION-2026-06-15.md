# Audit Remediation Report

> **Project**: par-term-emu-core-rust
> **Audit Date**: 2026-06-15
> **Last Updated**: 2026-06-16
> **Scope**: cumulative — round 1 (safe-surgical subset), round 2 (all remaining Security), ARC-001 (Terminal god-object decomposition), and **round 3 (ARC sweep, 2026-06-16: 13 more Architecture items)**. All work is on branch `fix/audit-remediation`, **not merged**.

---

## Execution Summary (cumulative)

| Area | Status | Resolved this cycle |
|------|--------|---------------------|
| **Security (all)** | ✅ **All 7 resolved** | SEC-001 (zlib cap); SEC-002 (public-bind warning); SEC-003 (replaced htpasswd-verify → maintained bcrypt/md-5/sha1, openssl-vector-tested); SEC-004 (PyO3 0.28→0.29, zero source changes); SEC-005 (WS Origin + CORS defense); SEC-006 (rustls-pemfile → pki-types PemObject); SEC-007 (AVIF disabled → `paste` no longer compiled). `cargo audit`: **5 vulnerabilities → 0**. |
| **ARC-001** (Terminal god object) | ✅ **Complete** | Decomposed from ~150 flat fields → **56** (31 cohesive sub-struct holders + 25 irreducible core: buffer, cursor, parser, current cell render state, event dispatch, graphics store). 31 sub-structs across 25 commits, all behavior-preserving (1834 tests pass). See the [ARC-001 Decomposition](#arc-001-decomposition) inventory. |
| **ARC-007** (observer dispatch) | ✅ **Safe fix done** | Observer callbacks now run inside `catch_unwind` — a panicking observer is isolated instead of unwinding through the `parking_lot` mutex (which doesn't poison → would silently corrupt state + re-fire events). Regression test added. The full out-of-lock dispatch redesign remains future work. |
| **ARC-002** (`PyTerminal` god object) | ✅ **Structural split done** | The single 5768-line / ~384-method `#[pymethods]` block is split into **18 cohesive blocks** (core in `mod.rs` + 17 themed `*_api.rs` files) via the pyo3 `multiple-pymethods` feature. `mod.rs` halved (→ 2869 lines); the Python `Terminal` class surface is **unchanged** (1834 Rust + 492 Python tests pass). The audit's stretch remedy — nested sub-objects (`term.colors.x`) for autocomplete discoverability — is a breaking major-version API change and remains future work. |
| **ARC-003 / QA-001** (PyTerminal↔PyPtyTerminal dup) | ✅ **Substantially done** | The ~155 duplicated methods are reduced to **26** via a shared `TerminalAccess` trait + `#[macro_export]` method macros in `python_bindings/common.rs` (emitted once, invoked for both types). **129 methods unified, ~258 duplicate copies deleted, net −1,200+ lines.** Also deduped the 8 hand-written `PyAttributes` literals → `From<&Cell>`. The remaining 26 are **genuinely non-unifiable** (8 diverge write-to-PTY vs process-locally, 13 complex/different return types, `scrollback_len` alt-screen, etc.) — unifying them would change behavior. Behavior preserved throughout (492 Python tests pass). |
| **Architecture (other)** | ✅ 20 / ⏭️ 6 | Previously: ARC-003, ARC-004, ARC-005, ARC-007 (safe fix), ARC-011, ARC-019, ARC-025. **Round 3 added: ARC-006, 008, 010, 015, 016, 017, 018, 020, 022, 023, 024, 026, 027.** Remain: ARC-009, 012, 013, 014, 021, 028 (each a large focused effort — see [Round 3](#round-3--arc-sweep-2026-06-16)). |
| **Code Quality** | ✅ 6 / ⏭️ rest | QA-001, QA-002, QA-003, QA-004, QA-007, QA-010 resolved. QA-005/006/008/009/011/012/013 remain. |
| **Documentation** | ✅ 8 / ⏭️ rest | DOC-005/013/017/019/020/021/022/023/024 resolved. DOC-001–004/006–012/015/016/018 remain. |
| **Verification** | ✅ | `make checkall` green: cargo check + clippy (0 warnings) + fmt + ruff + pyright + **1702 Rust tests (non-streaming) + 174 streaming tests + 492 Python tests**. One known-flaky PTY timing test (passes in isolation). |

**Totals**: ~67 audit items resolved across all rounds (round 3 alone closed 13 Architecture items). **All Security closed; both Critical architecture findings (ARC-001, ARC-002) substantially done; ARC-003/QA-001, ARC-004/QA-002, ARC-005/QA-004 done; round 3 closed ARC-006/008/010/015/016/017/018/020/022/023/024/026/027.** Remaining work: ARC-009, 012, 013, 014, 021, 028 (each a large, focused effort — see [Round 3](#round-3--arc-sweep-2026-06-16)), QA-005/006/008/009, and the documentation tail (DOC-001 etc.).

---

## Round 3 — ARC sweep (2026-06-16)

A focused pass closing 13 Architecture items. Each landed as its own verified commit on `fix/audit-remediation`; the full suite stayed green throughout (1702 Rust + 174 streaming + 492 Python tests; clippy 0 warnings).

| ID | Title | What changed |
|----|-------|--------------|
| **ARC-006** | Cap `terminal_events` + DRY typed polls | `MAX_TERMINAL_EVENTS` (10k) cap with oldest-eviction at the end of `process()`, shifting the observer dispatch index so it stays consistent. The 4 typed `poll_*` methods share one `extract_terminal_events` partition helper. Full consolidation to a single `poll_events()` (breaking API) remains future work. Tests: oldest-eviction + index-shift + under-cap-untouched. |
| **ARC-008** | Reuse `process()` APC passthrough buffer | New reusable `apc_passthrough` field (`mem::take`/restore = no per-call realloc) + a fast path that feeds ESC-free chunks straight to vte when the APC filter is idle (skips the copy + filter pass entirely). Extracted an `advance_parser` helper for the vte borrow workaround. |
| **ARC-010** | Pack `Cell.hyperlink_id` (NonZeroU32) | `Option<u32>` → `Option<NonZeroU32>` (niche-opt: `None` costs 0 extra bytes → ~4 bytes/Cell saved across millions of scrollback cells). Hyperlink IDs now start at 1. Boundaries converted (osc mint, write assign, search lookup, `PyAttributes` keeps Python-facing `Option<u32>` via `.get()`). **Part B (group the 9 Kitty fields in `TerminalGraphic`) deferred** — 119 access sites for negligible memory gain on a rare structure; not worth the regression risk. |
| **ARC-015** | Split `streaming` mega-feature | Moved the 6 binary-only deps (clap, anyhow, tracing, tracing-subscriber, reqwest, tar — **0 library uses**) behind a new `streaming-bin` feature. Library embedders using `streaming` no longer pull a CLI parser / HTTP client / tar extractor. Binary `required-features`, Makefile, and CI updated. Deeper `streaming-tls`/`auth`/`sysinfo` sub-features (cfg-gating the library) deferred. |
| **ARC-016** | `PyCallbackObserver` reentrancy guard | `thread_local!` flag + Drop guard: a Python observer callback that re-enters terminal processing on the same thread (which would deadlock on the non-reentrant Terminal mutex) now drops the reentrant event instead. Applied to both sync + queue observers; panic-safe; unit-tested. |
| **ARC-017** | `glyph_cache` → bounded LRU | Replaced the nuclear `cache.clear()` at 10k entries with `lru::LruCache` (cap 10k, evicts LRU one-at-a-time, hits refresh recency). `peek()`+`get()` pattern avoids the long-lived `&mut` borrow. |
| **ARC-018** | Reader-thread cancellation on Drop | `Drop for PtySession` now force-closes the PTY master fd **before** the 2s join-wait (mirroring the already-correct `cleanup_previous_session`), so the blocked `read()` can return EOF and the thread joins instead of being detached. Also sets `running=false`. Timeout kept as backstop. |
| **ARC-020** | Single `TerminalEvent→ServerMessage` dispatch + proto staleness | Extracted `protocol::terminal_event_to_server_message()`; the two ~25-arm duplicated `match` blocks (broadcast + per-session, ~560 lines) collapse to 5-line if-lets. `send_*` broadcast methods retained (Python binding uses them). `build.rs` now warns when `proto/terminal.proto` is newer than the checked-in `terminal.pb.rs` (no more silent wire-format drift). |
| **ARC-022** | Screenshot iTerm2 P3 boost behind config | The hardcoded Display-P3 conversion + 1.4× brightness is now gated on `ScreenshotConfig.iterm2_color_boost` (default `true` = behavior-preserving); Rust callers can opt into raw sRGB. Python picks it up when QA-005's options-dict lands (no new positional param added now). |
| **ARC-023** | pyright `pythonVersion` 3.14→3.12 | Aligned with `requires-python >=3.12` (the floor) so pyright flags 3.13/3.14-only syntax. 0 errors. |
| **ARC-024** | Makefile `typecheck` + `clippy` targets | Added standalone `typecheck` (cargo check --all-targets --all-features + pyright) and `clippy` (check-only, no auto-fix) + help/.PHONY. |
| **ARC-026** | Centralize scrollback circular-buffer math | The `(start+i)%max` read-index and `(start+1)%max` head-advance (4 sites across `grid/mod.rs` + `grid/scroll.rs`) now route through two `#[inline]` helpers — one auditable place for the modulo invariant. Deeper `ScrollbackBuffer` type (resize without full rebuild) deferred. |
| **ARC-027** | Rename confusing snapshot modules | `terminal_snapshot.rs`→`replay_snapshot.rs`, `snapshot.rs`→`semantic_snapshot.rs` (+ test file), `snapshot_manager.rs` kept. All mod/use refs updated; 101 snapshot/replay tests pass. |

### Remaining Architecture items (each a dedicated, focused effort)

These were assessed but **deliberately not rushed** — each is blocked, a major architectural program, or low-ROI churn, and the branch is currently fully green. **ARC-009 (RwLock) and ARC-014 (`#[pyo3_get_all]` proc-macro) are now DONE** (round 4, see commits). What remains:

- **ARC-021** (`TerminalAction` enum) — **a multi-session architectural program, not an incremental change.** `perform.rs` is a thin 104-line dispatcher (8 `Perform` methods delegating to handlers); the actual mutation lives in the sequence handlers (`csi.rs`/`osc.rs`/`dcs.rs`/`esc.rs`/`write.rs`), each taking `&mut Terminal` and mutating directly. Separating parse-from-mutate means rewriting the **entire handler layer** to emit actions + an executor — exactly what the audit flags "large; defer." A shallow Perform-callback-level enum would add indirection without achieving the isolation goal and needs a record/replay consumer to be useful. Needs a dedicated effort.
- **ARC-012** (make Cell/Grid fields private) — **poor ROI as scoped.** `Cell` lives at the crate root (`src/cell.rs`), so `pub(super)` is ineffective (its parent is the crate = same as `pub(crate)`); the clean version needs ~434 accessor additions across `terminal/`, `screenshot/`, `python_bindings/` for moderate encapsulation value on an internally-consumed type. Mechanical (compiler-guided) but high-churn/low-value.
- **ARC-013** (centralized `PyErr` mapping) — **empirically not actionable (round 3)**: `grep 'format!("{}", e)'` over `python_bindings/` returns **0** — *every* domain-error→`PyErr` site adds context (e.g. `"Screenshot error: {e}"`) that a `From<DomainError> for PyErr` + `?` would **lose** (degrading Python error messages); the rest are direct constructions (validation/lock-failures). The audit premise doesn't hold.
- **ARC-028** (StreamingConfig getter/setter boilerplate) — **macro_rules proven blocked (round 3)**: PyO3's `#[pymethods]` rejects macro-generated items (`macros cannot be used as items in #[pymethods] impl blocks`). The `#[pyo3(get, set)]` field restructure is invasive (`StreamingConfig` carries `tls`/`http_basic_auth`). Needs a dedicated proc-macro crate (the `par-term-emu-derive` crate built for ARC-014 could host an attribute macro that pre-expands before `#[pymethods]`, but the StreamingConfig wrapper struct would need restructuring).
- **ARC-002 tail** (nested sub-objects) / **ARC-007 full** (out-of-lock dispatch) — stretch / breaking-major-version, as previously noted.

---

## Resolved Issues ✅

### Security (all 7 — fully closed)
- **[SEC-001]** Unbounded zlib decompression (zip-bomb DoS) — `src/streaming/proto.rs` + `src/streaming/server.rs` — `decode_with_decompression` now reads from the `ZlibDecoder` in 8 KiB chunks into a bounded `Vec`, returning `StreamingError::InvalidMessage` at the 1 MiB `MAX_DECOMPRESSED_SIZE` cap (was: unbounded `read_to_end`). Both tungstenite acceptors switched to `accept_hdr_async_with_config` with an explicit `WebSocketConfig` (16 MiB message + frame caps); same caps applied to both axum `WebSocketUpgrade` handlers. Replaces tungstenite's 64 MiB default. No new dependencies; return types and error variants preserved.
- **[SEC-002]** Standalone streamer public-bind hardening — `src/bin/streaming_server.rs` — already defaulted `--host` to `127.0.0.1`; now also prints a loud stderr warning when binding a non-loopback interface with no `--api-key` / HTTP Basic auth configured (exposes an interactive shell otherwise).
- **[SEC-003]** Replaced abandoned `htpasswd-verify` chain (RUSTSEC-2022-0011/0004/0071/2025-0121) — new `src/streaming/auth_hash.rs` verifies htpasswd hashes (bcrypt, `$apr1$`, `$1$` MD5-crypt, `{SHA}`) via maintained RustCrypto primitives (`bcrypt`, `md-5`, `sha1`) + `base64`. The MD5-crypt core was ported from the canonical crypt(3) algorithm and locked down with `openssl`-generated known-answer vectors. Drops `rust-crypto`/`rustc-serialize`/`time`/`gcc`.
- **[SEC-004]** PyO3 0.28.3 → 0.29 (RUSTSEC-2026-0176/0177) — Cargo.toml version bump only; the codebase already uses modern PyO3 patterns so zero source changes were required (verified by a worktree-isolated agent: 1698 + 1822 tests pass on 0.29).
- **[SEC-005]** CSRF-via-WebSocket defense — new `StreamingConfig.allowed_origins` allowlist + `check_ws_origin`/`is_local_origin`; the `Origin` header is validated at all four WS entry points (2 tungstenite + 2 axum), defaulting to allow non-browser clients + local origins and reject remote browser origins (HTTP 403); a `tower-http` `CorsLayer` mirrors the policy on both HTTP routers. Wired into the Python binding + `--allowed-origins` CLI flag / `PAR_TERM_ALLOWED_ORIGINS`. Unit tests cover local/remote/allowlist/look-alike-host cases.
- **[SEC-006]** Replaced unmaintained `rustls-pemfile` (RUSTSEC-2025-0134) — PEM loading now uses `rustls-pki-types`' `PemObject` trait (`pem_reader_iter`/`pem_slice_iter`), already transitively available. No new dependency.
- **[SEC-007]** Dropped AVIF from the `image` crate (RUSTSEC-2024-0436, transitive) — `image` `default-features` disabled; the `ravif → rav1e → paste` chain is **no longer compiled into any build** (`cargo tree -i ravif` confirms). AVIF was never produced/consumed. *Residual*: `cargo audit` still shows a low-severity unmaintained *warning* for `paste` because Cargo defensively retains the optional-but-disabled entry in `Cargo.lock`; it is never built. 0 vulnerabilities remain.

### Architecture
- **[ARC-011]** `poll_subscribed_events` duplicated the 25-arm match — `src/terminal/mod.rs` — replaced the re-implemented `TerminalEvent → TerminalEventKind` match with a call to the existing `TerminalEvent::kind()` (`event.rs`), preserving the filter partition exactly.
- **[ARC-019]** Coprocess output buffer `Vec::remove(0)` (O(n)/line) — `src/coprocess.rs` — `output_buffer`/`error_buffer` switched `Vec<String>` → `VecDeque<String>` with O(1) `push_back`/`pop_front`; the two drain consumers (`read()`/`read_errors()`) convert back to `Vec<String>` via `mem::take().into_iter().collect()`, so the public API is unchanged.
- **[ARC-025]** Duplicated `emit_style` SGR closure (~78 lines × 2) — `src/grid/export.rs` — verified byte-for-byte identical, extracted a private `push_sgr_style(result, fg, bg, flags)` helper; both closures removed, 3 call sites updated. Output identical.
- **[ARC-001]** `Terminal` god-object decomposition — **complete**. See the dedicated [ARC-001 Decomposition](#arc-001-decomposition) section below for the full sub-struct inventory.
- **[ARC-002]** `PyTerminal` god-object split — **structural core done**. The single ~384-method `#[pymethods]` block is split into 18 cohesive blocks: core methods in `mod.rs` (halved, 5768→2869 lines) + 17 themed `*_api.rs` files (triggers, color, clipboard, metrics, search, selection, snapshot, scrollback, bookmark, multiplexing, image, shell-integration, notification, recording, badge, file-transfer, mouse, text). Enabled pyo3's `multiple-pymethods` feature so PyO3 merges them into the same Python `Terminal` class — **zero API change** (492 Python tests pass). The nested-sub-object discoverability (`term.colors.x`) — the audit's stretch remedy — is a breaking major-version change and remains future work.
- **[ARC-003 / QA-001]** `PyTerminal`↔`PyPtyTerminal` duplication — **substantially done**. Introduced a `TerminalAccess` trait (RPITIT `term_ref`/`term_mut`) + `#[macro_export]` method macros in `python_bindings/common.rs` that emit each shared `#[pymethods]` method once and invoke it for both types. **129 of 155 duplicated methods unified; ~258 duplicate copies deleted; net −1,200+ lines.** Also deduped 8 hand-written `PyAttributes` literals → `From<&Cell>`. The remaining 26 duplicates are **genuinely non-unifiable** (8 where `PyPtyTerminal` writes the escape sequence to the PTY child while `PyTerminal` parses it locally — `paste`, `set/push/pop_keyboard_flags`, `query_*`; 13 with divergent return types/logic — `screenshot`, `create_snapshot`, `resize`, `__repr__`, etc.; `scrollback_len` diverges on alt-screen) — unifying them would change behavior, so they're intentionally left. Added a borrowed `PtySession::terminal_ref()` so the `MutexGuard` ties to `&self`. Behavior preserved throughout (492 Python tests pass on every batch).
- **[ARC-005 / QA-004]** `Cell.combining` `Vec<char>` → `SmallVec<[char; 4]>` — `src/cell.rs`, `src/terminal/write.rs`. Combining marks (variation selectors, ZWJ, skin-tone modifiers, regional-indicator pairs) now store inline for ≤4 marks (the >99.9% case), so cells that carry them no longer heap-allocate and cloning them (scroll/reflow/snapshot — the audit's ~800k-clone reflow case) is a memcpy instead of a heap alloc. Rare longer clusters spill to the heap (no data loss / no cap). `Cell` stays `Clone` (SmallVec is not `Copy`), but the per-cell allocation pressure on the grid hot paths is eliminated. New regression benchmark `cloning_combining_cells_is_fast_at_scale` (800k clones of a 3-mark cell < 200ms). 1835 Rust + 492 Python tests pass.
- **[ARC-004 / QA-002]** Collapse the 3 near-identical WS handlers — `src/streaming/server.rs` (4199→4010 lines). Made `Client<S>` generic over the tungstenite stream type (`AsyncRead + AsyncWrite`) so the **plain** and **TLS** handlers now share one `run_ws_session` dispatch loop (~770 lines of duplicated 11-arm `ClientMessage` dispatch eliminated; the audit's "edit 3 handlers to add a message type" pain is gone for tungstenite). The **axum** handler can't become a `Client<S>` (different framework) but its `RequestRefresh`/`SnapshotRequest` arms now call shared `build_refresh_message`/`build_snapshot_message` helpers. **Bonus parity fix**: the old TLS handler silently *dropped* `Mouse`/`FocusChange`/`Paste`/`SelectionRequest`/`ClipboardRequest` (its own comment admitted "handled only in primary handlers") — the shared loop now handles them uniformly. Added an end-to-end WS smoke test (`tests/test_ws_smoke.rs`: real `StreamingServer` + `tokio_tungstenite` client, handshake + Ping→Pong round-trip). 174 streaming tests + 492 Python tests pass; clippy clean.

### Code Quality
- **[QA-002]** See **[ARC-004 / QA-002]** under Architecture above — the 3 near-identical WS handlers collapsed via a generic `Client<S>` + shared `run_ws_session` (~770 lines of duplicated dispatch eliminated).
- **[QA-004]** See **[ARC-005 / QA-004]** under Architecture above — `Cell.combining` is now `SmallVec<[char; 4]>` (inline for ≤4 marks), eliminating per-cell heap allocation on the grid hot paths.
- **[QA-001]** See **[ARC-003 / QA-001]** under Architecture above — the ~155 duplicated `PyTerminal`/`PyPtyTerminal` methods are deduped to 26 (the genuinely-divergent remainder), and the 8 hand-written `PyAttributes` literals now use `From<&Cell>`. (The ~120 `Ok::<_, ()>(lock())` sites that were in `src/python_bindings/pty.rs` were largely eliminated as a side effect — the migrated methods now use the shared macro form.)
- **[QA-003]** `Ok::<_, ()>(x.lock())` dead-branch anti-pattern (12 sites) — `src/streaming/server.rs` — collapsed `if let Ok(mut w) = Ok::<_, ()>(writer.lock()) { … }` → `let mut w = writer.lock(); …` (Pattern A, 6 sites); the 6 `terminal_for_refresh.lock()` sites (Pattern B, with dead `else { None }`) collapsed to direct evaluation. (The ~120 identical sites formerly in `src/python_bindings/pty.rs` were eliminated by the ARC-003/QA-001 macro dedup above.)
- **[QA-007]** Per-character `String` allocation in `html_escape`/`escape_xml` — `src/terminal/mod.rs`, `src/screenshot/formats/svg.rs` — rewritten to a single pre-sized `String` with `push_str`/`push`. Exact escape mappings preserved.
- **[QA-010]** `get_dirty_region` two-pass + panic-bait `unwrap()` — `src/terminal/mod.rs` — single-pass `fold` returning `Option<(usize, usize)>`, no bare `unwrap()`. Semantics and return type preserved.

### Documentation
- **[DOC-005]** Wrong default port 8080 → 8099 — `README.md`, `Makefile`, `src/bin/streaming_server.rs` (module rustdoc) — all par-term streaming-server default-port references corrected. (Unrelated `8080` bind-address examples elsewhere left per conservative-grep rule.)
- **[DOC-013/017]** Rust toolchain 1.75+ → 1.88+ — `README.md` — now matches `Cargo.toml` `rust-version` and `docs/BUILDING.md`.
- **[DOC-019]** STREAMING.md architecture diagram label "JSON Messages" → "Protobuf (binary)" — `docs/STREAMING.md` — matches the documented wire format.
- **[DOC-020]** ARCHITECTURE.md missing modules — `docs/ARCHITECTURE.md` — added `apc_filter.rs` (Kitty TGP APC filter). **Correction applied**: the audit's path `src/terminal/badge.rs` is wrong — the file is at `src/badge.rs` (top-level), so it was added to the Supporting Modules list instead.
- **[DOC-021]** Stale hard-coded test counts — `docs/ARCHITECTURE.md` — replaced brittle numbers (1,652 / 552 / 2,204) with runnable commands.
- **[DOC-022]** Missing "never `cargo build`" warning — `docs/BUILDING.md` — added prominent `⚠️` callout near the top.
- **[DOC-023]** Missing contributor guide — `CONTRIBUTING.md` (new, 113 lines) — covers dev setup, the `make checkall` requirement, the version-sync rule, the Rust↔Python binding-sync rule, the streaming-protocol 3-layer rule, and the PR workflow.
- **[DOC-024]** Missing module rustdoc — `src/pty_session.rs` — added `//!` module doc describing the `Arc<Mutex<Terminal>>` model, reader thread, `running` flag vs `try_wait()`/`wait()`, and generation counter.

---

## Requires Manual Intervention 🔧

These issues could not be safely auto-remediated. They require dedicated planning, are breaking changes, or depend on a codegen/design decision.

> **Update (round 3, 2026-06-16):** ARC-006, 008, 010, 015, 016, 020, 026, and 027 — previously listed below as open — are now **resolved** (see [Round 3](#round-3--arc-sweep-2026-06-16)). The entries that remain relevant below are the genuinely large/risky items: ARC-009, 012, 013, 014, 021, 028, the ARC-002 tail, QA-005/006/008/009, and DOC-001/018.

### Critical — remaining architecture
- **[ARC-002 tail] Nested sub-objects for Python discoverability** — `src/python_bindings/terminal/mod.rs`
  - **Status**: The structural core is **done** — the god-object `#[pymethods]` block is split into 18 cohesive files (above). What remains is the audit's *stretch* remedy: exposing cohesive nested `#[pyclass]` sub-objects (`term.clipboard.history`, `term.colors.default_fg`, `term.triggers.add(...)`) so autocomplete becomes usable.
  - **Why open**: It's a **breaking Python API change** (the audit itself flags it for a major-version bump / compat shim). It also needs the PyO3 parent-child sharing pattern (the sub-objects must reach the underlying `Terminal`, which `PyTerminal` owns directly — so it requires either the fiddly `Py<PyTerminal>` parent-ref pattern or restructuring `PyTerminal` to hold `Arc<Mutex<Terminal>>`). Defer to a dedicated major-version effort, not this branch.
  - **Recommended approach**: At the next major version, expose nested `#[pyclass]` sub-objects holding a parent ref; keep the flat methods as a deprecation shim for one release.

### High — large refactors (Security fully closed; these remain)
- **[QA-006] `row_text`/`cells_to_text` allocate a `Vec<String>` per row** — `src/terminal/mod.rs`, `src/grid/mod.rs`
  - **Why open**: The companion allocation fix to ARC-005 (which is done). `row_text`/`cells_to_text` build a `Vec<String>` per row (`N+1` allocations); a single-`String` `push_str` pass eliminates them. Deserves its own focused PR.
  - **Estimated effort**: Medium.
- **[ARC-006/008/009/010] mod.rs hot-path, locking, event-cap, layout** — `src/terminal/mod.rs`, `src/pty_session.rs`, `src/graphics/mod.rs`
  - **Why open**: Now unblocked by ARC-001. (ARC-007's panic-isolation safe fix is already done; the remaining ARC-007 work — moving observer dispatch fully out of the mutex — pairs naturally with these.)
  - **Estimated effort**: Medium each.
- **[QA-005] `screenshot` 17–19 positional params → options struct** — `src/python_bindings/*`
  - **Why open**: Public API change needing a deprecation shim; pair with QA-009 in one release for a single doc-sync.
  - **Estimated effort**: Medium.
- **[QA-008/009] Clone audit + typed Python exception hierarchy** — `src/python_bindings/*`
  - **Why open**: Need profiling judgment (QA-008) and an API-design decision on the exception hierarchy (QA-009).
  - **Estimated effort**: Medium.
- **[ARC-012..018, 020..028] Remaining architecture Medium/Low** — various
  - **Why open**: Lower-leverage. Schedule individually.

### Documentation (deferred, needs a decision)
- **[DOC-001] Regenerate `docs/API_REFERENCE.md` Data Classes from bindings** (Critical doc defect)
  - **Why deferred**: Documents dozens of `#[pyo3(get)]` properties that don't exist on the bindings. Largest doc defect, but the right fix is a **codegen decision** (hand-maintain vs auto-generate from struct fields) — regenerating by hand now would just re-drift on the next feature.
  - **Recommended approach**: Build a small generator that emits the Data Classes section from `src/python_bindings/types.rs` `#[pyo3(get)]` field lists; add a CI check. Then regenerate. Fold in DOC-002/003/004/006/007/009/010/011 (the per-method/per-class accuracy fixes).
  - **Estimated effort**: Medium (generator + first regen).
- **[DOC-018] Regenerate STREAMING.md env-var/CLI tables** — depends on the `clap` surface being stable for the next release; regenerate after any pending CLI changes land.

---

## ARC-001 Decomposition

`Terminal` was a ~150-field god object. It is now a compositor holding **29 cohesive sub-structs** plus **~36 irreducible-core flat fields** (the buffer, cursor, parser, current cell render state, event dispatch, and graphics store that genuinely *are* the terminal). Every extraction is **behavior-preserving** — existing accessor methods on `Terminal` delegate to the sub-struct, so all callers (including the Python bindings) are unaffected; the full **1833-test suite passes** and clippy is clean. Done across 23 commits on `fix/audit-remediation`, four delegated batches (one worktree-isolated) following a single proven pattern.

| # | Sub-struct | Holder | Consolidated fields |
|---|---|---|---|
| 1 | `ClipboardSyncState` | `clipboard_sync` | OSC 52 clipboard-sync (6) |
| 2 | `ProfilingState` | `profiling` | perf metrics + profiling (5) |
| 3 | `MouseHistoryState` | `mouse_history` | mouse event/position history (3) |
| 4 | `SearchState` | `search` | regex search (2) |
| 5 | `InlineImageState` | `inline_image_state` | inline image storage (2) |
| 6 | `RenderingState` | `rendering` | rendering hints + damage regions (2) |
| 7 | `MacroState` | `macros` | macro library + playback (3) |
| 8 | `TmuxState` | `tmux` | tmux control protocol (2) |
| 9 | `TriggerState` | `triggers` | trigger registry/highlights/actions (5) |
| 10 | `NotificationState` | `notifications_state` | OSC 9/777 + Feature 37 notifications (7) |
| 11 | `RecordingState` | `recording_state` | recording/replay (3) |
| 12 | `KeyboardState` | `keyboard_state` | Kitty keyboard flags + modifyOtherKeys (4) |
| 13 | `SyncState` | `sync_state` | synchronized-update mode + buffer (3) |
| 14 | `TitleState` | `title_state` | title stack + answerback (3) |
| 15 | `ShellState` | `shell_state` | shell-integration core (5) |
| 16 | `BookmarksState` | `bookmarks_state` | bookmarks (2) |
| 17 | `CharsetState` | `charset_state` | G0/G1 ACS charsets (3) |
| 18 | `HyperlinkState` | `hyperlink_state` | hyperlink store/IDs (3) |
| 19 | `ClipboardState` | `clipboard_state` | OSC 52 clipboard content + history (4) |
| 20 | `DcsState` | `dcs_state` | DCS/Sixel parser state (4) |
| 21 | `MarginState` | `margins` | scroll + left/right margins (5) |
| 22 | `TerminalModes` | `modes` | VT mode booleans/enums (12) |
| 23 | `ColorThemeState` | `theme` | palette + OSC 10/11/12 + iTerm2 render colors (18) |
| 24 | `SavedCursorState` | `saved_state` | DECSC/DECRC saved state (5) |
| 25 | `CommandHistoryState` | `command_history_state` | Feature 31 command/CWD history (5) |
| 26 | `ProgressBellState` | `progress_state` | progress bars + bell counter (3) |
| 27 | `UnicodeConfigState` | `unicode_state` | width + normalization config (2) |
| 28 | `SecurityFlagsState` | `security_state` | accept_osc7 + disable_insecure_sequences (2) |
| 29 | `BadgeState` | `badge_state` | OSC 1337 badge format + session vars (2) |
| 30 | `GraphicsState` | `graphics` | unified graphics store + Sixel limits + cell dimensions + iTerm2 multipart + file-transfer manager (5) |
| 31 | `EventBrokerState` | `events` | terminal/bell event buffers + dispatch index + observer registry + ID counters (6) |

**Remaining flat core** (~25 fields): the genuinely-irreducible terminal state — `grid`, `alt_grid`, `alt_screen_active`, `cursor`, `alt_cursor`, `fg`, `bg`, `underline_color`, `flags` (current SGR cell render state), `parser`, `apc_filter_state`, `apc_buffer`, `kitty_parser`, `pending_wrap`, `pixel_width`, `pixel_height`, `response_buffer`, `conformance_level`, `warning_bell_volume`, `margin_bell_volume`, `dirty_rows`, `selection`, `pane_state`, `event_subscription`, `tab_stops`.

**All cohesive groups extracted.** The two groups previously skipped (EventBroker, GraphicsState) were extracted in the final pass; EventBroker's dispatch logic now reads through `self.events.*` (and, per ARC-007, observer callbacks are `catch_unwind`-isolated).

---

## Verification Results

- **Build (`cargo check`)**: ✅ Pass — both `--features pyo3/auto-initialize` and `--features pyo3/auto-initialize,streaming`.
- **Rust lint (`make lint`)**: ✅ Pass — `cargo clippy --all-targets --all-features -- -D warnings` finished with **0 warnings**; `cargo fmt` clean.
- **Rust tests**: ✅ Pass — 1698 (non-streaming) + 1822 (streaming) passing. **One pre-existing flake**: `pty_session::tests::test_generation_counter_increments_on_pty_output` intermittently fails under full-suite parallel load (reader-thread timing race) but **passes 5/5 in isolation** (0.21s each). It does not touch code modified by this remediation (the increment ordering was fixed in commit `8b0201a`); known-flaky per the project's vault notes. Not a regression.
- **Python lint (`make lint-python`)**: ✅ Pass — `ruff format` (36 files unchanged), `ruff check` (all passed), `pyright` (0 errors, 0 warnings).
- **Python tests (`make test-python`)**: ✅ Pass — 492 passed, 71 skipped.

All checks green except the single known-flaky PTY timing test, which is unrelated to these changes.

---

## Files Changed

The cumulative remediation spans ~40 commits across three rounds. Rather than enumerate every file here, the authoritative record is:

```
git log --oneline main..HEAD
git diff --stat main..HEAD
```

Highlights by round:

- **Round 1 — safe-surgical** (`2fac382`, `0e545f9`, `6b62063`, `1c0b667`): SEC-001 (zlib cap + WS size limits); QA-003/007/010; ARC-011/019/025; DOC-005/013/017/019/020/021/022/023/024; new `CONTRIBUTING.md`.
- **Round 2 — Security** (`8e9008d`, `aec53e9`): SEC-002/003/004/005/006/007 — new `src/streaming/auth_hash.rs`, `rustls-pemfile` → `PemObject`, WS Origin/CORS (`check_ws_origin`, `build_cors_layer`, `allowed_origins` config + CLI/binding), PyO3 0.29, image AVIF disabled.
- **ARC-001** (`686256b` audit report + 23 `refactor(arc-001):` commits): `src/terminal/mod.rs` + the 29 feature/color/mode files whose field accesses were migrated. 29 cohesive sub-structs; ~36 irreducible-core flat fields remain.

**Branch**: `fix/audit-remediation`, **not merged**. See `git log --oneline main..HEAD`.

---

## Next Steps

1. **Round 3 (2026-06-16) closed 13 more Architecture items** (ARC-006/008/010/015/016/017/018/020/022/023/024/026/027) — see the [Round 3 table](#round-3--arc-sweep-2026-06-16). The branch is fully green (1702 Rust + 174 streaming + 492 Python tests; clippy 0 warnings).
2. **Remaining Architecture** — each is a dedicated, focused effort (not quick wins): **ARC-009** (coarse mutex split — high-risk concurrency, must not regress the GIL-vs-mutex invariant), **ARC-012** (Cell/Grid field privacy — large cascade), **ARC-013** (centralized `PyErr` mapping — *low leverage* per round-3 analysis: most `new_err` sites are direct constructions, not conversions), **ARC-014**/**ARC-028** (each needs a **new proc-macro crate**), **ARC-021** (`TerminalAction` enum — large), and the stretch items **ARC-002 tail** (nested sub-objects, breaking/major-version) + **ARC-007 full** (out-of-lock observer dispatch).
3. **Highest-leverage remaining non-ARC items**: **QA-006** (the `row_text`/`cells_to_text` single-String allocation fix — companion to ARC-005), **DOC-001** (the `API_REFERENCE.md` regeneration codegen decision — the only remaining *Critical* doc defect; folds in DOC-002/003/004/006/007/009/010/011), and **QA-005/008/009**.
4. **Investigate the flaky** `test_generation_counter_increments_on_pty_output` (passes in isolation; not introduced by this work).
5. **Re-run `/audit`** to refresh `AUDIT.md` against the current (much-improved) state.
6. **When ready to release**: extend the `[Unreleased]` CHANGELOG entry with the round-3 items, then merge `fix/audit-remediation` to `main`.
