# Audit Remediation Playbook — 2026-08-27

> Companion to `AUDIT.md` (same date). One entry per issue, ordered by the Remediation Plan
> phases. Written for the `/fix-audit` phase agents: each entry is executable without
> re-deriving the analysis. All line numbers verified at HEAD e83f415 — re-read files before
> editing; earlier phases may have shifted lines.
>
> Project gates: `make checkall` (Rust tests, streaming tests, clippy+fmt, Python
> ruff/pyright, pytest). Build with `make dev` (maturin) — **never `cargo build`** for
> PyO3 targets. Single Rust test: `cargo test --lib --no-default-features --features
> pyo3/auto-initialize <name>` (add `,streaming` for streaming tests).

---

## Phase 1 — Security (sequential, first)

### [SEC-001] CORS falls back to fully permissive when no origin allowlist is configured
- **Files**: `src/streaming/server.rs:2947-2960` (`build_cors_layer`; applied at :1637 and :1698), `docs/SECURITY.md:948`
- **Steps**:
  1. In `build_cors_layer`, replace the `None => CorsLayer::very_permissive()` branch with a layer that mirrors the WebSocket default policy: allow only local origins. Implement via `AllowOrigin::predicate` calling the same host-classification logic `is_local_origin` uses (extract that logic into a shared helper if it currently takes a full request rather than an origin value).
  2. Keep the `Some(list)` branch as-is (explicit allowlist).
  3. Add a Rust test: with no allowlist, a request with `Origin: https://evil.example` gets no `Access-Control-Allow-Origin` header; with `Origin: http://127.0.0.1:8099` it does; with an explicit allowlist the listed origin passes.
  4. Update `docs/SECURITY.md:948` if the wording needs to change (it should now be accurate as written — "mirrors the policy" becomes true).
- **Method**: The WS side already default-denies remote browsers via `check_ws_origin`/`is_local_origin`; the fix is to make HTTP consistent, not to invent a new policy. Do not tighten the explicit-allowlist path — operators who set `--allowed-origins` chose their exposure. Pitfall: `tower_http::cors::AllowOrigin::predicate` receives `&HeaderValue`; parse the origin's host out of it rather than comparing whole strings, and treat a missing/opaque origin the way `check_ws_origin` does.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming cors` (new test), then `make checkall`.

### [SEC-002] `/sessions` handler lacks Origin check
- **Files**: `src/streaming/server.rs:3121-3132` (`sessions_handler`); reference implementations `ws_handler:3104`, `stats_ws_handler:3151`
- **Steps**:
  1. Add the same `check_ws_origin` guard the two WS handlers use at the top of `sessions_handler`; on failure return `403 Forbidden`.
  2. Add a test: cross-origin request to `/sessions` is rejected; local-origin request succeeds; configured-allowlist origin succeeds.
- **Method**: One logical change with SEC-001 — land in the same commit or adjacent commits. The handler signature may need the headers/connect-info the WS handlers already take; copy their extraction pattern exactly.
- **Verify**: new test passes; `make checkall`.

---

## Phase 2 — Architecture (sequential)

### [ARC-002] Ship `py.typed` + generated stubs (or drop the classifier)
- **Files**: `pyproject.toml` (classifiers ~line 36), `python/par_term_emu_core_rust/` (add `py.typed`, `_native.pyi`), `src/lib.rs:181-291` (module registration = the authoritative export list), `.github/workflows/ci.yml` (stub check step), `Makefile`
- **Steps**:
  1. Evaluate `pyo3-stub-gen`: add it as an optional dev path (it requires annotating with its macros — if that is too invasive, generate the stub once from runtime introspection instead: a script that imports `_native`, walks `dir()`, and emits signatures from `__text_signature__`/docstrings).
  2. Whichever generator is chosen, commit the generated `python/par_term_emu_core_rust/_native.pyi` and an empty `python/par_term_emu_core_rust/py.typed`.
  3. Ensure both files ship: check `pyproject.toml`/maturin config includes package data (maturin includes package dir files by default; verify with a local wheel build `make dev` then inspect `.venv/.../par_term_emu_core_rust/`).
  4. Add a CI/Make step `stub-check`: `uv run python -c "import par_term_emu_core_rust"` plus `uv run pyright python/par_term_emu_core_rust/_native.pyi` (stub must parse cleanly).
  5. If full generation proves infeasible in this pass, minimum viable: hand-write stubs for `Terminal`, `PtySession`, `PtyTerminal`, the enums, and top-level functions; leave data classes `Any`-typed with a TODO-free comment noting incremental coverage; do NOT remove the classifier in that case since partial stubs + py.typed is still honest.
- **Method**: The registration list in `src/lib.rs:181-291` enumerates every exported name — use it as the checklist so nothing is missed. Do this before DOC-004/DOC-009/DOC-011 so doc fixes and stubs derive from the same signature pass. Pitfall: `#[setter]` properties (e.g. `set_max_sessions`) must appear as properties in the stub, not methods — that is exactly the DOC-004 error class.
- **Verify**: `make dev && uv run pyright python/ tests/` (pyright now sees real types), the new stub-check target, `make checkall`.

### [ARC-003] Streaming codec: macro-generate the Python dict conversion; make matches exhaustive
- **Files**: `src/python_bindings/streaming.rs:939` (`encode_server_message`), `:1312` (`decode_server_message`), `:1758` (`encode_client_message`), `:1904` (`decode_client_message`); `src/streaming/protocol.rs` (enum definitions); `derive/src/lib.rs` (existing derive crate); `src/streaming/proto.rs`
- **Steps**:
  1. **Decision (already made — do not re-litigate)**: take the incremental option, not the full prost-type collapse. Full collapse (deleting proto.rs) is 3–5 days and high-risk; the macro route delivers the compile-time safety at 1/4 the cost and does not preclude a later collapse.
  2. In `derive/`, add a derive or attribute macro (e.g. `#[derive(PyDictConvert)]`) that, for an enum, generates `fn to_py_dict(&self, py: Python) -> PyResult<Py<PyDict>>` mapping each variant's named fields to dict keys (snake_case, matching the current hand-written keys — extract the current key list per variant FIRST and pin it with tests, since dict keys are the public Python API).
  3. Before switching implementations, write characterization tests: for each `ServerMessage` variant, construct a sample, run the CURRENT `decode_server_message`, and snapshot the resulting dict keys/values (a Python test parametrized over all 37 variants is the cheapest form).
  4. Apply the macro to `ServerMessage`/`ClientMessage` (or their payload structs), replace the bodies of the four giant functions with calls into the generated conversions, keeping the `message_type: &str` public signatures unchanged.
  5. Remove every `_ =>` fallback arm in `src/streaming/proto.rs` and `src/python_bindings/streaming.rs` match statements over protocol enums so a new variant fails compilation at every site.
