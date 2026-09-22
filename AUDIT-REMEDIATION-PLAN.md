# Audit Remediation Plan — par-term-emu-core-rust

> Companion to `AUDIT.md` (audit 2026-09-22). One entry per issue, ordered to match
> AUDIT.md's `## Remediation Plan` phases. `/fix-audit` points each phase agent at its own
> entries. All commands run from the repo root. Build rule: never `cargo build` for the PyO3
> module — `make dev`. Rust test features: `cargo test --lib --no-default-features --features
> pyo3/auto-initialize` (add `,streaming` for streaming, `rust-only,mux,serde -- --test-threads=1`
> for mux).
>
> parsight: repository_id `repo-db09184b0394f6287256c56299eaeb0f`. After any restructure,
> reindex (`index_directory`, incremental) before trusting symbol locations; re-read files
> before editing (line numbers below are as-of 9fa2237).

## Phase 1 — Security, small independent fixes

### [SEC-001] Enforce read-only on Resize/FocusChange/SelectionRequest/ClipboardRequest
- **Files**: `src/streaming/server.rs` (tungstenite arms: Resize ~1210, FocusChange ~1264, SelectionRequest ~1317, ClipboardRequest ~1384; axum arms ~1745 and wherever the loop lands after ARC-001)
- **Steps**:
  1. In `run_ws_session`'s match, add `if read_only { continue; }` as the first statement of the `Resize`, `FocusChange`, `SelectionRequest`, and `ClipboardRequest` arms — exactly the pattern already used by `Input` (:1191), `Mouse` (:1235), `Paste` (:1290).
  2. In `handle_axum_websocket`'s match, add the same guard to `Resize` (~:1745).
  3. In the `ClipboardRequest` "get" branch, additionally gate on `terminal.allow_clipboard_read()` (the flag that gates OSC 52; default false).
