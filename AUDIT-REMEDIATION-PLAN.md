# Audit Remediation Plan — par-term-emu-core-rust

> **Companion to**: `AUDIT.md` (2026-09-26 /opus-audit cycle, HEAD 17060c6, v0.52.0)
> **Consumer**: `/fix-audit` — each entry is executable without re-deriving the analysis.
> Entries are ordered to match AUDIT.md's `## Remediation Plan` phases. IDs continue prior
> cycles (SEC-101+, ARC-021+, QA-110+, DOC-021+; QA-122 was merged into ARC-029 and is skipped).
>
> **Global verify gate** for every entry: `make checkall` (clippy + fmt + ruff + pyright + full
> Rust + Python suites) unless the entry narrows it. Run gates from the repo root. The repo is
> trunk-based with concurrent sessions — re-read any file before editing if another session may
> have touched it (R6/R9).

---

## Phase 2 — Blocking Architecture

### [ARC-021] Terminal god object — phase-1 non-breaking slice
- **Files**: `src/terminal/mod.rs` (screenshot fns at :2687-2761), `src/screenshot/`, `src/lib.rs` (feature wiring), `python/par_term_emu_core_rust/_native.pyi` (regenerate), `docs/API_REFERENCE.md` (only if signatures change — they must not)
- **Steps**:
  1. Move the body of `Terminal::screenshot`/`screenshot_to_file` into `src/screenshot/` as free functions over the existing `render_grid(&grid, cursor, graphics, config)` entry point. Keep `Terminal::screenshot` as a deprecated-forwarding wrapper (same signature, no Python-visible change). Regenerate stubs (`make stubs`) if docstrings surface differently; signatures unchanged means no `.pyi` delta beyond docstrings.
  2. Introduce `#[cfg(feature = "sim-slim")]`-style gating: add a `screenshot` feature enabled by `python`, `full`, and by default for `sim` today; document in `Cargo.toml` comments that `sim` can drop it. Do NOT change what `sim` enables by default in this slice — only make the gate exist and record the option (changing `sim` defaults is a release-note item).
  3. Extract ONE non-VT feature area as the pattern proof: `Benchmarks` (least coupled) or `ComplianceRunner` — move its methods to a service struct/extension trait taking `&Terminal`, keep deprecated forwarding methods on `Terminal` marked `#[doc(hidden)]` + `#[deprecated(since = "0.53.0", note = "...")]`. Rust callers keep compiling; Python surface unchanged.
  4. Verify sister repos compile against this checkout: `cargo check` in `../par-term` pointing at the path dependency (or `cargo publish --dry-run` level check via a patch section) and build `../par-term-emu-tui-rust`'s dependency with `uv run`/`make dev` there if feasible. At minimum run this repo's full suite.
- **Method**: This entry deliberately scopes /fix-audit to the non-breaking slice; the full multi-release service extraction is tracked on the board card. Pitfalls: (a) do not remove any public method in this slice — par-term references `terminal` at 77+ sites; (b) `multiple-pymethods` is enabled, so new trait-based methods are NOT auto-exposed to Python — that is what keeps the Python surface stable; (c) the `sim` feature rejects combining with `python` via `compile_error!` (`lib.rs:51`) — keep that intact. Use parsight `get_impact` on `Terminal::screenshot` and the extracted feature's methods before moving.
- **Verify**: `make checkall`; `cargo check --no-default-features --features rust-only,sim`; `cargo check --no-default-features --features rust-only,sim --no-default-features --features rust-only` both resolve; deprecated methods emit warnings but compile; Python suite green (surface unchanged).

### [ARC-022] Mux spawns PTYs under the global tree mutex (+ ARC-032, QA-115 — one batch)
- **Files**: `src/mux/tree.rs:241,308,378` (spawn call sites inside `new_session_with_env`/`new_window_with_cwd`/`split_pane_in_window`), `src/mux/dispatch.rs:229,502,663,219`, `src/mux/pane.rs:213-234` (`persisted_snapshot`), `src/mux/server.rs:842-852` (`reap_dead_panes`), `tests/mux_daemon.rs`
- **Steps**:
  1. **QA-115 first (smallest)**: in `reap_dead_panes`, take ONE `tree.lock()` guard across window lookup + last-pane check + `kill_window`/`kill_pane`; drop the guard before `broadcast_layout_change`. Delete the misleading "not reentrant" doc-comment line; keep the broadcast relock note.
  2. **ARC-032**: change `to_persist_state()` usage in `dispatch.rs:219` — under the tree lock, collect `Arc<RwLock<Terminal>>` handles (and pane metadata), release the lock, then call `persisted_snapshot` per pane outside it. Keep the generation+size cache for the (now lock-free) snapshot path; move the scrollback capping (`persist.rs:517`) to before any expensive clone is retained.
  3. **ARC-022**: add a two-phase tree API: `reserve_pane(...) -> Reservation` (validates geometry, allocates `PaneId` under the lock) and `commit_pane(reservation, pane)` / `Reservation::abort()`. In the three `cmd_*` handlers: reserve under lock → drop lock → `factory.create_pane` → re-lock, commit, wire `on_output`, recompute layout, set `mutated`, broadcast; on spawn error, abort the reservation and reply with the existing error shape. Update `MuxTree` callers via parsight `analyze_relationships` on `split_pane_in_window`/`new_window_with_cwd`/`new_session_with_env` to catch every caller.
  4. Add the regression test in `tests/mux_daemon.rs`: a `PaneFactory` test double sleeping ~300 ms in `create_pane`; start daemon; issue `split-window` and, concurrently, `list-panes`; assert `list-panes` replies before the spawn completes.
  5. Run the Windows VM mux suite per the CLAUDE.md playbook (`cargo test --lib --no-default-features --features rust-only,mux,serde mux:: -- --test-threads=1` plus `tests/mux_daemon.rs`) — ConPTY spawn timing is where the lock scope matters most. This requires running from the main checkout (worktree sessions cannot `prlctl exec`).
- **Method**: parking_lot is not reentrant — do not "simplify" by holding one guard across broadcast (deadlock). The persist path is correct-but-slow today (atomic tmp+fsync+rename, 0600); preserve D3.3's clean-shutdown guarantee — the shutdown save stays synchronous. The `on_output` wiring MUST happen on commit, not reservation, or split panes render blank again (the ARC-002-era bug fixed in 0.52.0).
- **Verify**: `make checkall`; `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` ; `cargo test --test mux_daemon --test mux_reattach --test mux_restart`; Windows VM step 4b; the new sleeping-factory test.

---

## Phase 3a — Security (remaining)