- **Method**: The characterization tests are the safety net — the wire/dict format must not change. parsight queries to enumerate touch points: `find_symbol name:"decode_server_message"`, `get_impact` on `ServerMessage`. Pitfall: some variants have custom conversions (bytes → PyBytes, nested structs); the macro needs an escape hatch (a `#[pydict(with = "path")]` field attribute) rather than forcing uniformity.
- **Verify**: the parametrized dict-shape tests pass unchanged before AND after the switch; `make checkall`; `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming`.

### [ARC-004] Decompose `src/streaming/server.rs` (4,024 lines)
- **Files**: `src/streaming/server.rs` → new `src/streaming/config.rs`, `src/streaming/session.rs`, `src/streaming/rate_limit.rs`; `src/streaming/mod.rs` (module decls + re-exports)
- **Steps**:
  1. Run SEC-001/SEC-002 first (Phase 1) — this split moves their code.
  2. Move, without behavior change: `TlsConfig` (:141), `HttpBasicAuthConfig` (:315), `StreamingConfig` (:390), `ApiAuthConfig` (:2785) → `config.rs`; `SessionMetrics` (:507), `SessionState` (:541), `SessionRegistry` (:960) → `session.rs`; the rate limiter (:1131) → `rate_limit.rs`. Line numbers WILL have shifted after Phase 1 — relocate by symbol name.
  3. Re-export every moved public type from `src/streaming/mod.rs` at its old path (`pub use server::X` becomes `pub use config::X` etc.) so Python bindings, tests, and the rlib surface see no path change.
  4. Rename the moved streaming `SessionState` → `StreamSessionState` while it moves (this completes ARC-010 in the same pass; update the ~internal call sites — `get_impact` on the symbol first, and grep `tests/test_streaming.rs`).
  5. Keep `server.rs` as the server loop + handlers only.
- **Method**: Mechanical moves with re-exports mean zero call-site churn outside the streaming module. Do it in one commit per extracted file (R2 phasing). Pitfall: `#[cfg(feature = "streaming")]` gating — the new files need the same cfg in `mod.rs`; and inline `#[cfg(test)]` modules inside moved code move with their subjects.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming` after each move; `make checkall` at the end; `git diff --stat` shows only moves + mod.rs.

---

## Phase 3a — Security (remaining)

### [SEC-003] Kitty file-load path check: fix substring test, align docs
- **Files**: `src/graphics/kitty.rs:810-855` (`load_file_data`; `contains("..")` at :820), `docs/SECURITY.md:673-688`
- **Steps**:
  1. Replace the substring test with a component-wise check: reject when any `Path::components()` element is `Component::ParentDir`. This stops falsely rejecting `my..notes.png` while keeping the `..` block.
  2. Do NOT add an allowlist-root option in this pass (scope: audit says minimum is doc alignment; the allowlist is future work — noted in ENH backlog territory, not filed).
  3. Edit `docs/SECURITY.md:673-688` to state plainly: absolute paths are readable by design (the requester already runs with user privileges); the `..` check is not a sandbox.
- **Method**: The security posture is documented-as-intended; this fix removes the false-rejection bug and the false implication of protection. Add a unit test for `my..notes.png` accepted, `a/../b` rejected.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize kitty`, `make checkall`.

### [SEC-004] Harden the debug log temp file
- **Files**: `src/debug.rs:60-67`
- **Steps**:
  1. Include the PID in the filename: `par_term_emu_core_rust_debug_rust_{pid}.log`.
  2. On Unix, open with `OpenOptions` + `custom_flags(libc::O_NOFOLLOW)` and `mode(0o600)` (`std::os::unix::fs::OpenOptionsExt`); keep plain options on Windows via `cfg`.
- **Method**: Logging is opt-in (`DEBUG_LEVEL`), so this is hardening, not a hot path. The PTY debug log is already per-process — mirror its naming. Pitfall: `libc` is already a dependency on Unix targets; gate imports with `#[cfg(unix)]`.
- **Verify**: `DEBUG_LEVEL=1` smoke run creates a 0600 pid-suffixed file; `make checkall`.

### [SEC-005] Query api_key doc consistency
- **Files**: `docs/STREAMING.md` (auth section), `web-terminal-frontend/README.md` if it mentions api_key
- **Steps**:
  1. No code change. Add one sentence to the auth docs: the frontend forwards `?api_key=` only when the server was started with `--allow-api-key-in-query`, which leaks keys into logs/history — prefer the first-message auth flow.
- **Verify**: doc renders; `make checkall` untouched.

---

## Phase 3b — Architecture (remaining; run ARC-009 before ARC-001; ARC-006 before ARC-007 — both edit Cargo.toml)

### [ARC-009] Single canonical frontend lockfile
- **Files**: `web-terminal-frontend/package-lock.json` (delete), `web-terminal-frontend/bun.lock` (keep), `Makefile` (`web-build-static` and any `npm` invocations), `.github/workflows/*.yml` (frontend steps)
- **Steps**:
  1. Grep Makefile + workflows for `npm `/`npx ` under `web-terminal-frontend`; convert to `bun install --frozen-lockfile` / `bun run build`.
  2. `git rm web-terminal-frontend/package-lock.json`.
  3. Add `package-lock.json` to `web-terminal-frontend/.gitignore`.
  4. Run `make web-build-static` and confirm `web_term/` output is produced (do not commit the rebuilt output unless it differs — if it differs, that difference is itself the ARC-001 evidence; note it in the report).
- **Method**: bun is the repo norm (bun.lock updated in recent dep commits). Pitfall: CI runners need bun installed — check the workflow already sets it up (the deps commit e83f415 touched workflows; read them).
- **Verify**: `make web-build-static` succeeds from a clean `node_modules` (`rm -rf web-terminal-frontend/node_modules` first); `git status` clean of `package-lock.json`.

