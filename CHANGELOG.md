# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security
- **Streaming server: unbounded zlib decompression (zip-bomb DoS).** `decode_with_decompression` in `src/streaming/proto.rs` called `ZlibDecoder::read_to_end` with no size cap, so a single small compressed protobuf frame could expand into gigabytes of memory and OOM-kill the server. It now reads from the decoder in 8 KiB chunks into a bounded `Vec` and returns `StreamingError::InvalidMessage` at the new 1 MiB `MAX_DECOMPRESSED_SIZE` cap. Additionally, both tungstenite WebSocket acceptors now use `accept_hdr_async_with_config` with an explicit `WebSocketConfig` (16 MiB `max_message_size` / `max_frame_size`) instead of relying on tungstenite's 64 MiB default, and the same 16 MiB caps are applied to both axum `WebSocketUpgrade` handlers. No new dependencies; return types and error variants unchanged.
- **Dependency vulnerabilities resolved (`cargo audit`: 5 → 0 vulnerabilities).** Replaced the abandoned `htpasswd-verify` crate (which pulled the unmaintained `rust-crypto` RUSTSEC-2022-0011, `rustc-serialize` RUSTSEC-2022-0004, `time` RUSTSEC-2020-0071, and `gcc` RUSTSEC-2025-0121) with direct htpasswd-hash verification in a new `src/streaming/auth_hash.rs` module using maintained RustCrypto primitives (`bcrypt`, `md-5`, `sha1`) + the existing `base64` dep — supports the same `$2y$`/`$apr1$`/`$1$`/`{SHA}` formats, with the MD5-crypt core ported from the canonical crypt(3) algorithm and locked down with `openssl`-generated known-answer vectors. Upgraded PyO3 0.28.3 → 0.29 (RUSTSEC-2026-0176/0177). Replaced the unmaintained `rustls-pemfile` (RUSTSEC-2025-0134) with the `rustls-pki-types` `PemObject` trait (already transitively available — no new dependency). Disabled the AVIF feature on the `image` crate so the unmaintained `paste` proc-macro (RUSTSEC-2024-0436, transitive via `ravif` → `rav1e`) is no longer compiled into any build.
- **Streaming server: CSRF-via-WebSocket defense.** New optional `StreamingConfig.allowed_origins` allowlist (also exposed as the Python `allowed_origins` property and the standalone `--allowed-origins` CLI flag / `PAR_TERM_ALLOWED_ORIGINS` env var). The WebSocket `Origin` header is now validated at all four WS entry points (two tungstenite acceptors + two axum handlers): by default, non-browser clients (no `Origin` header — curl, native TUI, the embedded library) and local (`localhost`/`127.0.0.1`/`::1`) browser origins are accepted while remote browser origins (e.g. a malicious page on `evil.com`) are rejected with HTTP 403; when an allowlist is configured, only those exact origins are accepted. A `tower-http` `CorsLayer` reflecting the same policy is applied to both HTTP routers.
- **Standalone streamer: warn on public bind without auth.** The binary already defaults `--host` to `127.0.0.1`; it now also prints a loud stderr warning when binding a non-loopback interface with no `--api-key` or HTTP Basic auth configured, since that exposes an interactive shell to anyone who can reach the port.