### [SEC-101] Kitty t=t arbitrary file deletion — gate file media
- **Files**: `src/graphics/kitty.rs:906-910,923-982` (`decode_payload`, `load_file_data`), `src/terminal/mod.rs:1456` (setter block beside `set_retain_kitty_temp_files`) and `:2876` (dispatch), `src/python_bindings/terminal/` (new binding), `src/streaming/` (config field, optional), `src/mux/pane.rs:458`, `tests/` (new Rust test file or `tests/test_graphics.py`)
- **Steps**:
  1. Add terminal state: `allow_file_media: FileMediaMode` enum `{ Off, TempOnly, All }`, default `TempOnly`; setter `set_allow_file_media(mode)` + getter beside the existing kitty setters; wire into `build_graphic`/`decode_payload` dispatch at `mod.rs:2876`.
  2. In `load_file_data`: for `t=t`, canonicalize the path; require `canonical starts_with one of [std::env::temp_dir(), /tmp, /dev/shm]` AND `file_name contains "tty-graphics-protocol"`; otherwise return the existing error without touching the file. For `t=f`, require mode `All` (still reject `..`).
  3. Replace the exists/is_file/metadata/read sequence with one `OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open()` (Windows: `FILE_FLAG_OPEN_REPARSE_POINT` equivalent or accept TOCTOU there with a comment), then `fstat` the handle for regular-file + size cap, then read from the handle. This closes SEC-103 in the same edit.
  4. Delete only after successful decode: move the `remove_file` from `:977-979` to after `decode_payload` returns Ok.
  5. Python binding for the new setter/getter (Google-style docstring with Args/Returns/Example — the repo contract). Streaming: add the field to the session config if a natural extension point exists (`ConnectedBuilder` pattern); otherwise leave streaming defaulting to the terminal's default and note it in docs.
  6. `src/mux/pane.rs:458`: keep `retain_temp_files` (deletion deferred to clients) — the gate now lives in the *client's* terminal state; add one line to the MUX.md-facing doc comment.
  7. Tests: (a) `t=t` naming an absolute path outside temp roots → error, file still exists; (b) `t=t` temp file with correct name but non-image bytes → error, file still exists; (c) `t=t` valid temp image with `TempOnly` → loads and file deleted; (d) mode `Off` refuses both media; (e) `..` still rejected. Rust-side tests preferred (no PTY needed — `Terminal::process` the APC bytes directly).
- **Method**: This is the audit's only empirically-reproduced security defect. Kitty's own rule (temp dir + `tty-graphics-protocol` in filename) is the compatibility anchor — legit tools name temp files this way. Keep error strings in the existing style so the TUI/frontends don't need changes. Do not widen to Sixel/iTerm2 file loading in this entry (they don't take paths today — verify with `find_code` "load file path graphics" before asserting).
- **Verify**: new tests pass (`cargo test --lib --no-default-features --features pyo3/auto-initialize kitty_file` or the chosen test name); targeted Python test if binding added; `make checkall`.

### [SEC-102] Kitty t=f arbitrary file read/display — same gate
- **Files**: `src/graphics/kitty.rs` (same functions as SEC-101)
- **Steps**: Landed by SEC-101 step 2 (`t=f` requires mode `All`). If SEC-101 was implemented without the `t=f` arm, add it here: mode `All` required; otherwise refuse. Consider auto-disabling file media when a streaming server is serving read-only viewers (one check in the session config wiring) — optional, note the choice in the changelog.
- **Method**: Lower severity than SEC-101 only because escape sequences have no read-back channel; the rendered bytes still reach streamed/screenshot output.
- **Verify**: tests from SEC-101 cover `t=f` under each mode; `make checkall`.

### [SEC-103] Check-then-read race in kitty file loading
- **Files**: `src/graphics/kitty.rs:943-971`
- **Steps**: Closed by SEC-101 step 3 (handle-based open + fstat). If reached independently, apply the same O_NOFOLLOW+fstat pattern.
- **Verify**: covered by SEC-101's tests; `make checkall`.

### [SEC-104] Cap mux client line length
- **Files**: `src/mux/server.rs:477-505` (read loop in `handle_client`)
- **Steps**: (1) After the ARC-022 batch lands (this loop may have moved), wrap the `read_line` accumulation with a byte budget (1 MiB): read in chunks or check `line.len()` after each read; on overflow, write an `%error` block (match the existing error emission style used for malformed commands), flush, and close that client connection. (2) Unit test: pipe a >1 MiB line into a client socket; assert the daemon replies with the error block, other clients still work, daemon doesn't grow unbounded (check via a second command).
- **Method**: Same-user-only exposure (peer euid check, `ipc.rs:94-126`), so this is hygiene, not a boundary fix. Don't change the framing protocol — cap only the accumulation.
- **Verify**: `cargo test --test mux_daemon` + new test; `make checkall`.

### [SEC-105] Cap hook-report JSON size
- **Files**: `src/mux/server.rs` (the same read loop / hook-report branch)
- **Steps**: Apply the same budget to the hook-report line (shares the loop); values are free text persisted into pane metadata and the state file, so also clamp individual value lengths at the existing validation point in `src/mux/hooks.rs` (e.g. keep the current reject-invalid behavior, add a max length consistent with what `persist.rs` stores).
- **Verify**: unit test with an oversized hook report; `make checkall`.

### [SEC-106] Track `paste` 1.0.15 unmaintained (RUSTSEC-2024-0436)
- **Files**: `Cargo.lock` (now committed per ARC-024), `Cargo.toml` (only if replacing)
- **Steps**: (1) After ARC-024 commits the lockfile, run `cargo audit` and record the advisory in the tracking place this repo uses (board watch item or `docs/SECURITY.md` dependency section). (2) Identify the transitive path (`cargo tree -i paste@1.0.15`) and note the direct dep responsible. (3) No code change unless a maintained `paste` successor is trivially available; revisit on the next dependency sweep.
- **Verify**: `cargo audit` output recorded; lockfile pinned so results are reproducible.