### [ARC-001] `web_term/` CI drift gate
- **Files**: `.github/workflows/deployment.yml:401-415` (release packaging), possibly `.github/workflows/ci.yml`
- **Steps**:
  1. Keep `web_term/` tracked (release packaging depends on it — commit b56004d proves removal breaks it). Add a drift gate instead of untracking.
  2. Add a CI job (in ci.yml, not just release): setup bun → `bun install --frozen-lockfile` in `web-terminal-frontend/` → run the `web-build-static` equivalent → `git diff --exit-code web_term/` → fail with a message naming `make web-build-static` as the fix.
  3. Confirm the Next.js build is deterministic first by running `make web-build-static` twice locally and diffing; if nondeterministic (hashed chunk names varying, build IDs, timestamps), pin the sources of nondeterminism (`generateBuildId` in `next.config`, `NEXT_TELEMETRY_DISABLED=1`) before enabling the gate — otherwise the gate flakes.
- **Method**: The gate turns a CLAUDE.md checklist rule into a machine check. Depends on ARC-009 (canonical package manager). Pitfall: determinism is the whole game; do not merge a flaky gate.
- **Verify**: CI job green on unchanged tree; intentionally touch a frontend source, rebuild, confirm the gate catches a missing rebuild (locally simulate: edit `web-terminal-frontend/components/Terminal.tsx` comment, run the gate script without rebuilding → must fail).

### [ARC-005] Split the streaming server binary; remove reachable `.expect`s
- **Files**: `src/bin/streaming_server/main.rs` (1,790 lines; `main` at :1302; expects at :1485, :1506, :1708; downloader :1043; auth resolution :1214) → new `src/bin/streaming_server/{cli.rs,frontend_download.rs,bootstrap.rs}`
- **Steps**:
  1. Move the clap `Args` struct + parsing helpers (`parse_size` :130, `parse_preset` :151 — currently flagged dead because dispatch is untraceable; they are used via clap attributes, do NOT delete) → `cli.rs`.
  2. Move the GitHub-release frontend downloader (:1043) → `frontend_download.rs`.
  3. Move TLS/auth resolution (:1214) and server wiring → `bootstrap.rs`; `main()` becomes sequencing.
  4. Replace the three `.expect("PTY session required for macro mode")` with an early validation in `cli.rs`/`bootstrap.rs` that exits with a usage error (`anyhow::bail!` or `clap::Error`) before any server starts.
  5. Existing sibling `theme.rs` shows the intended module pattern — match it.
- **Method**: The binary is `required-features = ["streaming-bin"]`; build/test with that feature. Pitfall: `ServerState` methods flagged dead (`handle_resize_requests` :392 etc.) are dispatched dynamically — verify with grep before touching anything while moving.
- **Verify**: `cargo build --bin par-term-streamer --no-default-features --features streaming-bin` (link check is fine for a bin target), `make streamer-run` smoke start, `make checkall`.

### [ARC-006] Replace `serde_yaml`
- **Files**: `Cargo.toml:62`, `src/macros.rs:159-177`
- **Steps**:
  1. Swap `serde_yaml = "0.9"` for `serde_yml` (the maintained fork with a near-identical API) in Cargo.toml.
  2. Update the two or three call sites in `src/macros.rs` (`serde_yaml::to_string`/`from_str` → `serde_yml::...`).
  3. Run the macro YAML round-trip tests (grep `yaml` in tests).
- **Method**: `serde_yml` is API-compatible for to_string/from_str; do not switch formats (JSON) — YAML import/export is public API.
- **Verify**: `cargo tree | grep -i serde_yaml` returns nothing; macro tests pass; `make checkall`; `cargo audit` no longer warns on RUSTSEC-2024-0320.

### [ARC-007] Trim tokio features; drop `paste` via `exr`
- **Files**: `Cargo.toml:68` (tokio), `Cargo.toml:110` (image features)
- **Steps**:
  1. Replace `tokio = { features = ["full"] }` with the enumerated set: start from `["rt-multi-thread", "net", "sync", "time", "macros", "io-util"]`, compile the streaming feature, and add any feature the compiler demands (`signal`, `fs` may be needed by the binary — check errors, add minimally).
  2. In the `image` dependency, remove `exr`, `hdr`, `dds` from the feature list (screenshot/graphics use PNG/JPEG/GIF/WebP/BMP/TIFF at most — decode paths are Sixel/iTerm2/Kitty payloads and screenshot encode).
  3. `cargo tree -i paste` → must return nothing. If something still pulls it, chase that edge before claiming done.
  4. Update the Cargo.toml comment at :111-116 to reflect the now-true state.
- **Method**: Two independent trims, one file — single commit is fine. The graphics tests (`kitty`, `iterm`, `sixel`, `screenshot`) are the regression net for removed decoders. Pitfall: iTerm2 inline images can theoretically carry any format the `image` crate reads; dropping a decoder changes behavior for exotic payloads — TIFF stays for that reason, EXR/HDR/DDS are not plausible terminal payloads.
- **Verify**: `cargo tree -i paste` empty; `make checkall`; `make test-rust` graphics tests green; note compile-time delta if visible.

### [ARC-008] Fix threading docs (Mutex → RwLock)
- **Files**: `CLAUDE.md:128` (PTY Architecture paragraph), `src/pty_session.rs:5` (module doc)
- **Steps**:
  1. CLAUDE.md: "`PtySession` wraps `Arc<Mutex<Terminal>>` (using `parking_lot::Mutex` ...)" → "`PtySession` wraps `Arc<RwLock<Terminal>>` (using `parking_lot::RwLock` — no poisoning). The background reader thread takes the write lock to `process()` output; queries take read locks."
  2. `src/pty_session.rs:5`: fix the self-contradictory sentence the same way.
  3. Run before DOC-007 (same file, CLAUDE.md).
- **Verify**: `grep -n "Mutex<Terminal>" CLAUDE.md src/pty_session.rs` returns nothing; `make checkall` (fmt only).

### [ARC-010] Rename streaming `SessionState`
- **Files**: post-ARC-004 `src/streaming/session.rs` (or `server.rs` if ARC-004 was skipped)
- **Steps**: Folded into ARC-004 step 4. If executed standalone: rename to `StreamSessionState`, fix call sites (`get_impact` on the symbol; grep `tests/test_streaming.rs`), keep a `pub use` alias only if the rlib surface exports it (check `src/streaming/mod.rs` re-exports; it is not in the Python API).
- **Verify**: `make checkall`.