### Changed
- **Internal code-quality refactors (no API or behavior change).** From the project audit: removed the `Ok::<_, ()>(lock())` dead-branch anti-pattern at 12 sites in `src/streaming/server.rs` (`parking_lot::Mutex::lock` cannot fail); rewrote `html_escape`/`escape_xml` to allocate a single `String` instead of one per character; made `get_dirty_region` a single pass with no panic-bait `unwrap()`; `poll_subscribed_events` now calls the existing `TerminalEvent::kind()` instead of duplicating its 25-arm match; the coprocess output/error buffers switched `Vec<String>` → `VecDeque<String>` (O(1) `push_back`/`pop_front` instead of O(n) `remove(0)`); and the duplicated `emit_style` SGR closure (~78 lines × 2) in `src/grid/export.rs` was extracted into a shared `push_sgr_style` helper.
- **`Terminal` god-object decomposition (ARC-001, internal — no API or behavior change).** The `Terminal` struct's ~150 flat fields were grouped into 31 cohesive `pub(crate)` sub-structs (e.g. `TerminalModes`, `ColorThemeState`, `ClipboardSyncState`, `ProfilingState`, `TriggerState`, `DcsState`, `MarginState`, `EventBrokerState`, `GraphicsState`), each held as a single field on `Terminal`. The remaining ~25 flat fields are the irreducible core (buffer, cursor, parser, current cell render state). Existing accessor methods on `Terminal` delegate to the sub-structs, so all callers — including the Python bindings — are unaffected. Behavior is preserved (the full Rust + Python test suites pass).
- **`PyTerminal` Python-binding split (ARC-002, internal — no Python API change).** The single ~5768-line / ~384-method `#[pymethods]` block on the `Terminal` Python class was split into 18 cohesive blocks: the core methods remain in `python_bindings/terminal/mod.rs` (halved to ~2869 lines) and 17 themed sibling files (`trigger_api`, `color_api`, `clipboard_api`, `metrics_api`, `search_api`, `selection_api`, `snapshot_api`, `scrollback_api`, `bookmark_api`, `multiplexing_api`, `image_api`, `shell_integration_api`, `notification_api`, `recording_api`, `badge_api`, `file_transfer_api`, `mouse_api`, `text_api`), each its own `#[pymethods] impl PyTerminal`. Enabled the `pyo3` `multiple-pymethods` Cargo feature so PyO3 merges all blocks into the same Python `Terminal` class — **the Python API surface is unchanged** (492 Python tests pass). (The audit's stretch remedy — nested sub-objects like `term.colors.x` for autocomplete discoverability — is a breaking major-version API change and is not included.)
- **`PyTerminal`/`PtyTerminal` method duplication eliminated (ARC-003/QA-001, internal — no Python API change).** The ~155 methods duplicated between the `Terminal` and `PtyTerminal` Python classes are reduced to **26** via a shared `TerminalAccess` trait (`term_ref`/`term_mut`, RPITIT) + `#[macro_export]` method macros in `python_bindings/common.rs` that define each shared method once and emit it for both classes. **129 methods unified, ~258 duplicate copies deleted, net −1,200+ lines.** The 8 hand-written `PyAttributes` literals are replaced by the existing `From<&Cell>` impl. The remaining 26 duplicates are genuinely non-unifiable (e.g. `PtyTerminal.paste()` writes to the PTY child while `Terminal.paste()` parses locally; `screenshot`/`resize`/`create_snapshot` have divergent logic) — unifying them would change behavior, so they're intentionally kept. Behavior is preserved (492 Python tests pass on every batch).
- **`Cell` combining marks stored inline (ARC-005/QA-004, performance — no API change).** The `Cell` grid's combining-marks field changed from `Vec<char>` to `SmallVec<[char; 4]>`, so cells with ≤4 combining marks (variation selectors, ZWJ, skin-tone modifiers, regional-indicator pairs — the >99.9% case) no longer heap-allocate, and cloning them during scroll/reflow/snapshot is a memcpy instead of a heap allocation. Rare longer clusters spill to the heap (no data loss). `Cell` stays `Clone` (SmallVec is not `Copy`), but per-cell allocation pressure on the grid hot paths is eliminated. A regression benchmark (`cloning_combining_cells_is_fast_at_scale`) locks in the characteristic.
- **WebSocket handler dedup (ARC-004/QA-002, internal — no protocol change).** The 3 near-identical ~380-line WebSocket connection handlers (plain TCP, TLS, axum) in `streaming/server.rs` were collapsed: the `Client` type is now generic over the tungstenite stream type (`Client<S>`), so the plain and TLS handlers share one `run_ws_session` dispatch loop (~770 lines of duplicated 11-arm `ClientMessage` dispatch eliminated — adding a new message type no longer requires editing 3 handlers). The axum handler's pure `RequestRefresh`/`SnapshotRequest` arms now call shared builder helpers. **Bonus parity fix**: the old TLS handler silently dropped `Mouse`/`FocusChange`/`Paste`/`SelectionRequest`/`ClipboardRequest` (a documented inconsistency); the shared loop now handles them uniformly. A new end-to-end WS smoke test (`tests/test_ws_smoke.rs`) guards the refactored path (real server + client, handshake + Ping→Pong round-trip). 174 streaming tests + 492 Python tests pass.

### Fixed
- **Panicking terminal observers no longer corrupt state (ARC-007 safe fix).** Observer callbacks (`on_zone_event`/`on_command_event`/`on_environment_event`/`on_screen_event`/`on_event`) now run inside `std::panic::catch_unwind`. Previously, a panicking observer (e.g. a misbehaving Python callback) would unwind through the `Terminal`'s `parking_lot::Mutex` — which does not poison — silently leaving the terminal inconsistent and re-firing already-dispatched events. The panic is now contained to the failing observer; dispatch completes normally and a regression test (`test_panicking_observer_is_isolated`) locks in the behavior. (The separate, larger work of moving observer dispatch fully out of the mutex remains future work.)

### Documentation
- **Default streaming-server port corrected (8080 → 8099)** in README, Makefile, and the `streaming_server` module rustdoc to match the actual clap `default_value`. The README's Rust toolchain requirement was updated 1.75+ → 1.88+ to match `Cargo.toml`'s `rust-version`. The STREAMING.md architecture diagram label now reads "Protobuf (binary)" instead of "JSON Messages". ARCHITECTURE.md gained the missing `apc_filter.rs` and `badge.rs` modules and now points readers at runnable test-count commands instead of stale hard-coded numbers. BUILDING.md gained a prominent "never `cargo build`; use `make dev`" callout. A new `CONTRIBUTING.md` documents the dev workflow, the version-sync and Rust↔Python binding-sync rules, and the PR process. `src/pty_session.rs` gained a module-level rustdoc comment.

## [0.42.4] - 2026-06-08

### Fixed
- **`has_updates_since()` unreliable on Windows after Ctrl+C.** The generation counter (`update_generation` / `has_updates_since`) was incremented at the end of a long terminal processing block in the PTY reader thread. If processing encountered a panic or unexpected code path — notably on Windows ConPTY after the child receives `CTRL_C_EVENT` — the counter would stay stuck even though data had already been written to the grid buffer. The `fetch_add(1)` now runs immediately after a successful `reader.read()`, before any processing, guaranteeing the counter always advances when PTY data arrives. ([#60](https://github.com/paulrobello/par-term-emu-core-rust/issues/60), [#61](https://github.com/paulrobello/par-term-emu-core-rust/pull/61))

## [0.42.3] - 2026-06-06

### Fixed
- **BCE (Background Color Erase) not implemented.** All VT erase operations (`EL` `\x1b[K`, `ED` `\x1b[J`, `ECH` `\x1b[X`, `DECSERA`) now fill erased cells with the current SGR background color instead of always resetting to black. This matches the behavior of iTerm2, Alacritty, and Windows Terminal. Grid erase methods now accept a `bg: Color` parameter; the CSI erase handler passes `self.bg` (current SGR background). Scroll/edit operations continue using default black bg (BCE does not apply to scroll). ([#57](https://github.com/paulrobello/par-term-emu-core-rust/issues/57), [#58](https://github.com/paulrobello/par-term-emu-core-rust/pull/58))

### Build / Tooling
- **Updated Rust crates:** bitflags 2.11.1→2.13.0, uuid 1.23.1→1.23.2, tokio 1.52→1.52.3, reqwest 0.13.3→0.13.4, sysinfo 0.39.2→0.39.3, unicode-segmentation 1.13→1.13.3, plus transitive bumps (hyper, log, zerocopy, yoke, etc.)
- **Updated Python dev deps:** pyright 1.1.409→1.1.410, pytest-asyncio 0.26→1.4, ruff 0.15.14→0.15.16
- **Updated web frontend deps:** next 16.2.6→16.2.7, react 19.2.6→19.2.7, react-dom 19.2.6→19.2.7, eslint 10.4.0→10.4.1, eslint-config-next 16.2.6→16.2.7, @types/node 25.9.1→25.9.2, @types/react 19.2.15→19.2.17

## [0.42.2] - 2026-05-29

### Fixed
- **PytestUnknownMarkWarning for `@pytest.mark.asyncio`.** Added `pytest-asyncio` to dev dependencies so pytest recognizes the asyncio mark used by streaming tests.
- **Top-anchored scroll region lines lost instead of entering scrollback.** `scroll_region_up()` only pushed rows into scrollback when the region covered the full screen. Partial top-anchored regions (e.g. Codex CLI's `CSI 1;{rows-1} r`) now correctly preserve evicted rows in primary-screen scrollback, matching iTerm2 behavior. Refactored scrollback insertion into a shared `push_rows_to_scrollback()` helper used by both full-screen and region scrolling paths. ([#54](https://github.com/paulrobello/par-term-emu-core-rust/pull/54), thanks @ziglerari)

### Build / Tooling
- **Updated Rust crates:** tikv-jemallocator 0.6→0.7, sysinfo 0.39.2→0.39.3, hyper 1.9→1.10, reqwest 0.13.3→0.13.4, plus transitive bumps (cc, displaydoc, http, log, memchr, mio, socket2, uuid, zerocopy)
- **Updated Python dev deps:** pygments 2.19.2→2.20.0, platformdirs 4.9.6→4.10.0, pytest-asyncio 1.3→1.4, ruff 0.15.14→0.15.15, virtualenv 21.3→21.4
- **Updated web frontend deps:** @bufbuild/buf 1.69→1.70

## [0.42.1] - 2026-05-08

### Fixed
- **Kitty TGP virtual placement: image truncated past 64th column/row.** `graphics::placeholder::diacritic_to_number` only mapped the first 64 of the 297 diacritics defined in the Kitty graphics protocol spec ([rowcolumn-diacritics.txt](https://github.com/kovidgoyal/kitty/blob/master/kittens/unicode_input/rowcolumn-diacritics.txt)). Once a client emitted a placeholder cell with `col >= 64`, the second-diacritic lookup returned `None`, `decode_placeholder_cell` rejected the cell, and the bounding-box scan in par-term-render's `scan_placeholder_cells` stopped at column 63 — so a Kitty image asked to fit in e.g. 75 cells rendered into only 64. Replaced the 64-arm match with the full 297-entry `DIACRITICS: &[char]` static table plus a `OnceLock<HashMap<char, u16>>` reverse lookup. `row` and `col` widened from `u8` to `u16` (the spec's 0..=296 range overflows `u8`); `msb` stays `u8` because that diacritic position only encodes 0..=255. New tests `test_diacritic_table_size` (asserts 297 entries) and `test_create_placeholder_past_64` (round-trips index 120/200) lock down the regression.
- **CSI 16t (report cell pixel size) returned a hardcoded 10×20.** `XTWINOPS Ps=16` now derives the response from `self.pixel_width / cols` and `self.pixel_height / rows` (both set by `Terminal::set_pixel_size` from host-supplied resize events), with the 10×20 default kept as a fallback for the pre-resize state. Clients querying `\x1b[16t` to size inline graphics (sixel/iTerm2/Kitty) now receive the renderer's actual cell pitch instead of a value that disagreed with both the renderer and `TIOCGWINSZ`.

### Build / Tooling
- **GitHub Actions updated to latest major versions.** Bumped `actions/checkout`, `actions/setup-python`, `actions/upload-artifact`, and `actions/download-artifact` to their current major versions.
- **Streaming binary build timeout increased to 30 minutes.** The cross-compilation step for the streaming server binary was hitting the default 20-minute timeout on slower runners.

## [0.42.0] - 2026-05-06

### Added
- **Kitty Terminal Graphics Protocol — query response (Phase 3 of 3)**: `KittyParser` now stores the `q=` (quietness) parameter as `pub quietness: u8`. `Terminal::filter_apc_and_advance` branches on `KittyAction::Query`: when `quietness < 2`, builds an APC reply (`ESC _ G [i={id};] OK ESC \`) and appends it to `response_buffer` (the same buffer ENQ/DA1/DSR replies use). `q=2` suppresses the reply. Echoes the inbound `i=<id>` if present; emits `\x1b_G;OK\x1b\\` if not. Four new tests (`query_emits_ok_response_on_response_buffer`, `quiet_mode_query_suppresses_response`, `query_without_image_id_emits_ok_without_id`, `transmit_does_not_emit_response`).
- **Kitty Terminal Graphics Protocol — APC ingestion (Phase 1 of 3)** _(Phase 2 lives in par-term/par-term-render — see par-term CHANGELOG)_: `Terminal::process` now intercepts Kitty TGP APC sequences (`ESC _ G ... ST`) before they reach `vte::Parser`. The previously orphaned `KittyParser` (in `src/graphics/kitty.rs`) is now wired into the byte stream:
  - New module `src/terminal/apc_filter.rs` implements a streaming byte-level state-machine pre-filter. Only APCs starting with `_G` (Kitty graphics) are intercepted; all others (`_X…`, etc.) pass through unchanged so that `vte` continues to silently swallow them per `State::SosPmApcString` semantics.
  - The pre-filter is necessary because `vte` 0.15 does not deliver APC payload bytes to `Perform` — `SosPmApcString` only handles state transitions, never invoking a payload callback.
  - Handles APC sequences split across multiple `process()` calls (the common case — Kitty TGP messages typically arrive as 4 KB chunks, each in its own APC), both `\x1b\\` (7-bit ST) and `\x9c` (C1 ST) terminators, and `ESC` bytes within payloads.
  - Completed Kitty APC payloads are forwarded to `KittyParser::parse_chunk`; on the final chunk (no `m=1`), `build_graphic` commits transmitted images and virtual placements (`U=1`) into `GraphicsStore`. Malformed APCs reset the parser and are silently dropped (no panic).
  - Three new fields on `Terminal`: `apc_filter_state`, `apc_buffer`, `kitty_parser`.
  - 10 unit tests for the filter state machine + 5 integration tests against `Terminal::process` covering split sequences, surrounding-text passthrough, and non-Kitty APC ignore.
- Phase 2 (rendering placeholder cells via virtual-placement lookup, in par-term-render) and Phase 3 (responding `OK` to TGP query commands so terminal-detection probes succeed) are still required for end-to-end visible Kitty TGP support.

### Fixed
- **Kitty TGP placeholder rendering: par-term hang on first display.** Writing an N×M Kitty TGP placeholder rectangle (e.g. 40×20 = 800 cells × 4 chars per cell = 3200 prints + 2400 combining-mark insertions) ran the default NFC Unicode normalization on every char and every combining-mark insertion, allocating thousands of intermediate `String`s and recalculating grapheme widths inside `Terminal::process` while the terminal write lock was held. par-term's renderer (which `try_read`s the same lock per pane per frame) saw the lock contested for hundreds of milliseconds and skipped frames; from the user's perspective every tab froze. Added a fast-path bypass: `print` skips normalization for `\u{10EEEE}` (the Kitty placeholder char) and `write_char`'s combining-mark branch skips normalization + width recalc when the target cell's base char is `\u{10EEEE}`. These cells encode an image ID, not real text, so the work was pure waste. New benchmark test `placeholder_cells_render_at_scale_quickly` asserts 40×20 placeholder ingestion completes in < 200 ms (in practice well under 10 ms). Plus `placeholder_cells_skip_unicode_normalization` pins down the cell-shape invariant.

### Changed
- **Dependency updates across all three sub-projects**:
  - **Rust** (`Cargo.toml`): bumped tokio 1.51→1.52, axum 0.8.8→0.8.9, tower-http 0.6.8→0.6.10, rustls 0.23.37→0.23.40, reqwest 0.13.2→0.13.3, bitflags 2.11.0→2.11.1, clap 4.6.0→4.6.1, uuid 1.23→1.23.1, libc 0.2.184→0.2.186.
  - **Python** (`pyproject.toml`): bumped rich 14→15, ruff 0.15.10→0.15.12, pyright 1.1.408→1.1.409, pre-commit 4.5.1→4.6.0.
  - **Frontend** (`web-terminal-frontend/package.json`): bumped next 16.2.3→16.2.5, react/react-dom 19.2.5→19.2.6, eslint 9→10, typescript 6.0.2→6.0.3, tailwindcss 4.2.2→4.2.4, @bufbuild/buf 1.67→1.69, @bufbuild/protobuf 2.11→2.12, autoprefixer 10.4→10.5, postcss 8.5.9→8.5.14.

## [0.41.1] - 2026-04-11

### Changed
- **Dependency updates across all three sub-projects**:
  - **Rust** (`Cargo.toml`): bumped pyo3 0.28.2→0.28.3, tokio 1.50→1.51, tokio-tungstenite 0.28→0.29, clap 4.5.60→4.6.0, uuid 1.22→1.23, image 0.25.9→0.25.10, swash 0.2.6→0.2.7, nix 0.31→0.31.2, proptest 1.10→1.11, tempfile 3.26→3.27, plus patch bumps (libc, subtle, zeroize, sysinfo, tar, tracing-subscriber, unicode-segmentation). No source changes required — all API-compatible.
  - **Python** (`pyproject.toml`): bumped pillow 12.1.1→12.2.0, maturin 1.12.6→1.13.1, pytest 9.0.2→9.0.3, rich 14.3.3→14.3.4, ruff 0.15.5→0.15.10.
  - **Frontend** (`web-terminal-frontend/package.json`): bumped next 16.1.6→16.2.3, react/react-dom 19.2.4→19.2.5, typescript 5.9.3→6.0.2 (major), @bufbuild/buf 1.66→1.67, @tailwindcss/postcss + tailwindcss 4.2.1→4.2.2, @types/node 25.3→25.6, postcss 8.5.8→8.5.9.

### Fixed
- **Flaky coprocess tests**: Replaced fixed `thread::sleep(200ms)` with a `poll_until(timeout_ms, fn)` helper (10ms poll interval, 2s deadline) throughout the coprocess test suite in both `src/coprocess.rs` and `tests/test_coprocess.rs`. Fixed-duration sleeps raced the OS scheduler under parallel CPU contention, causing intermittent `Some(true) vs Some(false)` status failures on loaded CI boxes. The full suite now passes 3/3 back-to-back runs. Affects: `test_coprocess_write_read`, `test_coprocess_feed_output`, `test_coprocess_dead_process`, `test_coprocess_stderr_capture`, `test_coprocess_auto_cleanup_never_policy`, `test_coprocess_restart_always_policy`, `test_coprocess_restart_on_failure_clean_exit`, `test_coprocess_restart_on_failure_nonzero_exit`, `test_coprocess_no_copy_output`.
- **Terminal.tsx reconnect forward-reference**: `scheduleRetry` called `connect()` before `connect` was declared (TDZ / `react-hooks/refs` violation). Introduced a `connectRef` forward-reference synced via `useEffect` so scheduled reconnects always invoke the current closure.
- **TerminalDebug.tsx ref-during-render**: The debug overlay read `debugLogs.current.length` during render, which React disallows. Replaced with a `logCount` state updated when logs are pushed.
- **page.tsx inline component definition**: `StatusIndicator` was defined inside the `Home` component body, so it was re-created every render and would reset its own state. Hoisted to module scope alongside a new `STATUS_CONFIG` table and now takes `status` as a prop.
- **Flaky `test_ioctl_returns_updated_size`**: Replaced fixed `time.sleep(0.3)` / `time.sleep(0.5)` windows with a `wait_for_either` poll loop (50ms interval, 5s deadline) that watches the subprocess log file for SIGWINCH / poll markers. Under CPU contention the fixed sleeps raced the scheduler and the spawned Python interpreter had not yet written its log line.
- **Slow `test_very_large_terminal`**: Added `@pytest.mark.timeout(30)` override — rendering a 200×100 (20k-cell) terminal to PNG legitimately takes 6-9s under CPU contention, exceeding the 5s global pytest timeout. The default stays tight for fast tests.

### Build / Tooling
- **Frontend lint migrated from `next lint` to standalone ESLint**: Next.js 16 removed the `next lint` subcommand. Added `eslint@^9.39.4` + `eslint-config-next@^16.2.3` as dev dependencies, replaced legacy `.eslintrc.json` with flat-config `eslint.config.mjs`, and updated the `lint` npm script to `eslint .`. Downgraded two compiler-focused `react-hooks` v7 rules (`set-state-in-effect`, `preserve-manual-memoization`) from `error` to `warn` — they are calibrated for React-Compiler codebases and fire false-positives on idiomatic Next.js SSR hydration and pre-Compiler useCallback patterns.

## [0.41.0] - 2026-03-11

### Added
- **`TriggerAction::SplitPane` and `ActionResult::SplitPane`**: New trigger action that instructs the frontend to open a split pane. Supports `direction` (`horizontal`/`vertical`), `focus_new_pane`, `target` (`active`/`source`), and optional `command` (either `SendText` or `InitialCommand`). Polled via `poll_action_results()` which returns dicts with `type="split_pane"`.
  - New supporting types: `TriggerSplitDirection`, `TriggerSplitTarget`, `TriggerSplitCommand` (all re-exported from `terminal` module)
  - Python `TriggerAction("split_pane", {...})` fully handled: `to_trigger_action()` parses all params; `poll_action_results()` serialises the result dict
  - Python binding `split_pane()` added to expose the action from Python

## [0.40.0] - 2026-03-08

### Added
- **VT100 ACS (Alternate Character Set) line-drawing support**: Applications like tmux that fall back from UTF-8 to ACS line-drawing now render correct box-drawing glyphs instead of raw ASCII letters.
  - `ESC ( 0` / `ESC ( B`: designate G0 charset as DEC Line Drawing / ASCII
  - `ESC ) 0` / `ESC ) B`: designate G1 charset as DEC Line Drawing / ASCII
  - `SO` (0x0E) / `SI` (0x0F): shift active charset to G1 / G0
  - Full 22-entry ACS→Unicode mapping applied in `write_char` when active charset is DecLineDrawing

### Fixed
- **Streaming `result_large_err` suppression**: Added self-documenting comments explaining why `#[allow(clippy::result_large_err)]` is used in WebSocket handshake callbacks — the `ErrorResponse` type is fixed by the tungstenite `Callback` trait and cannot be reduced or boxed.

## [0.39.8] - 2026-03-05

### Fixed
- **PTY env var stripping**: Fixed the re-apply loop to skip `DROP_VARS` so that stripped multiplexer env vars (TMUX, TMUX_PANE, STY, WINDOW, COLUMNS, LINES) are not re-added by the parent env re-application step.

## [0.39.7] - 2026-03-05

### Fixed
- **PTY env var stripping**: Fixed environment variable removal for multiplexer vars (TMUX, TMUX_PANE, STY, WINDOW, COLUMNS, LINES). The previous approach of skipping vars during iteration didn't work because `CommandBuilder::new()` pre-loads the full parent environment via `get_base_env()`. Now uses `env_remove()` to explicitly remove unwanted vars after base env is loaded.

## [0.39.6] - 2026-03-05

### Fixed
- **Streaming `screen_cleared` subscription**: Python clients could not subscribe to `"screen_cleared"` events via the streaming protocol because the string-to-EventType mapping was missing. The reverse conversion (EventType to string) worked, so events were emitted but unsubscribable.

### Docs
- Updated README, SECURITY.md, and CROSS_PLATFORM.md to document PTY multiplexer env var stripping (TMUX, TMUX_PANE, STY, WINDOW)

## [0.39.5] - 2026-03-04

### Added
- **`child_pid()` on `PtySession`**: New method that returns the PID of the spawned child process (shell or command) as `int | None`. Useful for process management (sending signals, monitoring, etc.).

### Fixed
- **PTY environment leakage**: Child PTY processes now strip tmux (`TMUX`, `TMUX_PANE`) and GNU Screen (`STY`, `WINDOW`) environment variables from the parent. Previously these leaked into spawned shells, causing tools like fzf to render in the parent multiplexer pane instead of the embedded PTY.

## [0.39.4] - 2026-03-04

### Added
- **ScreenCleared terminal event**: New event emitted when ED 2J (clear screen) or ED 3J (clear screen + scrollback) is received. Frontends can use this to invalidate scrollback zone/mark metadata so the scrollbar stays consistent with terminal state.
  - New `TerminalEvent::ScreenCleared { include_scrollback: bool }` variant
  - New `poll_screen_cleared_events()` method on Terminal to drain these events
  - Streaming protocol support with new `screen_cleared` message type and `ScreenCleared` event subscription
  - Python binding: `poll_screen_cleared_events()` returns `list[bool]`
- **OSC 133;C command extraction**: Shell integration now extracts the command text from `OSC 133;C;<command>` sequences sent by shell scripts before command execution markers.

### Changed
- **Login shell detection**: PTY session now detects `-l`/`--login` flags for shell spawning. (Note: portable-pty's CommandBuilder uses args[0] for both path resolution and arg0, so `$0` shows the shell path rather than `-bash`; the `-l` flag provides full login shell behavior regardless.)
- Added `nix` dependency (0.29) with process/term/signal features for Unix platforms

## [0.39.3] - 2026-02-25

### Fixed
- **Graphics scrollback clearing**: When clearing the terminal screen (ED 2) or screen including scrollback (ED 3), graphics stored in the scrollback buffer are now also properly cleared. Previously, only the active graphics were cleared, leaving orphaned graphics in scrollback.

## [0.39.2] - 2026-02-22

### Added
- **GitNexus MCP Integration**: Add code intelligence skills for exploring, debugging, impact analysis, and refactoring using the GitNexus knowledge graph
  - New skill files: `exploring/SKILL.md`, `debugging/SKILL.md`, `impact-analysis/SKILL.md`, `refactoring/SKILL.md`
  - Documentation added to `CLAUDE.md` and `AGENTS.md` with tool references and workflow guides
  - `.gitnexus` cache directory added to `.gitignore`

### Fixed
- **OSC52 empty clipboard writes**: Ignore empty clipboard write operations instead of processing them. Some applications send empty OSC52 sequences which should be no-ops.

## [0.39.1] - 2026-02-19

### Fixed
- **Increase OSC data size limit**: Raise `MAX_OSC_DATA_LENGTH` from 1 MB to 128 MB to support inline images via iTerm2/Kitty protocols. The previous limit silently dropped any image whose base64-encoded OSC sequence exceeded 1 MB (~750 KB raw).

## [0.39.0] - 2026-02-15

### Security
- **Constant-time auth comparisons (S-1, S-2)**: All API key and password comparisons now use `subtle::ct_eq()` to prevent timing attacks
- **API key query parameter disabled by default (S-3)**: Add `allow_api_key_in_query` config flag (default `false`) to `StreamingConfig` and CLI `--allow-api-key-in-query`. Query param API keys are logged by proxies and leaked via Referer headers; now opt-in only
- **Coprocess command injection prevention (S-4)**: Validate coprocess commands for shell metacharacters (`|;&$` etc.), path traversal (`..`), working directory traversal, and environment variable name format
- **Shell path validation (S-5)**: Validate `$SHELL` environment variable points to an existing file before use; fallback to `/bin/sh`
- **Image/graphics size limits (S-6 through S-9)**: Add `MAX_IMAGE_DIMENSION`, `MAX_IMAGE_DATA_SIZE`, `MAX_SIXEL_DIMENSION`, `MAX_SIXEL_COLORS`, and `GraphicsLimits` struct with configurable bounds for all graphics protocols
- **OSC string length limit (S-10)**: Add `MAX_OSC_DATA_LENGTH` (1MB) to reject oversized escape sequences
- **Clipboard size limit (S-11)**: Add `MAX_CLIPBOARD_CONTENT_SIZE` (10MB) to bound clipboard memory usage
- **TLS key permission validation (S-12)**: Warn on Unix when TLS private key file has group/world-readable permissions
- **Password file permission validation (S-13)**: Warn on Unix when htpasswd file has insecure permissions
- **Password memory zeroization (S-14)**: Add `zeroize` crate; `PasswordConfig` now zeroizes sensitive data on drop to prevent credential leakage in memory dumps
- **FFI safety documentation (S-15)**: Strengthen `SharedState::from_terminal()` safety contract documentation with pointer lifetime and exclusivity requirements
- **NaN handling in image size (Q-10)**: Image size `is_auto()` now handles NaN/Infinity correctly

### Fixed
- **Observer dispatch race (D-1)**: Fix duplicate event dispatch when `process()` is called multiple times before `poll_events()`. Events are now tracked with a dispatch index to prevent re-delivery to observers
- **Tab stop resize validation (Q-8)**: Guard tab stop array operations against zero-width terminal resize
- **Origin mode underflow (Q-9)**: Use `saturating_sub` in cursor positioning to prevent underflow when scroll region bounds are invalid
- **Scroll region return values (Q-7)**: `scroll_region_up()`/`scroll_region_down()` now return `bool` indicating success/failure
- **Clippy needless_return**: Fix clippy warning in `get_default_shell()`
- **Makefile: npm → bun**: Replace all `npm` commands with `bun` in web frontend targets (`web-install`, `web-dev`, `web-build`, `web-build-static`, `web-start`, `proto-typescript`) to match the project's actual package manager
- **Makefile: Add `test-rust-streaming` target**: Streaming Rust tests (69 additional tests) were never run; new target added to `test`, `checkall`, and help text
- **Missing `websockets` dev dependency**: Add `websockets` to `pyproject.toml` dev dependencies so streaming Python tests actually run instead of silently skipping
- **Streaming test skip guard**: Fix `test_streaming.py` skip logic to catch `RuntimeError`/`TypeError` (not just `ImportError`) when the streaming feature isn't compiled, so tests skip gracefully with `make dev`

### Added
- **Streaming server unit tests (T-1)**: 47 new tests for `validate_terminal_size()`, `HttpBasicAuthConfig::verify()`, `SessionRegistry` lifecycle, `StreamingConfig` defaults, and `ApiAuthConfig::is_configured()`
- **Recording system tests (T-5)**: 13 new tests for recording lifecycle, timestamp accuracy, event type recording, asciicast/JSON export format validation, and session ID uniqueness
- **Event system tests (T-4)**: 29 new tests for `TerminalEvent::kind()` exhaustive variant coverage, event queuing through `process()`/`poll_events()`, and event struct validation
- **HTML export tests (T-6)**: 20 new tests (up from 2) for text rendering, HTML escaping, text attributes (bold/italic/underline/strikethrough/dim/blink/hidden/reverse), color rendering, and wide characters
- **Search tests (T-8)**: 24 new tests (up from 2) for regex patterns, search options, match navigation, scrollback search, Unicode support, and API coverage
- **Python binding: `poll_upload_requests()`**: Add missing Python binding for `Terminal.poll_upload_requests()` to drain pending upload request events

### Changed
- **Dependencies**: Update all Rust, Python, and Node dependencies to latest versions. Notable: sysinfo 0.34→0.38, pyo3 0.28.1, ruff 0.15, maturin 1.12
- **MSRV**: Bump minimum supported Rust version from 1.75 to 1.88 (required by sysinfo 0.38)
- **Clippy auto-fix**: Replace `map_or(true, ...)` with `is_none_or(...)` in image deletion (Rust 1.82+)
- **CLAUDE.md**: Improve developer guidance with build commands, single-test examples, architecture overview, streaming protocol layers, and feature flags documentation

### Documentation
- **Instant Replay guide (DOC-1)**: New `docs/INSTANT_REPLAY.md` with SnapshotManager configuration, ReplaySession navigation, Python API reference, and memory tuning
- **C/C++ FFI guide (DOC-2)**: New `docs/FFI_GUIDE.md` with SharedCell/SharedState types, memory ownership rules, build instructions, and C code examples
- **Observer patterns guide (DOC-7)**: New `docs/OBSERVERS.md` with Rust/Python observer implementation, event categories, subscription filtering, and thread safety
- **Streaming Python examples (DOC-4)**: Add Python integration section to `docs/STREAMING.md` with StreamingConfig, TLS, and API key examples
- **README deduplication (DOC-5)**: Consolidate verbose "What's New" sections into brief summary pointing to CHANGELOG.md
- **BUILDING.md version fix (DOC-6)**: Remove stale version reference from title

## [0.38.0] - 2026-02-14

### Added
- **Instant Replay**: Add Instant Replay system with cell-level terminal snapshots, input-stream delta recording, and timeline navigation (Issue #47)
  - `TerminalSnapshot` and `GridSnapshot` structs capture complete terminal state (grids, cursors, colors, attributes, modes, scroll regions, tab stops) with memory size estimation
  - `Terminal::capture_snapshot()` and `Terminal::restore_from_snapshot()` for point-in-time state capture and restore
  - `SnapshotManager` manages a rolling buffer of snapshots with size-based eviction (default 4 MiB budget, 30-second interval), input-stream recording, `reconstruct_at()` for delta replay, and `find_entry_for_timestamp()` for timestamp-based lookup
  - `ReplaySession` provides timeline navigation with `seek_to()`, `step_forward()`, `step_backward()`, `seek_to_start()`, `seek_to_end()`, `seek_to_timestamp()`, `next_entry()`, and `previous_entry()`
  - Python binding: `capture_replay_snapshot()` returns dict with `timestamp`, `cols`, `rows`, `estimated_size_bytes`
- **General-Purpose File Transfer (OSC 1337 File= with inline=0)**: Full file download/upload support via the iTerm2 OSC 1337 `File=` protocol
  - New `FileTransfer` type and `FileTransferManager` with bounded ring buffer for completed transfers (default 32 entries, 50MB max size)
  - Downloads (`inline=0`): Base64 payload decoded, progress tracked, raw bytes stored for frontend retrieval via `take_completed_transfer()`
  - Multipart downloads (`MultipartFile`/`FilePart`): Chunked transfers routed through `FileTransferManager` with per-chunk progress events
  - Single-file downloads: Complete file received and stored in one step
  - Inline images (`inline=1`): Existing graphics path unchanged (no regression)
- **RequestUpload Protocol (OSC 1337 RequestUpload)**: Terminal-to-host file upload support
  - Host sends `RequestUpload=format=tgz`, terminal emits `upload_requested` event
  - Frontend responds via `send_upload_data(data)` (writes `ok\n` + base64 to PTY) or `cancel_upload()` (writes abort)
- **File Transfer Terminal Events**: Five new `TerminalEvent` variants: `FileTransferStarted`, `FileTransferProgress`, `FileTransferCompleted`, `FileTransferFailed`, `UploadRequested`
  - All events routed to `on_screen_event()` in the observer system
  - New `EventKind` variants for subscription filtering: `FileTransferStarted`, `FileTransferProgress`, `FileTransferCompleted`, `FileTransferFailed`, `UploadRequested`
- **Python Bindings: File Transfer API**: 9 new methods on both `Terminal` and `PtyTerminal`
  - Query: `get_active_transfers()`, `get_completed_transfers()`, `get_transfer(id)`
  - Retrieve: `take_completed_transfer(id)` (includes raw `data` bytes)
  - Control: `cancel_file_transfer(id)`, `send_upload_data(data)`, `cancel_upload()`
  - Config: `set_max_transfer_size(bytes)`, `get_max_transfer_size()`
- **Python Bindings: File Transfer Observer Events**: Observer callbacks receive `file_transfer_started`, `file_transfer_progress`, `file_transfer_completed`, `file_transfer_failed`, and `upload_requested` event dicts
- **Streaming Protocol: File Transfer Events**: Five new protobuf messages (`FileTransferStarted`, `FileTransferProgress`, `FileTransferCompleted`, `FileTransferFailed`, `UploadRequested`) and event types (`EVENT_TYPE_FILE_TRANSFER_STARTED=20` through `EVENT_TYPE_UPLOAD_REQUESTED=24`) for real-time WebSocket delivery
- **Python Bindings: Streaming File Transfer**: `encode_server_message()` and `decode_server_message()` support all 5 file transfer message types
- **Terminal Observer API**: New `TerminalObserver` trait enables push-based event delivery with deferred dispatch (events dispatched after `process()` returns). Supports category-specific callbacks (`on_zone_event`, `on_command_event`, `on_environment_event`, `on_screen_event`) plus catch-all `on_event`. Observer panic isolation via `catch_unwind` prevents one bad observer from crashing the terminal
- **Terminal Observer API: Subscription Filtering**: Observers can implement `subscriptions()` to receive only specific event kinds, avoiding unnecessary dispatch overhead
- **C-Compatible FFI**: New `SharedState` and `SharedCell` `#[repr(C)]` types provide a frozen snapshot of terminal state (dimensions, cursor, title, CWD, screen content with per-cell text/color/attributes) for C/C++ consumers. C API: `terminal_get_state()`, `terminal_free_state()`, `terminal_add_observer()`, `terminal_remove_observer()`
- **C FFI Observer Vtable**: `TerminalObserverVtable` struct with function pointers enables C consumers to register observers with `user_data` context pointer
- **Python Bindings: Sync Observer**: `Terminal.add_observer(callback, kinds=None)` registers a Python callable that receives event dicts after each `process()` call. Optional `kinds` parameter filters by event type
- **Python Bindings: Async Observer**: `Terminal.add_async_observer(kinds=None)` returns `(observer_id, asyncio.Queue)` tuple for async event consumption via `await queue.get()`
- **Python Bindings: Observer Management**: `Terminal.remove_observer(id)` and `Terminal.observer_count()` for observer lifecycle management
- **Python Convenience Wrappers**: `on_command_complete()`, `on_zone_change()`, `on_cwd_change()`, `on_title_change()`, `on_bell()` in `par_term_emu_core_rust.observers` module for common observer patterns

## [0.37.0] - 2026-02-13

### Added
- **Streaming Server: API Key Authentication**: New `api_key` field on `StreamingConfig` enables API key authentication for API routes (`/ws`, `/sessions`, `/stats`) while leaving static files (web frontend) unprotected. Accepted via `Authorization: Bearer <key>`, `X-API-Key: <key>` header, or `?api_key=<key>` query parameter. When both API key and HTTP Basic Auth are configured, either satisfies authentication. Wired from existing `--api-key` CLI flag (env: `PAR_TERM_API_KEY`). WebSocket-only server modes also validate auth during handshake
- **Streaming Server: Unified Auth Middleware**: New `ApiAuthConfig` struct and `api_auth_middleware` replace the old `basic_auth_middleware`, supporting both API key and HTTP Basic Auth in a single middleware. Auth is applied only to API routes via axum nested router pattern
- **Web Frontend: API Key Passthrough**: `getDefaultWsUrl()` now reads `api_key` from the page URL query params and appends it to the WebSocket URL, enabling `http://server:8099/?api_key=secret` to auto-authenticate the WS connection
- **Python Bindings: API Key Config**: `PyStreamingConfig` now exposes `api_key` as a constructor param (`api_key=None`) and getter/setter property. Masked as `api_key=***` in `__repr__`
- **Streaming Server: System Resource Statistics**: New optional system stats collection pushes CPU, memory, disk, network, and load average data to subscribed WebSocket clients. Enabled via `--enable-system-stats` CLI flag (env: `PAR_TERM_ENABLE_SYSTEM_STATS`) with configurable interval via `--system-stats-interval` (default 5s, env: `PAR_TERM_SYSTEM_STATS_INTERVAL`). Disabled by default
- **Streaming Server: Dedicated `/stats` Endpoint**: New WebSocket endpoint at `/stats` streams system stats as JSON to connected clients without requiring a terminal session. Requires `--enable-system-stats` flag. Provides CPU, memory, disk, network, load average, and host info at the configured interval
- **Streaming Protocol: `SystemStats` Message**: New `system_stats` server message type with nested `CpuStats`, `MemoryStats`, `DiskStats`, `NetworkInterfaceStats`, and `LoadAverage` structures. Includes static host info (hostname, OS name/version, kernel version) and dynamic metrics (CPU usage, memory, disk space, network I/O, load averages, uptime)
- **Streaming Protocol: `system_stats` Event Type**: New `EVENT_TYPE_SYSTEM_STATS = 18` for subscription filtering. Clients must subscribe to `system_stats` events to receive stats messages
- **Python Bindings: System Stats Config**: `PyStreamingConfig` now exposes `enable_system_stats` and `system_stats_interval_secs` as constructor params and getter/setter properties
- **Python Bindings: System Stats Decode**: `decode_server_message()` now returns full system stats data (cpu, memory, disks, networks, load_average, host info) as nested Python dicts/lists
- **Python Bindings: Missing Streaming Server Methods**: Added `send_cwd_changed()`, `send_trigger_matched()`, and `send_progress_bar_changed()` to `PyStreamingServer` for complete parity with Rust streaming server API
- **Kitty Graphics: Chunked Transmission**: Large images split across multiple DCS sequences are now properly accumulated and processed. Parser state persists on `Terminal` between chunks (`m=1` continues, `m=0` finalizes)
- **Kitty Graphics: Complete Delete Targets**: Implemented remaining `KittyDeleteTarget` variants — `AtCursor`, `InCell`, `OnScreen`, `ByColumn`, and `ByRow` now correctly remove graphics placements by position
- **Kitty Graphics: Placeholder Diacritics**: Unicode placeholder cells now include combining diacritics encoding row/column/MSB offsets in `Cell.combining`, enabling frontends to reconstruct full placeholder sequences
- **Screenshot: Synthetic Bold Rendering**: Bold text in screenshots is now visually emboldened using swash's `Render::embolden()` API (previously the `bold` parameter was accepted but ignored)
- **Screenshot: Font Load Failure Logging**: Emoji and CJK font load failures in the screenshot renderer now log error-level messages instead of being silently ignored

### Changed
- **Streaming Server: Auth middleware restructured** — `basic_auth_middleware` replaced by unified `api_auth_middleware` that handles both API key and HTTP Basic Auth. Auth is now applied only to API routes (`/ws`, `/sessions`, `/stats`) via nested router, leaving static file serving unprotected
- **Streaming: `PyPtyTerminal` methods gated with `#[cfg(feature = "streaming")]`** instead of `#[allow(dead_code)]` for clearer intent

### Removed
- Dead `DefaultSessionFactory` struct from streaming server (defined but never instantiated)
- Unused `advance_height` field from `GlyphMetrics` in screenshot font cache
- Unused `x_advance` and `y_advance` fields from `ShapedGlyph` in screenshot shaper

## [0.36.0] - 2026-02-11

### Added
- **Streaming Server: Per-Session Client Limits**: New `--max-clients-per-session` CLI flag and `PAR_TERM_MAX_CLIENTS_PER_SESSION` env var to cap concurrent clients per session (0 = unlimited). Enforced atomically via CAS loop in `try_add_client()`
- **Streaming Server: Input Rate Limiting**: New `--input-rate-limit` CLI flag and `PAR_TERM_INPUT_RATE_LIMIT` env var for per-client token bucket rate limiting (bytes/sec, 2x burst capacity). Applied to `Input` and `Paste` messages across all three WebSocket handlers (plain, TLS, Axum)
- **Streaming Server: Session Metrics**: New `SessionMetrics` struct tracks `messages_sent`, `bytes_sent`, `input_bytes`, `errors`, and `dropped_messages` per session with atomic counters. Metrics are included in `SessionInfo` for observability
- **Streaming Server: Terminal Size Validation**: `validate_terminal_size()` enforces bounds (2-1000 cols, 1-500 rows) on client resize requests and session creation. Invalid resize requests are logged and rejected
- **Streaming Server: Dead Session Reaping**: Session reaper now detects and cleans up sessions whose PTY process has exited and have no connected clients, via new `SessionFactory::is_session_alive()` trait method
- **Streaming Server: Broadcaster Health Check**: Reaper logs warnings when a session has active clients but no broadcast activity for 30+ seconds, aiding stalled broadcaster diagnosis
- **Streaming Server: `close_session()` Method**: New public method on `StreamingServer` handles session shutdown with delayed (500ms) factory teardown so clients receive the shutdown message
- **Streaming Server: WebSocket Query Parsing**: Plain and TLS listeners now use `accept_hdr_async` to capture URI query parameters during WebSocket handshake, enabling `?session=`, `?preset=`, and `?readonly` for non-Axum connections
- **Web Frontend: HyperlinkAdded Handler**: Terminal.tsx now handles `hyperlinkAdded` server messages, tracking hyperlinks by row and exposing an `onHyperlinkAdded` callback
- **Web Frontend: UserVarChanged Handler**: Terminal.tsx now handles `userVarChanged` server messages, maintaining a live Map of user variables and exposing an `onUserVarChanged` callback
- **Web Frontend: SelectionChanged Handler**: Terminal.tsx now handles `selectionChanged` server messages, syncing selection state to xterm.js (character and line modes) with automatic clipboard copy, and exposing an `onSelectionChanged` callback
- **Web Frontend: State Tracking**: page.tsx wires new callbacks to store hyperlinks (sliding window of 100) and user vars as React state for future UI consumption
- **Python Bindings: New Config Properties**: `PyStreamingConfig` now exposes `max_clients_per_session` and `input_rate_limit_bytes_per_sec` as constructor params and getter/setter properties
- **Shell Integration: `cursor_line` Field**: `TerminalEvent::ShellIntegrationEvent` now captures the absolute cursor line (`scrollback_len + cursor_row`) at the exact moment each OSC 133 marker is parsed. This enables correct per-marker positioning even when multiple markers arrive in a single frame
- **Shell Integration: `poll_shell_integration_events()`**: New convenience method on `Terminal` drains only shell integration events (keeping others queued), returning `ShellEvent` tuples with cursor position data
- **Shell Integration: `ShellEvent` Type Alias**: New `ShellEvent` type alias `(String, Option<String>, Option<i32>, Option<u64>, Option<usize>)` for typed shell event tuples
- **Streaming Protocol: `cursor_line` in `ShellIntegrationEvent`**: Protobuf and JSON protocol now include `cursor_line` field in shell integration events, propagated through all layers (proto, protocol, server, Python bindings)

### Fixed
- **Streaming Server: Shell Exit Deadlock**: Fixed potential deadlock when shell exits by dropping the PTY mutex guard before calling `close_session()`, and now properly notifies clients with a shutdown message
- **Streaming Server: PTY Write Error Handling**: All PTY write paths (input, mouse, focus, paste) now log errors and increment session error metrics instead of silently ignoring write failures

### Changed
- **BREAKING: `SessionState::try_add_client()`**: Now takes a `max_per_session: usize` parameter (0 = unlimited) instead of unconditionally accepting clients
- **BREAKING: `SessionInfo`**: Now includes five additional metrics fields (`messages_sent`, `bytes_sent`, `input_bytes`, `errors`, `dropped_messages`)
- **Streaming Server: Bounded Output Channel**: Output channel changed from `mpsc::unbounded_channel` to `mpsc::channel(1000)` for backpressure. All senders use `try_send()` instead of `send()`, dropping messages gracefully when the buffer is full
- **Streaming Server: Broadcast Metrics**: `SessionState::broadcast()` now tracks `messages_sent` and `dropped_messages` counters
- **Streaming Server: Idle Reaper Refactored**: Reaper now always runs (not gated by idle timeout config) to support dead session cleanup. Idle timeout reaping is conditional within the unified reaper loop

## [0.35.0] - 2026-02-10

### Fixed
- **Standalone Event Poller**: Fixed standalone mode's `poll_terminal_events()` silently dropping `ModeChanged`, `GraphicsAdded`, `HyperlinkAdded`, `UserVarChanged`, and `ProgressBarChanged` events via a `_ => {}` catch-all
- **HyperlinkAdded Event**: `TerminalEvent::HyperlinkAdded` now carries position data (`row`, `col`, `id`) and is actually emitted from the OSC 8 handler (was previously defined but never pushed to the event queue)
- **BREAKING: OSC 9;4 Progress Bar State Numbering**: Fixed `ProgressState` enum to match ConEmu/Windows Terminal spec - state 2 is now Error (was Indeterminate), state 3 is Indeterminate (was Warning), state 4 is Warning/Paused (was Error). Python `PyProgressState` discriminants updated to match
- **Python Streaming Bindings**: Added missing `encode_server_message` handlers for `cwd_changed`, `trigger_matched`, `user_var_changed`, and `progress_bar_changed` message types (decode already supported all variants)

### Added
- **XTVERSION Response**: Terminal now responds to `CSI > q` with `DCS > | par-term(version) ST`
- **DA1 OSC 52 Advertisement**: Primary Device Attributes response now includes parameter 52 to advertise OSC 52 clipboard support
- **Streaming Protocol: Mouse Input**: Clients can send mouse events (`MouseInput` message) with column, row, button, modifiers, and event type. Server translates to terminal escape sequences based on active mouse mode/encoding
- **Streaming Protocol: Focus Change**: Clients can send focus in/out events (`FocusChange` message). Server generates focus tracking escape sequences when focus tracking mode is active
- **Streaming Protocol: Paste Input**: Clients can send paste content (`PasteInput` message). Server wraps content in bracketed paste sequences when bracketed paste mode is active, or writes raw content otherwise
- **Streaming Protocol: Selection Sync**: Bidirectional selection synchronization via `SelectionChanged` (server→client) and `SelectionRequest` (client→server) messages supporting character, line, block, and word selection modes
- **Streaming Protocol: Clipboard Sharing**: Bidirectional clipboard access via `ClipboardSync` (server→client) and `ClipboardRequest` (client→server) messages for set/get operations with target support (clipboard, primary, select)
- **Streaming Protocol: Shell Integration Events**: `ShellIntegrationEvent` server message streams FinalTerm shell integration markers (`prompt_start`, `command_start`, `command_executed`, `command_finished`) with command text, exit codes, and timestamps
- **Streaming Protocol: Badge Changes**: `BadgeChanged` server message streams badge text updates from `OSC 1337 SetBadgeFormat` sequences
- **Streaming Protocol: Event Subscription**: `Subscribe` client message now fully implemented with per-client `HashSet<EventType>` filtering. Clients can subscribe to specific event types; unsubscribed events are filtered before broadcast. Applied in all 3 client loops (plain, TLS, Axum)
- **Streaming Server: New send_* Methods**: Added `send_mode_changed()`, `send_graphics_added()`, `send_hyperlink_added()`, `send_user_var_changed()`, `send_progress_bar_changed()`, `send_cursor_position()`, `send_badge_changed()`, `broadcast_to_session()` convenience methods to `StreamingServer`
- **Python Bindings: Streaming Server Methods**: All new `send_*` methods exposed on `PyStreamingServer`. New server/client message types supported in `encode`/`decode` functions
- **Web Frontend: Mouse Support**: Terminal.tsx now captures mouse events (click, release, move, scroll) and sends `MouseInput` messages when mouse tracking mode is active
- **Web Frontend: Focus Tracking**: Window focus/blur events sent as `FocusChange` messages when focus tracking mode is active
- **Web Frontend: Bracketed Paste**: Paste events intercepted and sent as `PasteInput` messages when bracketed paste mode is active
- **Web Frontend: Mode State Tracking**: `modeChanged` messages now update local state for `mouse_tracking`, `focus_tracking`, and `bracketed_paste` modes
- **New EventType Variants**: `Badge`, `Selection`, `Clipboard`, `Shell` added to subscription filtering system
- **New TerminalEvent Variants**: `BadgeChanged(Option<String>)`, `ShellIntegrationEvent { event_type, command, exit_code, timestamp }` added to core terminal event system

### Changed
- **BREAKING**: `TerminalEvent::HyperlinkAdded` changed from `HyperlinkAdded(String)` to struct variant `HyperlinkAdded { url: String, row: usize, col: usize, id: Option<u32> }`. All match sites must use struct destructuring
- **Protobuf Schema**: `proto/terminal.proto` expanded with 9 new message types and 4 new `EventType` enum values

## [0.34.0] - 2026-02-09

### Fixed
- **Terminal Mode Sync on Connect**: Clients connecting to existing streaming sessions now receive `ModeChanged` messages for all active non-default terminal modes (#31)
  - New `SessionState::build_mode_sync_messages()` sends mode state after `Connected` message in all WebSocket handlers (plain, TLS, Axum)
  - Synced modes: mouse tracking (x10/normal/button_event/any_event), mouse encoding (utf8/sgr/urxvt), bracketed paste, application cursor, focus tracking, cursor visibility, alternate screen, origin mode, insert mode, auto-wrap
  - Fixes mouse tracking and other modes not working when reconnecting to sessions where a TUI is already running
  - 16 new streaming integration tests, 13 new Rust unit tests

### Added
- **Terminal Mode Change Events**: DECSET/DECRST processing now emits `TerminalEvent::ModeChanged` events for real-time mode change broadcasting to connected clients
- **OSC 1337 RemoteHost**: Parse `RemoteHost=user@hostname` sequences for remote host integration (#29)
  - Supports `user@hostname` format (username is optional)
  - Updates `ShellIntegration` hostname and username fields
  - Treats `localhost`, `127.0.0.1`, and `::1` as local (clears hostname)
  - Emits `CwdChanged` event so frontends can react to remote host changes
  - Reuses existing streaming protocol `CwdChanged` message (no protocol changes needed)
  - `ShellIntegration` Python object now exposes `hostname` and `username` attributes
  - 14 Rust unit tests, 9 Python integration tests
- **OSC 934 Named Progress Bars**: Parse and manage multiple concurrent named progress bars (#22)
  - Protocol format: `OSC 934 ; action ; id [; key=value ...] ST` with `set`, `remove`, `remove_all` actions
  - Each bar has a unique ID, state (normal/indeterminate/warning/error), percentage (0-100), and optional label
  - New `named_progress_bars()`, `get_named_progress_bar(id)`, `set_named_progress_bar()`, `remove_named_progress_bar(id)`, `remove_all_named_progress_bars()` API (Rust and Python)
  - `ProgressBarChanged` terminal event emitted on create, update, and remove with action/id/state/percent/label
  - New `progress_bar_changed` streaming protocol message and `progress_bar` event type
  - Independent from existing OSC 9;4 single progress bar
  - 15 parser unit tests, 16 integration tests, 4 streaming tests, 17 Python integration tests
- **Unicode Normalization**: Configurable Unicode normalization (NFC/NFD/NFKC/NFKD) for text stored in terminal cells (#21)
  - New `NormalizationForm` enum with five forms: `None` (disabled), `NFC` (default), `NFD`, `NFKC`, `NFKD`
  - Terminal defaults to NFC (Canonical Composition) for consistent text storage
  - Normalization applied in VTE `print()` for decomposition and in `write_char()` for composition
  - New `normalization_form()` and `set_normalization_form(form)` Rust API
  - Python `NormalizationForm` enum (`Disabled`, `NFC`, `NFD`, `NFKC`, `NFKD`) with `Terminal.normalization_form()` and `Terminal.set_normalization_form()` methods
  - New `Cell::from_grapheme_normalized()` method for direct cell construction
  - 17 Rust unit tests, 13 Python integration tests
- **OSC 1337 SetUserVar**: Parse `SetUserVar=<name>=<base64_value>` sequences from shell integration scripts (#25)
  - Base64-decode values and store as user variables in terminal session state
  - New `get_user_var(name)` and `get_user_vars()` API (Rust and Python)
  - `UserVarChanged` terminal event emitted when a variable changes (includes old value)
  - User variables are accessible via badge session variables for format evaluation
  - New `user_var_changed` streaming protocol message and `user_var` event type
  - Python `poll_events()` / `poll_subscribed_events()` return `user_var_changed` event dicts
  - 9 Rust unit tests, 3 streaming protocol tests, 10 Python integration tests
- **Image Metadata Serialization**: Support for persisting and restoring graphics state with terminal sessions (#18)
  - New `serialization` module with `SerializableGraphic`, `GraphicsSnapshot`, and `ImageDataRef` types
  - `ImageDataRef` supports inline base64-encoded pixel data or external file path references for compact storage
  - `GraphicsStore.export_snapshot()` / `import_snapshot()` for full graphics state round-trip (placements, scrollback, animations)
  - `GraphicsStore.export_json()` / `import_json()` convenience methods for JSON serialization
  - Python `Terminal.export_graphics_json()` and `Terminal.import_graphics_json(json)` bindings
  - Added `Serialize`/`Deserialize` derives to `GraphicProtocol`, `ImageDisplayMode`, `ImageSizeUnit`, `ImageDimension`, `ImagePlacement`, `CompositionMode`, `AnimationState`, `AnimationControl`
  - Version-tagged snapshots (`GraphicsSnapshot.version`) for forward compatibility
- **Image Placement Metadata**: Parse and expose unified image placement modes from graphics protocols (#16)
  - New `ImagePlacement` struct with display mode, sizing, z-index, and sub-cell offset fields
  - New `ImageDimension` struct with unit support (auto, cells, pixels, percent)
  - **Kitty protocol**: Extracts columns/rows sizing (`c=`/`r=`), z-index for layering (`z=`), and sub-cell offsets (`x=`/`y=`)
  - **iTerm2 protocol**: Parses `width`/`height` with unit support (cells, `px`, `%`, auto), `preserveAspectRatio` flag, and `inline` flag
  - Exposed to Python via `Graphic.placement` property returning `ImagePlacement` object
  - New `ImagePlacement` and `ImageDimension` Python classes importable from the package
  - Enables frontends to implement inline/cover/contain rendering without protocol-specific logic
- **Original Image Dimensions**: All graphics protocols (Sixel, iTerm2, Kitty) now expose `original_width` and `original_height` on `TerminalGraphic` and Python `Graphic` objects
  - These preserve the original decoded pixel dimensions even when `width`/`height` change during animation
  - Enables frontends to calculate correct aspect ratios when scaling images to fit terminal cells
  - Python `Graphic.__repr__()` now includes `original_size=WxH`
- **Kitty Graphics Compression (o=z)**: Support for zlib-compressed image data in the Kitty graphics protocol
  - Parses the `o=z` transmission parameter to detect zlib-compressed payloads
  - Automatically decompresses data before pixel decoding (transparent to consumers)
  - Works with all transmission types: direct, file, temp file, and chunked transfers
  - New `was_compressed` metadata flag on `TerminalGraphic` for diagnostics/logging
  - Python `Graphic.was_compressed` property exposed for frontend diagnostics
  - 8 new Rust tests covering compression parsing, decompression, chunked transfers, and error handling

### Changed
- **Dependencies**: Migrated to PyO3 0.28 from 0.23, updating all Python binding patterns to the latest API
- **Dependencies**: `flate2` is now a non-optional dependency (previously only available under `streaming` feature), required for Kitty `o=z` decompression
- **Dependencies**: Added `unicode-normalization` v0.1.25 for Unicode text normalization support
- **Dependencies**: Updated multiple dependency versions across the project

## [0.33.0] - 2026-02-06

### Added
- **Multi-Session Streaming Support**: The streaming server now supports multiple concurrent terminal sessions
  - New `SessionState` struct encapsulates per-session terminal, broadcast channels, PTY writer, and client tracking
  - New `SessionFactory` trait allows custom session creation (e.g., PTY-backed sessions in the binary server)
  - New `SessionRegistry` for managing active sessions with idle timeout reaping
  - New `ConnectionParams` struct for passing session/preset/client parameters during WebSocket upgrade
  - New `SessionInfo` struct exposes session metadata (id, client_count, created_at)
  - Clients connect to specific sessions via `?session=<id>` query parameter
  - New sessions are auto-created on first connection (or via preset with `?preset=<name>`)
  - Idle sessions (no connected clients) are automatically reaped after configurable timeout
- **Shell Presets**: Named shell presets allow clients to request specific shell environments
  - CLI: `--preset python=python3 --preset node=node`
  - Clients connect with `?preset=name` to spawn a session with that shell
- **Client Identity & Read-Only Mode**: Connected message now includes `client_id` and `readonly` fields
  - Each WebSocket client receives a unique identifier
  - Read-only status is communicated in the connection handshake
- **Streaming Config Extensions**: New configuration options for multi-session support
  - `max_sessions`: Maximum concurrent sessions (default: 10)
  - `session_idle_timeout`: Seconds before idle sessions are reaped (default: 900, 0 = never)
  - `presets`: HashMap of preset name → shell command
- **New Error Variants**: `MaxSessionsReached`, `SessionNotFound`, `InvalidPreset` in `StreamingError`
- **Python Bindings**: `StreamingConfig` gains `max_sessions` and `session_idle_timeout` getters/setters; `decode_server_message` includes `client_id` and `readonly` in Connected dict
- **New Public Exports**: `ConnectionParams`, `SessionFactory`, `SessionFactoryResult`, `SessionInfo`, `SessionRegistry`, `SessionState` from `streaming` module
- **Streaming Protocol: ModeChanged Events**: New `ModeChanged` message notifies clients when terminal modes change
  - Includes `mode` name (e.g., "cursor_visible", "mouse_tracking", "bracketed_paste") and `enabled` boolean
  - New `EVENT_TYPE_MODE` subscription type; Python subscription name: `"mode"`
- **Streaming Protocol: GraphicsAdded Events**: New `GraphicsAdded` message notifies clients when images are added to the terminal
  - Includes `row` position and optional `format` ("sixel", "iterm2", "kitty")
  - New `EVENT_TYPE_GRAPHICS` subscription type; Python subscription name: `"graphics"`
- **Streaming Protocol: HyperlinkAdded Events**: New `HyperlinkAdded` message notifies clients when OSC 8 hyperlinks are added
  - Includes `url`, `row`, `col`, and optional `id` from the OSC 8 protocol
  - New `EVENT_TYPE_HYPERLINK` subscription type; Python subscription name: `"hyperlink"`

### Changed
- **Breaking**: `StreamingConfig` has three new required fields: `max_sessions`, `session_idle_timeout`, `presets`
- **Breaking**: `ServerMessage::Connected` variant has two new fields: `client_id: Option<String>`, `readonly: Option<bool>`
- **Breaking**: `ServerMessage::connected_full()` constructor takes two additional parameters (`client_id`, `readonly`)
- **Breaking**: `StreamingServer` internals refactored from single-terminal to multi-session architecture
- Binary server (`par-term-streamer`) refactored to use `BinarySessionFactory` for per-session PTY management

## [0.32.0] - 2026-02-06

### Added
- **Coprocess Restart Policies**: Coprocesses can now automatically restart when they exit
  - New `RestartPolicy` enum: `Never` (default), `Always`, `OnFailure` (restart on non-zero exit)
  - Configurable restart delay via `restart_delay_ms` to prevent tight restart loops
  - Dead coprocesses with `Never` policy are automatically cleaned up from the manager
  - Restart logic runs during `feed_output()` polling cycle
- **Coprocess Stderr Capture**: Coprocess stderr is now captured in a separate buffer
  - New `read_coprocess_errors()` / `read_errors()` methods on `PtySession` and `CoprocessManager`
  - Stderr is read via a dedicated background thread (previously discarded)
- **Trigger Notify/MarkLine as Frontend Events**: `Notify` and `MarkLine` trigger actions now emit `ActionResult` events instead of directly calling internal notification/bookmark methods
  - Frontends receive `notify` and `mark_line` entries from `poll_action_results()` with trigger_id, allowing custom handling
  - `MarkLine` action now supports an optional `color` parameter as RGB tuple (e.g., `"color": "255,128,0"`)
- **Streaming Protocol: Action Result Events**: New `ActionNotify` and `ActionMarkLine` messages in the streaming protocol
  - Frontends subscribed to `action` events receive trigger-driven notifications and line marks
  - New protobuf messages: `ActionNotify`, `ActionMarkLine` with `Color` support
  - New `EVENT_TYPE_ACTION` subscription type
  - New server methods: `send_action_notify()`, `send_action_mark_line()`
- **Python Bindings**: Updated `CoprocessConfig` with `restart_policy` and `restart_delay_ms` parameters; added `read_coprocess_errors()` to `PtyTerminal`; added `send_action_notify()` and `send_action_mark_line()` to `StreamingServer`

### Changed
- **Breaking**: `CoprocessManager.feed_output()` now takes `&mut self` instead of `&self` (manages restart lifecycle)
- **Breaking**: `Notify` and `MarkLine` trigger actions no longer directly enqueue notifications or add bookmarks; they emit `ActionResult` events for frontend handling via `poll_action_results()`
- **Breaking**: `TriggerAction::MarkLine` now has an additional `color: Option<(u8, u8, u8)>` field

## [0.31.1] - 2026-02-05

### Fixed
- **Trigger Column Mapping**: `TriggerMatch.col` and `TriggerMatch.end_col` now correctly report grid column positions for text containing wide characters (CJK, emoji) and multi-byte UTF-8 characters
  - Previously, regex byte offsets were used directly, producing incorrect column values for non-ASCII text
  - New `byte_offsets_to_grid_cols()` converts regex byte offsets to proper grid column indices
  - New `build_char_to_grid_col_map()` builds character-to-grid-column mapping that accounts for wide character spacers and combining characters
  - `process_trigger_scans()` now passes the column mapping to `scan_line()` for accurate position reporting
  - Trigger highlights now correctly overlay the matched text even with wide/combining characters in the same row

## [0.31.0] - 2026-02-05

### Fixed
- **Streaming Server Event Dispatch**: Terminal events (bell, title change, CWD change, trigger matches) are now actually dispatched to streaming clients
  - Added `poll_terminal_events()` task to streaming server that polls terminal events at 20Hz
  - Bell events, title changes, resize events, CWD changes, and trigger matches are now broadcast to all connected WebSocket clients
  - Previously, broadcast helpers existed but were never called

### Added
- **Streaming Protocol: CWD Change Events (OSC 7)**: New `CwdChanged` message in the streaming protocol
  - Includes old_cwd, new_cwd, hostname, username, and timestamp fields
  - New `EVENT_TYPE_CWD` subscription type
- **Streaming Protocol: Trigger Match Events**: New `TriggerMatched` message in the streaming protocol
  - Includes trigger_id, row, col, end_col, text, captures, and timestamp fields
  - New `EVENT_TYPE_TRIGGER` subscription type
- **Streaming: Enhanced Connected Message**: Connection handshake now includes additional terminal state
  - `badge`: Current badge text (from OSC 1337 badge format)
  - `faint_text_alpha`: Dim text alpha for SGR 2 rendering (0.0-1.0)
  - `cwd`: Current working directory (from OSC 7)
  - `modify_other_keys`: Current modifyOtherKeys mode (0-2)
- **Streaming: New broadcast helpers**: `send_cwd_changed()` and `send_trigger_matched()` on `StreamingServer`
- **Triggers & Automation (Feature 18)**: Regex-based pattern matching on terminal output with automated actions
  - `TriggerRegistry` with `RegexSet` for efficient multi-pattern matching across terminal output
  - Trigger actions: Highlight (with optional expiry), Notify, MarkLine, SetVariable (core-handled); RunCommand, PlaySound, SendText (emitted as events for frontend)
  - Capture group substitution (`$1`, `$2`, etc.) in action parameters
  - Trigger highlight overlays with time-based expiry
  - `StopPropagation` action to short-circuit remaining actions
  - New methods: `add_trigger()`, `remove_trigger()`, `set_trigger_enabled()`, `list_triggers()`, `get_trigger()`, `poll_trigger_matches()`, `process_trigger_scans()`, `get_trigger_highlights()`, `clear_trigger_highlights()`, `clear_expired_highlights()`, `poll_action_results()`
  - New event: `TriggerMatched` in `poll_events()`
- **Coprocess Management**: Run external processes alongside terminal sessions
  - `CoprocessManager` for spawning, stopping, and communicating with coprocesses
  - Automatic terminal output piping to coprocess stdin (configurable per coprocess)
  - Line-buffered stdout reading via background reader threads
  - Integrated with PTY reader thread for automatic output feeding
  - New PTY methods: `start_coprocess()`, `stop_coprocess()`, `write_to_coprocess()`, `read_from_coprocess()`, `list_coprocesses()`, `coprocess_status()`
- **Python Bindings**: Full PyO3 bindings for triggers and coprocesses
  - New classes: `Trigger`, `TriggerAction`, `TriggerMatch`, `CoprocessConfig`
  - Trigger methods on `Terminal` class
  - Coprocess methods on `PtyTerminal` class

## [0.30.0] - 2026-02-04

### Added
- **modifyOtherKeys Protocol**: XTerm extension for enhanced keyboard input reporting
  - State tracking for modifyOtherKeys mode (0=disabled, 1=special keys, 2=all keys)
  - CSI sequence parsing: `CSI > 4 ; mode m` to set mode
  - Query support: `CSI ? 4 m` returns `CSI > 4 ; mode m` response
  - New methods: `modify_other_keys_mode()` getter, `set_modify_other_keys_mode()` setter
  - Mode resets on terminal reset and alternate screen exit
  - 9 new tests for modifyOtherKeys functionality
- **Faint Text Alpha**: Configurable alpha multiplier for SGR 2 (dim/faint) text
  - New `faint_text_alpha` field in Terminal (default: 0.5 for 50% dimming)
  - New methods: `faint_text_alpha()` getter, `set_faint_text_alpha(alpha)` setter
  - Values clamped to 0.0-1.0 range
  - Propagated to screenshot renderer for consistent rendering
  - Python bindings for both Terminal and PtyTerminal classes

## [0.29.0] - 2026-02-04

### Added
- **OSC 7 Enhancements**: Percent-decoding, username/hostname parsing, port stripping, query/fragment removal, and path validation for `file://` URLs
- **Session Variable Sync**: OSC 7 now updates badge/session variables (`path`, `hostname`, `username`) so badge formats immediately reflect directory changes
- **CWD History Context**: CWD change log now records hostname and username; Python `CwdChange` exposes these fields
- **CWD Change Events**: New `TerminalEvent::CwdChanged` (and Python `cwd_changed` poll_events entry) fires on OSC 7 or manual `record_cwd_change`
- **Username Handling**: Shell integration stores optional username from `user@host` OSC 7 payloads

### Changed
- **API**: `record_cwd_change` now accepts optional `hostname` and `username` (defaults preserved); badge/session variables cleared when hostname/username unset
- **Dependencies**: Added `percent-encoding` and `url` crates for robust OSC 7 parsing

### Fixed
- **Badge Accuracy**: Badge variables `\(path)` and `\(hostname)` now stay in sync when updated via OSC 7
- **UTF-8 Paths**: Paths with spaces or Unicode characters from OSC 7 are correctly percent-decoded

## [0.28.0] - 2026-02-03

### Added
- **Badge Format Support (OSC 1337 SetBadgeFormat)**: iTerm2-style badge support for terminal overlays
  - New `badge` module with `SessionVariables` struct for session information
  - OSC 1337 SetBadgeFormat sequence parsing with base64-encoded format strings
  - Variable interpolation using `\(variable)` syntax (e.g., `\(username)@\(hostname)`)
  - Supports session prefix: `\(session.variable)` and direct: `\(variable)`
  - Built-in variables: `hostname`, `username`, `path`, `job`, `last_command`, `profile_name`, `tty`, `columns`, `rows`, `bell_count`, `selection`, `tmux_pane_title`, `session_name`, `title`
  - Custom variables via `set_custom(name, value)`
  - Security validation rejects shell injection patterns (`$()`, backticks, pipes, etc.)
  - Python bindings: `badge_format()`, `set_badge_format()`, `clear_badge_format()`, `evaluate_badge()`, `get_badge_session_variable()`, `set_badge_session_variable()`, `get_badge_session_variables()`
  - Session variables auto-sync with terminal state (title, dimensions, bell count)
  - Reference: [iTerm2 Badge Documentation](https://iterm2.com/documentation-badges.html)

### Fixed
- **Tmux Control Mode CRLF Handling**: Fixed parser to strip `\r` from `\r\n` line endings sent by tmux
- **Tmux Output Trailing Spaces**: Fixed `%output` notifications to preserve trailing spaces (regression from `.trim()` call)
- **OSC 133 Exit Code Parsing**: Fixed exit code extraction from `OSC 133 ; D ; <exit_code> ST` sequences

## [0.27.0] - 2026-02-01

### Added
- **Tmux Control Mode Auto-Detection**: Automatic detection and switching to tmux control mode
  - New `set_tmux_auto_detect(enabled)` method to enable/disable auto-detection
  - New `is_tmux_auto_detect()` method to check if auto-detection is enabled
  - Parser automatically switches to control mode when `%begin` notification is detected
  - Handles race conditions where tmux output arrives before `set_tmux_control_mode(True)` is called
  - When `set_tmux_control_mode(True)` is called, auto-detect is automatically enabled
  - Data before `%begin` is returned as `TerminalOutput` notification, allowing normal terminal display
  - Python bindings for `Terminal` class (PtyTerminal accesses via `terminal()` method)
  - Comprehensive Rust tests for auto-detection scenarios

### Changed
- `set_tmux_control_mode(true)` now also enables auto-detection for better race condition handling

## [0.26.0] - 2026-02-01

### Added
- **Session Recording Python Exports**: `RecordingEvent` and `RecordingSession` classes now exported from Python module
  - Import directly: `from par_term_emu_core_rust import RecordingEvent, RecordingSession`
  - Previously these types were registered but not exported in `__init__.py`

- **RecordingSession Enhanced API**: New properties to access recorded events and environment
  - `session.events` - List of `RecordingEvent` objects for iterating over recorded events
  - `session.env` - Dict of environment variables captured at recording start (TERM, COLS, ROWS, etc.)
  - Helper methods: `get_size()` returns (cols, rows), `get_duration_seconds()` returns float

- **RecordingEvent Properties**: Full access to event data
  - `event.timestamp` - Milliseconds since recording start
  - `event.event_type` - "Input", "Output", "Resize", or "Marker"
  - `event.data` - Raw bytes of the event
  - `event.metadata` - Optional (cols, rows) for resize events
  - `event.get_data_str()` - Helper to decode data as UTF-8 string

- **PtyTerminal Recording Methods**: Added missing recording methods to match Terminal API
  - `record_output(data)` - Record output data bytes
  - `record_input(data)` - Record input data bytes
  - `record_resize(cols, rows)` - Record terminal resize event
  - `record_marker(label)` - Add marker/bookmark to recording
  - `get_recording_session()` - Get current active recording session

### Changed
- **GitHub Workflows**: Added version consistency check that runs before all build jobs
  - Validates Cargo.toml, pyproject.toml, and __init__.py versions match
  - Fails fast before expensive builds if versions are out of sync
  - Added to both CI and deployment workflows

### Documentation
- Updated `docs/MACROS.md` with complete `RecordingSession` and `RecordingEvent` API documentation

## [0.25.0] - 2026-01-31

### Added
- **Configurable Unicode Width**: Full control over character width calculations for proper terminal alignment
  - New `UnicodeVersion` enum (Unicode9 through Unicode16, plus Auto) for version-specific width tables
  - New `AmbiguousWidth` enum (Narrow for Western, Wide for CJK) for East Asian Ambiguous characters
  - New `WidthConfig` class combining both settings with convenience constructors `WidthConfig.cjk()` and `WidthConfig.western()`
  - Terminal API: `width_config()`, `set_width_config()`, `set_ambiguous_width()`, `set_unicode_version()`, `char_width()`
  - Standalone functions: `char_width()`, `str_width()`, `char_width_cjk()`, `str_width_cjk()`, `is_east_asian_ambiguous()`
  - Python bindings for all new types and functions on both `Terminal` and `PtyTerminal`
  - Enables proper alignment for CJK text, Greek/Cyrillic letters, mathematical symbols, and box-drawing characters

## [0.24.0] - 2026-01-31

### Added
- **Configurable Unicode Width (Rust API)**: Add support for configuring the Unicode version used for character width calculations
  - New `UnicodeVersion` enum (Unicode9 through Unicode16, plus Auto) for version-specific width tables
  - New `AmbiguousWidth` enum (Narrow for Western, Wide for CJK) for East Asian Ambiguous characters
  - New `WidthConfig` struct combining both settings
  - Terminal API: `width_config()`, `set_width_config()`, `set_ambiguous_width()`, `set_unicode_version()`, `char_width()`
  - Standalone functions: `char_width()`, `str_width()`, `char_width_cjk()`, `str_width_cjk()`, `is_east_asian_ambiguous()`

## [0.23.0] - 2026-01-31

### Added
- **Configurable ENQ Answerback**: Terminal can now return a custom answerback string in response to ENQ (0x05)
  - New Rust APIs: `Terminal::answerback_string()` and `Terminal::set_answerback_string()`
  - Python bindings expose `answerback_string()` and `set_answerback_string()` on both `Terminal` and `PtyTerminal`
  - Disabled by default for security; answerback payload is delivered via the existing response buffer (`drain_responses()`)

### Fixed
- **Python Version Sync**: Bumped Python package version to match crate release and expose new answerback feature

## [0.22.1] - 2026-01-30

### Fixed
- **Search Unicode Bug**: Fixed `search()` and `search_scrollback()` returning byte offsets instead of character offsets for multi-byte Unicode text
  - `SearchMatch.col` now correctly returns the character (grapheme) column position, not the byte offset
  - `SearchMatch.length` now correctly returns the character count, not the byte length
  - `SearchMatch.text` now correctly extracts the matched text using character iteration
  - Affects text containing multi-byte characters (CJK, emoji, etc.)
  - Example: Searching for "World" in "こんにちは World" now returns `col=6` (correct) instead of `col=16` (byte offset)
  - Added comprehensive tests for Unicode search scenarios

## [0.22.0] - 2026-01-27

### Added
- **Regional Indicator Flag Emoji Support**: Proper grapheme cluster handling for flag emoji
  - Flag emoji like 🇺🇸, 🇬🇧, 🇯🇵 are now correctly combined into single cells
  - Two regional indicator codepoints are combined into one wide (2-cell) grapheme
  - Flags are stored with the first indicator as the base character and the second in the combining vector
  - Cursor correctly advances by 2 cells after writing a flag
  - Added `unicode-segmentation` crate dependency for grapheme cluster support
  - Comprehensive test suite for flag emoji in `tests/test_flag_emoji.rs`

### Fixed
- **Clippy Warning**: Fixed unnecessary unwrap warning in screenshot font_cache.rs

## [0.21.0] - 2026-01-20

### Changed
- **Migrated to `parking_lot::Mutex`**: Replaced all `std::sync::Mutex` usage with `parking_lot::Mutex` for improved performance and reliability
  - Eliminated Mutex poisoning risk across the entire library, including Python bindings and streaming server
  - Simplified lock acquisition by removing `.unwrap()` calls on lock results
  - Smaller mutex memory footprint (1 byte vs system-dependent size)
  - Faster lock/unlock operations under contention

## [0.20.1] - 2026-01-20

### Added
- **Safe Environment Variable API for Spawn Methods** (Issue #13): New methods to pass environment variables directly to spawned processes without modifying the parent process environment
  - `spawn_shell_with_env(env, cwd)` - Rust API to spawn shell with env vars and working directory
  - `spawn_with_env(command, args, env, cwd)` - Rust API to spawn command with env vars and working directory
  - Python `spawn_shell(env=None, cwd=None)` - Updated signature to accept optional env dict and cwd string
  - Safe for multi-threaded applications (Tokio) - no `unsafe { std::env::set_var() }` required
  - Backward compatible - existing code calling `spawn_shell()` without args still works
  - Env vars from method parameters override those from `set_env()` (applied last)

### Documentation
- Updated README.md with examples for the new env/cwd parameters

## [0.20.0] - 2025-12-23

### Added
- **External UI Theme File**: Web frontend UI chrome theme can now be customized after static build
  - New `theme.css` file in `web_term/` directory contains CSS custom properties
  - Edit colors without rebuilding: `--terminal-bg`, `--terminal-surface`, `--terminal-border`, `--terminal-accent`, `--terminal-text`
  - Changes take effect on page refresh - no rebuild required
  - Terminal emulator colors (ANSI palette) still controlled by server `--theme` option

### Fixed
- **Web Terminal On-Screen Keyboard Mobile Fix**: Fixed native device keyboard appearing when tapping on-screen keyboard buttons on mobile
  - Removed `focusTerminal()` call after on-screen keyboard input to prevent xterm's internal textarea from triggering native keyboard
  - Added active element blur on touch to ensure no input retains focus
  - Only focus terminal when hiding on-screen keyboard, not when showing or using it

### Changed
- **Theme Architecture**: Separated UI chrome theme from terminal emulator theme
  - UI chrome (status bar, buttons, containers) now uses external `theme.css`
  - Terminal emulator colors continue to be sent from server via protobuf

### Documentation
- Updated `docs/STREAMING.md` with new "UI Chrome Theme" section
- Updated `web-terminal-frontend/README.md` with theme customization guide
- Added theme customization to main README features list

## [0.19.5] - 2025-12-17

### Fixed
- **Streaming Server Shell Restart Input**: Fixed WebSocket client connections not receiving input after shell restart
  - PTY writer was captured once at connection time, becoming stale after shell restart
  - Now fetches the latest PTY writer each time input needs to be written
  - Ensures client keyboard input reaches the shell after any restart

## [0.19.4] - 2025-12-17

### Added
- **Python SDK Sync with Rust SDK**: Aligned Python streaming bindings with all Rust streaming features
  - `StreamingConfig.enable_http` - Enable/disable HTTP static file serving (getter/setter)
  - `StreamingConfig.web_root` - Web root directory for static files (getter/setter)
  - `StreamingServer.max_clients()` - Get maximum number of allowed clients
  - `StreamingServer.create_theme_info()` - Static method to create theme dictionaries for protocol functions
  - `encode_server_message("pong")` - Added missing pong message type support
  - `encode_server_message("connected", theme=...)` - Added theme support with name, background, foreground, normal (8 colors), bright (8 colors)

### Changed
- `StreamingConfig` constructor now accepts `enable_http` and `web_root` parameters (with backwards-compatible defaults)
- `StreamingConfig.__repr__()` now includes `enable_http` and `web_root` in output
- Updated deprecated `Python::with_gil` to `Python::attach` for PyO3 0.27 compatibility

## [0.19.3] - 2025-12-17

### Fixed
- **Shell Restart Hang**: Fixed streaming server hanging when attempting to restart the shell after exit
  - Added `cleanup_previous_session()` method to properly clean up old PTY resources before spawning new shell
  - Old writer is dropped first to unblock any blocked reads in the old reader thread
  - Old PTY pair is closed before creating new one
  - Old reader thread is waited on (with 2-second timeout) to ensure it finishes
  - Old child process is properly reaped to prevent zombie processes
  - Added detailed logging to shell restart process for easier debugging

### Security
- **Removed username from startup logs**: Streaming server no longer logs the HTTP Basic Auth username
  - Addresses CodeQL alert for cleartext logging of sensitive information (CWE-312, CWE-359, CWE-532)
  - Auth status still displayed as "ENABLED" or "DISABLED" without credential details

## [0.19.2] - 2025-12-17

### Fixed
- **Streaming Server Hang on Shell Exit**: Fixed server hanging indefinitely when the shell exits
  - Added shutdown signal mechanism using `tokio::sync::Notify` to gracefully terminate the broadcaster loop
  - The `output_broadcaster_loop` now listens for shutdown signals in its `select!` block
  - The existing `shutdown()` method now also signals the broadcaster to exit
  - Prevents the server from blocking indefinitely on `rx.recv()` when `output_tx` sender is never dropped

## [0.19.1] - 2025-12-16

### Fixed
- **Streaming Server Ping/Pong**: Fixed application-level ping/pong handling in the streaming server
  - Server was incorrectly sending WebSocket-level pong frames instead of protobuf `Pong` messages
  - Added `Pong` variant to `ServerMessage` protocol enum
  - Frontend heartbeat mechanism now properly receives pong responses
  - Fixes stale connection detection that was always failing due to missing pong responses

## [0.19.0] - 2025-12-16

### Added
- **Automatic Shell Restart**: Streaming server now automatically restarts the shell when it exits
  - Default behavior: shell is restarted automatically when it exits
  - New `--no-restart-shell` CLI option to disable automatic restart
  - New `PAR_TERM_NO_RESTART_SHELL` environment variable support
  - When restart is disabled, server exits when the shell exits
  - Shell restart preserves the PTY writer connection to streaming clients

- **Header/Footer Toggle in On-Screen Keyboard**: New layout toggle button in the keyboard header
  - Allows users to show/hide the header and footer directly from the on-screen keyboard
  - Visual indicator shows current state (blue when header/footer is visible)
  - Convenient for mobile users who want to maximize terminal space without closing the keyboard

- **Font Size Controls in On-Screen Keyboard**: Plus/minus buttons in keyboard header
  - Adjust terminal font size (8px to 32px) directly from the on-screen keyboard
  - Shows current font size between buttons
  - Buttons disabled at min/max limits

### Changed
- **StreamingServer Interior Mutability**: `set_pty_writer` now uses `&self` instead of `&mut self`
  - Enables updating PTY writer after shell restart without requiring mutable reference
  - Uses `RwLock` for thread-safe interior mutability

- **Web Frontend UI Improvements**:
  - Moved font size controls from main header to on-screen keyboard header
  - Repositioned floating toggle buttons side by side in bottom-right corner
  - Keyboard and header/footer toggle buttons now have consistent sizing

## [0.18.2] - 2025-12-15

### Added
- **Font Size Control**: User-adjustable terminal font size in web frontend
  - Plus/minus buttons in header to adjust font size (8px to 32px range)
  - Current font size displayed between buttons
  - Setting persisted to localStorage across sessions
  - Overrides automatic responsive sizing when set

- **Heartbeat/Ping Mechanism**: Stale WebSocket connection detection with automatic reconnection
  - Sends ping every 25 seconds, expects pong within 10 seconds
  - Closes and triggers reconnect on stale connections
  - Prevents "Connected" status showing for half-open sockets

### Security
- **Web Terminal Security Hardening**: Comprehensive security audit fixes for the web frontend
  - **Reverse-tabnabbing prevention**: Terminal links now open with `noopener,noreferrer` to prevent malicious links from hijacking the parent tab
  - **Zip bomb protection**: Added decompression size limits (256KB compressed, 2MB decompressed) to prevent memory exhaustion attacks
  - **Localhost probe fix**: WebSocket preconnect hints now gated to development mode only, preventing production sites from scanning localhost ports
  - **Snapshot size guard**: Added 1MB limit on screen snapshots to prevent UI freezes from oversized payloads

### Fixed
- **WebSocket URL Changes**: Changing the WebSocket URL while connected now properly disconnects and reconnects to the new server
- **Invalid URL Handling**: Invalid WebSocket URLs no longer crash the UI; displays friendly error message instead
- **Next.js Config Conflict**: Merged duplicate config files (`next.config.js` and `next.config.mjs`) into single file with `reactStrictMode` enabled
- **Toggle Button Overlap**: Moved header/footer toggle button left to avoid overlapping with scrollbar

## [0.18.1] - 2025-12-15

### Fixed
- **Web Terminal On-Screen Keyboard**: Fixed device virtual keyboard appearing when tapping on-screen keyboard buttons on mobile devices
  - Added `tabIndex={-1}` to all buttons in the on-screen keyboard component to prevent focus acquisition
  - Affects all keyboard sections: main keys, arrow keys, Ctrl shortcuts, symbol grid, macro buttons, and all UI controls

## [0.18.0] - 2025-12-14

### Added
- **Environment Variable Support**: All CLI options now support environment variables with `PAR_TERM_` prefix
  - Examples: `PAR_TERM_HOST`, `PAR_TERM_PORT`, `PAR_TERM_THEME`, `PAR_TERM_HTTP_USER`
  - Enabled via clap's `env` feature

- **HTTP Basic Authentication**: New password protection for the web frontend
  - `--http-user` - Username for HTTP Basic Auth
  - `--http-password` - Clear text password (env: `PAR_TERM_HTTP_PASSWORD`)
  - `--http-password-hash` - htpasswd format hash supporting bcrypt ($2y$), apr1 ($apr1$), SHA1 ({SHA}), MD5 crypt ($1$)
  - `--http-password-file` - Read password from file (auto-detects hash vs clear text)
  - Uses `htpasswd-verify` crate for hash verification

- **Comprehensive Streaming Test Suite**: 94 new tests for streaming functionality
  - Integration tests (`tests/test_streaming.rs`): Protocol message constructors, theme info, HTTP Basic Auth, StreamingConfig, binary protocol encoding/decoding, event types, streaming errors, JSON serialization
  - Unit tests in `broadcaster.rs`: Default implementation, client management, empty broadcaster operations
  - Unit tests in `proto.rs`: All message type encoding/decoding, Unicode content, ANSI escape sequences, event type conversions

### Changed
- **Dependencies**: Added `htpasswd-verify` and `headers` crates for HTTP Basic Auth support
- **Streaming Server**: Added `HttpBasicAuthConfig` and `PasswordConfig` types to `StreamingConfig`
- **Python Bindings**: Added exports for binary protocol functions (`encode_server_message`, `decode_server_message`, `encode_client_message`, `decode_client_message`) to `__init__.py`
- **Python Package Version**: Updated to 0.18.0 to match Cargo.toml

## [0.17.0] - 2025-12-13

### Added
- **Web Terminal Macro System**: New macro tab in the on-screen keyboard for creating and playing terminal command macros
  - Create named macros with multi-line scripts (one command per line)
  - Quick select buttons to run macros with a single tap
  - Playback with 200ms delay before each Enter key for reliable command execution
  - Edit and delete existing macros via hover menu
  - Stop button to abort macro playback mid-execution
  - Macros persist to localStorage across sessions
  - Visual feedback during playback (pulsing animation, stop button)
  - Option to disable sending Enter after each line (for text insertion macros)
  - Template commands for advanced macro scripting:
    - `[[delay:N]]` - Wait N seconds
    - `[[enter]]` - Send Enter key
    - `[[tab]]` - Send Tab key
    - `[[esc]]` - Send Escape key
    - `[[space]]` - Send Space
    - `[[ctrl+X]]` - Send Ctrl+X
    - `[[shift+X]]` - Send Shift+X (uppercase)
    - `[[ctrl+shift+X]]` - Send Ctrl+Shift+X
    - `[[shift+tab]]` - Reverse Tab
    - `[[shift+enter]]` - Shift+Enter

- **On-Screen Keyboard Enhancements**:
  - Permanent symbols grid on the right side with all keyboard symbols (32 keys)
  - Added Space and Enter buttons to modifier row
  - Added http:// and https:// quick insert buttons to modifier row
  - Added tooltips to Ctrl shortcut buttons explaining each shortcut
  - Expanded symbol keys: added `! @ # $ % ^ & * - _ = + : ; ' " , . ?`

### Changed
- **Web Frontend Dependencies**: Updated @types/node (25.0.1 → 25.0.2)
- **On-Screen Keyboard Layout**: Reorganized for better usability
  - Symbols now displayed as persistent grid instead of toggle row
  - Removed redundant Escape key from function key row
  - More compact vertical layout with reduced gaps

## [0.16.3] - 2025-12-08

### Fixed
- **Web Terminal: tmux/TUI DA Response Echo**: Fixed control characters (`^[[?1;2c^[[>0;276;0c`) appearing when running tmux or other TUI applications in the web terminal
  - Root cause: xterm.js frontend was generating Device Attributes (DA) responses when it received DA queries forwarded from the backend terminal
  - Solution: Registered xterm.js parser handlers to suppress DA1, DA2, DA3, and DSR responses (backend terminal emulator handles these)
  - Affected sequences: `CSI c` (DA1), `CSI > c` (DA2), `CSI = c` (DA3), `CSI n` (DSR), `CSI ? Ps $ p` (DECRQM)

### Added
- **jemalloc Allocator Support**: Optional `jemalloc` feature for 5-15% server throughput improvement
  - New Cargo feature: `jemalloc` (enabled separately from `streaming`)
  - Only available on non-Windows platforms (Unix/Linux/macOS)
  - Uses `tikv-jemallocator` v0.6

### Changed
- **Streaming Server Performance Optimizations**:
  - **TCP_NODELAY**: Disabled Nagle's algorithm on WebSocket connections for lower keystroke latency (up to 40ms improvement)
  - **Output Batching**: Time-based batching with 16ms window (60fps) reduces WebSocket message overhead by 50-80% during burst output
  - **Compression Threshold**: Lowered from 1KB to 256 bytes to compress more typical terminal output (prompts, short commands are 200-800 bytes)

- **Web Frontend Performance Optimizations**:
  - **WebSocket Preconnect**: Added preconnect hints for ws:// and wss:// to reduce initial connection latency by 100-200ms
  - **Font Preloading**: Preload JetBrains Mono to avoid layout shift and font flash

- **Web Frontend Dependencies**: Updated Next.js (16.0.7 → 16.0.8), @types/node (24.10.1 → 24.10.2)
- **Pre-commit Hooks**: Updated ruff (0.14.4 → 0.14.8)

## [0.16.2] - 2025-12-05

### Fixed
- **TERM Environment Variable**: Changed default `TERM` from `xterm-kitty` to `xterm-256color` for better compatibility with systems lacking kitty terminfo

## [0.16.1] - 2025-12-03

### Fixed
- **`cargo install` No Longer Requires `protoc`**: Pre-generated Protocol Buffer code is now included in the crate, eliminating the need to install the `protoc` compiler when building with the `streaming` feature
- Removed `prost-build` from default build dependencies (moved to optional `regenerate-proto` feature)
- CI workflow updated to remove unnecessary `protoc` installation steps

### Changed
- Protocol Buffer Rust code is now pre-generated in `src/streaming/terminal.pb.rs`
- Added new `regenerate-proto` feature for regenerating protobuf code from `proto/terminal.proto`

## [0.16.0] - 2025-12-03

### Changed
- **BREAKING: Binary Protocol for WebSocket Streaming**:
  - Replaced JSON-based WebSocket protocol with Protocol Buffers binary encoding
  - ~80% reduction in message sizes for typical terminal output
  - Optional zlib compression for payloads over 1KB (screen snapshots)
  - Wire format: 1-byte header (0x00=uncompressed, 0x01=compressed) + protobuf payload
  - Text WebSocket messages are no longer supported (binary only)

### Added
- **TLS/SSL Support for Streaming Server**:
  - New CLI options: `--tls-cert`, `--tls-key`, `--tls-pem` for enabling HTTPS/WSS
  - Supports separate certificate and key files or combined PEM file
  - Enables secure connections for production deployments
  - New `TlsConfig` struct in Rust API for programmatic TLS configuration

- **Protocol Buffers Infrastructure**:
  - New `proto/terminal.proto` schema file (single source of truth)
  - Rust code generation via `prost` + `prost-build` in `build.rs`
  - TypeScript code generation via `@bufbuild/protobuf` + `buf`
  - New `src/streaming/proto.rs` module for encode/decode with compression
  - New `lib/protocol.ts` helper module for frontend

- **Python Bindings for TLS and Binary Protocol**:
  - `StreamingConfig.set_tls_from_files(cert_path, key_path)` - Configure TLS from separate files
  - `StreamingConfig.set_tls_from_pem(pem_path)` - Configure TLS from combined PEM file
  - `StreamingConfig.tls_enabled` property - Check if TLS is configured
  - `StreamingConfig.disable_tls()` - Clear TLS configuration
  - `encode_server_message(type, **kwargs)` - Encode server messages to protobuf
  - `decode_server_message(data)` - Decode server messages from protobuf
  - `encode_client_message(type, **kwargs)` - Encode client messages to protobuf
  - `decode_client_message(data)` - Decode client messages from protobuf

- **Makefile Targets**:
  - `make proto-generate` - Generate protobuf code for Rust and TypeScript
  - `make proto-rust` - Generate Rust protobuf code only
  - `make proto-typescript` - Generate TypeScript protobuf code only
  - `make proto-clean` - Clean generated protobuf files

### Dependencies
- Added `prost` v0.14.1 (Rust protobuf runtime)
- Added `prost-build` v0.14.1 (Rust protobuf codegen, build dependency)
- Added `@bufbuild/protobuf` v2.10.1 (TypeScript protobuf runtime)
- Added `@bufbuild/protoc-gen-es` v2.10.1 (TypeScript protobuf codegen)
- Added `@bufbuild/buf` v1.61.0 (Protocol Buffers toolchain)
- Added `pako` v2.1.0 (TypeScript zlib compression)
- Added `rustls` v0.23.35 (TLS implementation)
- Added `tokio-rustls` v0.26.4 (Async TLS for Tokio)
- Added `rustls-pemfile` v2.2.0 (PEM file parsing)
- Added `axum-server` v0.7.3 (HTTPS server support)

## [0.15.0] - 2025-12-02

### Added
- **Streaming Server CLI Enhancements**:
  - `--download-frontend` option to download prebuilt web frontend from GitHub releases
  - `--frontend-version` option to specify version to download (default: "latest")
  - `--use-tty-size` option to use current terminal size from TTY for the streamed session
  - No longer requires Node.js/npm to use web frontend - can download prebuilt version

- **Web Terminal Onscreen Keyboard Improvements**:
  - Added Ctrl+Space shortcut (NUL character) for set-mark/autocomplete functionality

### Changed
- Documentation updated with new quick start using downloaded frontend
- Build instructions updated with `--no-default-features` flag

## [0.14.0] - 2025-12-01

### Added
- **Web Terminal Onscreen Keyboard**: Mobile-friendly virtual keyboard for touch devices
  - Special keys missing from iOS/Android keyboards: Esc, Tab, arrow keys, Page Up/Down, Home, End, Insert, Delete
  - Function keys F1-F12 (toggleable panel)
  - Symbol keys often hard to type on mobile: |, \, `, ~, {, }, [, ], <, >
  - Modifier keys: Ctrl, Alt, Shift (toggle to combine with other keys)
  - Quick Ctrl shortcuts: ^C, ^D, ^Z, ^L, ^A, ^E, ^K, ^U, ^W, ^R
  - Glass morphism design matching terminal aesthetic
  - Haptic feedback on supported devices
  - Auto-shows on mobile devices, toggleable on desktop
  - Proper ANSI escape sequence generation for all keys

- **OSC 9;4 Progress Bar Support** (ConEmu/Windows Terminal style):
  - New `ProgressState` enum with states: `Hidden`, `Normal`, `Indeterminate`, `Warning`, `Error`
  - New `ProgressBar` struct with `state` and `progress` (0-100) fields
  - Terminal methods: `progress_bar()`, `has_progress()`, `progress_value()`, `progress_state()`, `set_progress()`, `clear_progress()`
  - Full Python bindings for `ProgressState` enum and `ProgressBar` class
  - OSC 9;4 sequence parsing: `ESC ] 9 ; 4 ; state [; progress] ST`
  - Progress values are automatically clamped to 0-100

### Protocol Support
- **OSC 9;4 Format**:
  - `ESC ] 9 ; 4 ; 0 ST` - Hide progress bar
  - `ESC ] 9 ; 4 ; 1 ; N ST` - Normal progress at N%
  - `ESC ] 9 ; 4 ; 2 ST` - Indeterminate/busy indicator
  - `ESC ] 9 ; 4 ; 3 ; N ST` - Warning progress at N%
  - `ESC ] 9 ; 4 ; 4 ; N ST` - Error progress at N%

## [0.13.0] - 2025-11-27

### Added
- **Streaming Server Enhancements**:
  - `--size` CLI option for specifying terminal size in `COLSxROWS` format (e.g., `--size 120x40` or `-s 120x40`)
  - `--command` / `-c` CLI option to execute a command after shell startup (with 1 second delay for prompt settling)
  - `initial_cols` and `initial_rows` configuration options in `StreamingConfig` for both Rust and Python APIs

- **Python Bindings Enhancements**:
  - New `MouseEncoding` enum (`Default`, `Utf8`, `Sgr`, `Urxvt`) for mouse event encoding control
  - Screen buffer control: `use_alt_screen()`, `use_primary_screen()` for direct screen switching
  - Mouse encoding: `mouse_encoding()`, `set_mouse_encoding()` for controlling mouse event format
  - Mode setters: `set_focus_tracking()`, `set_bracketed_paste()` for direct mode control
  - Title control: `set_title()` for programmatic title changes
  - Bold brightening: `bold_brightening()`, `set_bold_brightening()` for legacy terminal behavior
  - Color getters: `link_color()`, `bold_color()`, `cursor_guide_color()`, `badge_color()`, `match_color()`, `selection_bg_color()`, `selection_fg_color()`
  - Color flag getters: `use_bold_color()`, `use_underline_color()`

### Changed
- `StreamingConfig` now includes `initial_cols` and `initial_rows` fields (default: 0, meaning use terminal's current size)

## [0.12.0] - 2025-11-27

### Fixed
- **Terminal Reflow Improvements**: Multiple fixes to scrollback and grid reflow behavior during resize
  - Prevent content at top from being incorrectly pushed to scrollback during resize
  - Use correct column width when pulling content from scrollback
  - Pull content back from scrollback when window widens
  - Push TOP content to scrollback while keeping BOTTOM visible on reflow (matches expected terminal behavior)
  - Preserve excess content in scrollback during reflow operations

## [0.11.0] - 2025-11-26

### Added
- **Full Terminal Reflow on Width Resize**: Both scrollback AND visible screen content now reflow when terminal width changes
  - **Scrollback Reflow**: Previously, changing terminal width would clear all scrollback to avoid panics from misaligned cell indexing. Now implements intelligent reflow similar to xterm and iTerm2
  - **Main Grid Reflow**: Visible screen content now also reflows instead of being clipped
    - **Width increase**: Unwraps previously soft-wrapped lines into longer lines
    - **Width decrease**: Re-wraps lines that no longer fit, preserving all content
  - Preserves all cell attributes (colors, bold, italic, etc.) during reflow
  - Handles wide characters (CJK, emoji) correctly at line boundaries
  - Properly manages circular buffer during scrollback reflow
  - Respects max_scrollback limits when reflow creates additional lines
  - Significant UX improvement for terminal resize operations

### Changed
- Height-only resize operations no longer trigger reflow (optimization)
- Scrollback buffer is now rebuilt (non-circular) after reflow for simpler indexing
- Main grid now extracts logical lines and re-wraps them on width change

## [0.10.0] - 2025-11-24

### Added
- **Emoji Sequence Preservation**: Complete support for complex emoji sequences and grapheme clusters
  - **Variation Selectors**: Preserves emoji vs text style presentation (U+FE0E, U+FE0F)
    - Example: ⚠ vs ⚠️ (warning sign in text vs emoji style)
  - **Skin Tone Modifiers**: Supports Fitzpatrick scale skin tones (U+1F3FB-U+1F3FF)
    - Example: 👋🏽 (waving hand with medium skin tone)
  - **Zero Width Joiners (ZWJ)**: Preserves multi-emoji sequences
    - Example: 👨‍👩‍👧‍👦 (family), 🏳️‍🌈 (rainbow flag)
  - **Regional Indicators**: Proper handling of flag emoji
    - Example: 🇺🇸 (US flag), 🇬🇧 (UK flag)
  - **Combining Characters**: Supports diacritics and other combining marks
    - Example: é (e + combining acute accent)
  - New `grapheme` module with comprehensive Unicode detection utilities
  - Enhanced `Cell` structure with `combining: Vec<char>` field for grapheme cluster storage
  - New methods: `Cell::get_grapheme()` and `Cell::from_grapheme()`
  - Python bindings now export full grapheme clusters through `get_line()` and `row_text()`

- **Web Terminal Frontend**: Modern Next.js-based web interface for the streaming server
  - Built with Next.js 16, React 19, TypeScript, and Tailwind CSS v4
  - **Mobile-Responsive Design**: Fully functional on phones and tablets
    - Responsive font sizing (4px mobile to 14px desktop)
    - Hideable header/footer to maximize terminal space
    - Touch support for mobile keyboard activation
    - Orientation change handling with automatic refit
    - Optimized scrollback (500 lines mobile, 1000 desktop)
    - Disabled cursor blink on mobile for battery savings
  - **Auto-Reconnect**: Exponential backoff (500ms to 5s max) with cancel button
  - Theme support with configurable color palettes
  - Nerd Font support for file/folder icons
  - WebGL renderer with DOM fallback
  - React 18 StrictMode compatible
  - Dev server binds to all interfaces (0.0.0.0) for mobile testing
  - New Makefile targets for web frontend development

- **Terminal Sequence Support**:
  - **CSI 3J**: Clear scrollback buffer command
  - Improved cursor positioning for snapshot exports

### Fixed
- **Graphics Scrollback**: Graphics now properly preserved when scrolling into scrollback buffer
  - Added `scroll_offset_rows` tracking for proper graphics rendering
  - Tall Sixel graphics preserved when bottom is still visible
  - Fixed premature scroll_offset during Sixel load
- **Sixel Scrollback**: Content now saved to scrollback during large Sixel scrolling operations
- **Kitty Graphics Protocol**: Fixed animation control parsing bugs
  - Support for both padded and unpadded base64 encoding
  - Corrected frame action handling for animations

### Changed
- **Breaking**: `Cell` struct no longer implements `Copy` trait (now `Clone` only)
  - Required for supporting variable-length grapheme clusters
  - All cell copy operations now require explicit `.clone()` calls
  - Performance impact is minimal due to efficient cloning

### Dependencies
- Added `unicode-segmentation = "1.12"` for grapheme cluster support

## [0.9.1] - 2025-11-23

### Fixed
- **Theme Rendering**: Fixed theme color palette application in Python bindings
  - Colors now properly use configured ANSI palette instead of hardcoded defaults
  - Affects `get_visible_lines()` method in `PtyTerminal`
  - Ensures theme colors are consistently rendered across all output methods
  - Resolves foreground and background colors using the active palette

### Added
- **Makefile**: Added `install-force` target for force uninstall and reinstall

## [0.9.0] - 2025-11-22

### Added
- **Graphics Protocol Support**: Comprehensive multi-protocol graphics implementation
  - **iTerm2 Inline Images** (OSC 1337): PNG, JPEG, GIF support with base64 encoding
  - **Kitty Graphics Protocol** (APC G): Advanced image placement with reuse and animations
  - **Sixel Graphics**: Enhanced with unique IDs and configurable cell dimensions
  - Unified `GraphicsStore` with scrollback support and memory limits
  - Animation support with frame composition and timing control
  - Graphics dropped event tracking for resource management

- **Pre-built Streaming Server Binaries**: Download ready-to-run binaries from GitHub Releases
  - Linux (x86_64, ARM64), macOS (Intel, Apple Silicon), Windows (x86_64)
  - No compilation needed - just download and run
  - Includes separate web frontend package (tar.gz/zip) for serving the terminal interface
  - Published to crates.io for Rust developers: `cargo install par-term-emu-core-rust --features streaming`

## [0.8.0] - 2025-11-19

### Fixed
- **Keyboard Protocol Reset**: Automatically reset Kitty Keyboard Protocol flags when exiting alternate screen buffer
  - Prevents TUI apps from leaving keyboard in bad state if they fail to disable protocol on exit
  - Clears both main and alternate keyboard flag stacks
  - Ensures clean terminal state after TUI app termination

## [0.7.0] - 2024-11-19

### Added
- **Buffer Controls**: Configurable limits for system resources
  - `set_max_notifications()` / `get_max_notifications()`: Limit OSC 9/777 notification backlog
  - `set_max_clipboard_sync_events()` / `get_max_clipboard_sync_events()`: Limit clipboard event history
  - `set_max_clipboard_event_bytes()` / `get_max_clipboard_event_bytes()`: Truncate large clipboard payloads
- **XDG Base Directory Compliance**: Shell integration now follows XDG standards
- **Improved Session Export**: Enhanced `export_asciicast()` and `export_json()` with explicit session parameters

### Changed
- **Shell Integration**: Migrated to XDG Base Directory specification for better standards compliance
- **Export APIs**: Session parameter now explicit in export methods for clearer API

### Documentation
- Comprehensive documentation for all new features and buffer controls
- Updated examples for new buffer control APIs

## [0.6.0] - 2024-11-15

### Added
- **Comprehensive Color Utilities API**: 18 new Python functions for color manipulation
  - Brightness and contrast: `perceived_brightness_rgb()`, `adjust_contrast_rgb()`
  - Basic adjustments: `lighten_rgb()`, `darken_rgb()`
  - WCAG accessibility: `color_luminance()`, `is_dark_color()`, `contrast_ratio()`, `meets_wcag_aa()`, `meets_wcag_aaa()`
  - Color mixing: `mix_colors()`, `complementary_color()`
  - Color space conversions: `rgb_to_hsl()`, `hsl_to_rgb()`, `rgb_to_hex()`, `hex_to_rgb()`, `rgb_to_ansi_256()`
  - Advanced adjustments: `adjust_saturation()`, `adjust_hue()`
- **iTerm2 Compatibility**: Matching NTSC brightness formula and contrast adjustment algorithms
- **Python Bindings**: All color utilities exposed via `par_term_emu_core_rust` module
- **Fast Native Implementation**: Rust-based for optimal performance

## [0.5.0] - 2024-11-10

### Added
- **Bold Brightening Support**: Configurable bold brightening for improved terminal compatibility
  - `set_bold_brightening()` method: Enable/disable bold text brightening for ANSI colors 0-7
  - iTerm2 Compatibility: Matches iTerm2's "Use Bright Bold" setting behavior
  - Automatic Color Conversion: Bold text with ANSI colors 0-7 automatically uses bright variants 8-15
  - Snapshot Integration: `create_snapshot()` automatically applies bold brightening when enabled

### Changed
- Enhanced `create_snapshot()` to automatically apply bold brightening when enabled

### Documentation
- New section in `docs/ADVANCED_FEATURES.md` with bold brightening examples

## [0.4.0] - 2024-11-01

### Added
- **Session Recording and Replay**: Record terminal sessions with timing information
  - Multiple event types: input, output, resize, custom markers
  - Export formats: asciicast v2 (asciinema) and JSON
  - Session metadata capture
  - Markers/bookmarks support
- **Terminal Notifications**: Advanced notification system
  - Multiple trigger types: Bell, Activity, Silence, Custom
  - Alert options: Desktop, Sound (with volume), Visual
  - Configurable settings per trigger type
  - Activity/silence detection
  - Event logging with timestamps
- **Enhanced Screenshot Support**:
  - Theme configuration options
  - Custom link and bold colors
  - Minimum contrast adjustment
- **Buffer Statistics**: Comprehensive terminal content analysis
  - `get_stats()`: Detailed terminal metrics
  - `count_non_whitespace_lines()`: Content line counting
  - `get_scrollback_usage()`: Scrollback buffer tracking

### Changed
- Improved screenshot configuration with theme settings
- Enhanced export functionality for better session capture

## [0.3.0] - 2024-10-20

### Added
- **Text Extraction Utilities**: Smart word/URL detection, selection boundaries
  - `get_word_at()`: Extract word at cursor with customizable word characters
  - `get_url_at()`: Detect and extract URLs
  - `select_word()`: Get word boundaries for double-click selection
  - `get_line_unwrapped()`: Get full logical line following wraps
  - `find_matching_bracket()`: Find matching brackets/parentheses
  - `select_semantic_region()`: Extract content within delimiters
- **Content Search**: Find text with case-sensitive/insensitive matching
  - `find_text()`: Find all occurrences
  - `find_next()`: Find next occurrence from position
- **Static Utilities**: Standalone text processing functions
  - `Terminal.strip_ansi()`: Remove ANSI codes
  - `Terminal.measure_text_width()`: Measure display width
  - `Terminal.parse_color()`: Parse color strings

## [0.2.0] - 2024-10-10

### Added
- **Screenshot Support**: Multiple format support
  - Formats: PNG, JPEG, BMP, SVG (vector), HTML
  - Embedded JetBrains Mono font
  - Programming ligatures support
  - Box drawing character rendering
  - Color emoji support with font fallback
  - Cursor rendering with multiple styles
  - Sixel graphics rendering
  - Minimum contrast adjustment
- **PTY Support**: Interactive shell sessions
  - Spawn commands and shells
  - Bidirectional I/O
  - Process management
  - Dynamic resizing with SIGWINCH
  - Environment control
  - Event loop integration
  - Context manager support
  - Cross-platform (Linux, macOS, Windows)

### Changed
- Improved Unicode handling for wide characters and emoji
- Enhanced grid rendering for box drawing characters

## [0.1.0] - 2024-10-01

### Added
- Initial stable release
- **Core VT Compatibility**: VT100/VT220/VT320/VT420/VT520 support
- **Rich Color Support**: 16 ANSI, 256-color palette, 24-bit RGB
- **Text Attributes**: Bold, italic, underline (multiple styles), strikethrough, blink, reverse, dim, hidden
- **Advanced Cursor Control**: Full VT100 cursor movement
- **Line/Character Editing**: VT220 insert/delete operations
- **Rectangle Operations**: VT420 fill/copy/erase/modify rectangular regions
- **Scrolling Regions**: DECSTBM support
- **Tab Stops**: Configurable tab stops
- **Terminal Modes**: Application cursor keys, origin mode, auto wrap, alternate screen
- **Mouse Support**: Multiple tracking modes and encodings
- **Modern Features**:
  - Alternate screen buffer
  - Bracketed paste mode
  - Focus tracking
  - OSC 8 hyperlinks
  - OSC 52 clipboard operations
  - OSC 9/777 notifications
  - Shell integration (OSC 133)
  - Sixel graphics
  - Kitty Keyboard Protocol
  - Tmux Control Protocol
- **Scrollback Buffer**: Configurable history
- **Terminal Resizing**: Dynamic size adjustment
- **Unicode Support**: Full Unicode including emoji and wide characters
- **Python Integration**: PyO3 bindings for Python 3.12+

[0.37.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.36.0...v0.37.0
[0.36.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.35.0...v0.36.0
[0.35.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.34.0...v0.35.0
[0.34.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.33.0...v0.34.0
[0.33.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.32.0...v0.33.0
[0.32.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.31.1...v0.32.0
[0.31.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.31.0...v0.31.1
[0.31.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.30.0...v0.31.0
[0.30.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.29.0...v0.30.0
[0.29.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.28.0...v0.29.0
[0.28.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.27.0...v0.28.0
[0.27.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.26.0...v0.27.0
[0.26.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.25.0...v0.26.0
[0.25.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.24.0...v0.25.0
[0.24.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.23.0...v0.24.0
[0.23.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.22.1...v0.23.0
[0.22.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.22.0...v0.22.1
[0.22.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.21.0...v0.22.0
[0.21.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.20.1...v0.21.0
[0.20.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.20.0...v0.20.1
[0.20.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.5...v0.20.0
[0.19.5]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.4...v0.19.5
[0.19.4]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.3...v0.19.4
[0.19.3]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.2...v0.19.3
[0.19.2]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.1...v0.19.2
[0.19.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.0...v0.19.1
[0.19.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.18.2...v0.19.0
[0.18.2]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.18.1...v0.18.2
[0.18.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.18.0...v0.18.1
[0.18.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.17.0...v0.18.0
[0.17.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.16.3...v0.17.0
[0.16.3]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.16.2...v0.16.3
[0.16.2]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.16.1...v0.16.2
[0.16.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.16.0...v0.16.1
[0.16.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.9.1...v0.10.0
[0.9.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/paulrobello/par-term-emu-core-rust/releases/tag/v0.1.0
