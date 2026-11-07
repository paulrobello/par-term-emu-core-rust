# Audit Remediation Plan — opus re-audit 2026-11-06 (evening)

> Companion to `AUDIT.md` (same run, tag `audit-2026-11-06-opus`). `/fix-audit` reads this file
> for per-issue execution detail; phase/conflict/blocking structure lives in AUDIT.md's
> `## Remediation Plan`. Order here matches those phases. Every entry is executable without
> re-analysis. Work in a worktree per repo Git Workflow; gate with `make checkall` while
> iterating and `make checkall-ci` before any done claim.
>
> Repo-specific verification keys:
> - core tests: `cargo test -p par-term-emu-core <filter>`
> - mux/attach tests: `cargo test -p par-mux --features mux-bin,attach <filter>` (mux-bin **alone** matches 0 attach tests — the `attach` feature is required)
> - streaming tests: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming <filter>`
> - fast gate: `PATH="$PWD/.venv/bin:$PATH" make checkall`
> - caps table regen: `python3 scripts/gen_caps_table.py` (check mode is the `caps-table-check` gate leg)

## Phase 3a — Security

### [SEC-001] Orphaned iTerm2 multipart transfers retain unbounded memory
- **Files**: `crates/par-term-emu-core/src/terminal/graphics.rs:216-270` (`handle_multipart_file_start`), `crates/par-term-emu-core/src/terminal/file_transfer.rs` (`start_download` :116-147, `FileTransferManager` :74-105), `docs/SECURITY.md` (generated caps table)
- **Steps**:
  1. In `file_transfer.rs`, add `/// cap: Concurrent in-progress downloads from client file-transfer requests.` above a new `pub(crate) const DEFAULT_MAX_ACTIVE_TRANSFERS: usize = 8;` (mirroring `DEFAULT_MAX_COMPLETED` at :93 and the mux pattern `MAX_CONCURRENT_UPLOADS = 8` in `crates/par-mux/src/mux/uploads.rs:43`). Store it as `max_active: usize` in the struct, initialized in `Default`.
  2. In `start_download`, before inserting: if `self.active_transfers.len() >= self.max_active`, fail-oldest — take the entry with the smallest `started_at`, move it to `completed_transfers` via `push_completed` with `TransferStatus::Failed("superseded by new transfer (active cap)")`, then proceed.
  3. In `graphics.rs` `handle_multipart_file_start`, in the file-transfer branch (`:241-269`), BEFORE assigning the new `iterm_multipart_buffer`: `if let Some(old) = self.graphics.iterm_multipart_buffer.take() { if old.is_file_transfer { if let Some(old_id) = old.transfer_id { let _ = self.graphics.file_transfer_manager.fail_transfer(old_id, "superseded by MultipartFile restart"); } } }` — then construct the new state (keep the existing `Some(...)` assignment semantics; the buffer's Vec chunks of a non-file (inline) transfer simply drop as before).
  4. Regenerate the caps table so the new constant appears in `docs/SECURITY.md`: `python3 scripts/gen_caps_table.py`.
  5. Regression test in `file_transfer.rs` (or the graphics tests where OSC handling is tested): start `MultipartFile` A (size 100), append one FilePart, start `MultipartFile` B; assert A is in `completed_transfers` with a Failed status, B is the sole active transfer, and manager memory footprint is bounded. Add a second test driving 9 concurrent starts asserting the active count never exceeds 8.
- **Method**: The orphan is unreachable after buffer replacement — `FilePart` appends only to the current buffer's `transfer_id` — so no sweep can find it; the only correct eviction points are the replace site (step 3) and admission (step 2). Do NOT try to sweep by age: an in-progress transfer may legitimately pause. `fail_transfer` already moves entries to the bounded ring, so reuse it. The inline-image branch needs no change: replacing the buffer frees the old chunks wholesale.
- **Verify**: `cargo test -p par-term-emu-core file_transfer` and `cargo test -p par-term-emu-core iterm`; then `PATH="$PWD/.venv/bin:$PATH" make checkall` (the `caps-table-check` leg must pass with the regenerated table).

### [SEC-002] Query-param API key not percent-decoded
- **Files**: `src/streaming/server/http.rs:138-148`, `:363-372`
- **Steps**:
  1. Where the `api_key` query param is extracted in both routes, percent-decode the value before `ct_eq` (reuse the frontend/download module's existing percent-decoding helper if present; otherwise a minimal `percent-decode` on the `&str` — no new dependency: hand-roll the standard `%XX` loop used elsewhere in the repo, or `urlencoding` if already in Cargo.lock — check first).
  2. If hand-rolling is awkward, the acceptable alternative (documented, no code): extend the existing startup warning for `--allow-api-key-in-query` to state the key must be URL-safe (`A-Za-z0-9_-`), and validate at config load, refusing non-URL-safe keys when query auth is enabled. Pick ONE of the two; decoding is the better fix.
- **Method**: Functional edge, not exploitable — the goal is that a configured key either works on the query channel or is rejected loudly at startup, never fails silently. Keep the timing-safe comparison on the decoded value.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming http` plus a unit test asserting a key containing `%26` matches via query after decoding.

### [SEC-003] Host-header allowlist on loopback-bound static routes (optional)
- **Files**: `src/streaming/server/http.rs` (static web_term/ serving paths)
- **Steps**:
  1. When the server binds a loopback address and HTTP is enabled, reject (403) requests whose `Host` header names neither a loopback literal (`127.0.0.1`, `::1`, `localhost`) nor the configured bind address/host. Apply to static routes only; WS and origin-checked routes keep their existing checks.
  2. Add a config note in `docs/STREAMING.md` (one paragraph) and keep the behavior on by default for loopback binds — it cannot break legitimate loopback use.
- **Method**: DNS-rebinding protection. Do NOT gate behind a flag (a flag defaults to off and the protection evaporates); loopback Host values are a closed set.
- **Verify**: streaming test suite filter above + a test asserting a non-loopback Host on the static route gets 403 while `localhost:8099` passes.

### [SEC-004] Failed-auth throttling (optional)
- **Files**: `src/streaming/server/http.rs:94-180`, `:324-396`
- **Steps**:
  1. Add a small per-connection attempt counter with escalating delay (e.g., 50ms × 2^attempts, capped 5s) applied on 401 responses in both the axum middleware and the tungstenite handshake callback.
  2. No shared-state rate limiter — per-connection is enough (each guess costs a connection) and avoids lock contention on the hot path.
- **Method**: Keep it dumb and local; bcrypt already slows Basic. The point is to remove free high-speed guessing on the API-key `ct_eq` path.
- **Verify**: streaming test filter + a test making N bad attempts and asserting the Nth response is delayed (assert the counter state, not wall-clock).

## Phase 3b — Architecture

### [ARC-001] python/python-test feature parity gate
- **Files**: root `Cargo.toml` `[features]`, `scripts/` (new `check_python_features.py`), `Makefile` (gate leg)
- **Steps**:
  1. New `scripts/check_python_features.py` (stdlib `tomllib` only, modeled on `scripts/check_api_surface.py`): parse root `Cargo.toml`; extract the feature lists for `python` and `python-test`; assert `set(python) - {"pyo3/extension-module", "pyo3/auto-initialize"} == set(python_test) - {"pyo3/extension-module", "pyo3/auto-initialize"}` — i.e., the set difference between the two full lists is exactly the pyo3 activation feature pair (one has extension-module, the other auto-initialize; both may carry `pyo3` itself).
  2. Makefile target `python-features-check: ## Root python/python-test feature lists stay in lockstep` running `python3 scripts/check_python_features.py`, wired into `checkall` in the **early deterministic block** next to `api-surface-check` (fail-fast convention, R11 ordering).
- **Method**: Every other lockstep invariant has a named gate; this is the same shape. Assert on sets, not order, so cosmetic reordering doesn't fail.
- **Verify**: `PATH="$PWD/.venv/bin:$PATH" make checkall` (leg runs); temporarily drop `screenshot` from `python-test` in a scratch edit to confirm the gate fails, then revert.

### [ARC-002] default-session dual ownership in close_session
- **Files**: `src/streaming/server/mod.rs:498`, `src/streaming/server/sessions.rs:97-150`, `:242-247`
- **Steps**:
  1. Read `resolve_session` (:242) and the reaper's default guard (:300) first — the fix must match the contract the reaper already encodes.
  2. In `close_session`, when `session_id == "default"` and no factory is configured: take the session AND clear `self.default_session` (`self.default_session = None` if the field is `Option`, or restructure to a registry-only lookup if trivial). A subsequent `resolve_session("default")` should then recreate/find the default session the same way `with_config` did at startup.
  3. Add a regression test: `close_session("default")` in single-session mode, then `resolve_session("default")` — assert the returned session is live (PTY writer present, shutdown not fired), and `broadcast()` does not target the dead instance.
- **Method**: Two owners is the defect class; if the restructure in step 2's parenthetical is more than ~30 lines, do the minimal take-and-clear now and leave the single-owner restructure as a note. Do not change factory-mode behavior (multi-session servers legitimately close "default").
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming session` + `make checkall`.

### [ARC-003] Dead broadcast_to_session with wrong-session fallback
- **Files**: `src/streaming/server/sessions.rs:90-103`
- **Steps**:
  1. Confirm zero callers still (parsight `find_symbol broadcast_to_session` with `repository_id: par-term-emu-core-rust`, then grep `src/ tests/ python/` for the name).
  2. Delete the method. If any Python binding exposes it (check `src/python_bindings/streaming.rs`), remove the binding too and note the surface change (lower `check_api_surface.py` baseline in the same commit if a `PtyTerminal`/`Terminal` method count changes — this is on StreamingServer, likely not baseline-tracked, verify).
  3. In the same edit, add one doc line on `send_to_session` and `broadcast_all` stating their miss semantics (miss is a miss / iterates all), so the API's three names stop being a trap.
- **Method**: Deleting uncalled public API beats documenting a trap. The wrong-session fallback (miss → default session's clients) is the worst broadcast failure mode; it must not survive even refactored.
- **Verify**: `cargo build` via `PATH="$PWD/.venv/bin:$PATH" make dev` + `make checkall`; streaming tests above.

### [ARC-004] caps-table-check gate placement
- **Files**: `Makefile:744` (checkall leg order)
- **Steps**: Move the `caps-table-check` leg from its current position (after the test legs) into the early deterministic block, adjacent to `mux-docs-check`. No other change.
- **Method**: R11 fail-fast convention; the leg is stdlib-only and needs no build artifacts, so nothing earlier generates what it consumes.
- **Verify**: `PATH="$PWD/.venv/bin:$PATH" make checkall` passes; `make -n checkall | grep -n caps-table` shows it before the test legs.

### [ARC-005] Broadcast fan-out lock (watch — NO ACTION)
- **Files**: `crates/par-mux/src/mux/server/broadcast.rs:45-60`
- **Steps**: none. Observation card (`watch`, recheck 180d): `push_to_clients_skipping` holds the `Clients` map lock across per-client `try_send` + eviction scan. Sound at tens of clients. Re-trigger: aggregate client count on one daemon grows by an order of magnitude → shard the map or move to per-client sinks.

## Phase 3c — Code Quality

### [QA-001] Upload verbs block tokio workers (+ fold QA-007)
- **Files**: `src/streaming/server/messages.rs:290-475`, `src/streaming/server/ws_session.rs:125,238`, `src/streaming/mux_factory/upload.rs` (read-only context)
- **Steps**:
  1. In `handle_client_message`'s four upload arms (begin/chunk/commit/abort at :296,347,406,444), wrap each handler call in `tokio::task::spawn_blocking` (or wrap once around a helper taking the verb): `let replies = tokio::task::spawn_blocking(move || handle_upload_begin(...)).await` mapping a JoinError to the existing error-report shape. The handlers return `Vec<ServerMessage>` — the boundary is already clean; the required state (upload pipe, config) must be cloneable or moved carefully — read what each handler captures first.
  2. While in the region, fold QA-007: extract the repeated read-only-refusal + `upload_pipe` resolution + error-report scaffolding (~25 lines × 4) into one private helper taking a closure for the verb body.
  3. Replace the doc comment at messages.rs:290-294 with the now-true statement (verbs offload via spawn_blocking; only the sync upload pipe interaction blocks a blocking-pool thread).
- **Method**: The module already contains the correct idiom (`metadata_session_off_runtime().await` at ws_session.rs:62) — match it. Keep the bounded-reply semantics identical (same timeouts, same error strings) so golden/protocol tests don't shift. Do NOT touch `mux_factory/upload.rs` worker internals in this fix.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming upload` and the full streaming filter; `make checkall`. If upload integration tests exist under `tests/test_streaming.rs`, run `uv run pytest tests/test_streaming.rs -k upload` is N/A (Rust) — use the cargo filters.

### [QA-002] Extract shared agent-state-core.ts
- **Files**: `crates/par-mux/integrations/extensions/par-mux-agent-state.ts` (367 lines), `crates/par-mux/integrations/extensions/par-mux-omp-agent-state.ts` (565 lines), new `crates/par-mux/integrations/extensions/agent-state-core.ts`
- **Steps**:
  1. Diff the two extensions; the shared machinery is: `sendRequestAttempt` (27/32, byte-identical), `drainStateQueue` (250/256), `sendRequestNow` (65/70), `updateSessionRef` (133/138), and near-dupes `reportSession`, `sendState`, `releaseAgent`.
  2. Create `agent-state-core.ts` exporting a factory (`createAgentStateCore(config)`) parameterized by: request endpoint name, state field mapping, and agent-kind identifier. Move the retry/backoff loop, queue drain, session-ref update, and release logic into it verbatim from the claude extension (it is the older/leaner fork).
  3. Reduce each extension to its field mapping + kind-specific wiring importing the core. Preserve each file's public behavior exactly — these ship to users' editors via the installer; no signature changes.
  4. Check the installer path (`crates/par-mux/src/mux/integrations/`) for how extension files are shipped — the new core file must be included wherever the two forks are (grep the install/copy list for the existing filenames and add the core).
- **Method**: Extract-then-adopt order keeps the diff reviewable: core lands holding the claude version's logic; the omp extension switches to it in the same commit so the tree is never half-migrated. Drift already started in `reportSession`/`sendState`/`releaseAgent` — the claude version is the reference; note any omp-only behavior in those three and parameterize rather than delete.
- **Verify**: `make checkall` (the integrations dir has gates? check `integrations` targets; at minimum ensure no Rust-side checksum/pinning of these files breaks — grep for their filenames in scripts/ and Makefile). Manually `bunx tsc --noEmit` each extension if a tsconfig exists in `integrations/`, else eyeball-compile. State in the PR that runtime verification is the ENH-064 vitest harness's job once it exists.

### [QA-003] WindowSession god type — scope extension of the existing ARC-002 board card
- **Files**: board card `01a1b406f3c97481848d637110109365` (title `[ARC-002] De-complexify attach input dispatch and god files`); code in `crates/par-mux/src/mux/attach/render/`
- **Steps**: No separate fix agent run — this finding UPDATES that card's scope (done at audit time; see its notes). When the card's table-drive refactor executes: (1) fold the six seed/status flags (`status_dirty`, `status_snapshot_inflight`, `landing_reseed`, `seed_snapshot_inflight`, `seed_raced_panes`, `status_end` — `render/mod.rs:933-957`) into one `StatusInflight` sub-struct with an explicit state enum; (2) split input routing's state out of `WindowSession` so `route_mouse`/`mouse_hit` own their context; (3) `csi_key`/`copy_mode_char` table-drive per the card's original plan.
- **Method**: parsight churn data: `route_mouse` (175 edits) + `mouse_hit` (226 edits in 14d) are the repo's top real hotspots, both mutating the one shared 40+-field context. Type-boundary split BEFORE the function-body work would force ARC-002's call sites through two reworks — hence fold, not parallel.
- **Verify**: belongs to the ARC-002 card's own criteria.

### [QA-004] Drain hot-path per-line allocations
- **Files**: `src/streaming/mux_factory/stream.rs:154,212,229`
- **Steps**:
  1. The outer loop already snapshots `pane_num` at :154 — build `let pane_prefix = format!("{pane_num} ");` and `let link_prefix = format!("%{}", link.pane());` once per outer iteration (the second only when a link exists).
  2. Replace `line.starts_with(&format!("{pane_id} "))` with `line.starts_with(&pane_prefix)` (or `strip_prefix` if the remainder is used) and `id == format!("%{}", link.pane())` with `id == link_prefix`.
- **Method**: Two heap allocations per pane-output line on the highest-frequency path; the value compared changes only on retarget. No behavior change — pure allocation hoist.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming mux_factory` + `make checkall`.

### [QA-005] Generated golden variant coverage
- **Files**: `src/streaming/proto_golden_tests.rs:877` (`proto_golden_fixtures_cover_every_variant`), possibly `src/streaming/protocol.rs`
- **Steps**:
  1. Replace the hand-maintained exhaustive match with a variant-enumeration check. Prefer a tiny local macro over adding `strum` as a dependency (check Cargo.lock first; if `strum` is already present, use `EnumIter`). The cleanest dependency-free route: a `const VARIANT_NAMES: &[&str]` maintained next to the enum in `protocol.rs` with a `ProtoConvert`-derived or hand-rolled assert that its length equals the enum's discriminant count (a `match` returning name per variant with no wildcard arm — the wildcard ban already applies, so adding a variant forces the name list).
  2. The golden test then asserts every name in `VARIANT_NAMES` has a fixture, and the no-wildcard name-list match makes a new variant a compile error until named.
- **Method**: The invariant is a tripwire — keep it failing loudly on a new variant while removing the per-addition edit of a 69-branch function. Do not change fixture bytes; golden values are pinned.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming proto_golden` — all goldens still byte-identical; add a scratch variant temporarily to prove the new mechanism fails, then revert.

### [QA-006] Per-cell format! in grid export
- **Files**: `crates/par-term-emu-core/src/grid/export.rs:61-65`, `:330`
- **Steps**:
  1. Replace `result.push_str(&format!("..."))` per cell/attribute with `write!(result, "...")` via `std::fmt::Write` (import `std::fmt::Write as _`). Follow the existing pattern in `crates/par-mux/src/mux/emit.rs` (`write!(line, ...)`).
  2. At :330 (cursor positioning) same treatment. Keep the exact same escape bytes — export output is compared in tests.
- **Method**: `format!` allocates a temp String per cell; `write!` into the existing buffer avoids it. Identical output, no API change. This path serves both on-demand export and the streaming refresh (`export_visible_screen_styled`).
- **Verify**: `cargo test -p par-term-emu-core export` (existing export tests assert bytes) + `make checkall`.

### [QA-007] folded into QA-001 (same agent, same region) — no separate entry

### [QA-008] impl_repr! macro
- **Files**: `src/python_bindings/common.rs` (macro layer), call sites `pty.rs:995`, `streaming.rs:642`, `enums.rs:191,272,326,410`
- **Steps**:
  1. Add `impl_repr!` to `common.rs` next to the existing shared-method macros: `macro_rules! impl_repr { ($ty:ident, $fmt:expr) => { ... impl __repr__ returning format!($fmt, self fields?) ... } }` — survey the six sites first; if formats differ structurally, parameterize the body expression.
  2. Replace the six impls with macro invocations. **Coordinate with DOC-004**: `streaming.rs` is a conflict file (both agents edit it) — read current state before editing.
- **Method**: The `common.rs` macro layer is the established DRY home (~155 shared methods already). Pure dedup, no behavior change; each `__repr__` output string must stay byte-identical (pyclass reprs appear in Python tests).
- **Verify**: `PATH="$PWD/.venv/bin:$PATH" make dev && uv run pytest tests/ -k repr` (or the relevant file) + `make checkall` (stub-docs gate re-checks docstrings).

### [QA-009] Stale worktrees and leftover branch
- **Files**: `.claude/worktrees/ws-latency` (branch `worktree-ws-latency`, merged), `.claude/worktrees/select-layout-presets` (branch at `d7b71cd1`, NOT merged), branch `worktree-agent-a728a435bb5693108` (merged, worktree gone)
- **Steps**:
  1. `git worktree remove .claude/worktrees/ws-latency && git branch -d worktree-ws-latency` (safe: merged).
  2. `git branch -d worktree-agent-a728a435bb5693108` (merged, no worktree).
  3. For `select-layout-presets`: check `git log main..worktree-select-layout-presets --oneline` — if it holds unmerged commits, DO NOT delete; ask Paul (or check the kanban board/mesh for the owning session) and record the answer on the card. If Paul says abandon: `git worktree remove --force` + `git branch -D`.
  4. Verify `git worktree list` shows only the main checkout (plus any still-owned trees from step 3).
- **Method**: The shared-target stale-binary risk (project memory) makes stale worktrees more than clutter: a `cargo` build in one clobbers binaries another checkout's gate is running. Deletion of merged worktrees is safe; the unmerged one is a scope question, not a hygiene one.
- **Verify**: `git worktree list`; `make checkall` unaffected (no source change).

### [QA-010] install.sh / check_mux_docs.py complexity
- **Files**: `install.sh:298` (`main`, cyclomatic 40), `scripts/check_mux_docs.py:496` (`collect_problems`, 36)
- **Steps**: opportunistic only — next time either file changes for other reasons, split `main` into phase functions (`detect`, `fetch`, `verify`, `install`) and `collect_problems` into per-check helpers. No standalone change now.
- **Method**: Both are low-churn; a dedicated refactor commit adds review load for no behavior gain.
- **Verify**: `bash -n install.sh`; `python3 scripts/check_mux_docs.py` exit 0 (its gate leg already runs in checkall).

### [QA-011] Tab strip loses tab-width bounds on width change — ✅ FIXED PRE-RUN 2026-11-06 (commit f9a3d1e9 on main; card closed 3/3). Fix agents: SKIP this entry.
- **Files**: `crates/par-mux/src/mux/attach/render/session.rs:1453-1459` (`draw_tab_strip`), regression test near the existing tab tests (`crates/par-mux/src/mux/attach/tabs.rs` tests or `render/tests/`)
- **Steps**:
  1. In `draw_tab_strip`'s rebuild branch, carry `bounds` across exactly like `palette`:
     ```rust
     if self.chrome.tab_strip.cols() != cols {
         let palette = self.chrome.tab_strip.palette;
         let bounds = self.chrome.tab_strip.bounds;
         self.chrome.tab_strip = TabStrip::new(cols);
         self.chrome.tab_strip.palette = palette;
         self.chrome.tab_strip.bounds = bounds;
     }
     ```
  2. Regression test: construct a session/strip, set `bounds = TabBounds { min: 12, max: 30 }`, call `draw_tab_strip` (or the resize path that reaches it) with a different `host_cols`, assert `tab_strip.bounds` still `{12, 30}`. Use the `spawn_attach_with_config` harness (memory: hand-rolled PTY spawns hang in the host probe) or a unit-level test on the strip/session if the function is testable in isolation — `draw_tab_strip` needs the full session; prefer adding the assertion to an existing resize test in `render/tests/` if one drives `resize_to`.
  3. While there, confirm the session-strip rebuild (`draw_session_strip`, session.rs:1413-1419) needs nothing — it already restores `plus = false`; its bounds stay default by design (session strip has no width bounds config).
- **Method**: `TabStrip::new` defaults `bounds: TabBounds::default()` (0/0); the rebuild preserved `palette` but missed `bounds`. Daemon restarts guarantee a cols transition during the re-land (that's the user's reliable repro); any resize triggers it too. Config file is unaffected (verified live: file held `tab-width-min = 12` while the TUI showed 0). `scroll: None` reset on rebuild is CORRECT (offsets are width-relative) — do not "fix" it.
- **Verify**: `cargo test -p par-mux --features mux-bin,attach tab` (attach filter; mux-bin alone false-greens) + `make checkall`; manual repro: set min width, restart daemon, confirm the floor holds.

## Phase 3d — Documentation

> All DOC entries: run `PATH="$PWD/.venv/bin:$PATH" make checkall` after the batch — the doc gates
> (api-surface, stub-docs, doc-links, mux-docs, pinned-tests, caps-table) re-verify the edited surfaces
> mechanically. Sequence within 3d: DOC-005/010/011 first, then DOC-015's ARCHITECTURE.md half, then
> DOC-012 (stamp) last. `docs/API_REFERENCE.md` carries nine issues — one agent, sequential edits.

### [DOC-001] delete_image/get_image_by_id id type
- **Files**: `docs/API_REFERENCE.md:705,708`
- **Steps**: change both signatures from `id: int` to `id: str`; add one sentence: programmatically added `InlineImage`s carry `id=None` until the terminal assigns one — call `get_images()` to discover ids.
- **Method**: source of truth `src/python_bindings/terminal/image_api.rs:129,161` (`id: &str`); runtime-verified TypeError on int.
- **Verify**: `make checkall` (api-surface + stub gates); `uv run pytest tests/test_terminal.py -k image` still green (no source change).

### [DOC-002] Badge example shadowed builtins
- **Files**: `docs/API_REFERENCE.md:199-205`
- **Steps**: rewrite the example using non-builtin names (e.g., `env`, `host` — anything not in the builtin set at `crates/par-term-emu-core/src/badge.rs:128-138`: username, hostname, profile_name, …); show real expected output; add one sentence: built-in variable names are resolved from live terminal state and cannot be overridden via `set_badge_session_variable`.
- **Method**: runtime-verified the current example returns `"@"`; the docs currently teach a no-op pattern.
- **Verify**: `make checkall`; run the rewritten example in a venv python to confirm its printed output matches the doc.

### [DOC-003] Screenshot formats claim HTML
- **Files**: `docs/API_REFERENCE.md:471`
- **Steps**: drop HTML from the Supported Formats list; add: HTML export is a separate API (`export_html()`), not a screenshot format. Truth: `ImageFormat` Png/Jpeg/Svg/Bmp (`screenshot/mod.rs:63-66`).
- **Verify**: `make checkall`.

### [DOC-004] Docstring compliance lift
- **Files**: `src/python_bindings/streaming.rs:880,1035,1105` (+ neighboring server methods), `src/python_bindings/types/recording.rs:365,388`, `src/python_bindings/terminal/mod.rs:841`
- **Steps**: for each listed method add Google-style `Args:`/`Returns:` (and `Example:` where cheap) following `input_api.rs:12-49` as the model. Prioritize the three streaming.rs server methods (`start`, `send_input`, `send_title`) and their neighbors; recording.rs and terminal/mod.rs one-liners after.
- **Method**: mechanically verifiable one-liners; keep claims runtime-true (copy parameter names/signatures from `_native.pyi`, which is generated and exact). **Conflict file**: `streaming.rs` also carries QA-008's repr macro — whoever runs second reads current state.
- **Verify**: `make dev && make stubs` regenerates `_native.pyi` docstrings; `make checkall` (stub-docs gate); spot-check `python3 -c "import par_term_emu_core_rust as m; help(m._native.StreamingServer.start)"`.

### [DOC-005] ARCHITECTURE.md mux inventory resync
- **Files**: `docs/ARCHITECTURE.md:296-316`
- **Steps**: re-sync the "Multiplexer Daemon" module list against `ls crates/par-mux/src/mux/` — add `metrics.rs` + `metrics/`, `plugins.rs`, `popup.rs`, `theme.rs`, `update.rs`, `worktree.rs`, `git_status.rs`, `machine_config.rs`, `events.rs`, `pane_stream.rs`, `process_cpu.rs`, `shutdown_inhibit.rs`, `test_wait.rs` with one-line purposes (pull from each file's module doc). Alternative: convert the list to the runnable-command pattern (`ls crates/par-mux/src/mux/`) the doc already uses elsewhere — pick whichever matches the surrounding sections' style.
- **Method**: no enumeration gate covers this list; the fix is a resync plus (optionally) a note that the authoritative source is the directory.
- **Verify**: `make checkall`; `ls crates/par-mux/src/mux/ | wc -l` vs the list length recorded in the PR.

### [DOC-006] DESIGN.md feature table rows
- **Files**: `crates/par-term-emu-core/DESIGN.md:86-96`
- **Steps**: add rows `self-update → par-term-emu-core/self-update` and `otel → streaming, par-term-emu-core/otel, dep:opentelemetry` (from root `Cargo.toml`), plus a note that par-mux's own `otel` feature forwards to the core member (ARC-001 shared-layer decision). Alternatively stamp the table "as of phase 2" with a cutoff note — adding the rows is better.
- **Verify**: `make checkall` (pinned-tests-check reads DESIGN.md citations — ensure no new citation-looking strings).

### [DOC-007] CONFIG_REFERENCE.md discoverability
- **Files**: `docs/CONFIG_REFERENCE.md:1-5` (Overview), `:368-379`
- **Steps**: Overview gains one sentence: par-mux's `config.toml` reference (all `[daemon]` and `[client]` keys) lives in `docs/MUX.md` under "Configuration file". Lift the existing pointer paragraphs into a TOC'd section ("Where settings live").
- **Verify**: `make checkall` (doc-links gate).

### [DOC-008] Web README WS port
- **Files**: `web-terminal-frontend/README.md:91`
- **Steps**: `ws://127.0.0.1:8080` → `ws://localhost:8099/ws` (the actual default in `app/page.tsx:98`, dev and prod).
- **Verify**: `make checkall`; the `web_term` drift gate if it watches the README (it watches `web_term/` build output — README-only change needs `make web-build-static` only if web_term/ is regenerated).

### [DOC-009] MUX.md flag pointer
- **Files**: `docs/MUX.md:149-165`
- **Steps**: after the quick-reference block, add: "Full flag list: `par-mux --help` (44 flags); the sections below document the rest." No need to enumerate the 18 missing flags inline.
- **Verify**: `make checkall` (mux-docs gate).

### [DOC-010] ARCHITECTURE.md attach/streaming sublists
- **Files**: `docs/ARCHITECTURE.md:307,332-354`
- **Steps**: add `picker.rs`, `settings_write.rs`, `sound.rs`, `text_input.rs` to the attach list; `tree_watcher.rs`, `watcher_common.rs`, and `mux_factory/` contents (`lifecycle.rs`, `stream.rs`, `mirror.rs`, `control.rs`, `upload.rs`) to the streaming list — or mark both sublists non-exhaustive if the doc style prefers brevity.
- **Verify**: `make checkall`.

### [DOC-011] terminal/ list omits export_snapshot.rs
- **Files**: `docs/ARCHITECTURE.md:214-246`
- **Steps**: add one line for `export_snapshot.rs` (purpose from its module doc).
- **Verify**: `make checkall`.

### [DOC-012] Verification stamp refresh (RUN LAST in 3d)
- **Files**: `docs/ARCHITECTURE.md:5`
- **Steps**: after DOC-005/010/011 (+DOC-015's half) land in the same phase agent, update "Last verified against" to `v0.79.0 (2026-11-06)`.
- **Method**: one stamp, once, after the content it scopes is true.
- **Verify**: `make checkall`.

### [DOC-013] DESIGN.md phase-2 formulas qualifier
- **Files**: `crates/par-term-emu-core/DESIGN.md:297-306`
- **Steps**: prefix the feature-formula block with "As of the phase-2 split:" or annotate `mux-bin = mux + clap` with "(since widened — see Cargo.toml)".
- **Verify**: `make checkall`.

### [DOC-014] VT_SEQUENCES.md CSI u wording
- **Files**: `docs/VT_SEQUENCES.md:50`
- **Steps**: reword to match line 285: DECSMBV is `CSI Ps SP u` (space intermediate); bare `CSI Ps u` is not the margin-bell volume sequence.
- **Verify**: `make checkall`.

### [DOC-015] graphics_store placement
- **Files**: `docs/API_REFERENCE.md:437-438` (remove), `docs/ARCHITECTURE.md` (add — same edit as DOC-005's region)
- **Steps**: delete the two "(Rust API only)" rows from the Python Terminal class table; add a line in ARCHITECTURE.md's terminal-core section noting `graphics_store()`/`graphics_store_mut()` are Rust-only accessors.
- **Verify**: `make checkall` (api-surface gate must not complain about removal — it counts methods, not doc rows).

### [DOC-016] char_width module-level entry
- **Files**: `docs/API_REFERENCE.md:917,919-924`
- **Steps**: add `char_width(c, config=None)` to the module-level function list; fix the instance-method entry at :917 to show the `config` param.
- **Verify**: `make checkall`.

### [DOC-017] TerminalClipboard section
- **Files**: `docs/API_REFERENCE.md:284`
- **Steps**: add a `### TerminalClipboard` heading consistent with the other 66 classes (prose already accurate).
- **Verify**: `make checkall`.

### [DOC-018] Zones example off-by-one
- **Files**: `docs/API_REFERENCE.md:628`
- **Steps**: `abs_row_end: 1` → `abs_row_end: 2` (runtime-verified).
- **Verify**: `make checkall`; rerun the example snippet in the venv.

### [DOC-019] poll_events fold
- **Files**: `docs/API_REFERENCE.md:579-582`
- **Steps**: merge the three "Now also returns…" sentences into one entry listing the full return set.
- **Verify**: `make checkall`.

### [DOC-020] Web README framework versions
- **Files**: `web-terminal-frontend/README.md:7,128`
- **Steps**: per the style guide ("avoid duplicating dependency versions"), drop the concrete versions rather than updating them; if a mention is needed, say "Next.js (App Router) + React" generically.
- **Verify**: `make checkall`.

### [DOC-021] doc-path waivers
- **Files**: `docs/MUX.md:638,1435`, `docs/DOCUMENTATION_STYLE_GUIDE.md:197,210,212`
- **Steps**: add `<!-- doc-path: runtime -->` waiver comments (check the exact waiver spelling the doc-links gate expects — see existing waived paths in other docs) for `%APPDATA%/par-mux/config.toml`, `plugins/installed.json`, and the style guide's example paths.
- **Verify**: `make checkall` (doc-links gate stops flagging them).

### [DOC-022] pub(crate) doc gaps
- **Files**: `src/streaming/session.rs:321-375`, `crates/par-mux/src/mux/server/broadcast.rs:333`, `crates/par-term-emu-core/src/terminal/sequences/csi/cursor.rs:130`
- **Steps**: add `///` doc comments on the viewer-lifecycle methods, `broadcast_layout_change`, and `handle_csi_cursor` (one to three lines each: contract + caller).
- **Method**: internals only; keep comments to constraints, not narration. **Conflict file**: broadcast.rs also carries the ARC-005 watch (no-action) — only this entry edits it.
- **Verify**: `make checkall` (missing_docs lint is not gate-clean repo-wide — these are `pub(crate)`, so only compile+test legs matter).