### [ARC-011] Make the `sim` feature marker honest
- **Files**: `Cargo.toml:177`
- **Steps**: Add a comment above `sim = []`: intentionally an empty marker naming the headless profile; it works only with `default-features = false`. Optionally add to `src/lib.rs`: `#[cfg(all(feature = "sim", feature = "python"))] compile_error!("`sim` is the headless profile; disable default features");`.
- **Method**: Prefer comment + compile_error together — the error makes misuse loud, the comment explains it. Check CI builds don't combine the two features before adding the error.
- **Verify**: `cargo check --no-default-features --features sim` passes; `cargo check --features sim` fails with the clear message (if compile_error added); `make checkall`.

### [ARC-012] Derive crate version-sync + publish story
- **Files**: `derive/Cargo.toml:3`, `.github/workflows/publish-crates.yml`, `CLAUDE.md` (Version Sync section)
- **Steps**:
  1. CLAUDE.md Version Sync list: add a bullet — `derive/Cargo.toml` version-bumps only when derive code changes; the main crate's dep spec (`Cargo.toml:52`) must match the published derive version.
  2. `publish-crates.yml`: before the main-crate publish step, add a conditional step: query crates.io for the derive crate's current version (`cargo search` or the crates.io API); if `derive/Cargo.toml`'s version is not published, `cargo publish -p <derive-crate-name>` first, then wait for index propagation before the main publish.
- **Method**: Do not bump the derive version now — it is internally consistent (0.45.0 matches published). This is CI + docs only.
- **Verify**: `act`-style dry run is overkill; YAML lint (`actionlint` if available) + manual review; `make checkall` untouched.

### [ARC-013] Terminal aggregate — no dedicated action
- **Files**: `src/terminal/mod.rs`
- **Steps**: No standalone work. When other fixes touch `mod.rs`, opportunistically move free helpers (`cells_to_text`, `html_escape`, `sanitize_clipboard_content`) into existing submodules. Record as guidance, not a task.
- **Verify**: n/a.

### [ARC-014] Public surface curation — deferred
- **Files**: `src/lib.rs:37-69`
- **Steps**: No action this cycle. Requires surveying `../par-term`'s actual imports and a planned semver-major release. Leave for a release-planning session.
- **Verify**: n/a.

---

## Phase 3c — Code Quality (QA-001 first; QA-002/QA-003 after ARC-003; QA-008 before QA-005; QA-007 after QA-001)

### [QA-001] Fix `Terminal::get_word_at` display-column/char-index confusion
- **Files**: `src/terminal/screen.rs:350-383`, `src/text_utils.rs:16` (`get_word_at`), `:182` (`select_word`), Python bindings at `src/python_bindings/common.rs:1447,1695`, tests in `tests/` + inline
- **Steps**:
  1. Read `src/text_utils.rs::get_word_at` — it operates on grid cells (display-column aware). Confirm its delimiter set matches the current `screen.rs` behavior for ASCII (write a quick comparison test first; if delimiters differ, adopt the `text_utils` set and note the behavior change).
  2. Replace the body of `Terminal::get_word_at` with a call to `text_utils::get_word_at(&self.grid, col, row, ...)`; same for `Terminal::select_word` → `text_utils::select_word`.
  3. Delete the now-dead string-flattening logic from `screen.rs`.
  4. Add Rust tests: line `"日本語 word"` — `get_word_at` at display col 0/2/4 returns `日本語`; col beyond the CJK run returns `word`; emoji + ZWJ line; a col landing on a wide-char spacer cell resolves to the char. Add matching Python tests in `tests/` via `terminal.get_word_at(col, row)`.
  5. Do NOT delete anything else from `text_utils.rs` — QA-007 handles remaining dead code after this lands.
- **Method**: The cell-based implementation sidesteps the col→char mapping entirely — that is why routing through it is correct rather than patching the arithmetic. Pitfall: `select_word` return conventions (`(start_col, end_col)` vs byte offsets) — match the existing Python-visible tuple contract (`docs/API_REFERENCE.md` documents it; keep it stable).
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize word` and `make dev && uv run pytest tests/ -k word -v`; `make checkall`.

### [QA-002] Decompose `decode_server_message` (complexity 240)
- **Files**: `src/python_bindings/streaming.rs:1312` (+ siblings :939, :1904)
- **Steps**: If ARC-003 landed, this is largely done — verify the four functions now delegate to generated conversions and each remaining hand-written piece is < ~50 lines; extract any residual per-family helpers (`output_to_dict`, `graphics_to_dict`, ...). If ARC-003 was NOT executed, extract per-message-family functions from the giant match, one commit per family, preserving dict keys exactly (characterization tests from ARC-003 step 3 apply either way — write them first).
- **Method**: Behavior-preserving decomposition; dict keys are public API.
- **Verify**: parametrized dict-shape tests; `make checkall`; parsight `calculate_cyclomatic_complexity` on the file shows no function > ~40.

### [QA-003] Replace the five `Connected` constructors with a builder
- **Files**: `src/streaming/protocol.rs:883,900,922,944,968`; call sites in `src/streaming/server.rs` (`build_connect_message`), `tests/test_streaming.rs`
- **Steps**:
  1. Add `ServerMessage::connected_builder()` returning a `ConnectedBuilder` with optional `screen`, `theme`, etc., and a `build()` producing the variant; implement `connected_full` in terms of it.
  2. Mark the four partial constructors `#[deprecated(note = "use connected_builder()")]`; migrate `build_connect_message()` and tests to the builder.
  3. Update the CLAUDE.md "extending Connected" checklist to name the builder as the single edit site.
- **Method**: Builder means the next optional field is one method, not a doubling of constructors. Keep the deprecated fns one release for rlib consumers.
- **Verify**: `cargo test --no-default-features --features pyo3/auto-initialize,streaming` (deprecation warnings only in tests we migrated away anyway); `make checkall`.

### [QA-004] Dedupe `screenshot`/`screenshot_to_file`/`resize_pixels` across binding wrappers
- **Files**: `src/python_bindings/terminal/mod.rs:267,345,379`, `src/python_bindings/pty.rs:184,431,504`, `src/python_bindings/common.rs` (macro layer; migration note at :28)
- **Steps**:
  1. Read the existing macro pattern in `common.rs` (it generates shared accessors for both wrapper types).
  2. Add the three method bodies to that macro layer (they differ only in how they reach the inner `Terminal` — the same axis the existing macros abstract).
  3. Delete the duplicated bodies from both wrapper files; confirm `#[pyo3(signature = ...)]` attributes carry over identically (the 20-param default list must stay byte-identical — copy it once into the macro).
  4. Docstrings move with the methods; API_REFERENCE is already correct (no doc change).