- **Method**: The bypass exists because the guard was added per-arm historically. Adding it to four more arms is the whole fix; no state changes. If ARC-001 has already landed, the arms exist once — guard them there instead, and skip step 2.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming` green; add one test in `src/streaming/server.rs` tests connecting read-only and asserting a Resize produces no `resize_tx` send (or manually: connect with `?readonly=1` per docs/STREAMING.md and confirm no SIGWINCH). `make checkall`.

### [SEC-002] Bounds-check selection before byte slicing
- **Files**: `src/terminal/screen.rs:430-470` (`get_selected_text` Character and Block arms), `src/terminal/screen.rs:403-410` (`set_selection`), `src/streaming/server.rs` SelectionRequest arm (~:1363)
- **Steps**:
  1. In `set_selection`, clamp: `start.0 = start.0.min(self.cols.saturating_sub(1))` (same for `end.0`, and rows against `self.rows`) — or store raw and clamp in the read path; pick one site (clamping in `set_selection` is simpler and fixes all callers).
  2. In `get_selected_text`'s Character arm, replace `line_text[row_start..row_end]` with a char-boundary-safe slice: walk `line_text.char_indices()` to the first byte offset with char-position >= row_start (columns ≈ chars for this text; graphemes are already joined into cells), same for row_end; slice those byte offsets. Block arm (`&line_text[start_col..end_col.min(len)]`): same treatment.
  3. In the server's SelectionRequest arm, reject `start_col/end_col >= cols` or rows >= rows with the existing debug-log + continue pattern.
- **Method**: The panic is "display column used as UTF-8 byte index". The byte-offset walk is O(line) per row, fine for a selection read. Unit test: write "aé🇺🇸b" into a row, set a selection with start_col inside the emoji, call `get_selected_text()` — must not panic.
- **Verify**: new unit test in `src/terminal/tests/` passes; `cargo test --lib --no-default-features --features pyo3/auto-initialize screen` green; existing selection tests (`grep -rn selection src/terminal/tests/`) unchanged and green; `make checkall`.

### [SEC-003] Incremental OSC/DCS accumulation caps
- **Files**: `src/terminal/sequences/osc/mod.rs` (enforcement at :36), `src/terminal/sequences/dcs/mod.rs:73-131` (`dcs_put`), `src/terminal/mod.rs:663-668` (`DEFAULT_MAX_OSC_DATA_LENGTH`), `src/terminal/perform.rs` (hook/put entry points)
- **Steps**:
  1. OSC: the dispatch-time check is too late — vte buffers the whole payload first, and the default is 128 MiB. Add incremental enforcement: track `osc_put` accumulation is inside the vte crate, so the practical lever is (a) lowering `DEFAULT_MAX_OSC_DATA_LENGTH` to `1 << 20` (1 MiB), and (b) checking `max_osc_data_length` in `Terminal::process_internal` against an in-flight counter (add `osc_pending: usize` to state; on each `osc_put` byte add and if > max, set a `discard_osc` flag until the next `osc_dispatch`, which returns early). If the vte `Perform` implementation here receives per-byte `osc_put`, that hook is the natural place (`perform.rs` — add `fn osc_put`).
  2. DCS: in `dcs_put`'s two `push` sites (`:127`, `:130`), enforce `if self.dcs_state.dcs_buffer.len() >= MAX_DCS_BUFFER { self.dcs_state.dcs_overflow = true; return; }` with `const MAX_DCS_BUFFER: usize = 64 * 1024`; on `dcs_unhook`, if `dcs_overflow`, clear the flag and return without processing. 64 KiB comfortably exceeds any legitimate XTGETTCAP/DECRQSS/sixel-param payload (sixel pixel data streams to the parser, not this buffer).
  3. CHANGELOG: note the OSC default change (128 MiB → 1 MiB) as behavior-affecting under Unreleased.
- **Method**: Verify current behavior first: `grep -n 'fn osc_put' src/terminal/perform.rs` — if absent, vte's default `osc_put` is a no-op and accumulation lives entirely inside vte's `osc_raw` Vec; in that case the incremental cap must go in `process_internal` (count bytes since last ESC-terminator heuristic is unreliable — prefer implementing `osc_put` on the Perform impl, which vte calls per byte). Document whichever enforcement point you implement next to the constant.
- **Verify**: new tests: a 2 MiB OSC payload leaves `max_osc_data_length` at default and produces no state change + bounded memory (assert an internal counter or just no panic/OOM at 64 MiB payload in a `--release`-mode manual run); a >64 KiB XTGETTCAP DCS is dropped without processing. `cargo test --lib --no-default-features --features pyo3/auto-initialize` green; `make checkall`.

## Phase 2 — Structural, blocking

### [ARC-016] Per-instance shutdown handle
- **Files**: `src/mux/server.rs` (static at :188, reset at :136, polls at :115/:152, `request_shutdown` at :194), `src/bin/par_mux/main.rs:66-68`
- **Steps**:
  1. Delete `static SHUTDOWN_REQUESTED` (:188) and `MuxServer::request_shutdown` (:193-196).
  2. Add field `shutdown: Arc<AtomicBool>` to `MuxServer` (init `false` in `bind`/`bind_with_tree`), `pub fn shutdown_handle(&self) -> Arc<AtomicBool>`.
  3. Poll `self.shutdown.load(Relaxed)` at :115 and :152; delete the `store(false)` at :136. For `run_persisting`'s post-run final save (:116), check `*self.shutdown` instead of the static.
  4. `main.rs`: before `install_sigterm_handler`, create `static HANDLE: OnceLock<Arc<AtomicBool>>`; store `server.shutdown_handle()`; the `on_sigterm` handler loads and stores `true`. (Signal handler only does an atomic store — still async-signal-safe.)
  5. Fix test callers of `request_shutdown()` (grep `request_shutdown` in `src/mux/server.rs` tests ~:974 and `tests/`) to hold the handle before spawning `run`.
- **Method**: The static exists for the signal handler; the OnceLock indirection preserves that while making instances independent. Tests at `:938`/`:995` spawn servers on parallel threads — after this change they cannot cross-stop each other.
- **Verify**: `cargo test --no-default-features --features rust-only,mux,serde -- --test-threads=1` and `-- --test-threads=8` (the parallel run is the point) green; `make checkall`.

### [ARC-002] Decompose dispatch_issued + parse_command (⊇ QA-101, QA-104)
- **Files**: `src/mux/server.rs:280-778`, `src/mux/command.rs:380-599`, `src/mux/tree.rs` (op signatures), `src/mux/hooks.rs` (parse_line consumer), new `src/mux/dispatch.rs`, `src/mux/mod.rs`
- **Steps**: follow `docs/fable/ENH-009-mux-dispatch-decomposition.md` steps 1–5 in full (Args struct, `parse_<cmd>` fns, `Line`/`parse_line` enum, `MuxCommand::mutates()`, `Outcome`/`Ctx`, `cmd_<name>` handlers in `dispatch.rs`, tree ops returning `WindowId`, `push_to_clients`). The audit fix = that plan minus nothing; ENH-009 is the executable detail.
- **Method**: The existing string-round-trip tests (`src/mux/server.rs:1040-1680`, `tests/mux_*.rs`) are the oracle — zero test edits allowed. Run them after each sub-step, not only at the end.
- **Verify**: per ENH-009's Verify section: mux suite green with zero test edits; parsight CC < 15 for both functions after reindex; `.expect(` count in `server.rs` down 5; `mutates()` table test; `make checkall`.

### [ARC-001] Unify the two WebSocket loops
- **Files**: `src/streaming/server.rs:1126-1485` (tungstenite), `:1637-1851` (axum), `tests/test_ws_smoke.rs`
- **Steps**:
  1. Extract from `run_ws_session`'s match an `async fn handle_client_message(self: &Arc<Self>, session: &Arc<StreamSessionState>, client_id: Uuid, read_only: bool, subscriptions: &mut Option<HashSet<EventType>>, msg: ClientMessage) -> Result<Vec<ServerMessage>>` containing the 11 arm bodies verbatim (including the SEC-001 guards if landed).
  2. `run_ws_session`'s loop: decode → `handle_client_message` → send each returned reply. `handle_axum_websocket`'s loop: same call, converting `AxumMessage` payloads first.
  3. Delete the `_ => {}` catch-all — the match inside `handle_client_message` must be exhaustive (no wildcard).
  4. Add to `tests/test_ws_smoke.rs`: start the server with `start_with_http` on an ephemeral port, connect to `/ws`, send a `Mouse` message (`encode_client_message`), assert the PTY writer received the encoded bytes (use a `SessionFactory` stub capturing writes, as the in-process router tests in `server.rs` already do).
- **Method**: Alternative shape (a `WsTransport` trait wrapping both stream types so the axum path reuses `Client<S>` and `handle_axum_websocket` is deleted) is acceptable if it lands cleaner — the invariant is one arm-set, compiler-enforced exhaustiveness, and the mouse-over-axum test. Note `GlobalClientGuard`/`SessionClientGuard` handling differs between the paths (axum reserves slots earlier); keep reservation points where they are, only unify the message handling.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming` green; `cargo test --test test_ws_smoke --no-default-features --features pyo3/auto-initialize,streaming` green including the new test; `make checkall`.

### [DOC-001] Remove the dead multiplexing API from the reference
- **Files**: `docs/API_REFERENCE.md:108,114,127` (TOC), `:849-861` (method list), `:1657`, `:1788`, `:1945` (class sections); `docs/ARCHITECTURE.md:362`
- **Steps**:
  1. In the TOC delete the `PaneState`, `SessionState`, `WindowLayout` entries.
  2. Under "Session Management" delete the nine method bullets (`serialize_session`, `deserialize_session`, `create_session_state`, `capture_pane_state`, `restore_pane_state`, `get_pane_state`, `set_pane_state`, `clear_pane_state`, `create_window_layout`), keep `set_remote_session_id`/`remote_session_id`, and add: "> The pane/window session-state API (`PaneState`, `SessionState`, `WindowLayout` and the `*_pane_state` methods) was removed in 0.50.0 — see the [0.50.0 changelog entry](CHANGELOG.md) for the rationale and the `replay_snapshot` replacement."
  3. Delete the three class sections at :1657/:1788/:1945.
  4. `docs/ARCHITECTURE.md:362`: delete the `pane_state: Option<PaneState>` line from the `Terminal` struct listing.
- **Method**: Ground truth is `python/par_term_emu_core_rust/_native.pyi` — after editing, `grep -nE 'PaneState|SessionState|WindowLayout|pane_state' docs/API_REFERENCE.md docs/ARCHITECTURE.md` must return only the removal note.
- **Verify**: that grep shows only the note; `make checkall` (docs aren't gated, but run it as the commit gate).

## Phase 3a — Security (remaining)

### [SEC-005] Non-blocking PTY writes, Input/Paste payload caps
- **Files**: `src/streaming/server.rs:1200-1208` (Input), `:1298-1310` (Paste), `:45` (WS_MAX_MESSAGE_SIZE context), `src/streaming/config.rs:336` (rate-limit default context)
- **Steps**:
  1. Cap `Input` payloads at 64 KiB and `Paste` at 256 KiB before writing (log + drop beyond, same style as SEC-003).
  2. Input arm: clone the payload `Vec<u8>`, clone the `Arc` writer handle, drop the `writer.lock()` guard, then `tokio::task::spawn_blocking(move || writer.write_all(&payload))` (or `block_in_place` — the server runs on multi-thread runtime; check `#[tokio::main(flavor)]` first and use spawn_blocking for safety). Ignore/log the JoinError.
  3. Paste arm: copy `bracketed_paste_start/end` + payload out while holding `session.terminal.write()`, drop the guard, then write via spawn_blocking exactly as Input.
- **Method**: The freeze chain is: PTY not drained → kernel buffer full → `write_all` blocks the tokio worker → other Input arms block on `writer.lock()` → Paste additionally stalls the reader thread's `process()`. Async writes break the chain at the first link.
- **Verify**: new test: a stub session whose PTY write blocks (a pipe nobody reads, or a factory stub that sleeps) — with two clients sending Input, the second client's Ping round trip still completes within a timeout. `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming`; `make checkall`.

### [SEC-004] Handshake timeout + pre-handshake slot reservation
- **Files**: `src/streaming/server.rs:833-866` (plain), `:926-944` (TLS), `:1102` (`try_add_client`)
- **Steps**: wrap both accept awaits: `match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(...)).await`; on `Err(_)` (timed out) or a handshake failure, drop the stream. Move `try_add_client()` ahead of the handshake await and keep the `GlobalClientGuard` alive across it (drop on failure). Mirror on the TLS path. Check the axum path: hyper's header timeout applies only if a timer is installed by `axum::serve`; verify empirically (a test that opens a TCP conn and sends nothing, asserting the server closes it within ~35 s is slow — instead assert the guard count via `/stats` on the raw paths).
- **Method**: The pre-check `can_accept_client` at :833/:926 is advisory; reservation is what bounds resources. Keep the advisory check as a fast path.
- **Verify**: test: N+1 concurrent raw TCP connects (no upgrade) where N = max_clients; the extra is refused/closed promptly rather than lingering; existing max-clients tests still green; `make checkall`.

### [SEC-006] tls-pem permission check in from_pem
- **Files**: `src/streaming/config.rs:82-96` (from_files check), `:126-174` (from_pem)
- **Steps**: extract the mode check into `fn reject_world_readable(path: &Path) -> Result<(), ...>`; call it from both `from_files` (key path) and `from_pem` (the combined PEM path). Unix-only (`#[cfg(unix)]`), same as today.
- **Verify**: unit test: chmod 0644 a temp PEM → `from_pem` errors; 0600 → ok. `make checkall`.

### [SEC-007] Frontend download integrity + size cap
- **Files**: `src/bin/streaming_server/frontend_download.rs` (URL at :28/:41-49, download :104-122, extract :130-143)
- **Steps**:
  1. Read stream with a 50 MiB byte cap (error out past it) instead of buffering unbounded.
  2. Before `remove_dir_all(web_root)` (:130), require `web_root.join("index.html").exists()` unless an explicit `--force-web-download` flag was passed (add to cli.rs).
  3. Integrity: fetch `<asset>.sha256` published alongside the archive; verify with `sha2` (already in the dep tree via streaming) before extraction; if the sidecar is missing, fail with instructions. Until a release actually publishes `.sha256` assets, gate this on release process — if not available, land the size cap + refuse-to-delete now and file the checksum wiring as follow-up in the same card's notes.
- **Verify**: unit test with a local HTTP stub serving an oversized body → error before extraction; `make checkall`.

### [SEC-008] Constant-time basic-auth verify
- **Files**: `src/streaming/config.rs:245-260`
- **Steps**: always run the password verification: compute `user_ok = username_bytes.ct_eq(configured)`; `pass_ok = verify_password(input, configured_hash_or_dummy)`; return `user_ok & pass_ok` (bitwise-AND the booleans after converting). Add a `const DUMMY_BCRYPT_HASH` (valid bcrypt hash of a random fixed string, cost matching the real one) used when the username mismatches. Keep the length-mismatch path from leaking: hash the input regardless.
- **Verify**: existing auth tests green; a timing test is impractical in CI — assert via code review that no early return remains. `make checkall`.

### [SEC-009] Redacting Debug impls
- **Files**: `src/streaming/config.rs:190` (`HttpBasicAuthConfig`), `:200` (`PasswordConfig`), `:265` (`StreamingConfig`), `src/streaming/server.rs:2582-2588` (`Debug for StreamingServer`)
- **Steps**: hand-write `impl fmt::Debug for PasswordConfig` emitting `PasswordConfig { secret: "***" }`; same for the api_key field in `StreamingConfig`'s manual Debug (derive on the rest via a nested redacted struct or manual field list). Update the `StreamingServer` Debug to delegate to the redacted config Debug.
- **Verify**: `format!("{config:?}")` contains neither the password nor the api_key in a unit test. `make checkall`.

### [SEC-010] X-Frame-Options + CSP frame-ancestors
- **Files**: `src/streaming/server.rs:729-732, 790-793` (the two ServeDir routers)
- **Steps**: add a `tower::Layer` via `map_response` (tower is already a dep) setting `X-Frame-Options: DENY` and `Content-Security-Policy: frame-ancestors 'none'` on every response from the static routers (and harmless on API routes — apply at the outermost router if simpler). Alternatively enable tower-http's `set-header` feature and use `SetResponseHeaderLayer::overriding`.
- **Verify**: router unit test (the `tower::ServiceExt::oneshot` pattern already in `server.rs` tests): GET `/` → both headers present. `make checkall`.

### [SEC-011] Reserve client slot before session creation; validate session ids
- **Files**: `src/streaming/server.rs:1101-1104` (`resolve_session` before `try_add_client`), `:224-240` (`ConnectionParams::from_query`)
- **Steps**:
  1. Reorder `handle_axum_websocket`'s prologue (and the tungstenite equivalents at :1066/:1079): `try_add_client` (global) → `resolve_session` → session-slot add; guards drop on error in reverse order. Careful: `resolve_session` failure must release the global slot (the existing `GlobalClientGuard` handles drop-order — re-verify).
  2. In `from_query`, reject session ids not matching `^[A-Za-z0-9_-]{1,64}$`.
  3. Add one line to `docs/STREAMING.md` security section: "the API key / Basic credential is a single trust boundary — any authenticated client may attach to any session."
- **Verify**: test: max_clients reached + new connection with a fresh session id → `MaxClientsReached` before the factory spawns (assert spawn count via a counting `SessionFactory` stub). Invalid-id test: `?session=../x` rejected. `make checkall`.

## Phase 3b — Architecture (remaining)

### [ARC-003] Persistence off the lock
- **Files**: `src/mux/persist.rs`, `src/mux/server.rs`, `src/mux/pane.rs`
- **Steps**: follow `docs/fable/ENH-008-mux-persist-off-lock.md` in full (split `save_to` into `write_state`, worker thread with drain-newest coalescing, shutdown join ordering, optional snapshot cache). Must land AFTER ARC-002.
- **Method**: the restart/reattach suites assert save-then-restore round trips; the only timing-sensitive assertion is "save after every mutating command" (`server.rs` ~:1658) — give tests a flush path (poll the file mtime with a deadline, or expose `flush_persist` under `#[cfg(test)`).
- **Verify**: per ENH-008's Verify section (mux suite green, coalescing test, latency measurement, `make checkall`).

### [ARC-004] Drop tokio from the mux feature
- **Files**: `Cargo.toml:213-219`, `tests/mux_feature_isolation.rs:4`
- **Steps**: remove `"tokio"` from the `mux` feature array; rewrite the comment at :213-214 ("Needs real PTYs and a local socket"); fix the doc comment in the test file.
- **Verify**: `cargo tree --no-default-features --features rust-only,mux -e normal | grep -c tokio` = 0; `cargo check --no-default-features --features rust-only,mux,serde`; CI D1 still passes conceptually (`cargo tree --features sim` unchanged); `make checkall`.

### [ARC-005] Gate subtle/zeroize + the streaming module root
- **Files**: `Cargo.toml:141-142,206`, `src/lib.rs:77`, `src/streaming/mod.rs`, `.github/workflows/ci.yml:140`
- **Steps**: per `docs/fable/ENH-013-crate-package-hygiene.md` steps 3–4 and 7 (optional + feature list, `#[cfg(any(feature = "streaming", feature = "python", feature = "python-test"))] pub mod streaming;`, fix any importer that fails outside those features, extend the CI grep to `tokio|portable-pty|subtle|zeroize|prost|axum`).
- **Verify**: the five `cargo check` profiles listed in ENH-013 step 4; the extended tree grep is empty for `sim`; `make dev` still builds (python bindings compile protocol.rs via the `python` feature); `make checkall`.

### [ARC-007] Finish the Python binding dedup (⊇ QA-106, QA-107)
- **Files**: `src/python_bindings/common.rs` (macro set at :36-2081, stale allow at :28), `src/python_bindings/pty.rs` (34 `Ok::<_,()>` sites; duplicated methods incl. `:966-1019`, `:184-197`), `src/python_bindings/terminal/recording_api.rs:25-68`, `src/python_bindings/terminal/mod.rs:135-148`, `python/par_term_emu_core_rust/_native.pyi`
- **Steps**:
  1. Mechanical rewrite of all 34 `if let Ok(term) = Ok::<_, ()>(lock) { ... } else { Err(...) }` sites in `pty.rs` to `let term = self.inner.terminal().write();` + body (parking_lot cannot fail). `grep -c 'Ok::<_, ()>' src` = 0 after.
  2. Add `impl_terminal_exports_and_input!` (or extend `impl_terminal_exports` at :2017) carrying: `export_asciicast`, `export_asciicast_v3`, `export_recording_json`, `resize_pixels`, `paste`, `create_snapshot`, `debug_info`, `get_char`, `get_line`, `get_scrollback_usage`, `get_stats`, `pop_keyboard_flags`, `push_keyboard_flags`, `query_cursor_color`, `query_default_bg`, `query_default_fg`, `query_keyboard_flags`, `resize`, `scrollback_len`, `set_ansi_palette_color`, `set_keyboard_flags` — written once against `TerminalAccess` (`term_ref`/`term_mut`). Invoke from `terminal/mod.rs` and `pty.rs`; delete both hand copies.
  3. Unify `resize_pixels` on `usize` (Terminal's width) — `PtyTerminal`'s `u16` version disappears.
  4. Delete the `#[allow(dead_code)]` at `common.rs:28`.
  5. `make stubs && make stub-check`.
- **Method**: enumerate the duplicated surface first with parsight (`get_symbol_context` on each method name, `repository_id` scoped) so nothing is missed; the near-duplicate lane found cosine 0.93–0.97 pairs. Python-visible signatures must not change (same names, same args) — the `.pyi` diff after regeneration should be empty or cosmetic.
- **Verify**: `grep -c 'Ok::<_, ()>' src` = 0; `make stub-check` clean with an empty-signature-diff expectation; `uv run pytest tests/test_terminal_bindings.py tests/test_pty.py -q` green; `make checkall`.

### [ARC-008] Align the gates
- **Files**: `.github/workflows/ci.yml:180`, `.pre-commit-config.yaml:43`, `Makefile:257,277`, CI test job (~:88-113)
- **Steps**: change both clippy invocations to `--features python,streaming,mux,serde` (after ARC-004/005 so the string is final); add a CI step after "Run Rust tests": `cargo test --no-default-features --features pyo3/auto-initialize,streaming` (Linux job acceptable); optionally define `RUST_LINT_FEATURES` in the Makefile and have CI call `make clippy`.
- **Verify**: CI config is dispatch-only — validate locally by running the exact three commands; `make lint` green; `make checkall`.

### [ARC-009] Single DEC mode table (⊇ QA-105)
- **Files**: `src/terminal/sequences/csi/mode.rs:103-309`
- **Steps**: per `docs/fable/ENH-015-dec-mode-table.md` steps 1–2 (`dec_mode_label`, `set_dec_private_mode(param, enabled)` with asymmetric arms keeping explicit `if enabled`); DECRQM list consolidation in `report.rs` is optional here (ENH-015 adds the symmetry test — include step 3 of that plan's test if cheap).
- **Verify**: existing modes tests green unedited; parsight CC of both handlers < 5 after reindex; `make checkall`.

### [ARC-006] Terminal constructor defaults
- **Files**: `src/terminal/mod.rs:956-1229` (constructor), sub-struct definitions above `:788`
- **Steps**: per-sub-struct: add `#[derive(Default)]` where defaults are zero/empty; `impl Default` where non-zero (e.g. `saved_fg: White`); keep `MarginState`, tab stops, `Grid` behind `fn new(cols, rows)`. Rewrite `with_scrollback` to construct the geometry-dependent three explicitly and default the rest. Target < 60 lines.
- **Method**: the constructor is the only place field defaults are stated — after the change, adding a field means adding it to the struct + Default, not to a 270-line list. Behavior-identical by construction; the 2,516 unit tests are the oracle.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize` green; `wc` of the constructor region < 60 effective lines; `make checkall`.

### [ARC-010] log:: in mux instead of eprintln!
- **Files**: `src/mux/scrape.rs:364,373,384,392,423,429`, `src/mux/persist.rs:482,493`, `src/mux/server.rs:117,773`, `src/bin/par_mux/main.rs:37,51`
- **Steps**: replace each `eprintln!` in `src/mux/*.rs` with `log::warn!`/`log::error!` by severity (save failures = error; scrape/pattern load issues = warn). In `main.rs`, install a minimal logger before `run_persisting`:
  ```rust
  struct StderrLog;
  impl log::Log for StderrLog { fn error(&self, ...) { eprintln!(...) } ... }
  static LOGGER: StderrLog = StderrLog;
  log::set_logger(&LOGGER).map(|_| log::set_max_level(log::LevelFilter::Info)).ok();
  ```
  (10 lines; `log` is already an unconditional dep). Keep the two `main.rs` startup `eprintln!`s as-is or route through the logger.
- **Verify**: `grep -rn 'eprintln!' src/mux` = 0; mux suite green (stderr no longer spammed during scrape-fallback tests — confirm output is quiet); `make checkall`.

### [ARC-011] Bounded client channels + eviction
- **Files**: `src/mux/server.rs:215,795-862`
- **Steps**: per `docs/fable/ENH-012-mux-client-backpressure.md` (`sync_channel(4096)`, `push_to_clients` with `try_send`, eviction on `Full`, unit test). If ARC-002 already introduced `push_to_clients`, this reduces to the channel swap + the eviction branch + the test.
- **Verify**: per ENH-012's Verify section; `make checkall`.

## Phase 3c — Code Quality (all)

### [QA-102] Polling helpers; migrate sleep-based tests
- **Files**: new `tests/conftest.py`, new `tests/common/mod.rs`, `tests/test_pty.py` (22 sites), `tests/test_pty_resize_sigwinch.py`, `tests/test_macros_extended.py`, `tests/test_streaming.py`, `tests/test_screenshot.py`, `tests/test_nested_shell_resize.py`, `tests/test_ioctl_size.py`, `tests/test_macros.py`, `tests/mux_daemon.rs:53`, `tests/mux_reattach.rs:46`, `tests/mux_restart.rs:107-146` (hoist source)
- **Steps**:
  1. `tests/conftest.py`: add the `wait_for(predicate, timeout=5.0, interval=0.02)` helper from the domain report (module-level function importable as `from conftest import wait_for` or expose via a fixture).
  2. Migrate `tests/test_pty.py` first: `time.sleep(n); content = term.content(); assert X in content` → `assert wait_for(lambda: X in term.content())`. Keep `test_coprocess.rs`-style justified negative sleeps (commented).
  3. `tests/common/mod.rs`: move `wait_until`/`wait_for`/`wait_for_pid` from `tests/mux_restart.rs`; add `mod common;` to the four mux test files; replace `mux_daemon.rs:53`'s fixed 300 ms with a `wait_until`-style loop on `connect_local_stream(&path).is_ok()`; same for `mux_reattach.rs:46`.
  4. Migrate the remaining Python files in the order listed (each is mechanical).
- **Method**: The one justified fixed sleep asserts a negative (nothing happened) — leave those, comment why. ENH-011's `wait_for_text` supersedes the conftest helper when it lands; the helper then becomes a thin wrapper — coordinate so the same file isn't migrated twice (if ENH-011 lands first, migrate `test_pty.py` against `wait_for_text` instead).
- **Verify**: `grep -c 'time.sleep' tests/test_pty.py` ≤ 1; `uv run pytest tests/ -v --timeout=5 --timeout-method=thread` green (the CI invocation); `cargo test --no-default-features --features rust-only,mux,serde -- --test-threads=1` green; suite wall time does not increase; `make checkall`.

### [QA-108] Assert the observable in 33 no-assert tests
- **Files**: `src/screenshot/renderer.rs:1194-1206` + 3 more, `src/screenshot/utils.rs`, `src/screenshot/error.rs`, `src/pty_session.rs:1489-1520` (9 tests), `src/pty_error.rs`, `src/terminal/tests/terminal_tests.rs:3188-3194` (3), `tests/test_terminal_bindings.py:80-140` (6), `tests/test_streaming.py:465`, `tests/test_pty.py:243`, `tests/test_macros_extended.py` (2), `tests/test_nested_shell_resize.py`, `tests/test_terminal.py`
- **Steps**: per the domain report's per-file remedies: `test_set_env*` → assert via a spawn (`session.env().get`); clipboard-history trio → `assert_eq!(history.len(), 1)` + content; `test_terminal_bindings.py` six → use `term.cursor_visible` / mode getters (they exist — verified); `test_max_clients_limit` → three websockets, assert the third refused; `test_render_straight_underline_pixels` → build a `Renderer` with default config (fonts are embedded) and assert the pixel row at `cell_height - 2`; delete any test that genuinely has nothing observable.
- **Method**: Work file-by-file; each conversion is 3–10 lines. Do not weaken an assertion into `assertTrue(x is not None)` shape — assert values.
- **Verify**: count of assertion-free tests via the domain report's list = 0 (re-run the same detection: tests whose body contains no `assert`); `make test` green; `make checkall`.

### [QA-112] Delete seven dead public functions
- **Files**: `src/debug.rs:193(log_vt_input),353(log_cursor_move),387(log_grid_op),424(log_mode_change)`, `src/grapheme.rs:140(is_wide_grapheme_with_config)`, `src/macros.rs:388(MacroPlayback::reset)`, `src/screenshot/renderer.rs:543-546(contains_regional_indicators + its `#[allow(dead_code)]`)`, `CHANGELOG.md`
- **Steps**: delete each function (and any doc comment); `CHANGELOG.md` Unreleased → Removed: "unused public items: debug.rs log_* helpers (superseded by the `debug_*!` macros routing through `logf`), `is_wide_grapheme_with_config`, `MacroPlayback::reset`, `contains_regional_indicators` (superseded by the cell-scanning path)". Schedule AFTER the 0.50.0 publish card clears.
- **Method**: Verified dead against this repo AND both sister repos (`par-term`, `par-term-emu-tui-rust`). Before deleting, re-run the check: parsight `get_symbol_context` on each name (scoped to this repo) + `grep -rn <name> ../par-term/src ../par-term-emu-tui-rust` (adjust paths) — zero non-definition hits.
- **Verify**: `cargo check --all-targets --features python,streaming,mux,serde` and `rust-only` variants compile; `make checkall`.

### [QA-113] Panic containment in mux client threads
- **Files**: `src/mux/server.rs:208-267` (`handle_client`), `src/mux/tree.rs:201-504` (`.expect`s)
- **Steps**: after ARC-002, re-evaluate: if the five dispatcher `.expect`s are gone, wrap `handle_client`'s command-processing section in `std::panic::catch_unwind(AssertUnwindSafe(...))` and on `Err` write `emit_block(n, "internal error", false)` + `log::error!`; remaining tree `.expect`s that are genuinely invariant may stay, or convert to `MuxError::Internal`.
- **Verify**: unit test: a poisoned command (test-only injection or a forced unwrap path) yields an error block and the client connection survives one more command; `make checkall`.

### [QA-114] Stale/over-broad `#[allow]` cleanup
- **Files**: `src/python_bindings/common.rs:28`, `src/python_bindings/terminal/mouse_api.rs:45`, `src/python_bindings/terminal/mod.rs`, `src/python_bindings/terminal/trigger_api.rs`, `src/streaming/session.rs`, `src/streaming/server.rs`, `src/graphics/mod.rs` (7 `type_complexity`)
- **Steps**: delete the `common.rs:28` allow (dead after ARC-007); `record_mouse_event`: forward `pixel_x/y` into `MouseEvent` or rename to `_pixel_x/_pixel_y` and drop the allow; replace each `clippy::type_complexity` allow with a named `type` alias (pattern precedent: commit b7e5ad2).
- **Verify**: `grep -rn 'allow(' src | wc -l` drops by ≥ 9; `make lint` green; `make checkall`.

### [QA-115] Clock-read unwraps
- **Files**: `src/macros.rs:88-90,320-323`, `src/debug.rs:142`
- **Steps**: `.unwrap()` → `.unwrap_or_default()` on the three `duration_since(UNIX_EPOCH)` sites.
- **Verify**: `grep -n 'UNIX_EPOCH' src/macros.rs src/debug.rs` shows no bare `.unwrap()`; `make checkall`.

### [QA-116] PyMacroEvent repr + docstring
- **Files**: `src/python_bindings/types/recording.rs:249,265-272`, `python/par_term_emu_core_rust/_native.pyi`
- **Steps**: `__repr__` uses `{:?}` on the `Option` fields; field docs say `"key"`, `"delay"`, or `"screenshot"` (lowercase — tests at `tests/test_macros.py:29,44,56` assert it); `make stubs`.
- **Verify**: `uv run pytest tests/test_macros.py -q` green; `make stub-check`; `make checkall`.

### [QA-117] One push_to_clients helper
- **Files**: `src/mux/server.rs:801-803,817,860-862`
- **Steps**: folded into ARC-002 step 3 / ARC-011; if neither has run, do it standalone: `fn push_to_clients(clients: &Clients, line: String)` replacing the three `retain` copies.
- **Verify**: `grep -c 'retain(|(_, tx)|' src/mux/server.rs` ≤ 1; mux tests green; `make checkall`.

### [QA-110] Split KittyParser::build_graphic
- **Files**: `src/graphics/kitty.rs:521-867`
- **Steps**: extract `decode_payload(&self, cmd) -> Result<RgbaBuffer, GraphicsError>`, `apply_placement`, `apply_frame_command`, `apply_delete_command` — pure code motion, no logic edits; `build_graphic` becomes dispatch. Then `parse_chunk` (:232) if time allows.
- **Method**: The module has 1,747 lines of tests — they are the oracle; run after each extraction, not only at the end. This module shipped 3 bugs in v0.43.1; resist "improving" behavior while moving code.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize kitty` green; parsight CC of `build_graphic` < 20 after reindex; `make checkall`.

### [QA-111] Terminal.tsx hooks + component tests + lint gate
- **Files**: `web-terminal-frontend/components/Terminal.tsx:92,191-559,561-774`, `web-terminal-frontend/vitest.config.ts:15`, `package.json`, `eslint.config.mjs`
- **Steps**:
  1. Route the stale-closed props (`fontSize` already has `fontSizeRef`; add `onFocus`, `onRefit`, `onSendInput`, `applyTheme`, `onHyperlinkAdded`, `onSelectionChanged`, `onUserVarChanged`) through refs updated in a small effect — OR split the mount effect into `useXtermInstance`/`useTerminalInput`/`useResizeHandling` hooks with correct deps.
  2. Delete or use the `status` state (:92).
  3. Widen `vitest.config.ts` include to `['**/__tests__/**/*.test.{ts,tsx}']`; add `components/__tests__/Terminal.test.tsx` with happy-dom + a mocked `TerminalConnection` (injectable via `connectionRef`).
  4. Fix the remaining warnings; then add `--max-warnings 0` to the lint script (AFTER the warnings are gone, or `make checkall` goes red).
  5. `make web-build-static` after any component change (project rule).
- **Verify**: `npm run lint` (0 warnings under `--max-warnings 0`); `npx vitest run` green incl. the new test; `make web-build-static`; `make checkall`.

### [QA-118] OnscreenKeyboard decomposition
- **Files**: `web-terminal-frontend/components/OnscreenKeyboard.tsx:52-78,673`
- **Steps**: extract `MacroEditor` + `MacroList` components and a `useStoredMacros()` hook that validates the `localStorage` parse (`Array.isArray` + per-item `name`/`script` string checks) before `setMacros`; `make web-build-static`.
- **Verify**: manual keyboard render unchanged; a corrupted localStorage entry yields `[]` not a crash (unit test on the hook); `make checkall`.

### [QA-119] event_to_dict native types (breaking — schedule for next major)
- **Files**: `src/python_bindings/observer.rs:19-294`, `_native.pyi`, `docs/API_REFERENCE.md`
- **Steps**: NOT this cycle. When a breaking window opens: per-variant `#[pyclass]` events or a `PyDict` with native ints/bools; keep `event_to_dict` behind `poll_events_legacy` for one release; CHANGELOG breaking note.
- **Verify**: n/a now — file as the card's "deferred" note.

### [QA-120] Test hygiene odds and ends
- **Files**: `tests/test_pty_resize_sigwinch.py:48`, `tests/test_streaming.py:44-57`, `tests/test_streaming_dict_api.py:49-62`, `src/pty_session.rs:1665`
- **Steps**: `except:` → `except OSError:` in the generated child script; replace the `try/import except: X = Any` fallback with `pytest.importorskip("websockets")` at module top (deletes 13 `# type: ignore`); `#[ignore]` gets a reason string.
- **Verify**: `uv run pytest tests/test_streaming.py tests/test_streaming_dict_api.py -q` green; `grep -c 'type: ignore' tests/test_streaming*.py` = 0; `make lint-python`; `make checkall`.

## Phase 3d — Documentation (all)

### [DOC-003] README 0.50.0 truth + split (absorbs DOC-016)
- **Files**: `README.md:22,26`
- **Steps**: replace line 26's paragraph with the bullet structure from the domain report: **Removed (breaking)** / **Added: mux Phase 2/4** / **Phase 5 agent + scrape tiers** / **Phase 6 resume** (text supplied verbatim in the domain report — use it); fix "will start from that type" → "started from that type"; append "and Phase 6 agent-session resume" to the line 22 summary.
- **Verify**: README 0.50.0 section agrees with `CHANGELOG.md:8-19` on persistence (agent_session persists, format v2) and mentions 6.2–6.4; land before the 0.50.0 publish card clears.

### [DOC-002] docs/MUX.md + mux coverage (absorbs ARC-013)
- **Files**: new `docs/MUX.md`; `docs/ARCHITECTURE.md` (Overview + Supporting Modules + feature block), `CLAUDE.md` (Key Source Layout + feature table), `README.md` (Features + Documentation), `docs/BUILDING.md` (mux build/test target), `docs/RUST_USAGE.md` (mux feature row)
- **Steps**: write `docs/MUX.md` per the domain report's outline (purpose; `cargo build --bin par-mux --no-default-features --features mux`; CLI; socket/state paths per platform; pane env contract; command table with the fixed reply shapes FROM `src/mux/command.rs` + `emit.rs` at the current commit; hook JSON contract; agent-pattern override dir; persistence + quarantine; SIGTERM vs SIGKILL; the serialized-test note). Vendor `par-mux.md` into `docs/` (or a stub stating its location and repo) so the 25+ `see par-mux.md` citations resolve. Then the five cross-file additions.
- **Method**: extract the command/reply tables mechanically from `src/mux/command.rs` (the `match *name` arms) and `src/mux/emit.rs` — do not write them from memory. If ARC-002 landed, tables come from the post-refactor handlers.
- **Verify**: every path `docs/MUX.md` cites exists (`grep -oE 'src/[a-z_/]+\.rs' docs/MUX.md` all resolve); `make checkall`.

### [DOC-004] ARCHITECTURE.md inventory + feature refresh
- **Files**: `docs/ARCHITECTURE.md:42,486,505-515,945-990`, `CLAUDE.md:107`
- **Steps**: :42 → `streaming-bin`; :486 + CLAUDE.md:107 → "16 themed"; :511 → drop `session.rs` from the types list; regenerate the :945-990 feature block from current `Cargo.toml:190-246` (add `python-test`, `serde`, `mux`, `streaming-bin`, both `[[bin]]`); add the mux section per DOC-002.
- **Verify**: each file path ARCHITECTURE.md cites under `src/` exists (`grep -oE 'src/[a-z_/]+\.rs' docs/ARCHITECTURE.md | while read f; do [ -e "$f" ] || echo "MISSING $f"; done` → empty).

### [DOC-005] CONTRIBUTING streaming-protocol section
- **Files**: `CONTRIBUTING.md:105-115`
- **Steps**: replace the two stale lines with: `src/streaming/session.rs` (`SessionRegistry::build_connect_message()`); "When extending `Connected`, add one method on `ConnectedBuilder` plus the field in `Connected`/builder/`build()`, then update `build_connect_message()`. Do not add partial constructors." (matches CLAUDE.md step 5).
- **Verify**: diff against CLAUDE.md's "Adding Streaming Message Types" — no contradiction.

### [DOC-006] Feature tables complete
- **Files**: `CLAUDE.md:79-88` (+ drop the "~line 52" per DOC-018), `docs/RUST_USAGE.md:436-447`
- **Steps**: add rows for `mux`, `serde`, `python-test`, `streaming-bin` (wording in the domain report); fix `full` = `python` + `streaming` + `streaming-bin`.
- **Verify**: table matches `Cargo.toml [features]` exactly.

### [DOC-007] Broken cross-references
- **Files**: `docs/STREAMING.md:626-629`, `docs/VT_SEQUENCES.md:459`, `docs/ADVANCED_FEATURES.md:288`, `docs/BUILDING.md:374`, `docs/VT_TECHNICAL_REFERENCE.md:53-56`
- **Steps**: prefix the four frontend paths with `web-terminal-frontend/`; anchor → `#sixel-graphics-dcs-q`; rewrite the ADVANCED line to the in-file `#color-utility-functions` anchor; BUILDING line → "Rust output: `src/streaming/terminal.pb.rs` (checked in; `build.rs` verifies its checksum, `make proto-rust` regenerates)"; prefix the three `mod.rs` cites with `src/terminal/sequences/`.
- **Verify**: re-run parsight `find_broken_doc_links` (min_confidence high) → zero findings in these files.

### [DOC-008] Stale version pins
- **Files**: `README.md:216-219`, `docs/RUST_USAGE.md` (8 lines), `docs/MATURIN_BEST_PRACTICES.md:517`
- **Steps**: replace `"0.46"` with `"0.50"` (or drop the number for "the current crates.io version"); refresh/remove the MATURIN scorecard stamp.
- **Verify**: `grep -rn '0\.46' README.md docs/RUST_USAGE.md` empty.

### [DOC-009] SECURITY.md mux section
- **Files**: `docs/SECURITY.md`
- **Steps**: after a mux security pass runs (see AUDIT.md coverage note), add "Multiplexer Daemon Security": socket permissions per platform, no auth on the control protocol (same-user boundary), state-file contents/location, hook-report validation, quarantine, `sh -c` spawn quoting. If the pass hasn't run, write the section from current code and mark the trust-boundary statement as describing today's behavior.
- **Verify**: section cross-checked against `src/mux/server.rs:63-67`, `persist.rs`, `hooks.rs`.

### [DOC-010] Binding docstrings
- **Files**: `src/python_bindings/pty.rs` (84 methods needing sections), `src/python_bindings/streaming.rs` (50), nine `terminal/*_api.rs` files
- **Steps**: add `Args:`/`Returns:` to every method with parameters or non-trivial return in `pty.rs` and `streaming.rs` first; one `Example:` per file on the primary entry point; then the nine api files. Regenerate `make stubs`.
- **Verify**: `make stub-check`; spot-check 10 methods in `_native.pyi` carry the sections; `make checkall`.

### [DOC-011] Document the 53 undocumented public Rust symbols
- **Files**: `src/terminal/mod.rs:788,155,162`, `src/terminal/metrics.rs:423`, `src/streaming/config.rs:17`, `src/cell.rs:73-191`, `src/color.rs:63`, `src/sixel.rs:46-309`, `src/debug.rs:410`, `src/python_bindings/streaming.rs:1064-1107`, `src/bin/streaming_server/bootstrap.rs:325`
- **Steps**: `///` one-liners (full list in the domain report); `Terminal` gets the crate-summary wording from `src/lib.rs:1-20`.
- **Verify**: parsight `list_symbols documented:false visibility:public` on `src/` → near-zero (mux test-module exceptions acceptable).

### [DOC-012] examples/README.md index
- **Files**: `examples/README.md`
- **Steps**: add a "Streaming" category (`streaming_demo.py`, `streaming_debug.py`, `streaming_client.html` + the `make dev-streaming` prerequisite) and either index the five `*_test.py` diagnostics under "Rendering diagnostics" or move them to `tests/manual/` (prefer indexing — moving breaks nothing but changes URLs).
- **Verify**: every `examples/*.py|html` on disk appears in the index (`ls examples | diff` against the table).

### [DOC-013] max_osc_data_length in the reference
- **Files**: `docs/API_REFERENCE.md`, `docs/CONFIG_REFERENCE.md`
- **Steps**: add the two bullets under the terminal limits section (text in the domain report); cross-link from CONFIG_REFERENCE "Core Security Settings". Note: if SEC-003 lowered the default, document the new value.
- **Verify**: both methods appear in API_REFERENCE; value matches `src/terminal/mod.rs`.

### [DOC-014] STREAMING.md headings
- **Files**: `docs/STREAMING.md:1596, ~262`
- **Steps**: rename `:1596` to "### TLS CLI Options"; add "### Command-Line Options and Environment Variables" above the full table (~:262) and list it in the TOC.
- **Verify**: TOC links resolve.

### [DOC-015] README links
- **Files**: `README.md:159-177,634-662`
- **Steps**: add `[Quick Start Guide](QUICKSTART.md)` to the Documentation list; `See [CONTRIBUTING.md](CONTRIBUTING.md)` in Contributing.
- **Verify**: both links resolve.

### [DOC-017] workflows README
- **Files**: `.github/workflows/README.md`
- **Steps**: add a row each for `claude.yml` and `claude-code-review.yml` (trigger, purpose, required secrets — read the two files first).
- **Verify**: all 7 workflow files appear.

### [DOC-018] CLAUDE.md line references
- **Files**: `CLAUDE.md` (Version Sync section)
- **Steps**: drop the line-number references ("line 3", "line 9", "~line 52" → `Cargo.toml` `version` key etc.); keys are unique.
- **Verify**: `grep -nE 'line [0-9]+' CLAUDE.md` empty.