### [SEC-107] SECURITY.md: document the kitty file-medium risk + new gate
- **Files**: `docs/SECURITY.md` ("Graphics Protocol File Loading Security")
- **Steps**: Rewrite the section after SEC-101/102 land: state that file media (`t=f`/`t=t`) are driven by terminal output (untrusted), describe the new `FileMediaMode` default (`TempOnly`) and the temp-dir+filename rule, and the decode-before-delete order. Fold into DOC-022's SECURITY.md pass if that runs after SEC fixes (it does — see blockers).
- **Verify**: manual read-through against the implemented code paths; links resolve (`make -C docs` n/a — check relative links by hand or with the doc agent's script approach).

---

## Phase 3b — Architecture (remaining)

### [ARC-023] CI triggers
- **Files**: `.github/workflows/ci.yml:3-4`
- **Steps**: (1) Add to `ci.yml`: `on: push: branches: [main]` + `pull_request:` with `paths-ignore: ["**.md", "docs/**"]`. (2) To control cost, split a cheap `quick` job (ubuntu: fmt check, clippy `--all-targets --no-default-features --features pyo3/auto-initialize`, `cargo test --lib --no-default-features --features pyo3/auto-initialize`, the `sim` cargo-tree guard) triggered on push/PR, and keep the full 3-OS×3-Python matrix on `workflow_dispatch` + the new push trigger only for `release/**` branches if desired. (3) If the maintainers prefer dispatch-only, instead document that decision in `CONTRIBUTING.md` — but the cheap job is the recommendation.
- **Method**: trunk-based repo with concurrent sessions; the `sim` tree guard and Windows-only compile errors are exactly what `make checkall` on macOS cannot catch. Concurrency: add `concurrency: group: ci-${{ github.ref }}, cancel-in-progress: true`.
- **Verify**: push a docs-only commit (no run) and a code commit (quick job runs green) on a branch; inspect the Actions tab. Note: this repo's CI workflows are dispatch-only by history — surface the change in the commit message.

### [ARC-024] Commit Cargo.lock + --locked
- **Files**: `.gitignore`, `Cargo.lock` (new), `.github/workflows/ci.yml`, `deployment.yml`, `release.yml`, `publish-*.yml` (any cargo build/test invocations), `CLAUDE.md` (Windows playbook step 4 note says "no --locked — Cargo.lock is not tracked" → update)
- **Steps**: (1) Remove `Cargo.lock` from `.gitignore`. (2) `cargo generate-lockfile` (or use the existing local one) and commit. (3) Add `--locked` to every CI/release `cargo build|test|check` invocation. (4) Update the CLAUDE.md Windows-VM step 4 comment (self-improvement rule: the now-false note gets fixed in the same change).
- **Method**: Cargo ignores dependency lockfiles of library dependencies, so crate consumers are unaffected. Binaries (`par-term-streamer`, `par-mux`) are the reason the lockfile belongs in git.
- **Verify**: `cargo build --locked --offline` succeeds in CI config dry-run; `git status` shows Cargo.lock tracked; CLAUDE.md note updated.

### [ARC-025] Native HAS_STREAMING constant
- **Files**: `src/lib.rs:277-278,334-341` (`register_constants`), `src/python_bindings/streaming.rs` (only if a new registration fn is cleaner), `python/par_term_emu_core_rust/__init__.py:75-95,166-177`
- **Steps**: (1) In `register_constants` (or the constants module it delegates to), add `HAS_STREAMING = cfg!(feature = "streaming")` exposed as a module constant. (2) In `__init__.py`, set `_has_streaming = _native.HAS_STREAMING` and keep the stub-class exports gated on it (`__all__` computed accordingly) so `import` never fails but construction is honestly detectable. (3) Update `docs/API_REFERENCE.md` streaming intro + `docs/STREAMING.md` capability note. (4) Changelog entry under "Changed".
- **Method**: Non-breaking by design. Do NOT remove the stub classes in this entry (breaking; needs the major-version window).
- **Verify**: `make dev && uv run python -c "import par_term_emu_core_rust as m; print(m._has_streaming)"` — dev build (with streaming) prints True; check the non-streaming path via `cargo test` on the constants or by building `--no-default-features --features python` locally if feasible; `make checkall`.

### [ARC-026] Layering moves
- **Files**: `src/terminal/replay_snapshot.rs` → `src/grid/snapshot.rs` (+ re-export), `src/graphics/mod.rs:399,433` + new `src/time.rs`, `src/streaming/py_convert.rs` → `src/python_bindings/streaming_convert.rs`, `src/streaming/protocol.rs:230-280`, `src/streaming/mux_factory.rs:22`, `src/mux/client.rs`, `src/grid/mod.rs:207,224`, `src/lib.rs`
- **Steps** (each sub-step is its own commit):
  1. `GridSnapshot`: `git mv src/terminal/replay_snapshot.rs src/grid/snapshot.rs`; in `src/terminal/replay_snapshot.rs` (keep the module) `pub use crate::grid::snapshot::*;` so every existing path (`terminal::replay_snapshot::GridSnapshot`, serde format untouched) keeps working. Fix `grid/mod.rs:207,224` to the new local path. Run the mux serde round-trip suite.
  2. `unix_millis`: move to `src/time.rs` (`pub(crate) fn unix_millis() -> u64`), `pub(crate) use` from `terminal` for compat; update `graphics/mod.rs:399,433`.
  3. `py_convert.rs`: `git mv` to `src/python_bindings/streaming_convert.rs`; update `lib.rs` module wiring so the `python`-gated pyo3 module lives under python_bindings; streaming keeps pure-Rust types only. This removes one reason `lib.rs:80` must compile streaming under `python` — if after the move the cfg wiring can decouple them, do it; if not, leave the cfg and note why.
  4. `pydict` attributes: inventory the 35 `#[cfg_attr(... pydict(...))]` sites in `protocol.rs`; move conversion to the binding side (a `to_py_dict` impl in `streaming_convert.rs` or the existing derive pointed at a binding-side sidecar). If the derive macro lives in `derive/` and supports a path argument, prefer that. This sub-step is the largest; timebox it and split from 1-3 if the diff exceeds ~400 lines.
  5. `MuxClient` ordered-stream mode: add `MuxClient::connect_stream()` returning a single mpsc of an `enum MuxEvent { Reply(..), Notification(..) }` preserving wire order; switch `mux_factory.rs` to it; delete the doc comment explaining why internals were reached into.
- **Method**: parsight `get_impact` on `GridSnapshot` and `py_convert` functions before each move. The serde JSON on disk must not change — the move is module-path only. Sister repos import `terminal::*` paths: keep every re-export. Do sub-steps 1-2 + 5 first (mechanical), 3-4 can follow.
- **Verify**: per sub-step: `make checkall`; after 1: `cargo test --no-default-features --features rust-only,mux,serde mux::`; after 4: `uv run pytest tests/test_streaming.py`; after 5: `cargo test --test mux_daemon`.

### [ARC-027] Observer-push event delivery in the streaming binary
- **Files**: `src/bin/streaming_server/bootstrap.rs:38,69,96,217-240,315,552-578`
- **Steps**: (1) Register a `TerminalObserver` per session (`Terminal::add_observer`, `src/observer.rs`) whose callback maps via the existing `terminal_event_to_server_message(event)` and sends into an `tokio::sync::mpsc`. (2) The session's broadcast task drains the mpsc instead of the 50 ms `interval` loops; delete both copies (`:217-240`, `:552-578`). (3) Replace `Arc<Mutex<PtySession>>` with `Arc<PtySession>`; for the `&mut self` operations (resize, restart) use the existing interior-mutability surface or a dedicated `Mutex` only around those — check `PtySession`'s API first (`resize` may already be `&self` via atomics/locks internally; if not, smallest change: keep `Arc<Mutex<PtySession>>` but only for the mutation paths and drop it from the event path). (4) Update `tests/test_streaming.rs` for event-latency assertions if any rely on polling.
- **Method**: The observer dispatches after `process()` with no locks held — this removes both the 50 ms latency and the idle write-lock contention with the PTY reader thread. The two loops are near-copies; deleting both closes the drift class.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming` ; `cargo test --test test_streaming` ; manual `make streamer-run` smoke: output appears without the 50 ms tick (type and watch latency), `make checkall`.

### [ARC-028] WsTransport unification + server.rs split (after QA-110, QA-114)
- **Files**: `src/streaming/server.rs:844-961,963-1115,1240,1653-1795,1947-2133` → new `src/streaming/{accept.rs,session_loop.rs,http.rs}` + slim `server.rs`
- **Steps**: (1) Define `trait WsTransport { async fn send(&mut self, msg: ServerMessage) -> Result<..>; async fn recv(&mut self) -> Result<Option<ClientMessage>>; async fn close(self); }` (adjacency: reuse the existing `Client<S>` codec). Implement for the tungstenite-backed `Client<S>` and for the axum socket pair. (2) Extract `run_session<T: WsTransport>` containing connect message, mode-sync prelude, subscription set, keepalive timer, rate limiter, and the `select!` body calling the (post-QA-114) per-variant handlers. Delete `run_ws_session`/`handle_axum_websocket` bodies in favor of it. (3) Collapse `start_websocket_only`/`start_websocket_only_tls` over an `Acceptor` enum (Identity | Tls(TlsAcceptor)). (4) Move auth middleware (`http_basic_auth_middleware` ~:2240), origin checks, CORS/security headers into `http.rs`; accept logic into `accept.rs`; the session loop into `session_loop.rs`; keep `server.rs` as the facade re-exporting the public surface (crate-internal — public API is `StreamingServer`, unchanged). (5) Add an axum-route integration test in `tests/test_ws_smoke.rs` sending `Mouse` and asserting the PTY received the encoded sequence (the ARC-001-era gap).
- **Method**: strictly after QA-110 (writer queue) and QA-114 (per-variant handlers) — this refactor reshapes the code they rewrite. Exhaustiveness: keep NO wildcard arm on `ClientMessage` (existing ARC-001 property).
- **Verify**: `cargo test --features streaming` ; `cargo test --test test_ws_smoke` (incl. the new Mouse test); `make checkall`; grep that `ClientMessage` matches remain non-wildcarded.

### [ARC-029] debug_*! on the log facade (absorbs QA-122)
- **Files**: `src/debug.rs` (macros `:136-164` region), all 86 `debug_*!` macro sites (crate-wide, mechanical), `src/mux/*` (26 `log::` sites — leave, already `log`), `src/grid/scroll.rs:152,198` (`eprintln!` → `log::debug!`), `src/bin/streaming_server` (already `tracing` — bridge via `tracing`'s log feature or leave), `Cargo.toml` if a `log` feature flag is needed
- **Steps**: (1) Re-implement `debug_error!`/`debug_warn!`/`debug_info!`/`debug_trace!` as thin wrappers over `log::error!` etc. with `target = "pter::debug"` (or per-category targets), preserving the existing gating semantics (no-op unless the env-var level enables them AND a sink is installed). (2) Keep `debug.rs`'s file sink as an opt-in `log::Log` implementation installed only by the Python module init and the two binaries. (3) Land after QA-112 (which fixes the hot path); QA-112's AtomicU8 fast-path check stays as the first gate so the log-facade call overhead isn't paid when disabled. (4) Replace the two `eprintln!` in `grid/scroll.rs` (still behind `#[cfg(debug_assertions)]`? — prefer removing the cfg gate and relying on level filtering). (5) Changelog "Changed": embedders can now route core diagnostics via any `log` backend.
- **Method**: do NOT delete the env-var/file-sink behavior — Python users and the Windows debugging playbook rely on it. The macros must keep lazy `format_args!` semantics from QA-112.
- **Verify**: `make checkall`; `uv run python -c` smoke with `DEBUG_LEVEL=trace` writing the temp file as before; embedder path: `cargo test --no-default-features --features rust-only,sim` still compiles (no log backend installed, macros no-op).

### [ARC-030] FFI behind an opt-in feature
- **Files**: `Cargo.toml:51` (add `ffi = []`), `src/ffi.rs:1` (`#![cfg]`-style gating or `#[cfg(feature = "ffi")]` on the module in `lib.rs`), `docs/FFI_GUIDE.md` (mark experimental), `src/terminal/tests/ffi_tests.rs` (gate)
- **Steps**: (1) Add `ffi` to `[features]` (not in any default or aggregate). (2) Gate the `ffi` module declaration and `ffi_tests` behind it. (3) FFI_GUIDE.md: add "Experimental, opt-in; not part of the stable ABI" and the enabling instructions. Option (b) (completing the surface with lifecycle fns + cbindgen) is deliberately NOT in this entry — record it as future work on the doc page.
- **Verify**: `cargo check` default (ffi symbols absent from the cdylib: `nm` grep optional); `cargo test --features ffi` runs ffi_tests; `make checkall`.

### [ARC-031] Deprecate dead Broadcaster
- **Files**: `src/streaming/broadcaster.rs`, `src/streaming/mod.rs:82`
- **Steps**: (1) `#[deprecated(since = "0.53.0", note = "unused; sessions broadcast via StreamSessionState; removal in the next breaking release")]` on the type + its public impl items. (2) `#[allow(deprecated)]` on its own tests. (3) Changelog "Deprecated".
- **Verify**: `make checkall` (no new warnings outside the marked items); `cargo test -p` broadcaster tests still pass.

### [ARC-033] checkall uses check-only lints
- **Files**: `Makefile:303` (and the `lint`/`lint-python` targets it references)
- **Steps**: `checkall` should depend on check-only forms: `cargo clippy --all-targets -- -D warnings` (no `--fix`), `cargo fmt --check` (or `-- --check` per edition), `ruff format --check`, `ruff check` (no `--fix`), plus the existing test targets. Keep `lint` as the mutating fixer. Update any CI step that relied on `checkall` auto-fixing.
- **Method**: A gate that rewrites files can pass by editing code — contradicts the surgical-format policy and surprises concurrent sessions.
- **Verify**: introduce a formatting error in a scratch branch → `make checkall` fails without modifying the file; revert.

### [ARC-034] Drop tokio test-util from [dependencies]
- **Files**: `Cargo.toml:88`, (dev-dependency at `:182` already has it)
- **Steps**: Remove `test-util` from the `tokio` entry under `[dependencies]`; `cargo check --features streaming` then `cargo test --features streaming` (the `rate_limit.rs:91-124` cfg(test) blocks use the dev-dep).
- **Verify**: `cargo tree -e features -i tokio` no longer shows test-util for non-test builds; `make checkall`.

### [ARC-035] Gate serde_yaml_ng
- **Files**: `Cargo.toml`, `src/macros.rs:159-172`, `src/lib.rs` (module gating)
- **Steps**: Add feature `macros-yaml` (enabled by `python` and `full`); `#[cfg(feature = "macros-yaml")]` on the YAML save/load paths in `macros.rs` (return a clear error or fall back to JSON-only when disabled — check what the Python layer calls; `python` enables it so Python behavior is unchanged). Alternatively fold into the existing `serde` feature if that's cleaner given `mux` enables `serde`.
- **Verify**: `cargo check --no-default-features --features rust-only,sim` and `--features rust-only,mux` no longer pull `serde_yaml_ng` (`cargo tree -i serde_yaml_ng`); `make checkall` (python path unchanged).

### [ARC-036] macro_export → pub(crate)
- **Files**: `src/python_bindings/common.rs:48-2477` (16 macros)
- **Steps**: Convert each `#[macro_export] macro_rules! impl_terminal_*` to plain `macro_rules!` + `pub(crate) use name;` after definition. Fix any in-crate references that relied on crate-root paths (they mostly call them unqualified — should be unaffected). These macros exist only under the `python` feature; keep the cfgs.
- **Verify**: `make checkall`; `cargo check --no-default-features --features rust-only` unaffected; confirm no `macro_export` remains in the file.

### [ARC-037] Root clutter
- **Files**: repo root (`debug/*.py`, `theme.css`, audit artifacts), new `scripts/debug/` or `docs/audits/2026-09-26/`
- **Steps**: (1) `git mv debug/*.py scripts/debug/` (or delete if they reference stale APIs — inspect first; they're ad-hoc). (2) Decide with the user-facing docs: `theme.css` is referenced by web_term or docs? `grep -r theme.css` before moving; if it's stray, delete (it's user-created content — confirm via git log before deleting). (3) Audit artifacts: this entry runs at the END of /fix-audit's cycle, and DOC-038 owns archiving — coordinate: only move `debug/` and `theme.css` here.
- **Verify**: `git status` clean of root strays; nothing references moved/deleted paths (`grep -rn "debug/" README.md docs/ Makefile`).

### [ARC-038] streaming main/run_mux_mode shared skeleton
- **Files**: `src/bin/streaming_server/main.rs:96-143,167-650`, `src/bin/streaming_server/bootstrap.rs`
- **Steps**: (1) Extract into `bootstrap.rs`: `serve_until_ctrl_c(server)` (signal handling + graceful shutdown) and `print_banner(config, addr, tls)` — QA-114 names these; this entry does the fuller skeleton: `async fn run_server(cfg, factory)` covering build-config → build-server → spawn tasks → await shutdown, parameterized by the `SessionFactory` (the existing trait). (2) `main`/`run_mux_mode` shrink to config parsing + calling the skeleton with their respective factory. (3) Land AFTER QA-114's main() extraction to avoid double-refactor churn; if QA-114 already did the whole skeleton, this entry is done — verify and close.
- **Verify**: `cargo build --features streaming-bin`; run both modes (`make streamer-run` smoke; `par-mux` smoke if a mux mode exists in this binary per its CLI); `make checkall`.

---

## Phase 3c — Code Quality

### [QA-110] Serialized per-session PTY writer — CRITICAL, do first
- **Files**: `src/streaming/session.rs:84` (writer field), `src/streaming/server.rs:1293,1369,1405,1445,1466` (Input/Mouse/FocusChange/(Resize ok)/Paste arms), `tests/test_streaming.rs`
- **Steps**:
  1. Designate one writer per session: add to the session state an `mpsc::Sender<WriteOp>` where `WriteOp` is bytes (+ optionally a completion channel); spawn ONE `tokio::task::spawn_blocking` loop (or `std::thread`) that receives ops and does `writer.lock()` + `write_all` sequentially. On channel close, exit.
  2. Replace the four input arms' direct lock+write with `enqueue_input(&session, bytes)` (fire-and-forget send; on send error, reply/log the existing error shape). Mouse/FocusChange stop touching the writer on the async path entirely — this restores the SEC-005 guarantee.
  3. Fold QA-118: while touching `session.rs:84`, switch the writer lock to `parking_lot::RwLock` (module norm) or keep `std` but justify; the serialization makes the lock internal to the writer loop.
  4. Test in `tests/test_streaming.rs`: connect a client, send N=50 sequential `Input` messages ("a"..), read the PTY side (the test harness has a PTY handle — see existing input tests) and assert the exact sequence arrived in order. Add a Mouse+Input interleave test asserting no corruption.
  5. Check `should_send`/reply paths don't depend on write ordering.
- **Method**: Review against SEC-005 (the security constraint is "PTY writes never block the tokio runtime") and the input rate limiter in the same arms — do not bypass the limiter when enqueuing. `parking_lot::Mutex` has no fairness: that's the reorder mechanism; the channel IS the queue. Backpressure: use a bounded channel (e.g. 1024) and drop/reject with the existing error style when full rather than unbounded growth.
- **Verify**: new ordering tests green; `cargo test --features streaming`; `make checkall`. This unblocks QA-114, QA-119, ARC-028, QA-121.

### [QA-111] Re-enable Python PTY tests
- **Files**: `tests/test_pty.py:10`, `tests/test_pty_resize_sigwinch.py:20`, `tests/test_nested_shell_resize.py:14`, `.github/workflows/ci.yml:101-109`, `Makefile` (new `test-pty` target)
- **Steps**: (1) Delete the three `pytestmark = pytest.mark.skip(...)` lines; register a `pty` marker in `pyproject.toml` (`[tool.pytest.ini_options] markers`) and mark the three modules `-m pty` OR keep files unmarked and rely on explicit invocation. (2) Add `make test-pty`: `uv run pytest tests/test_pty.py tests/test_pty_resize_sigwinch.py tests/test_nested_shell_resize.py -v`. (3) CI: keep `--ignore` for the three files in the default job (they need a TTY-stable runner), but add a dedicated CI job on ubuntu running `make test-pty` (Linux runners support PTYs — the hang was likely the fixed sleeps, not the platform). (4) Replace fixed `time.sleep` in these files with `tests/conftest.py`'s `wait_until` poller (prior-cycle QA-102 helper already exists). (5) Run locally; fix whatever surfaces (these tests have been dark since 2025-11 — expect some rot; fix tests that assert outdated behavior, and file genuine bugs found as new board items rather than editing them away).
- **Method**: The original skip reason ("PTY tests hang in CI") predates the `wait_until` helper. Understand each failing test before changing it (R-rule: the bug may be in the code).
- **Verify**: `make test-pty` green locally; `make checkall`; CI job green on a branch push.

### [QA-112] Debug logger off the hot path — land alone, before other sequences edits
- **Files**: `src/debug.rs:136-164`, `src/terminal/sequences/csi/mod.rs:28-33`, ~34 eager `format!` sites in `src/terminal/**`, `src/grid/**`
- **Steps**: (1) `static LOG_LEVEL: AtomicU8` (map the env-var levels to u8; initialize lazily from `DEBUG_LEVEL` on first read or in the logger constructor). `is_enabled()` reads the atomic with `Relaxed`. The mutex is taken only inside `log()` for the file write. (2) `csi_dispatch_impl`: move the params `Vec<i64>` construction inside the enabled branch. (3) Convert `debug::log(.., &format!(..))` sites to the `debug_*!` macros (lazy `format_args!`); the 12 in `terminal/graphics.rs` first, then the rest (`grep -rn "debug::log" src/terminal src/grid`). (4) Isolated commit — QA-114's style/report splits and ARC-029 depend on this shape.
- **Verify**: `make checkall`; micro-benchmark optional (`cargo bench bench_sgr_heavy` before/after, interleaved runs per the bench methodology memory); confirm `DEBUG_LEVEL=trace` still logs correctly (one manual run).

### [QA-113] send_sigwinch dedup
- **Files**: `src/pty_session.rs` ~`:879`, `:1095`, `:1207`
- **Steps**: (1) Extract `fn send_sigwinch(pid: u32) -> io::Result<()>`: `kill(-pid, SIGWINCH)`, on failure fall back to `kill(pid, SIGWINCH)`, log at debug on fallback. Add the `// SAFETY:` comment (pid is a live child we spawned; SIGWINCH is async-safe). (2) Call from all three sites; delete the divergent copies; `resize_pixels` now logs failures like `resize`. (3) Existing resize tests must stay green (`tests/test_pty_resize_sigwinch.py` — un-darkened by QA-111; if still dark, Rust-side resize tests cover it).
- **Verify**: `make checkall`; `make test-pty` (if QA-111 landed) or `cargo test --lib --no-default-features --features pyo3/auto-initialize resize`.

### [QA-114] Split the dispatch giants (after QA-110; mux part after Phase 2)
- **Files**: `src/streaming/server.rs:1240`, `src/terminal/sequences/csi/style.rs:10`, `src/terminal/sequences/csi/report.rs:7`, `src/bin/streaming_server/main.rs:167`, `src/mux/server.rs:415`, `src/mux/dispatch.rs:138`
- **Steps**:
  1. `handle_client_message` (after QA-110): extract each arm body into `fn handle_input/handle_mouse/handle_focus/handle_paste/...` sharing `enqueue_input(session, bytes)` and a small `ReplyCtx` for the read-only check + metrics + reply pattern. The match stays exhaustive (no wildcard).
  2. `handle_csi_style` (after QA-112): split the `38`/`48` extended-color sub-parser into `fn parse_extended_color(params: &mut impl Iterator<Item=i64>) -> Option<ColorSpec>`; separate modifyOtherKeys/XTMODKEYS handling into their own fns. Behavior-preserving; the VT style test suite is the guard.
  3. `handle_csi_report`: group arms per report family (cursor position, device attributes, mode report...) into helpers.
  4. `main` (streaming server): extract `serve_until_ctrl_c` + banner printing (ARC-038 may absorb this — coordinate, do it once).
  5. `dispatch_command` + `handle_client` (mux; after the ARC-022 batch): move each arm's body into the existing `cmd_*` functions so `dispatch_command` becomes parse → match → call. The `Outcome`/`mutated` discipline from the ARC-002 design applies (`MuxCommand::mutates()` if it exists; otherwise a table).
- **Method**: Pure refactors — no behavior change. parsight `detect_changes` on the diff after each split to confirm only the intended symbols moved. Run the split commits separately per function.
- **Verify**: `make checkall` after each; `cargo test` full for streaming/csi/mux respectively; complexity re-check `calculate_cyclomatic_complexity` on the split fns (each should land <15).

### [QA-115] reap_dead_panes single lock scope — see ARC-022 batch (step 1)
*(executed as part of the Phase 2 ARC-022 entry; verify criterion: no interleaved lock acquisition between the last-pane decision and the kill).*

### [QA-116] SAFETY comments on unsafe
- **Files**: `src/pty_session.rs` (3), `src/mux/foreground.rs:245,285,346,360`, `src/bin/streaming_server/cli.rs:22`, `src/mux/client.rs:606`, `src/mux/pane.rs:1224`, `src/bin/par_mux/main.rs:293,308,349,362`, `src/ffi.rs:237-256`
- **Steps**: (1) At each site, write a `// SAFETY:` comment stating the actual invariant (e.g. fork site: async-signal-safety of the post-fork window — review it, don't just label it; if the fork path does something non-async-signal-safe, file a bug instead of labeling). (2) Add `#![warn(clippy::undocumented_unsafe_blocks)]` at the crate root (or per-module if the lint fires on generated code). (3) Test in a worktree if concurrent sessions are active.
- **Verify**: `make checkall` with the lint enabled — zero undocumented blocks.

### [QA-117] Drop unneeded unsafe impl Send/Sync
- **Files**: `src/python_bindings/observer.rs:447-448,487-488`
- **Steps**: Delete the four `unsafe impl Send/Sync` lines. Compile: pyo3 0.29 `Py<T>` is `Send + Sync`. If anything fails to derive, the manual impl was masking a real problem — restore and file a finding instead of forcing.
- **Verify**: `make checkall` (worktree if other sessions are live on main).

### [QA-118] Writer lock — folded into QA-110 (step 3). Verify: no `std::sync` locks remain under `src/streaming/` (`grep -rn "std::sync::R" src/streaming/`).

### [QA-119] mux_factory blocking write (after QA-110)
- **Files**: `src/streaming/mux_factory.rs:292,326-331`
- **Steps**: Move the daemon-socket write (`writeln!`+`flush` under the sync mutex) into `spawn_blocking`, or reuse QA-110's enqueue pattern if the mux adapter can share it. `SendKeysWriter` (`:292`) gets the same treatment. Do not run in parallel with QA-110 (shared design).
- **Verify**: `cargo test --features streaming,mux` (mux-factory tests); `make checkall`.

### [QA-120] Stub drift check in CI
- **Files**: `.github/workflows/ci.yml`, `Makefile:281` (`stubs` target)
- **Steps**: (1) In a CI job that already builds the wheel (or after `make dev`), run `make stubs` then `git diff --exit-code python/par_term_emu_core_rust/_native.pyi` — fail with "run make stubs" on drift. (2) Optional stronger check: `mypy.stubtest par_term_emu_core_rust._native --allowlist` — start with the diff check only. (3) Land after ARC-021's slice (which may regenerate the stub) or before it — coordinate ordering to avoid a red first run.
- **Verify**: CI run with a deliberately stale stub fails with the instruction; clean run passes.

### [QA-121] Large-file splits (last among streaming/server items; per-file `propose_decomposition` first)
- **Files**: `src/streaming/server.rs` (via ARC-028 — do not duplicate), `src/pty_session.rs` (reader thread → `src/pty_reader.rs`), others listed in AUDIT.md are longer-horizon
- **Steps**: (1) `pty_session.rs` (3,326 lines): move the reader thread (`start_reader_thread` and its helpers) into `src/pty_reader.rs`; keep `pub(crate)` re-exports so callers are unchanged. parsight `propose_decomposition` on the file first; `get_impact` on `start_reader_thread`. (2) `streaming/server.rs`: already covered by ARC-028 — verify and skip here. (3) `terminal/mod.rs` / `common.rs` / `kitty.rs` / `mux/server.rs`: file a follow-up board item rather than executing — each needs its own design pass (ARC-021 owns terminal/mod.rs).
- **Verify**: `make checkall`; PTY tests (`make test-pty` or Rust pty tests) green after the reader-thread move.

### [QA-123] FFI cell_count from len
- **Files**: `src/ffi.rs:142,206-208`
- **Steps**: Set `cell_count = cells_boxed.len() as u32` after the `into_boxed_slice` round-trip; switch the returned pointer to `Box::into_raw(cells) as *mut Cell` (documented pairing with the existing drop path) instead of `as_mut_ptr` + `mem::forget`. Keep the free path symmetric.
- **Verify**: `cargo test --features ffi ffi_tests` (gate per ARC-030) ; `make checkall`.

### [QA-124] Mouse event_type enum
- **Files**: `src/streaming/protocol.rs:541,812,1393,1609`, `src/streaming/server.rs` (the `event_type != "release"` checks), `src/streaming/proto.rs` (wire conversions stay strings on the wire), `src/python_bindings/streaming.rs` (dict conversion), `tests/test_streaming.rs`
- **Steps**: Add `pub enum MouseEventType { Press, Release, Move }` (check actual wire values first — the string set is whatever the frontend sends in `web-terminal-frontend/lib/protocol.ts`); parse at the protocol decode boundary; unknown strings are a decode error (not a silent press). Wire format unchanged (proto stays string/enum-per-proto rules).
- **Verify**: `cargo test --features streaming`; `uv run pytest tests/test_streaming.py`; frontend unchanged (verify with `make web-build-static` only if protocol.ts types change — they shouldn't).

### [QA-125] Builders for protocol constructors
- **Files**: `src/streaming/protocol.rs` (the 4 non-pyo3 `#[allow(too_many_arguments)]` sites), `src/streaming/server.rs` (2), `src/screenshot/renderer.rs` (2 — evaluate; may be legitimate)
- **Steps**: For each non-pyo3 site, introduce a builder struct in the `ConnectedBuilder` style (the crate's existing pattern) and delete the `#[allow]`. Keep the pyo3 `#[new]` constructors' allows (Python kwargs need them).
- **Verify**: `make checkall` — allow count drops by exactly the converted sites; `cargo test --features streaming`.

### [QA-126] Near-duplicate helpers
- **Files**: `src/python_bindings/types/graphics.rs:167` (→ call `src/graphics/mod.rs:519`), `src/python_bindings/{pty.rs:187, terminal/mod.rs:142}`, `src/python_bindings/streaming.rs` (encode fns), `src/mux/pane.rs` (create_pane variants), `src/screenshot/renderer.rs:831-910`
- **Steps**: (1) `sample_half_block`: the Python binding wraps the Rust fn — delete the duplicate body, call through. (2) `resize_pixels` binding dup: if the bodies are truly identical modulo class, use the existing `common.rs` macro layer; if they differ (PTY vs local), document why and leave. (3) encode fns: extract a shared generic. (4) `create_pane` variants: hoist common spawn+register into one helper with an options struct. (5) underline renderers: extract the shared underline-drawing loop. Each is a separate commit; skip any where unification would change behavior (state the reason in the commit).
- **Verify**: `make checkall` after each; `uv run pytest tests/ -k "graphics or resize"`; screenshot golden tests if present (`tests/test_screenshot.py`).

### [QA-127] Tighten weak asserts
- **Files**: `tests/test_terminal.py` (25 `is not None`), `tests/test_terminal_bindings.py` (11)
- **Steps**: For each standalone `assert x is not None`, assert the actual value (e.g. `assert fg_color == (r, g, b)` with the expected constants, or at minimum a type/range check). Keep the ones that are immediately followed by value checks — remove only redundancy where it reads better. Mechanical pass; no behavior change.
- **Verify**: `uv run pytest tests/test_terminal.py tests/test_terminal_bindings.py`.

### [QA-128] Remove known-safe unwraps
- **Files**: `src/streaming/server.rs:2277` (`HeaderValue::from_static`), `src/terminal/file_transfer.rs:165` (use `expect` with the invariant, or restructure — the value is constructed-valid; prefer `from_static`-style construction where an API exists)
- **Verify**: `make checkall`; grep confirms the unwraps are gone.

### [QA-129] Dead-code list — NO code change
- **Steps**: None. The parsight dead-code candidates were all verified live (benches, proc-macro, `.pyi`, FFI exports, pymodule, hook fns, clap value_parser, macro-called helpers, bootstrap handlers). `GraphicsStore::with_limits` stays (public API; sister projects unconfirmed). Do not delete anything. This entry exists so /fix-audit doesn't "helpfully" act on the raw parsight list.
- **Verify**: none (informational).

---

## Phase 3d — Documentation

### [DOC-021] QUICKSTART install paths
- **Files**: `QUICKSTART.md`
- **Steps**: (1) Installation section: open with `pip install par-term-emu-core-rust` and `uv add par-term-emu-core-rust` (copy README's phrasing). (2) New "Install from source" subsection: prerequisites (Rust toolchain), then `make setup-venv`, `make dev`, then a use snippet — verify each command on a fresh clone (or in a worktree with a clean venv) before committing. (3) "Next Steps": link `docs/BUILDING.md` and `CONTRIBUTING.md` (not CLAUDE.md). (4) "Help & Support": link `https://github.com/paulrobello/par-term-emu-core-rust/issues`.
- **Verify**: follow the doc top-to-bottom in a fresh clone/worktree; both paths work.

### [DOC-022] SECURITY.md mux rewrite — after SEC-101/104/105 (card depends on SEC-101)
- **Files**: `docs/SECURITY.md` ("Multiplexer Daemon Security"), `docs/SECURITY.md` (graphics section per SEC-107)
- **Steps**: (1) Read the current code first: `src/mux/ipc.rs` (per-UID 0700 dir, owner/mode verification, SO_PEERCRED/getpeereid refusal), `src/mux/hooks.rs` (validation of state/label), `src/mux/persist.rs` (0600-at-create state file incl. per-session env), `src/win_resume.rs` (Windows transport + cmd bridge `%VAR%` limitation). (2) Rewrite the four subsections to match; keep the "no dedicated mux audit has run" note ONLY if still true after this cycle's SEC-104/105 (it addressed hygiene; the note's fate is a judgment call — update wording to reflect the 2026-09-26 pass covered mux hygiene). (3) Fold in SEC-107 (kitty file-media section). (4) Cross-link MUX.md "Socket and State Paths". (5) Fix the TOC emoji-slug entries while in the file (DOC-033's SECURITY.md half).
- **Verify**: every claim in the section traces to a code path (spot-check the four cited files); links resolve.

### [DOC-023] README test commands
- **Files**: `README.md` ("Running Tests")
- **Steps**: Replace `cargo test` / `pytest tests/` with `make test`, `make test-rust`, `make test-python`; add one line that plain `cargo test` fails without the `python-test` feature setup (link `docs/BUILDING.md` for the why and for single-test invocations).
- **Verify**: run each documented command from a clean shell; they succeed.

### [DOC-024] README frontend build (bun, port 3000)
- **Files**: `README.md` ("Web Terminal Frontend")
- **Steps**: `npm install` → `make web-install` (or `bun install`); "port 8030" → 3000; point to `make web-dev` / `make web-build-static`. Cross-check against `web-terminal-frontend/README.md` and `package.json` `dev` script.
- **Verify**: `make web-dev` serves on 3000 as documented.

### [DOC-025] Unpin dependency snippets
- **Files**: `README.md` (feature table snippets), `docs/RUST_USAGE.md` (9 sites)
- **Steps**: Replace `version = "0.50"` snippets with the versionless `cargo add par-term-emu-core-rust --no-default-features -F pty_session` form (or a `X.Y` placeholder) + one line pointing to crates.io for the current version. Add a "keep versions out of docs" note to `docs/DOCUMENTATION_STYLE_GUIDE.md` (prevents the third recurrence).
- **Verify**: `grep -rn '"0.50"' README.md docs/RUST_USAGE.md` returns nothing; the `cargo add` line is syntactically valid (dry-run it).

### [DOC-026] Feature tables accurate (after ARC-034/035)
- **Files**: `README.md`, `docs/RUST_USAGE.md`, `docs/ARCHITECTURE.md`
- **Steps**: (1) Describe `full` as `python + streaming + streaming-bin` explicitly and note what it excludes. (2) Replace ARCHITECTURE.md's verbatim `[features]` paste with a pointer to `Cargo.toml` + the prose table (the paste has already drifted: `mux` line missing `clap`). (3) If ARC-035 added `macros-yaml`, include it.
- **Verify**: every feature in `Cargo.toml` appears in both tables with a correct description; ARCHITECTURE.md no longer contains the pasted block.

### [DOC-027] Env drop list current
- **Files**: `docs/SECURITY.md` ("Inherited Environment"), `docs/CROSS_PLATFORM.md`
- **Steps**: Read `DROP_VARS` in `src/pty_session.rs` (12 vars + `PAR_MUX_*` prefix rule); replace both doc lists with the full set or an explicit "the authoritative list is `DROP_VARS` in src/pty_session.rs" plus the current contents; document the prefix rule and why (agent nesting, tokens).
- **Verify**: `grep -n DROP_VARS src/pty_session.rs` contents match the doc word-for-word (or the doc points at the constant and lists it with a synced-on date).

### [DOC-028] Environment variable reference table
- **Files**: `docs/CONFIG_REFERENCE.md` ("Environment Variables"), `docs/STREAMING.md`, new section or table
- **Steps**: (1) Sweep the code: `grep -rn 'env::var("PAR_TERM' src/` plus `env::var(` generally (also check computed names via `find_code` "read environment variable"). Documented so far: `PAR_TERM_REPLY_XTWINOPS`, `DEBUG_LEVEL`, `PAR_MUX_ENV`, `PAR_MUX_ALLOW_NESTED`, `XDG_RUNTIME_DIR`, `PAR_TERM_FORCE_WEB_DOWNLOAD` (missing from STREAMING.md — add it). (2) Replace the CONFIG_REFERENCE claim with a table: variable | component (core/mux/streamer) | default | effect | doc link. (3) STREAMING.md: add the missing var to its table.
- **Verify**: every `env::var` hit appears in the table; the table's claims match the code defaults.

### [DOC-029] Stub docstrings (after ARC-021)
- **Files**: `scripts/generate_stubs.py`, `python/par_term_emu_core_rust/_native.pyi` (regenerated)
- **Steps**: (1) In the generator, for each function/class emit its runtime `__doc__` as a docstring in the stub (import the built module or parse from the binding metadata — the generator already imports the module to introspect; adding `__doc__` output is mechanical). (2) Truncate very long docstrings to a sensible cap if the stub balloons (check size before/after; keep under ~1.5x). (3) `make stubs` to regenerate; commit the stub with the generator change. (4) Long-term annotations (`#[pyo3(text_signature)]`) noted in the generator's header comment as future work — not in this entry.
- **Verify**: `make stubs && git diff --stat` shows only the expected growth; pyright still passes (`make lint-python`); spot-check hover-worthy methods (`screenshot`, `process`) carry summaries.

### [DOC-030] Binding Examples (after ARC-021)
- **Files**: `src/python_bindings/terminal/{bookmark,image,metrics,notification,scrollback,search,selection,text}_api.rs`, `src/python_bindings/pty.rs`, `src/python_bindings/streaming.rs`
- **Steps**: For each file (start with `pty.rs`, `streaming.rs`): add an `Example` section to every public-method docstring per `docs/DOCUMENTATION_STYLE_GUIDE.md` (Google style). Examples must be runnable-looking and correct against the actual signatures (cross-check `docs/API_REFERENCE.md` for the shape). Batch per file; `make stubs` if docstrings surface in the stub.
- **Verify**: `make lint-python` (docstring linters if configured); `grep -c "Example" ` per file > 0 for all ten files; spot-execute two examples in `uv run python`.

### [DOC-031] Vendor/publish the mux decision record
- **Files**: `docs/par-mux.md`, new `docs/mux-decisions.md` (if vendoring), references in `src/mux/**` comments citing D-numbers
- **Steps**: Preferred: vendor — create `docs/mux-decisions.md` with one line per D-number (decision + rationale) sourced from `~/Repos/par-agent-os/par-mux.md` (readable on this machine); update `docs/par-mux.md` to point there instead of the home path; keep D-number citations in code valid. Alternative (if the source doc should stay canonical): add the public URL once it exists — not available now, so vendor.
- **Verify**: no doc pointer resolves outside the repo (`grep -rn "par-agent-os" docs/ src/` → only historical CHANGELOG mentions); every D-number cited in `src/mux/` has an entry.

### [DOC-032] Legacy event docs (decide removal/deprecation with ARC-021's window)
- **Files**: `python/par_term_emu_core_rust/observers.py`, `docs/API_REFERENCE.md` (`poll_events_legacy` bullets), `src/python_bindings/terminal/mod.rs` (the `poll_events_legacy` docstring)
- **Steps**: (1) Fix the type hints in `observers.py` to `Callable[[dict[str, Any]], None]` (0.50.0 ships native values). (2) Correct "pre-0.51" → "pre-0.50" in both doc surfaces. (3) Decision: emit `DeprecationWarning` naming the removal version, OR update the "kept for one release" wording to reflect indefinite retention. Coordinate with ARC-021's deprecation window so both deprecations share a release note. The warning path is recommended (the methods are 2 releases old with zero usage signals).
- **Verify**: `uv run pytest tests/ -k observer`; pyright clean on observers.py; the docstring and API_REFERENCE.md agree on version and plan.

### [DOC-033] Broken intra-doc links
- **Files**: `docs/API_REFERENCE.md`, `docs/SECURITY.md`
- **Steps**: (1) `CHANGELOG.md#0500---2026-09-21` → `../CHANGELOG.md#0500---2026-09-23` (verify the anchor: GitHub slugifies "## [0.50.0] - 2026-09-23" to `#0500---2026-09-23`). (2) SECURITY.md TOC: remove the ✅/❌ emoji from the two headings (slugs then match) or compute the emoji-leading slug and use it — removing the emoji also fixes the style-guide scannability point. (3) SEC-107/DOC-022 may have already touched SECURITY.md — read current state first.
- **Verify**: every relative link in both files resolves (script-check or manual click-through on github.com).

### [DOC-034] Fence language tags
- **Files**: `docs/VT_TECHNICAL_REFERENCE.md` (15), `docs/ADVANCED_FEATURES.md` (6), `docs/MACROS.md` (5), `docs/STREAMING.md` (3), `docs/TESTING_KITTY_ANIMATIONS.md` (3), `docs/CONFIG_REFERENCE.md`, `docs/GRAPHICS_TESTING.md`, `docs/MATURIN_BEST_PRACTICES.md` (1 each)
- **Steps**: For each bare ``` fence, add the language: `text` for terminal output/diagrams, `bash`, `python`, `rust`, `yaml` as appropriate. Mechanical; watch for fences containing fences (nested) — those get `text`.
- **Verify**: `grep -rn '^```$' docs/` returns only intentional closers (a bare-fence opener scan: `grep -rnE '^```\s*$' docs/` cross-checked against line parity).

### [DOC-035] Trim README What's New
- **Files**: `README.md`
- **Steps**: Keep the current release (0.52.0) + the latest breaking-change note (0.50.0's pane-state removal pointer); move the remaining seven releases' prose out (it already exists in CHANGELOG.md — delete, don't duplicate); fix nothing else. If the 0.45/0.44 dead-path prose is kept anywhere, correct or drop it (it's being dropped).
- **Verify**: README renders with Features near the top; `git diff` shows deletions only in the What's New block; CHANGELOG.md already contains the dropped content (spot-check one entry).

### [DOC-036] CHANGELOG compare links
- **Files**: `CHANGELOG.md` (bottom link-reference block)
- **Steps**: Add `[0.38.0]: ...compare/v0.37.0...v0.38.0` through `[0.52.0]` plus `[Unreleased]: ...compare/v0.52.0...HEAD`, matching the existing URL pattern (`https://github.com/paulrobello/par-term-emu-core-rust/compare/...`). Verify each tag exists (`git tag -l 'v0.4*' 'v0.5*'`); adjust if tags differ from the pattern.
- **Verify**: render check — headings 0.38.0+ become links; every compare URL loads.

### [DOC-037] Missing docs + missing_docs lint
- **Files**: `src/screenshot/mod.rs`, `src/grid/scroll.rs` (`Grid::resize`), `src/mux/ipc.rs` (`ConnectionAbort`), `src/python_bindings/observer.rs` (two `new` fns), `src/lib.rs` (lint)
- **Steps**: (1) Write the missing doc comments (module `//!` for screenshot; Google-style for the binding `new` fns; standard rustdoc for the rest). (2) Add `#![warn(missing_docs)]` to `lib.rs` — if it fires widely on generated `terminal.pb.rs`, add `#[allow(missing_docs)]` on that module only (generated code is exempt per the style guide). If the lint produces a flood beyond the five sites listed, gate it to `warn` and fix the listed five, filing the remainder as a follow-up rather than allowing them wholesale.
- **Verify**: `cargo doc --no-deps` builds without warnings for the five sites; `make checkall` with the lint enabled.

### [DOC-038] Archive stale audit artifacts — LAST
- **Files**: `AUDIT-REMEDIATION.md`, `AUDIT-REMEDIATION-PLAN.md` (09-22 cycle versions — note: the 09-26 cycle's PLAN is live and consumed by /fix-audit; archive only after /fix-audit completes), `AUDIT.md`
- **Steps**: After /fix-audit finishes this cycle: `git mv AUDIT.md AUDIT-REMEDIATION.md AUDIT-REMEDIATION-PLAN.md docs/audits/2026-09-26/` (create the dir; same for the 09-22 set if still at root — check `git log` — the 09-22 AUDIT.md was overwritten, so only its remediation files may remain). Update any references (ARC-037 coordinates the root cleanup).
- **Verify**: repo root has no AUDIT files after archiving; `docs/audits/` holds the cycle set; `grep -rn "AUDIT.md" docs/ CLAUDE.md` references updated or acceptable as historical.

---

## Cross-entry sequencing summary (for /fix-audit)

1. **QA-110** → 2. **QA-112** (isolated) → 3. **QA-113**, **QA-111**, **SEC-101(+102/103)**, **SEC-104/105**(after Phase 2) in parallel → 4. **Phase 2** (ARC-022 batch, ARC-021 slice) can start once QA-110 is in (different files) → 5. **QA-114** (needs QA-110 + Phase 2 mux batch + QA-112 for the csi parts) → 6. **ARC-028** → 7. **QA-121** → 8. Documentation wave (DOC-022 gated on SEC; DOC-026 on ARC-034/035; DOC-029/030 on ARC-021; DOC-032 on the deprecation decision; **DOC-038 last**).

**Never parallel**: QA-110 ∥ QA-119; QA-112 ∥ (QA-114 csi parts); anything on `src/streaming/server.rs` outside the QA-110 → QA-114 → ARC-028 → QA-121 chain; the ARC-022 mux batch ∥ QA-114's mux part.