- **Method**: This is the completion of the migration `common.rs:28` already announces. Pitfall: pyo3 macro-in-macro hygiene — the existing accessor macros show the working pattern for `#[pymethods]` blocks generated by macro; follow it exactly rather than inventing a new expansion shape.
- **Verify**: `make dev && uv run pytest tests/ -k screenshot -v`; Python `inspect.signature` of both classes' `screenshot` matches pre-change (add a test asserting the two classes' signatures are equal); `make checkall`.

### [QA-005] Frontend test harness
- **Files**: `web-terminal-frontend/package.json`, new `web-terminal-frontend/vitest.config.ts`, new tests under `web-terminal-frontend/lib/__tests__/`, `Makefile` (`test-web`, wire into `checkall`)
- **Steps**:
  1. Requires QA-008's `TerminalConnection` extraction — do that first.
  2. `bun add -d vitest @vitest/ui happy-dom` (or jsdom); add `"test": "vitest run"` script.
  3. Tests: `lib/protocol.ts` encode/decode round-trips for representative message types; `TerminalConnection` with a mock WebSocket — connect/backoff schedule, heartbeat send + stale-pong disconnect, dispatch routing, snapshot size-guard rejection, local-echo gating.
  4. Makefile: `test-web: cd web-terminal-frontend && bun run test`; append to `checkall`.
- **Method**: Test the extracted pure modules, not the React component — no need for React Testing Library in this pass. Mock WS as a class with controllable event emission.
- **Verify**: `make test-web` green; `make checkall` includes and passes it.

### [QA-006] Decompose `write_char`
- **Files**: `src/terminal/write.rs:20` (~500 lines)
- **Steps**:
  1. Extract in this order, one commit each, running the Unicode test files between: `try_combine_regional_indicator` (the flag-pairing block), `try_apply_combining_mark` (combining/ZWJ/variation-selector block), `write_normal_cell` (the straight-line cell write incl. wide-char spacer handling). `write_regional_indicator_first` shows the intended helper shape.
  2. Keep all helpers private to the module; no signature changes to `write_char` itself.
- **Method**: The existing tests (flag emoji, skin-tone, ZWJ files) are the safety net — run them per extraction, not just at the end. Do not change behavior; resist cleanups beyond the three extractions.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize` (full lib — write paths are everywhere); `make checkall`; complexity of `write_char` drops below ~40 (parsight `calculate_cyclomatic_complexity`).

### [QA-007] Remove verified dead code
- **Files**: `src/cell.rs:437,459` (also :260, :287, :420 — lower confidence, verify), `src/graphics/mod.rs:401,609,626`, `src/grid/export.rs:254`, `src/graphics/serialization.rs:357`, `src/text_utils.rs` (post-QA-001 leftovers), `python/par_term_emu_core_rust/debug.py`, `src/mouse.rs:7` (`MouseMode::X10`)
- **Steps**:
  1. **External-consumer check first**: `grep -rn "from_grapheme_normalized\|recalculate_width\|cell_size\|remove_virtual_placement\|get_placeholder_graphic\|export_visible_screen_styled\|export_json_pretty" ../par-term/src ../par-term-emu-tui-rust/` and `grep -rn "debug\b\|DebugLogger\|log_render" ../par-term-emu-tui-rust/src`. Anything referenced externally gets `#[deprecated]` this cycle instead of deletion; report which.
  2. Delete confirmed-unreferenced items; for `debug.py`, if the TUI project imports it, leave it and note; if not, delete the module and its `__init__` export if any.
  3. `MouseMode::X10`: do NOT delete — ENH-006 plans to wire it (DECSET 9). Leave with a comment only if clippy complains (it's a pub enum variant; it won't).
  4. `text_utils.rs`: after QA-001, delete any remaining zero-caller helpers EXCEPT what QA-001 now uses.
- **Method**: Public rlib surface means rustc is silent — the greps are the evidence. Deletion of public API is semver-relevant: batch these into the changelog under a minor/major note per the repo's changelog rules.
- **Verify**: `make checkall`; `grep` confirms no dangling references; CHANGELOG.md updated with removals.

### [QA-008] Extract `TerminalConnection`; split OnscreenKeyboard data from behavior
- **Files**: `web-terminal-frontend/components/Terminal.tsx` (init effect ~line 278, ~670 lines), new `web-terminal-frontend/lib/terminal-connection.ts`, `components/OnscreenKeyboard.tsx`, new `lib/keyboard-layouts.ts`
- **Steps**:
  1. Create `TerminalConnection` (framework-free class): owns the WebSocket, reconnect/backoff state, heartbeat timer + stale-pong detection, message decode + a typed event-emitter/callback map for dispatch. Constructor takes url + callbacks; exposes `connect()`, `dispose()`, `send(msg)`.
  2. Move the corresponding logic out of the init effect verbatim (behavior-preserving); the component subscribes callbacks and keeps only xterm wiring + React state.
  3. Move the static key-layout data arrays from `OnscreenKeyboard.tsx` into `lib/keyboard-layouts.ts`; component keeps behavior.
  4. `make web-build-static` after (CLAUDE.md rule) — but per the audit constraint the /fix-audit agent does this, committing rebuilt `web_term/` with the change.
- **Method**: This is the enabling move for QA-005; do not add tests here (QA-005's job), do not redesign the protocol handling — extraction only. Pitfall: React StrictMode double-invocation of effects — `dispose()` must be idempotent and the effect must return it.
- **Verify**: `cd web-terminal-frontend && bun run build` clean; manual smoke via `make streamer-run-http` (connect, type, resize, kill server → reconnect banner); `make web-build-static`.

### [QA-009] Delegate duplicated color/pixel logic to core
- **Files**: `src/python_bindings/types/graphics.rs:137,168`, `src/graphics/mod.rs:351,375`, `src/sixel.rs:209`, `src/terminal/screen.rs:212` (`rgb_to_hsl`), `src/terminal/colors.rs:252`, `src/color_utils.rs:313,347`
- **Steps**:
  1. Bindings: make `types/graphics.rs` `sample_half_block` and pixel accessors call the `src/graphics/mod.rs` implementations (add pub(crate) fns there if needed). Reference pattern: `src/python_bindings/color_utils.rs` (delegates everything).
  2. HSL: keep `color_utils::to_hsl`/`from_hsl` as canonical. Rewrite `screen.rs::rgb_to_hsl` as a thin adapter: call `to_hsl`, divide s/l by 100.0 into the `ColorHSL` 0–1 scale. First write a test capturing current outputs of BOTH paths on a color sweep — if the formulas disagree beyond scale (the audit says the saturation formulas differ), the canonical result wins; note the delta in the report since `colors.rs:252` consumers may see small value changes.
  3. Add a round-trip property test: for a grid of RGB values, `from_hsl(to_hsl(c)) ≈ c` within 1/255 per channel, and adapter scale consistency.
- **Method**: The drift is the bug — unifying may change third-decimal HSL outputs on one path; that is intended and must be called out, not hidden.
- **Verify**: new tests; `make dev && uv run pytest tests/ -k "hsl or color" -v`; `make checkall`.

### [QA-010] Stop the `CSI Ps * x` spurious DECREQTPARM reply
- **Files**: `src/terminal/sequences/csi/mod.rs:83-90` (dispatch), `src/terminal/sequences/csi/report.rs:188-205` (handler it wrongly reaches)
- **Steps**:
  1. In the CSI dispatch for final byte `x`, check the intermediates: if intermediate is `*` (DECSACE), consume as a no-op (return without reply). Only bare `x` (no intermediate) proceeds to DECREQTPARM.
  2. Add a Rust test: feed `\x1b[2*x` → assert no response bytes are queued; feed `\x1b[x` → assert the DECREQTPARM reply still comes.
  3. Full DECSACE support is ENH-005 — do not implement it here.
- **Method**: Match how other intermediates are already distinguished in `csi/mod.rs` (e.g. the `$` and `'` families) — follow the existing dispatch idiom.
- **Verify**: new tests; `make checkall`. Then DOC-003 can document DECSACE as "parsed, not implemented".

### [QA-011] Narrow broad `pytest.raises(Exception)`
- **Files**: `tests/test_macros_extended.py:178,186,192,306`
- **Steps**: Each site's comment names the expected type (e.g. IOError/OSError). Change to `pytest.raises(OSError)` (or the actual type — run each test, observe the raised type, pin it), remove the `# noqa: B017`.
- **Verify**: `uv run pytest tests/test_macros_extended.py -v`; `make checkall`.

### [QA-012] Split `_native` registration by submodule
- **Files**: `src/lib.rs:181` (registration fn, complexity 127)
- **Steps**: Extract grouped helper fns (`register_core(m)?`, `register_streaming(m)?`, `register_types(m)?`, ...) called from the main `#[pymodule]` fn, keeping registration order identical. Pure mechanical grouping.
- **Verify**: `make dev && uv run python -c "import par_term_emu_core_rust as p; print(len(dir(p)))"` — count unchanged pre/post; `make checkall`.

### [QA-013] Library logging via `eprintln!`
- **Files**: `src/python_bindings/observer.rs:353,391`, `src/terminal/mod.rs:741`
- **Steps**: Add `log = "0.4"` (check it isn't already a dep), replace the three `eprintln!` with `log::warn!`/`log::error!` preserving messages. Do not add a logger initializer — that's the embedder's job.
- **Verify**: `make checkall`; grep for remaining production `eprintln!` outside `#[cfg(debug_assertions)]`/bin targets.

### [QA-014] CSI dispatch complexity — monitor only
- **Files**: `src/terminal/sequences/csi/{style.rs,mode.rs}`
- **Steps**: No action. Flat table-like VT dispatch is idiomatic. Revisit if churn continues (3 changes/90 days currently).
- **Verify**: n/a.

---

## Phase 3d — Documentation (DOC-003 after QA-010; DOC-007 after ARC-008; DOC-004/009/011 after ARC-002; DOC-016 after ARC-004)

### [DOC-001] Fix QUICKSTART.md
- **Files**: `QUICKSTART.md:22,32,124-125,130,142,145,152,204,219-220`
- **Steps**:
  1. :22 — "Rust 1.75+" → "Rust 1.98+".
  2. :32 — replace `<repository-url>` with `https://github.com/paulrobello/par-term-emu-core-rust`.
  3. :124-125 — replace the pinned `par-term-web-frontend-v0.9.0.tar.gz` URL with a link to `https://github.com/paulrobello/par-term-emu-core-rust/releases/latest` and an unversioned instruction ("download the `par-term-web-frontend-*.tar.gz` asset").
  4. :130, :145 — port 8080 → 8099.
  5. :142, :152 — `--features streaming` → `--features streaming-bin` (both the `cargo build --bin par-term-streamer` and `cargo install` forms).
  6. :204, :220 — "33 example scripts" → "39" or drop the number ("the example scripts in `examples/`").
  7. :219 — API documentation link → `docs/API_REFERENCE.md`.
- **Method**: Verify the corrected build command actually runs before writing it: `cargo build --bin par-term-streamer --no-default-features --features streaming-bin`.
- **Verify**: run the corrected commands; `grep -n "1.75\|8080\|v0.9.0\|streaming\b" QUICKSTART.md` shows no stale forms.

### [DOC-002] Fix docs/RUST_USAGE.md
- **Files**: `docs/RUST_USAGE.md:81,83,89,91,97,99,108,111,182,217,302-307,314,348,355,362,418-427`
- **Steps**:
  1. Recipes at :81/:89 — add `"pty_session"` to the feature lists of every recipe whose examples use `PtySession` (the "no Python" and "streaming" recipes).
  2. :307 — `use std::sync::Mutex;` → `use parking_lot::Mutex;` and note parking_lot in the recipe's `[dependencies]`; or keep std and fix call sites to `.lock().unwrap()`. Choose parking_lot (matches the crate's own internals).
  3. :348-349, :355, :362 — align `.lock()` usage with the choice above.
  4. :418-427 — add `pty_session` and `sim` rows to the feature table (source of truth: `Cargo.toml` `[features]` and CLAUDE.md's feature table).
  5. Version pins :97,:99,:108,:111 — `0.43` → `0.46`.
  6. **Compile-check every recipe**: create a scratch crate in the scratchpad, paste each `[dependencies]` block + example, `cargo check` with a `path = ` dependency on the repo. This is the acceptance bar — the audit found these fail; the fix must prove they compile.
- **Verify**: scratch-crate `cargo check` per recipe passes; `make checkall` untouched.

### [DOC-003] Fix docs/VT_TECHNICAL_REFERENCE.md false claims
- **Files**: `docs/VT_TECHNICAL_REFERENCE.md:128-151,314-316,328,354-368,1361,1382,1397,1401,1456-1476,1513-1514`
- **Steps**:
  1. :128-151 — remove the XTPUSHCOLORS/XTPOPCOLORS section or mark ❌ Not implemented (they are not wired; ENH-003 may add them later — do not pre-document).
  2. :314-316, :1401, :1513-1514 — alt-screen modes 47/1047/1048: mark ❌ (only 1049 supported); keep 1049 rows.
  3. :354-368, :1382 — DECSACE: change "✅ Full" to "Parsed and ignored (no-op)" per QA-010's landed behavior.
  4. :328, :1397 — mode 9 X10 mouse: mark ❌ (enum variant exists, never set).
  5. :1361, :1456-1460 — charset switching G0/G1: flip to ✅ Implemented, citing `src/terminal/sequences/esc.rs:118-125,425-594`, `perform.rs:69-75`, `write.rs:21-25`.
  6. :1474-1476 — fix the `CSI q` note (bare `CSI q` is XTVERSION per `csi/report.rs:66-78`); renumber Known Limitations (currently skips 3).
- **Method**: Every edit must be checked against the dispatch tables in `src/terminal/sequences/` — the audit verified each claim; do not re-add anything without finding the match arm.
- **Verify**: for each remaining ✅ in the edited sections, grep the named handler exists; `grep -n "XTPUSHCOLORS\|1047\|1048\|DECSACE" docs/VT_TECHNICAL_REFERENCE.md` shows only ❌/no-op rows.

### [DOC-004] Fix API_REFERENCE.md wrong signatures
- **Files**: `docs/API_REFERENCE.md:928,931,970-981,2119-2120`
- **Steps**:
  1. :931 — delete the `diff_snapshots()` entry (implementation is ENH-001; document when it exists).
  2. :970-981 — move the four color conversions out of the Static section; document as instance methods with float params per `src/python_bindings/terminal/color_api.rs:31,60,81,110` (saturation/lightness 0.0–1.0).
  3. :928 — `debug_log_snapshot(label)` — add the required positional arg (`common.rs:1819`).
  4. :2119-2120 — re-document `max_sessions`/`session_idle_timeout` as properties (`config.max_sessions = 10`), matching `streaming.rs:184-199`.
  5. If ARC-002's stub exists, cross-check each corrected signature against `_native.pyi`.
- **Verify**: `make dev && uv run python` — execute each corrected example snippet, no exceptions.

### [DOC-005] Sync docs/ARCHITECTURE.md to v0.46.0
- **Files**: `docs/ARCHITECTURE.md:5,256,429-460,936-950`
- **Steps**:
  1. :936-950 — add `pty_session` and `sim` rows; correct `python`/`streaming-bin` dependency lists (source: `Cargo.toml [features]`).
  2. :5 — "Last verified against v0.45.0" → v0.46.0.
  3. :256 — 35 → 37 server message types (count the `protocol.rs:227-684` enum at fix time — may have grown).
  4. Add missing module entries: `src/ffi.rs` (C ABI), `src/observer.rs`, `src/zone.rs`, `src/bin/streaming_server/` (post-ARC-005 layout if landed), `src/unicode_width_config.rs`, `src/unicode_normalization_config.rs`, `src/streaming/terminal.pb.rs` (generated).
  5. :429-460 — extend the data-flow diagram: insert the Kitty APC pre-filter stage before the VTE parser (consistent with :169) and add observer/streaming/FFI consumers beside the Python API.
- **Verify**: every file path cited resolves (`for f in $(grep -oE 'src/[a-z_/.]+\.rs' docs/ARCHITECTURE.md | sort -u); do test -f "$f" || echo "MISSING $f"; done`).

### [DOC-006] Fix README install refs
- **Files**: `README.md:1152-1155,1167-1168,1481-1482,1489`
- **Steps**:
  1. :1152-1155 — dependency table `version = "0.43"` → `"0.46"`; fix the "Rust Only" row's feature guidance per DOC-002's corrected recipes (needs `pty_session` for PTY).
  2. :1167-1168, :1481-1482 — replace the versioned-filename-under-`latest` URL with the releases-page link (same fix as DOC-001 step 3).
  3. :1489 — `web_term/README.md` → `web-terminal-frontend/README.md`.
- **Verify**: `test -f web-terminal-frontend/README.md`; no `v0.45.0.tar.gz`/`0.43` remain in install sections.

### [DOC-007] Fix CONTRIBUTING.md / CLAUDE.md stale binding paths
- **Files**: `CONTRIBUTING.md:67,75-76`; `CLAUDE.md` (Key Source Layout `types.rs`; Python Binding Sync `src/python_bindings/terminal.rs` ×2)
- **Steps**: Replace `src/python_bindings/terminal.rs` → `src/python_bindings/terminal/` (adding: new methods go in the themed `*_api.rs` file or the `common.rs` macro layer); `types.rs` → `types/`. Run after ARC-008 (same CLAUDE.md).
- **Verify**: `grep -n "python_bindings/terminal.rs\|types.rs" CONTRIBUTING.md CLAUDE.md` → no hits (mind `types.rs` false positives on other paths).

### [DOC-008] Fix docs/STREAMING.md TOC, `--max-clients`, missing config rows
- **Files**: `docs/STREAMING.md:21,22,34-42,38,302,464-484`
- **Steps**:
  1. Fix the three anchors: `#server-messages`/`#client-messages` → the actual GitHub-slug of `### Server Messages (ServerMessage oneof)` etc.; `#http-static-file-serving` — add the heading or retarget the entry.
  2. :34-42 — sync the Advanced Features sub-list to the actual heading order; add Multi-Session Management and ThemeInfo.
  3. :302 — `--max-clients` "(0=unlimited)" → "(0 rejects all connections)" per `server.rs:1332,1339`.
  4. :464-484 — add `allowed_origins` row to the StreamingConfig table; document `--preset` (`src/bin/streaming_server/main.rs:346`).
- **Verify**: a markdown link checker over the file (or manual anchor click-through in a rendered preview); flag count = 38/38 documented.

### [DOC-009] API_REFERENCE.md omissions + TOC
- **Files**: `docs/API_REFERENCE.md`
- **Steps**: Add documented entries (Google-style, matching neighbors) for: `ProgressBar` class, `SelectionMode` enum, `Terminal.progress_bar()`/`has_progress()` (`common.rs:1115,1126`), module functions `char_width_cjk`/`str_width`/`str_width_cjk`/`is_east_asian_ambiguous`, `encode_client_message`/`decode_client_message`. Add TOC rows for `## C-Compatible FFI`, `## See Also`, and the 7 missing `###` headings. Use ARC-002's stub as the signature source.
- **Verify**: `make dev && uv run python -c "import par_term_emu_core_rust as p; [getattr(p, n) for n in ['ProgressBar','SelectionMode','char_width_cjk','str_width','str_width_cjk','is_east_asian_ambiguous','encode_client_message','decode_client_message']]"`.

### [DOC-010] VT_SEQUENCES.md charset section; CurrentDir claim
- **Files**: `docs/VT_SEQUENCES.md:384,474-484`
- **Steps**:
  1. Add a "Character Sets" section: `ESC ( C` / `ESC ) C` G0/G1 designation (charsets supported per `esc.rs:425-594`), DEC Special Graphics/ACS, SO (0x0E) / SI (0x0F) — and add SO/SI rows to the Control Characters table.
  2. :384 — mark `OSC 1337;CurrentDir=` as not supported (or delete the row); ENH-002 may wire it later.
- **Verify**: every sequence added cites its handler; grep the handler exists.

### [DOC-011] Derive doc-forwarding + docstring backfill
- **Files**: `derive/src/lib.rs:26` (`pyo3_get_all`), `src/python_bindings/types/*.rs` (esp. `recording.rs:175-195`, `trigger.rs:256`), `src/python_bindings/streaming.rs`
- **Steps**:
  1. **Code half (biggest leverage)**: extend the derive macro to read each field's `///` doc attrs and emit them as `#[pyo3(get)]` property docs (pyo3 supports `#[pyo3(get)] /// doc` forwarding via `text_signature`/doc attrs on generated getters; in a proc macro, attach `#[doc = "..."]` to the generated getter fns). Bump `derive/Cargo.toml` version (triggers ARC-012's publish path).
  2. Verify with `make dev && uv run python -c "import par_term_emu_core_rust as p; assert p.PerformanceMetrics.<field>.__doc__"`.
  3. **Backfill half**: add Google-style docstrings (Args/Returns, Example where useful) to the undocumented 64/96 in `types/*.rs`, and Example sections to the top `streaming.rs` functions. Incremental — prioritize `recording.rs`, `trigger.rs`.
- **Method**: Run after ARC-002 so stub text and docstrings are written once. The derive change affects ~69 classes at once — that is the point.
- **Verify**: the `__doc__` assertion above; `make checkall`; spot-check `help(PerformanceMetrics)` output.

### [DOC-012] Archive README "What's New"
- **Files**: `README.md:16-992`, `CHANGELOG.md`
- **Steps**: Keep the latest 2–3 release sections in README; for each older section, diff its content against the corresponding CHANGELOG.md entry and merge anything missing into CHANGELOG before deleting from README; leave a pointer line ("Full history: CHANGELOG.md").
- **Verify**: no information lost (spot-diff three random old versions); README length drops ~900 lines.

### [DOC-013] CONFIG_REFERENCE.md OSC cap
- **Files**: `docs/CONFIG_REFERENCE.md` (Core Security Settings)
- **Steps**: Add `max_osc_data_length` / `set_max_osc_data_length` with default (128 MiB per `src/terminal/mod.rs:632,2107` — read the actual default constant at fix time) and type, mirroring SECURITY.md:966-967 wording.
- **Verify**: values match the code constant.

### [DOC-014] README doc links
- **Files**: `README.md:1098-1113`
- **Steps**: Add `docs/OBSERVERS.md`, `docs/INSTANT_REPLAY.md`, `docs/FFI_GUIDE.md` rows to the Documentation section.
- **Verify**: all three files exist; links resolve.

### [DOC-015] Archive stale audit artifacts
- **Files**: `AUDIT-2026-06-15.md`, `AUDIT-REMEDIATION-2026-06-15.md` (repo root)
- **Steps**: `git rm` both (history preserves them). Do NOT touch the current-cycle `AUDIT.md`/`AUDIT-REMEDIATION-PLAN.md`. (`/fix-audit` deletes the current AUDIT.md itself at wrap-up.)
- **Verify**: parsight `find_broken_doc_links` count drops accordingly on next reindex.

### [DOC-016] Fix stale code comments
- **Files**: `src/ffi.rs:267-268` (+ param name at the same site), `src/streaming/server.rs:413` (post-ARC-004: wherever `StreamingConfig` moved)
- **Steps**:
  1. `ffi.rs` — comment says observer events are JSON-encoded and names the param `event_json`; actual output is Debug format (`ffi.rs:321`). Fix the comment and rename the param to `event_text` (param rename is ABI-safe in C; header/doc `FFI_GUIDE.md:221` already says Debug — check whether the guide names the param and sync).
  2. `server.rs`/`config.rs` — idle-timeout doc comment "default: 300" → 900 (actual at `:457`).
- **Verify**: `make checkall`; FFI doc/impl grep agree.

### [DOC-017] Note proto vs serde-JSON naming divergence
- **Files**: `docs/STREAMING.md:763-815`
- **Steps**: Add a short admonition: the protobuf field names differ from the serde-JSON tags (`cwd_changed` vs `cwdchanged`, etc.); JSON consumers must use the serde tags in `src/streaming/protocol.rs`. Optionally add a two-column example.
- **Verify**: the named example tags match `protocol.rs` serde attributes.

### [DOC-018] Fix research note paths
- **Files**: `docs/research/OSC-9-4-PROGRESS-BAR-IMPLEMENTATION.md:8,333`
- **Steps**: Replace the absolute `/Users/probello/Repos/research/...` link with a relative or removed reference; `sequences/osc.rs` → `sequences/osc/` (current layout).
- **Verify**: paths resolve.

---

## Post-phase verification (Phase 4)

1. `make checkall` — full gate, zero failures.
2. `make dev && uv run pytest tests/ -x -q` — Python suite green.
3. `cargo test --no-default-features --features pyo3/auto-initialize,streaming` — streaming suite green.
4. `make web-build-static` — frontend builds; commit rebuilt `web_term/` if any frontend source changed.
5. `cargo audit` — no new advisories; RUSTSEC-2024-0320 and `paste` gone.
6. Doc link sweep: parsight `find_broken_doc_links` after reindex — count strictly lower than pre-remediation.
