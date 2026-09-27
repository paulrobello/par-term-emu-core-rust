# Audit Remediation Playbook

> Companion to `AUDIT.md` (2026-09-26, run 2, HEAD d53ed92, cycle tag `audit-2026-09-26-r2`).
> Entries follow the `## Remediation Plan` phase order. Each entry is written so a `/fix-audit`
> Opus 5 agent can execute it without re-deriving the analysis. Line numbers are from d53ed92.
> Re-read every file before editing (parsight `get_source_window`), because earlier phases move lines.
>
> **Standing gates** (run after each batch; details are in CLAUDE.md):
> - `make checkall` is the full gate.
> - Mux work: `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`.
> - Streaming work: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::`.
> - Windows-sensitive changes (mux, PTY, `cfg(windows)`): the Windows VM playbook in CLAUDE.md, run from the **main checkout**.
> - Build the Python bindings with `make dev-streaming`, never `make dev`, before `make stubs` (the stub-regen trap).

---

## Phase 1 — Security (sequential)

### [SEC-108] Client never verifies mux server identity; legacy `/tmp` probe
- **Files**:
  - `src/mux/ipc.rs:293-303` (`connect_local_stream`), `:118-125` (`peer_is_current_user`), `:404` (`current_uid`), `:419-421` (`legacy_socket_path`)
  - `src/mux/client.rs:49-58` (`connect_or_spawn`)
  - `docs/SECURITY.md` "Socket Permissions" (~:933-957)
  - Tests: in `src/mux/ipc.rs` / `src/mux/client.rs` test modules, or `tests/mux_daemon.rs`
- **Steps**:
  1. In `ipc.rs`, add a Unix-only `fn server_is_current_user(stream: &LocalStream) -> bool`. It has the same body as `peer_is_current_user` (`peer_creds()` → `euid()` → `== current_uid()`, fail closed). Reuse `peer_is_current_user` directly if its signature fits, since the check is symmetric.
  2. In `connect_local_stream`'s `#[cfg(unix)]` arm, after `LocalStream::connect(name)?`, return `Err(io::Error::new(io::ErrorKind::PermissionDenied, "par-mux: socket server is owned by another user"))` unless the check passes. This covers `connect`, `connect_or_spawn`, explicit `--socket` paths and the legacy probe.
  3. Add `pub(crate) fn legacy_socket_is_trustworthy(path: &Path) -> bool` in `ipc.rs` (Unix). It uses `std::fs::symlink_metadata(path)` and requires `file_type().is_socket()` (`std::os::unix::fs::FileTypeExt`) and `metadata.uid() == current_uid()` (`MetadataExt`).
  4. In `client.rs` `connect_or_spawn`, change the `#[cfg(unix)]` probe so it runs only when `std::env::var_os("XDG_RUNTIME_DIR").is_none()` **and** `legacy_socket_is_trustworthy(&legacy)`. Update the doc comment above it: pre-0.52 builds used `$XDG_RUNTIME_DIR` when set (verified in `git show 17060c6~30:src/mux/ipc.rs:335-345`), so the temp-dir legacy path is only a real upgrade target when XDG is unset.
  5. Windows (same PR if feasible, otherwise a follow-up card): in `connect_local_stream`'s `#[cfg(windows)]` arm, get the server PID (`GetNamedPipeServerProcessId`, or interprocess `peer_creds()` which returns the PID on Windows in 2.4.4). Open its process token and compare the user SID with the current token's SID, refusing on mismatch. If `windows-sys` features are missing, add only the ones needed (`Win32_System_Pipes`, `Win32_Security`, `Win32_System_Threading`).
  6. Tests (Unix):
     - A unit test for `legacy_socket_is_trustworthy`: a regular file returns false, a nonexistent path returns false, and a socket bound in a tempdir returns true.
     - A test that the client refuses a socket whose euid differs. Cross-user setup needs root, so factor the comparison into `fn euid_matches(peer: Option<u32>, me: u32) -> bool` and unit-test that, plus an integration test showing a same-user connect still works.
  7. Update `docs/SECURITY.md` Socket Permissions: the client verifies the server euid, and the legacy probe is skipped under XDG and requires a same-owner socket.
- **Method**:
  - The accept side already enforces peer identity. The missing half is the client trusting the server.
  - Putting the check in `connect_local_stream` rather than in `connect_or_spawn` closes every path at once.
  - Do not remove the legacy probe entirely. It protects upgrades of long-running pre-0.52 daemons on non-XDG hosts (macOS is per-user `$TMPDIR` anyway).
  - Enumerate callers with parsight `get_symbol_context connect_local_stream` (streaming `mux_factory.rs` uses it too). Every caller should get the same behavior.
  - Pitfall: `peer_creds()` needs `interprocess::local_socket::traits::StreamCommon` in scope.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - Windows VM `cargo check --all-targets` plus the mux lib test filter
  - `make checkall`

### [SEC-109] Kitty zlib bomb; unbounded APC and chunk buffers
- **Files**:
  - `src/graphics/kitty.rs:505` (`data_chunks.push`), `:536-560` (`get_data`, `decompress_zlib`), `:1163` (existing `MAX_IMAGE_PIXELS` use)
  - `src/terminal/apc_filter.rs:123-146` (`apc_buffer.push`)
  - `src/terminal/mod.rs:2811-2839` (caller)
  - `docs/SECURITY.md:895`
  - `fuzz/fuzz_targets/kitty.rs` (unchanged, but re-run it)
- **Steps**:
  1. Add constants in `kitty.rs`:
     - `pub const MAX_KITTY_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;` (accumulated decoded chunk bytes)
     - `pub const MAX_KITTY_DECOMPRESSED_BYTES: usize = MAX_IMAGE_PIXELS * 4;` (check the type and cast).
  2. In `parse_chunk`, before `self.data_chunks.push(decoded)`, compute `let total = self.data_chunks.iter().map(Vec::len).sum::<usize>() + decoded.len();`. Better, keep a running `data_bytes: usize` field reset in `reset()`. If `total > MAX_KITTY_PAYLOAD_BYTES`, call `self.reset()` and return `Err(GraphicsError::KittyError("payload exceeds limit"))`.
  3. Rewrite `decompress_zlib(data, limit)`. Keep the empty-input early return. Use `ZlibDecoder::new(data).take(limit as u64 + 1).read_to_end(&mut out)`, and return `Err(GraphicsError::KittyError("decompressed payload exceeds limit"))` if `out.len() > limit`. Choose the limit per call: for `f=24`/`f=32` with `s` and `v` known, use `s*v*3` or `s*v*4` via `checked_mul`, falling back to `MAX_KITTY_DECOMPRESSED_BYTES`. For PNG, use `MAX_KITTY_DECOMPRESSED_BYTES`.
  4. `get_data` currently swallows the decompression error and falls back to raw. Change it to `pub fn get_data(&self) -> Result<Vec<u8>, GraphicsError>` and propagate. Callers: find them with parsight `get_symbol_context KittyParser::get_data` (for example `build_graphic`). Update every caller. Keeping a raw fallback on a limit error would feed compressed bytes to the decoder, so propagate instead.
  5. In `apc_filter.rs`, add `const MAX_KITTY_APC_BYTES: usize = 96 * 1024 * 1024;` (base64 of 64 MiB plus parameters). Once `apc_buffer.len() >= MAX`, stop appending and set a `truncated`/`overflow` flag in the state. When the APC terminates, skip `on_kitty` (drop it) and `debug_error!` once. Either widen `ApcFilterState` with an `Overflow` variant that swallows bytes until ST, or keep a bool beside the buffer. The goal: memory stops growing and the rest of the APC is consumed without being rendered.
  6. `docs/SECURITY.md:895`: state that the 1 MiB zlib cap applies to the streaming wire protocol (`proto.rs`). Add a line covering the kitty caps (payload 64 MiB, decompressed ≤ `s*v*4` or `MAX_IMAGE_PIXELS*4`, APC 96 MiB).
  7. Tests:
     - `kitty.rs`: build a zlib stream of 512 MiB of zeros (`flate2::write::ZlibEncoder` over `std::io::repeat(0).take(..)`; around 500 KB compressed, fine in a test), base64 it, send it with `a=T,f=32,s=1,v=1,o=z` through `parse_chunk` and `build_graphic`, and assert `Err`. Assert the error arrives quickly: `decompress_zlib` with a small limit returns `Err` without allocating more than `limit+1`.
     - A unit test on `decompress_zlib` with limit 16 against 1 KiB of input.
     - `apc_filter`: feed `ESC _ G` plus `MAX+10` bytes plus `ESC \`, and assert `on_kitty` is not called and the buffer capacity does not exceed about MAX.
- **Method**: The bomb works because decompression finishes before any size check. `take(limit+1)` makes the check streaming. The chunk and APC caps close the uncompressed paths (a chain of `m=1` chunks, or one huge APC). 64 MiB matches the existing file-transfer caps. Pitfall: `data_chunks` must reset on every `reset()` path, including errors, or a later APC inherits the counter.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize kitty`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize apc`
  - `cargo +nightly fuzz run kitty -- -max_total_time=60 -rss_limit_mb=512` from `fuzz/`
  - `make checkall`

### [SEC-111] `t=t` parent-directory race — open and delete the canonical path
- **Files**: `src/graphics/kitty.rs:1063-1098` (canonicalize, then `open_no_follow(path)`), `:1128-1134` and `:1011` (delete by `path`)
- **Steps**:
  1. Hoist `canonical` so it is computed for `t=t` (and `t=f`, harmlessly) before step 4. Pass `&canonical` to `open_no_follow` and use it in error messages.
  2. After opening, capture `(dev, ino)` from `file.metadata()` (Unix `MetadataExt`).
  3. At the delete site, re-`symlink_metadata(&canonical)`, compare `(dev, ino)` with the captured pair, and `remove_file(&canonical)` only on a match. On a mismatch, skip the delete and `debug_error!`. Windows: delete `canonical` without the inode check (O_NOFOLLOW is not available there either). Document that in a comment.
  4. Test (Unix): create a `tty-graphics-protocol` PNG in a tempdir under an allowed temp root, load it with `t=t`, and assert it was deleted. Then replace the file between load and delete (hard to time), or unit-test the `(dev, ino)` comparison helper directly.
- **Method**: `canonicalize` resolves every intermediate symlink. Opening the canonical string means a later directory swap cannot redirect the open. The dev/inode check keeps the delete on the same file that was read. Keep the behavior "delete only after a successful decode".
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize kitty`; Windows VM `cargo check --all-targets`; `make checkall`.

### [SEC-110] Mux line cap bypassed by an unterminated stream; UTF-8 split drop
- **Files**: `src/mux/server.rs:490-530` (the read loop in `handle_client`), `MAX_CONTROL_LINE_BYTES`, `tests/mux_daemon.rs:862-883`
- **Steps**:
  1. Replace `reader.read_line(&mut line)` with a byte-level loop over a `Vec<u8>` buffer (`let mut buf: Vec<u8>`):
     ```rust
     match reader.fill_buf() {
         Ok([]) => /* EOF: same handling as Ok(0) today, using buf.is_empty() */,
         Ok(chunk) => {
             let nl = chunk.iter().position(|&b| b == b'\n');
             let take = nl.map_or(chunk.len(), |i| i + 1);
             buf.extend_from_slice(&chunk[..take]);
             reader.consume(take);
             if buf.len() > MAX_CONTROL_LINE_BYTES { /* existing oversize arm */ }
             if nl.is_some() { break; }
         }
         Err(e) if is_poll_wake(&e) => { /* existing eviction check; buf is kept */ }
         Err(e) => /* existing error arm */,
     }
     ```
     After the loop, convert with `String::from_utf8(buf)`. On `Err`, set `undecodable = true` (the existing flag) instead of silently losing bytes. Timeout wakes no longer drop partial multibyte sequences, because the bytes stay in `buf`.
  2. Keep the oversize arm's behavior identical: register the client if needed, emit the `line exceeds 1 MiB budget` block, and `break 'connection`. The oversize check must run on every chunk.
  3. Fix the test comment at `tests/mux_daemon.rs:862-883`. Add `oversize_unterminated_stream_closes_connection`: write more than 1 MiB with no newline in one continuous burst of 64 KiB writes (no sleeps) from a thread, then assert the error block arrives and the daemon survives (a second client can `list-sessions`).
  4. Add a unit test or daemon test where `send-keys -l` carries a multibyte character split across two writes with a pause longer than `EVICTION_POLL` (200 ms) between them, and assert the pane receives the full character.
- **Method**: `BufRead::read_line` loops internally until newline or EOF, so the length check never runs mid-stream. `fill_buf`/`consume` returns control after each chunk. Pitfall: keep QA-134's later extraction simple by writing this loop as a self-contained block. QA-134 will lift it into `read_control_line`.
- **Verify**: `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`; the mux lib filter; Windows VM mux filter; `make checkall`.

### [SEC-112] Kitty file-media gate unreachable: single-session server and PtyTerminal
- **Files**:
  - `src/streaming/server.rs:355-371` (`with_config`); reference `:585-594` (factory path)
  - `src/pty_session.rs` (add pass-throughs)
  - `src/python_bindings/pty.rs`; `src/python_bindings/terminal/*_api.rs` (find the existing Terminal `set_allow_file_media` binding with parsight `find_symbol set_allow_file_media`, and copy its mode parsing)
  - `python/par_term_emu_core_rust/_native.pyi`, `docs/API_REFERENCE.md` (PtyTerminal section), `tests/test_pty*.py` or a new `tests/test_kitty_file_media.py`
- **Steps**:
  1. In `with_config`, before building the default session: `terminal.write().set_allow_file_media(config.kitty_file_media);`. Add a comment tag `(SEC-101/SEC-112)` matching the factory site.
  2. Add a Rust test that `StreamingServer::with_config(term, addr, StreamingConfig { kitty_file_media: FileMediaMode::Off, .. })` leaves `term.read().allow_file_media() == Off` (use the real getter name).
  3. `PtySession`: add `pub fn set_allow_file_media(&self, mode: FileMediaMode)` (write lock) and `pub fn allow_file_media(&self) -> FileMediaMode` (read lock).
  4. `pty.rs`: add `#[pyo3(signature = (mode))] fn set_allow_file_media(&self, mode: &str) -> PyResult<()>` and `fn get_allow_file_media(&self) -> String`, with the same string vocabulary (`off`/`temp_only`/`all`) and error type as the Terminal binding. Give both Google-style docstrings with Args, Returns and Example.
  5. `make dev-streaming && make stubs`, then check the `.pyi` diff contains only the two new methods.
  6. API_REFERENCE PtyTerminal section: add both methods (DOC-040 later rewrites the inheritance sentence).
  7. Add a Python test: `PtyTerminal(80,24).set_allow_file_media("off")`, then `get_allow_file_media() == "off"`, and an invalid mode raises `ValueError`.
- **Method**: The gate was applied only where the server creates terminals through the factory. The single-session constructor receives a caller-built terminal, so the config never reaches it. Follow the CLAUDE.md binding-sync checklist (Rust impl, binding, docs, tests).
- **Verify**: the streaming lib filter; `uv run pytest tests/ -k file_media -v`; `make stub-check`; `make checkall`.

---

## Phase 2 — Architecture (sequential; unblocks QA/DOC)

### [ARC-041] Early pane output lost; triplicated two-phase spawn (+QA-F)
- **Files**:
  - `src/mux/pane.rs:335-342` (`on_output`), and `PaneFactory::create_pane` plus its implementors (`:426`, `:441+` default factory, `:613`, `:634`, the test doubles around `:734`)
  - `src/pty_session.rs:190,227,707`
  - `src/mux/dispatch.rs:229-273,515-555,694-735`
  - `src/mux/server.rs:968-985` (`wire_all_pane_outputs`, `pane_output_sink`)
  - `src/mux/tree.rs` `begin_*`/`complete_*`
- **Steps**:
  1. Confirm the ordering first: `PtySession::spawn_internal` calls `start_reader_thread` at `:707`, and the reader clones `output_callback: Arc<Mutex<Option<_>>>` (read the reader loop to see whether it reads the callback per chunk under the mutex). If it re-reads per chunk, the fix is to install the callback **before spawn**.
  2. Extend the spawn context passed to `create_pane` (the struct built from the `begin_*` plan; find it with `get_symbol_context create_pane`) with `output_sink: Option<OutputCallback>`. In each factory implementation, after constructing the `PtySession` and **before** calling `spawn*`, call `session.set_output_callback(sink)` when present.
  3. In `dispatch.rs`, build the sink with `pane_output_sink(ctx.clients, plan.pane_id)` from the reserved pane id before dropping the tree lock, and put it in the spawn context. Remove the post-`complete_*` `pane.on_output(...)` loops.
  4. Extract `fn spawn_two_phase<P, T>(ctx: &DispatchCtx, begin: impl FnOnce(&mut MuxTree) -> Result<P, MuxError>, complete: impl FnOnce(&mut MuxTree, P, MuxPane) -> Result<T, MuxError>) -> Result<T, MuxError>`. It locks, runs `begin`, drops the lock, calls `create_pane` with the sink, relocks, runs `complete`, and writes the cwd note. Rewrite `cmd_new_session`, `cmd_split_window` and `cmd_new_window` on top of it with early returns (`?`), flattening the nested `match`. Keep the rollback semantics: `complete_split` must still kill a pane whose target vanished.
  5. `wire_all_pane_outputs` (restore path): leave it as is, or reuse the sink construction. Restored panes spawn inside `from_persist_state`, so keep the current behavior unless it is trivial to thread the sink through (note it in the report).
  6. Delete `MuxTree` one-shot wrappers (`tree.rs:300-432`) only if they are test-only. Check with `find_symbol` and `get_symbol_context`, and migrate tests to `begin_*`/`complete_*` if you remove them. If in doubt, leave them.
  7. Test: a `PaneFactory` double whose pane emits `b"PROMPT$ "` immediately after spawn, plus a hook that delays `complete_*` (for example, hold the tree lock from another thread for 100 ms). Assert the observing client receives a `%output` containing `PROMPT$`.
- **Method**: The pane id is reserved in `begin_*`, so the sink can be built before the pane exists. `pane_output_sink` only needs the client registry and the id. Enumerate factory implementors with parsight `analyze_relationships query_type=overrides target=create_pane`. Pitfall: sink installation must happen before `spawn*` or the race remains.
- **Verify**: the mux lib filter; `cargo test --test mux_daemon ...`; the Windows VM mux filter (ConPTY spawn timing); `make checkall`.

### [ARC-042] Single-source wheel feature set
- **Files**: `pyproject.toml:74-77`; `.github/workflows/deployment.yml:238,293,299,344,382`; `.github/workflows/publish-testpypi.yml:61,83`; `Makefile:118-180`
- **Steps**:
  1. `pyproject.toml`: `[tool.maturin] features = ["pyo3/extension-module", "streaming"]`.
  2. Remove `--features streaming` from the maturin `args` in `deployment.yml`. maturin reads pyproject, and CLI features would be additive but duplicated. Check the TestPyPI args get no feature flag either.
  3. Makefile: `dev` should get streaming from pyproject automatically (`maturin develop` honors `[tool.maturin] features`). Keep `dev-streaming` as an alias of `dev` (other docs and memory reference it), and add `dev-fast: uv run maturin develop --release --no-default-features --features pyo3/extension-module,python` for streaming-less builds. Verify the flag combination works (maturin may need `--features python` because `python` is the default).
  4. Run `make dev`, then `uv run python -c "import par_term_emu_core_rust as p; print(p.HAS_STREAMING)"` and expect `True`.
- **Method**: With a single source in pyproject, wheels, sdist, TestPyPI and local dev all agree. Pitfall: maturin's `features` in pyproject combined with `--no-default-features` on the CLI. Test `make dev-fast` actually builds.
- **Verify**: the `make dev` + HAS_STREAMING check; `make stubs && git diff --stat python/` (no streaming classes lost); `uv run maturin sdist` and inspect that `pyproject.toml` in the sdist carries the features; `make checkall`.

### [ARC-043] Make `swash` optional behind `screenshot`
- **Files**: `Cargo.toml:148,215`
- **Steps**: `swash = { version = "0.2.7", optional = true }` and `screenshot = ["dep:swash"]`. Check whether any other feature pulls swash (it should not).
- **Verify**:
  - `cargo tree --no-default-features --features rust-only -i swash` must fail with "did not match"
  - `cargo check --no-default-features --features rust-only`
  - `cargo check --no-default-features --features sim`
  - `make checkall`

### [ARC-044] `mux-bin` feature for clap
- **Files**: `Cargo.toml:60-63,237`; `Makefile` (every `--features ...mux` that builds the `par-mux` binary; grep `par-mux`/`--bin par-mux`); `.github/workflows/*.yml` (the same grep); `docs/MUX.md` build lines
- **Steps**:
  1. `mux = ["pty_session", "interprocess", "widestring", "serde", "dirs", "dep:toml"]` (drop `clap`), and add `mux-bin = ["mux", "clap"]`.
  2. `[[bin]] par-mux` `required-features = ["mux-bin"]`.
  3. Update every build/install invocation of the binary to `--features mux-bin` (lib-only test invocations keep `mux`). Check the `lint` clippy features line includes `mux-bin`, otherwise the binary won't be linted.
  4. Check `src/bin/streaming_server` doesn't depend on mux's clap. It uses `streaming-bin`, which already has clap.
- **Method**: This mirrors the `streaming`/`streaming-bin` split. par-term enables `mux` as a library and stops compiling clap. Enumerate call sites with `grep -rn "features.*mux" Makefile .github docs README.md QUICKSTART.md CLAUDE.md`.
- **Verify**:
  - `cargo tree --no-default-features --features rust-only,mux -i clap` fails (not present)
  - `cargo build --no-default-features --features mux-bin --bin par-mux`
  - `make checkall`

---

## Phase 3a — Security (remaining)

### [SEC-113] Harden the Python debug log
- **Files**: `python/par_term_emu_core_rust/debug.py:25,55-62`
- **Steps**: Change `DEBUG_FILE` to `Path(tempfile.gettempdir()) / f"par_term_emu_debug_python_{os.getpid()}.log"`. Open with `fd = os.open(DEBUG_FILE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, "O_NOFOLLOW", 0), 0o600)` and `self.file_handle = os.fdopen(fd, "w", buffering=1)`. On `OSError`, disable logging (the existing except arm). Update any docs that name the old filename (`grep -rn par_term_emu_debug_python docs README.md`).
- **Verify**: `DEBUG_LEVEL=1 uv run python -c "import par_term_emu_core_rust.debug as d; ..."` shows a file with mode 0600 (`stat -f %Lp`); `make lint-python`; `make checkall`.

### [SEC-114] Track `paste` (RUSTSEC-2024-0436)
- **Files**: none, unless a direct dependency can drop it
- **Steps**: `cargo tree -i paste` to find the parent crate. If a newer version of the parent drops `paste`, bump it. Otherwise record the advisory in `docs/SECURITY.md` (Dependencies section) as accepted and informational, and add an `audit.toml`/`deny.toml` ignore entry with a justification only if a `cargo audit`/`cargo deny` step exists.
- **Verify**: `cargo audit` shows only the known item (or none).

---

## Phase 3b — Architecture (remaining)

### [ARC-039] Terminal god object — bounded slice: MacroEngine + TriggerEngine, pin forwarder removal
- **Files**: `src/terminal/macros.rs` (16 methods, `:11-137`); `src/terminal/trigger.rs:137-402` (the `impl Terminal` block: `add_trigger`…`trigger_registry_mut`); `src/terminal/mod.rs:2698,2713`; `src/terminal/metrics.rs:290-320`; `CHANGELOG.md`
- **Steps**:
  1. Pin removal: update every `#[deprecated(since = "0.53.0", note = ...)]` forwarder so the note says `"... will be removed in 0.55.0"`. Record this in CHANGELOG `[Unreleased]` under Deprecated.
  2. **Scope guard**: only these two services this cycle. Do not touch colors, clipboard or notifications.
  3. Create `pub struct MacroEngine<'a> { term: &'a mut Terminal }` (or free functions in `terminal::macros` taking `&mut Terminal`, matching the `TerminalBenchmarks` pattern from c928d73; read `src/terminal/benchmarks.rs` and copy its shape exactly). Move the method bodies. Leave `#[doc(hidden)] #[deprecated(since = "0.54.0", note = "use terminal::MacroEngine; removed in 0.56.0")]` forwarders on `Terminal`.
  4. Do the same for triggers (`TriggerEngine`). Note that `process_trigger_scans` is called internally from the write path, so the internal call sites (parsight `get_symbol_context process_trigger_scans`) must call the new service, not the deprecated forwarder, to keep `-D warnings` clean.
  5. Python bindings keep their method names and call the new services (`src/python_bindings/terminal/*_api.rs`; find them with `find_code "load_macro binding"`). The Python surface stays unchanged, so no stub diff is expected.
  6. Before starting, check par-term call sites: `grep -rn "\.load_macro\|\.add_trigger\|\.play_macro\|poll_trigger_matches" ../par-term/src`. Record the count in the report. Forwarders keep it compiling.
  7. Add parity tests: each forwarder gives the same result as the service (copy the benchmarks parity-test pattern).
- **Method**: This is the slice that shrinks `impl Terminal` without breaking Python or par-term. Use `get_impact` on `Terminal::add_trigger` and `Terminal::load_macro` for blast radius.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize`; `make test-python`; `make stub-check` (no diff); `make checkall`.

### [ARC-040] Push-triggered cheap CI; bench rotates only on a green SHA
- **Files**: `.github/workflows/ci.yml`; new `.github/workflows/ci-fast.yml`; `.github/workflows/bench.yml:31`
- **Steps**:
  1. Create `ci-fast.yml` (`on: push: branches: [main]` and `pull_request`), with one `ubuntu-latest` job:
     - checkout, `dtolnay/rust-toolchain@stable` with clippy and rustfmt, `Swatinem/rust-cache` (pin tags per `~/.claude/guides/git-ci.md` and verify each ref resolves with `gh api repos/<owner>/<repo>/git/ref/tags/<tag>`)
     - `cargo fmt --check`
     - `cargo clippy --all-targets --no-default-features --features rust-only,mux-bin,serde,streaming -- -D warnings`
     - `cargo test --lib --no-default-features --features pyo3/auto-initialize`
     - the `sim` tree guard (copy the exact step from `ci.yml`)
  2. `bench.yml`: before rotating the tag, query `gh api repos/${{ github.repository }}/commits/${{ github.sha }}/check-runs --jq '[.check_runs[] | select(.name=="<ci-fast job name>") | .conclusion] | first'`, and set `BENCH_ROTATE_TAG=0` unless it is `success`. Give the job `checks: read` permission.
  3. Keep `ci.yml` dispatch-only (the full multi-OS matrix). Note the split in CONTRIBUTING.md.
- **Method**: Cheap feedback on push without the ~11 min full matrix. Pitfall: pin action tags, and don't use floating majors unless one exists.
- **Verify**: `actionlint` if available; push a branch and confirm the job runs (outward-facing, so confirm with the user before pushing).

### [ARC-045] Commit Cargo.lock
- **Files**: `.gitignore:4`, `Cargo.lock`, `.github/workflows/{ci,ci-fast,deployment,bench}.yml`, `CLAUDE.md:74`, `docs/BUILDING.md`
- **Steps**: Remove `Cargo.lock` from `.gitignore`, run `cargo generate-lockfile` (or keep the existing one), `git add Cargo.lock`. Add `--locked` to CI `cargo build/test/clippy` and maturin (`args: --locked ...`). Update the CLAUDE.md Windows playbook step 4 note to "use --locked". `git archive` now includes the lockfile.
- **Verify**: `cargo build --locked --no-default-features --features rust-only`; `make checkall`.

### [ARC-046] Layering moves
- **Files**: `src/grid/mod.rs:207-208,224`; `src/terminal/replay_snapshot.rs` (the `GridSnapshot` definition); `src/graphics/mod.rs:399,433`; the `unix_millis` definition in `src/terminal/`; `src/streaming/py_convert.rs` → `src/python_bindings/`
- **Steps**: Move `GridSnapshot` (and its serde derives) to `src/grid/snapshot.rs` and re-export it from `terminal::replay_snapshot` (`pub use crate::grid::snapshot::GridSnapshot;`). Move `unix_millis` to `src/time.rs` (`pub(crate)`) with a re-export at the old path. Move `py_convert.rs` under `python_bindings/` (it is `#[cfg(feature = "python")]`) and fix imports (`get_symbol_context` for each moved symbol).
- **Verify**: `cargo test --no-default-features --features rust-only,mux,serde`; `cargo check --no-default-features --features sim`; `make checkall`.

### [ARC-047] Push-based events in the streaming binary
- **Files**: `src/bin/streaming_server/bootstrap.rs:38,69,86,96,218-226,315,557-565`; `src/bin/streaming_server/main.rs:~618-636`
- **Steps**: Implement a `TerminalObserver` (see `src/python_bindings/observer.rs` and the `TerminalObserver` trait via `find_symbol TerminalObserver`) that forwards events into a `tokio::sync::mpsc::UnboundedSender` (events are small and bounded by terminal activity). Register it per session at creation, replace both 50 ms `interval` loops with `while let Some(ev) = rx.recv().await`, and change `Arc<Mutex<PtySession>>` to `Arc<PtySession>` where the mutex only serialized `&self` calls (check each use).
- **Verify**: `cargo build --no-default-features --features streaming-bin`; the streaming lib filter; a manual smoke test with `make streamer-run` (optional); `make checkall`.

### [ARC-048] WsTransport unification + `server.rs` split (after QA-144/143/147)
- **Files**: `src/streaming/server.rs:852,971,1784-1900,2078-2240`
- **Steps**: Define `trait WsTransport { async fn send(&mut self, bytes: Vec<u8>) -> Result<..>; async fn recv(&mut self) -> Option<Result<Vec<u8>, ..>>; }` with impls for tungstenite and axum sockets. Write one `run_session<T: WsTransport>` containing the connect message, keepalive, rate limiter and `select!`. Make both entry points thin wrappers. Then split the file into `server/{mod,accept,session,handlers,http}.rs` with no logic change.
- **Verify**: the streaming lib filter; `cargo test --test test_streaming --features streaming,...` (use the invocation from the Makefile `test-rust-streaming` target); `make checkall`.

### [ARC-049] `debug_*!` on the log facade
- **Files**: `src/debug.rs`; `src/grid/scroll.rs:152,198`
- **Steps**: Keep the macro names. Re-implement them as `log::{error,warn,info,debug,trace}!(target: $cat, ...)`. Keep the file logger as an optional `log::Log` implementation installed by the binaries and the Python module init (so existing `DEBUG_LEVEL` behavior holds for Python users). Replace the `eprintln!` calls with `log::warn!`.
- **Verify**: `DEBUG_LEVEL=3 make test-python` still writes the log file; `make checkall`.

### [ARC-050] Opt-in `ffi`; deprecate `Broadcaster`
- **Files**: `src/lib.rs:59`; `Cargo.toml` `[features]`; `src/streaming/mod.rs:82`
- **Steps**: Add `ffi = []`, then `#[cfg(feature = "ffi")] pub mod ffi;`. Check that no in-crate code uses `ffi` (`get_symbol_context`). Add `#[deprecated(since = "0.54.0", note = "unused; will be removed in 0.55.0")]` on `pub use broadcaster::Broadcaster` (put it on the type if re-export deprecation is not supported). Add CHANGELOG Deprecated/Changed entries.
- **Verify**: `cargo check --features ffi --no-default-features --features rust-only`; `make checkall`.

### [ARC-051] Non-mutating `lint-check`
- **Files**: `Makefile:266-275,312`
- **Steps**: Add `lint-check:` with `cargo clippy --all-targets --features python,streaming,mux-bin,serde,streaming-bin -- -D warnings`, `cargo fmt --check`, `uv run ruff format --check`, `uv run ruff check`, `uv run pyright`. Change `checkall` to depend on `lint-check` instead of `lint` and `lint-python`.
- **Verify**: `make checkall` on a clean tree leaves `git status` clean.

### [ARC-052] Gate `tokio` test-util and `serde_yaml_ng`
- **Files**: `Cargo.toml:82,88,182`
- **Steps**: Remove `"test-util"` from `[dependencies] tokio` (the `[dev-dependencies]` entry at `:182` keeps it). For `serde_yaml_ng`: if `src/macros.rs` is compiled in every profile, leave it and note why. If `macros` is only used by python/terminal features that are always on, it stays unconditional. Verify with `cargo tree -e features -i serde_yaml_ng --no-default-features --features sim`.
- **Verify**: `cargo check --no-default-features --features streaming`; `make checkall`.

### [ARC-053] Stop leaking binding macros
- **Files**: `src/python_bindings/common.rs` (16 `#[macro_export]`)
- **Steps**: Replace `#[macro_export]` with `macro_rules!` plus `pub(crate) use name;` after each definition (the Rust 2018+ in-crate macro export idiom). Update `crate::name!` call sites to path imports if needed.
- **Verify**: `cargo doc --no-deps` shows no root macros; `make checkall`.

### [ARC-055] Shutdown save off the tree lock
- **Files**: `src/mux/server.rs:202`; `src/mux/persist.rs:711-724` (`collect_persist_capture` and `capture` from ARC-032)
- **Steps**: Replace `save_to_with_origin(&self.tree.lock(), …)` with `let cap = collect_persist_capture(&self.tree.lock());` (the lock drops at the semicolon), then `let state = cap.capture(); write_state(&state, origin)` using the same helpers as the per-command path (`get_symbol_context collect_persist_capture`).
- **Verify**: the mux lib filter; the `mux_daemon` tests (shutdown save round-trip); `make checkall`.

### [ARC-056] Build stamp: verify the git toplevel
- **Files**: `build.rs:51-75,125-148`
- **Steps**: Run `git rev-parse --show-toplevel` and compare its canonicalized path with `CARGO_MANIFEST_DIR`. On a mismatch, use the source-digest path. Add a comment that `-dirty` is commit-granular (or add `cargo:rerun-if-changed=src` on the git path).
- **Verify**: `cargo build --no-default-features --features mux-bin --bin par-mux && ./target/debug/par-mux --version`; `make checkall`.

### [ARC-057] Fuzz the mux targets in CI and `fuzz-all`
- **Files**: `.github/workflows/fuzz.yml:18`; `Makefile:~855-866`
- **Steps**: Add `mux_parse_command` and `mux_hook_report` to the matrix list. Add `fuzz-mux_parse_command` and `fuzz-mux_hook_report` Makefile targets modeled on the existing ones, and add them to `fuzz-all` (updating its help text "all four" to "all six").
- **Verify**: `make fuzz-mux_parse_command FUZZ_SECONDS=10`; `make fuzz-all FUZZ_SECONDS=5`.

### [ARC-054] Root clutter (run LAST)
- **Files**: `debug/`, `theme.css`, root `AUDIT*.md`
- **Steps**: Check what references `theme.css` and `debug/*` (`grep -rn "theme.css\|debug/" --include=*.md --include=Makefile --include=*.py --include=*.toml .`). Move the debug scripts to `scripts/debug/`. Leave `theme.css` if referenced, otherwise ask. Note that `/fix-audit` deletes the root AUDIT files at wrap-up. Do not archive them yourself unless the user asks.
- **Verify**: `make checkall`; parsight `find_broken_doc_links` count drops.

---

## Phase 3c — Code Quality

### [QA-130] Read locks for read-only PtySession getters
- **Files**: `src/pty_session.rs:1369,1381,1393,1414,1437,1443,1450,1459,1467,1473,1550` (plus any other `self.terminal.write()` whose body only calls `&self` methods; there are 15 write sites)
- **Steps**:
  1. For each `write()` site, check the called Terminal method's receiver (`find_symbol` with `scope_path "Terminal::"`). If it is `&self`, change the site to `read()`.
  2. Screenshot sites: `screenshot::render_terminal(&Terminal, …)` takes `&Terminal`, so `read()` works.
  3. Update the module doc (line 5) if it needs to.
  4. Add a test: spawn a PTY session (`echo hi`) and hold `session.terminal().read()` (or equivalent accessor) on thread A. On thread B, call `content()` and `size()` and assert they complete within 1 s while A holds the read guard.
- **Method**: `parking_lot::RwLock` allows many readers. The reader thread's `process()` still takes the write lock, and that is correct. Pitfall: a getter that calls a `&mut self` method (for example one that clears dirty flags) must stay `write()`.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session`; `make test-pty`; `make checkall`.

### [QA-131] Bounded streaming input queue (+SEC-D) — batch with QA-141
- **Files**: `src/streaming/session.rs:84,89,329-371`; `src/streaming/config.rs:422`
- **Steps**:
  1. Change `pty_input_tx: RwLock<Option<mpsc::UnboundedSender<Vec<u8>>>>` to a bounded `mpsc::Sender<Vec<u8>>` with capacity `INPUT_QUEUE_MESSAGES` (for example 256), plus an `AtomicUsize queued_bytes` on the session for the byte budget (`MAX_QUEUED_INPUT_BYTES = 4 * 1024 * 1024`).
  2. In `enqueue_pty_input`:
     - If `queued_bytes + len > budget`, or `tx.try_send(bytes)` returns `Full`, drop the input, `metrics.dropped_messages.fetch_add(1)` (add the field if absent, and expose it wherever metrics are serialized), and emit `debug_warn!` rate-limited to once per second.
     - On `Closed`, bump `metrics.errors` and log.
     - The drain task decrements `queued_bytes` after each write.
  3. The drain task uses `rx.blocking_recv()` (the bounded receiver supports it). Log the `pty_writer == None` drop and count it.
  4. QA-141 (same edit): switch `pty_writer` and `pty_input_tx` to `parking_lot::RwLock`, removing the `.ok()` poison swallowing.
  5. Test: a session with a `pty_writer` whose `Write` impl blocks forever (a `std::sync::mpsc` gate). Enqueue 10 MiB in 64 KiB chunks, then assert `queued_bytes <= budget` and `dropped_messages > 0`.
- **Method**: Keep one channel per session so ordering stays serialized (the QA-110 invariant). Byte budgeting protects against a few very large pastes as well as many small messages.
- **Verify**: the streaming lib filter; `make test-rust-streaming` (check the target name in the Makefile); `make checkall`.

### [QA-132] Re-enable the screenshot/emoji tests; stop the streaming test skipping on a hang (after QA-130)
- **Files**: `tests/test_screenshot.py:258,275,340`; `tests/test_streaming.py:356`; `Makefile` (`test-pty` selection)
- **Steps**: Read how QA-111 structured `make test-pty` (`grep -n test-pty -A6 Makefile`) and how CI ignores that family. Rename or mark the two PTY screenshot tests so they fall into that family (for example `@pytest.mark.pty` if a marker exists, otherwise follow the file-naming convention). Replace the unconditional skip with `pytest.mark.skipif(os.environ.get("CI") == "true", reason=...)`. Run the emoji test locally. If it passes, remove its skip. If it hangs, find the hang with `timeout 60 uv run pytest tests/test_screenshot.py::<name> -x` and report instead of forcing it. For `test_streaming.py:356`, replace `pytest.skip(...)` with `pytest.fail(...)`, or `xfail(strict=False, reason=...)` if the timing is inherently racy.
- **Verify**: `make test-pty`; `uv run pytest tests/test_screenshot.py -v`; `make checkall`.

### [QA-133] Streamer `main`, `run_mux_mode`, `handle_csi_report` (+ARC-P)
- **Files**: `src/bin/streaming_server/main.rs:97-143,168-473,588`; `src/bin/streaming_server/bootstrap.rs`; `src/terminal/sequences/csi/report.rs:7-247`
- **Steps**:
  1. Add a `RunMode::Mux` variant (or pass a server future) so `run_mux_mode` uses `serve_until_ctrl_c`.
  2. Move the TLS, auth and presets resolution from `main` into `bootstrap::build_config(&args) -> Result<StreamingConfig>`.
  3. Split `handle_csi_report` into `report_dsr`, `report_da`, `report_decrqm`, `report_xtversion` (etc.) keyed on the final byte and private-marker, leaving a thin `match` dispatcher, as QA-114 did for `style.rs`.
- **Method**: This is a behavior-preserving refactor. Report sequences have extensive tests (`grep -rn "DECRQM\|DSR" src/terminal/tests`). Run them.
- **Verify**: `cargo build --no-default-features --features streaming-bin`; `cargo test --lib --no-default-features --features pyo3/auto-initialize csi`; `make checkall`; parsight `calculate_cyclomatic_complexity main` < 20 after a reindex.

### [QA-134] `handle_client` registration helper + `read_control_line` (after SEC-110, ARC-041)
- **Files**: `src/mux/server.rs:430-623`
- **Steps**: Add `fn ensure_registered(registered: &mut bool, clients: &Clients, client_id, tx: &Sender, evicted: &Arc<AtomicBool>, abort: &mut Option<ConnectionAbort>)`, which holds the `expect` in one place. Replace the four copies. Extract SEC-110's byte loop into `fn read_control_line(reader, evicted) -> LineRead { Line(String), Oversize, Undecodable, Closed, Evicted }`.
- **Verify**: the mux lib filter and `mux_daemon`; `make checkall`.

### [QA-135] Blocking writes in tokio tasks (after QA-131)
- **Files**: `src/streaming/mux_factory.rs:326-335`; `src/bin/streaming_server/main.rs:618-636`
- **Steps**: Wrap the resize write (`writer.lock()` + `writeln!` + `flush`) in `tokio::task::spawn_blocking` (clone the `Arc` writer in). Change the initial command to look up the default `StreamSessionState` and call `enqueue_pty_input(format!("{cmd}\n").into_bytes())` (find the registry accessor with `get_symbol_context enqueue_pty_input`), keeping the existing delay if it waits for shell readiness.
- **Verify**: the streaming lib filter; `cargo build --no-default-features --features streaming-bin`; `make checkall`.

### [QA-136] SAFETY comments + lint
- **Files**: `src/mux/foreground.rs:245,285,346,360`; `src/mux/client.rs:404`; `src/mux/pane.rs:666`; `src/mux/tree.rs:1456`; `src/bin/par_mux/main.rs:293,308,349,362`; `src/bin/streaming_server/cli.rs:22`; `src/ffi.rs`
- **Steps**: Before each `unsafe` block, add `// SAFETY: <invariant>` stating what makes it sound (valid fd, the post-fork only-async-signal-safe rule, null checks, and so on). Add `#![warn(clippy::undocumented_unsafe_blocks)]` to `src/lib.rs` and both `main.rs` files, and fix anything the lint flags.
- **Verify**: `make checkall` (clippy `-D warnings`).

### [QA-137] Remove `unsafe impl Send/Sync`
- **Files**: `src/python_bindings/observer.rs:447-448,487-488`
- **Steps**: Delete the four impls. If compilation fails, a field is not `Send`/`Sync`. Report which field instead of re-adding the impl.
- **Verify**: `cargo check --features python`; `make checkall`.

### [QA-138] Stub drift check (after ARC-042)
- **Files**: `Makefile` (`stub-check`), `.github/workflows/ci.yml`
- **Steps**: Add a `stub-drift:` target: `$(MAKE) dev && $(MAKE) stubs && git diff --exit-code python/par_term_emu_core_rust/_native.pyi`. After ARC-042, `dev` is streaming-enabled. Run it in the CI Python job, not in `checkall` (it rebuilds).
- **Verify**: `make stub-drift` exits 0 on a clean tree.

### [QA-139] Split the `pty_session` reader (after QA-130)
- **Files**: `src/pty_session.rs:479-710,763-1006`
- **Steps**: Convert `pty_session.rs` to `pty_session/mod.rs` and move `start_reader_thread` plus its helpers to `pty_session/reader.rs` (`impl PtySession` in a child module keeps private-field access). Behavior must not change. Leave `kitty.rs` for a follow-up. Run parsight `propose_decomposition file_path=src/graphics/kitty.rs` and attach the output to the report.
- **Verify**: `make test-pty`; the Windows VM `cargo check --all-targets`; `make checkall`.

### [QA-140] Replace fixed sleeps in PTY tests
- **Files**: the `src/pty_session.rs` test module (lines 1805, 1845, 2174, 2227, 2274, 2303, 2602, 2610, 2626, 2702)
- **Steps**: Add `fn wait_until(deadline: Duration, mut pred: impl FnMut() -> bool) -> bool` in the test module (copy the `mux_factory.rs` test `wait` helper). Replace each sleep-then-assert with `assert!(wait_until(Duration::from_secs(5), || session.content().contains("…")))`. Keep sleeps that deliberately test timing (read the comment first).
- **Verify**: run `cargo test --lib ... pty_session` three times in a row; `make checkall`.

### [QA-141] parking_lot for session RwLocks — done inside QA-131.

### [QA-142] Delete the dead `TerminalDebug.tsx`
- **Files**: `web-terminal-frontend/components/TerminalDebug.tsx`
- **Steps**: Confirm zero importers (`grep -rn TerminalDebug web-terminal-frontend --include=*.ts --include=*.tsx`), then delete. Run `make web-build-static` only if the web gate requires it.
- **Verify**: `make test-web`; `cd web-terminal-frontend && bun run lint`.

### [QA-144] Mouse event enum (before QA-143)
- **Files**: `src/streaming/protocol.rs:541,812,1393,1609`; `src/streaming/server.rs` `handle_mouse`; `src/streaming/proto.rs` conversions; `src/python_bindings/streaming.rs` dict conversion
- **Steps**: Add `pub enum MouseEventKind { Press, Release, Move, Scroll }` (use the actual vocabulary from the proto and parsing code). Parse at the proto boundary, rejecting unknown values with a protocol error. Keep the wire format unchanged (still a string in the proto). Update the Python dict conversion to emit the same strings.
- **Verify**: the streaming lib filter; `tests/test_streaming.rs`; `make checkall`.

### [QA-143] `ClientCtx` parameter object
- **Files**: `src/streaming/server.rs:1251,1471,1617`
- **Steps**: Add `struct ClientCtx<'a> { session: &'a Arc<StreamSessionState>, transport_label: &'a str, client_id: ClientId, read_only: bool }`. Change the three handlers to take `&ClientCtx` and remove their `#[allow(clippy::too_many_arguments)]`.
- **Verify**: `make checkall`.

### [QA-145] Near-duplicate helpers
- **Files**: `src/graphics/mod.rs:519` / `src/python_bindings/types/graphics.rs:167`; `src/python_bindings/terminal/mod.rs:142` / `src/python_bindings/pty.rs:187`; `src/mux/pane.rs:426,613,634`; `src/screenshot/renderer.rs:831-910`; `src/python_bindings/enums.rs:256/275,492/510`
- **Steps**: For each pair, diff the two bodies first. Merge them only if they are identical or differ in parameters. Keep the Rust-core version and have the binding call it. Skip any pair with a semantic difference and note it. Use parsight `find_duplicate_code min_lines=8` to confirm the current pairs.
- **Verify**: `make checkall`.

### [QA-146] FFI `Box::into_raw`
- **Files**: `src/ffi.rs:142,207-208`
- **Steps**: Replace the `Vec` + `as_mut_ptr` + `mem::forget` sequence with `let boxed = v.into_boxed_slice(); let len = boxed.len(); let ptr = Box::into_raw(boxed) as *mut T;`, with the free function reconstructing via `Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len))`. Take `len` from the slice, not the separate `cell_count`. If ARC-050 lands first, compile with `--features ffi`.
- **Verify**: `cargo test --lib --features ffi ...` (or the default when `ffi` is ungated); `make checkall`.

### [QA-147] Production unwraps
- **Files**: `src/streaming/server.rs:2408`; `src/terminal/file_transfer.rs:165`
- **Steps**: Replace the header value with `HeaderValue::from_static("…")` when constant, or `.expect("static header value is valid")`. Replace `remove(&id).unwrap()` with `if let Some(x) = self.map.remove(&id)` plus an early return, or `.expect("<invariant>")` if the key was just checked.
- **Verify**: `make checkall`.

### [QA-148] Strengthen weak asserts
- **Files**: `tests/test_terminal.py` (19 sites), `tests/test_terminal_bindings.py` (9)
- **Steps**: For each standalone `assert x is not None`, add an assertion on the value (type, a range, or equality with the expected content). Find the sites with `grep -n "assert .* is not None$" tests/test_terminal*.py`.
- **Verify**: `uv run pytest tests/test_terminal.py tests/test_terminal_bindings.py -v`.

### [QA-149] Deprecation parity for `PtySession::screenshot*`
- **Files**: `src/pty_session.rs:1409,1431`
- **Steps**: Add `#[deprecated(since = "0.54.0", note = "use screenshot::render_terminal(&session.terminal().read(), …); removed in 0.56.0")]`, matching the `Terminal` forwarders. Check the Python binding `pty.rs`: if it calls these, switch it to the free function so `-D warnings` stays clean.
- **Verify**: `make checkall`.

---

## Phase 3d — Documentation

### [DOC-039] API_REFERENCE parameter lists + checker (after SEC-112)
- **Files**: `docs/API_REFERENCE.md` (lines in AUDIT.md), new `scripts/check_api_reference.py`, `Makefile` (`stub-check`)
- **Steps**:
  1. Write `scripts/check_api_reference.py`:
     - Parse `_native.pyi` with `ast` into `{Class: {method: [param names excluding self]}}` (include `@staticmethod` and `@classmethod`).
     - Parse API_REFERENCE by tracking the current class from `## Class`/`### Class` headings, and match lines `- \`(\w+)\(([^)]*)\)\``.
     - Compare parameter names in order (strip defaults `=…` and annotations). Print mismatches and exit 1.
  2. Run it, then fix every reported line using the `.pyi` and the binding's `#[pyo3(signature = …)]` as ground truth. Also fix each described return shape (read the binding's docstring).
  3. Add `uv run python scripts/check_api_reference.py` to `stub-check`.
- **Method**: The stub is generated from the compiled module, so it is ground truth for names and arity. Pitfall: build with `make dev-streaming` (or `make dev` after ARC-042) so the streaming classes exist in the stub.
- **Verify**: the script exits 0; `make stub-check`.

### [DOC-040] PtyTerminal method availability (after SEC-112)
- **Files**: `docs/API_REFERENCE.md:1152`; `docs/SECURITY.md:572`; `README.md:22,120`
- **Steps**: Generate the availability list with `uv run python -c "import par_term_emu_core_rust as p; t=set(dir(p.Terminal)); q=set(dir(p.PtyTerminal)); print(sorted(m for m in t-q if not m.startswith('_')))"`. Replace "inherits all Terminal methods" with the accurate statement, and add a collapsed table of the Terminal-only methods (or a link to a generated section). In SECURITY.md:572, document both `Terminal.set_allow_file_media` and the new `PtyTerminal.set_allow_file_media`.
- **Verify**: `grep -n "inherits all" docs README.md` returns nothing.

### [DOC-041] `kitty_file_media` config docs (after SEC-112)
- **Files**: `docs/API_REFERENCE.md:2117-2153`; `docs/STREAMING.md:480-500`
- **Steps**: Add the constructor parameter and the property (`off`/`temp_only`/`all`, default `temp_only`, applied to every session terminal including the single-session default). In STREAMING.md, state the binary runs `temp_only`, or document `--kitty-file-media` if ENH-021 has landed (check `grep -n kitty_file_media src/bin/streaming_server/cli.rs`).
- **Verify**: `grep -n kitty_file_media docs/API_REFERENCE.md docs/STREAMING.md`.

### [DOC-042] Kitty file-media examples and tables
- **Files**: `docs/ADVANCED_FEATURES.md:1241-1255,1278`; `docs/VT_SEQUENCES.md:511-512`; `docs/VT_TECHNICAL_REFERENCE.md:1137-1138`
- **Steps**: Put `term.set_allow_file_media("all")` before the `t=f` example, with a `> **Security**:` callout linking `SECURITY.md#kitty-graphics-protocol-file-transmission` (verify the anchor exists). Annotate both tables: "gated by `allow_file_media` (default `temp_only`); `t=f` needs `all`; `t=t` needs a temp-root path whose name contains `tty-graphics-protocol`".
- **Verify**: `make`-independent. Check the rendered anchor with parsight `find_broken_doc_links`.

### [DOC-043] 0.53.0 CHANGELOG breaking and deprecated notes
- **Files**: `CHANGELOG.md:8-48`; `README.md` What's New
- **Steps**: In the 0.53.0 section add:
  - `### Breaking (Rust embedders)`: "`default-features = false` builds must enable `screenshot` (or `sim`) to keep `crate::screenshot` / `Terminal::screenshot*`. Add `features = [..., "screenshot"]`."
  - `### Deprecated`: `Terminal::screenshot` → `screenshot::render_terminal`, `Terminal::screenshot_to_file` → `screenshot::save_terminal`, and `Terminal::benchmark_*` / `run_benchmark_suite` → `terminal::TerminalBenchmarks`, each "removal planned 0.55.0" (matching ARC-039 step 1).
  - Added: `TerminalBenchmarks`, `make test-pty`, and the weekly bench regression gate.

  Mirror a one-line Breaking note in the README What's New.
- **Verify**: `grep -n "Breaking\|Deprecated" CHANGELOG.md | head`.

### [DOC-044] Feature tables (after ARC-042/043/044)
- **Files**: `docs/RUST_USAGE.md:462-476`; `docs/ARCHITECTURE.md:964-1020`; `README.md:246-251`
- **Steps**: Rebuild the RUST_USAGE table from `Cargo.toml [features]` (`sed -n '/^\[features\]/,/^\[/p' Cargo.toml`). Add the `screenshot` (with swash) and `mux-bin` rows, fix `python`/`python-test`/`sim`/`full`, and note the pyproject feature source from ARC-042. Replace ARCHITECTURE's pasted block with a link to RUST_USAGE. Add a README row for screenshot-capable Rust builds.
- **Verify**: every feature name in `Cargo.toml [features]` appears in the RUST_USAGE table (a quick script or manual diff).

### [DOC-045] MATURIN_BEST_PRACTICES refresh (after ARC-042)
- **Files**: `docs/MATURIN_BEST_PRACTICES.md:110-125,128,136-140,238-243`
- **Steps**: Remove literal versions and quote the real files by reference ("see `pyproject.toml` `[tool.maturin]`"). Update the `maturin>=` floor from `pyproject.toml`. Describe single-source features (streaming included), and remove the per-workflow `--features` advice.
- **Verify**: `grep -n "0.45.0\|1.13.3" docs/MATURIN_BEST_PRACTICES.md` returns nothing.

### [DOC-046] `cargo install` command (verify first; after ARC-044)
- **Files**: `README.md:277`; `QUICKSTART.md:160`
- **Steps**: In a scratch dir, run `cargo install --path /path/to/repo --features streaming-bin --root /tmp/ci-test` (a local path install reproduces the default-features behavior without downloading) and record the result. Regardless of outcome, change the docs to `cargo install par-term-emu-core-rust --no-default-features --features streaming-bin --bin par-term-streamer`, and add `cargo install par-term-emu-core-rust --no-default-features --features mux-bin --bin par-mux`. State the verified outcome in the report.
- **Verify**: the docs commands match `deployment.yml:181`.

### [DOC-047] README Running Tests
- **Files**: `README.md:641-649`
- **Steps**: Replace with `make test` / `make test-rust` / `make test-python` / `make test-pty` (one line each) and a link to `docs/BUILDING.md` for single-test invocations (copy from CLAUDE.md "Running Tests").
- **Verify**: every `make` target named exists (`make -n <target>`).

### [DOC-048] README web frontend
- **Files**: `README.md:596-607`
- **Steps**: Replace npm and port 8030 with `make web-install`, `make web-dev` (bun, http://localhost:3000) and `make web-build-static`. Verify the target names and port (`grep -n "^web-" Makefile`; `grep -n 3000 web-terminal-frontend/package.json`).
- **Verify**: `make -n web-dev`.

### [DOC-049] Dependency pins
- **Files**: `README.md:248-251`; `docs/RUST_USAGE.md:82,92,100,102,111,114,313`
- **Steps**: Replace `"0.50"` with `"0.53"` and add one line: "check crates.io for the latest; `cargo add par-term-emu-core-rust --no-default-features --features …` picks it automatically."
- **Verify**: `grep -rn '"0.50"' README.md docs/` returns nothing.

### [DOC-050] Env drop list
- **Files**: `docs/SECURITY.md:261-262,320`; `docs/CROSS_PLATFORM.md:82-83`; source `src/pty_session.rs:575-590`
- **Steps**: Copy the 12 `DROP_VARS` entries verbatim and the `PAR_MUX_*` prefix rule. Add "source of truth: `DROP_VARS` in `src/pty_session.rs`".
- **Verify**: the list count equals the count in the source.

### [DOC-051] Env-var reference
- **Files**: `docs/CONFIG_REFERENCE.md:825-833`; `docs/STREAMING.md`
- **Steps**: Enumerate the env vars read: `grep -rn 'env = "\|env::var\|var_os(' src --include=*.rs` (`PAR_TERM_*` clap env, `DEBUG_LEVEL`, `XDG_RUNTIME_DIR`, `PAR_MUX_*`, `SHELL`, …). Replace the "no environment variables" statement with a table (variable, component, effect). Add `--force-web-download` / `PAR_TERM_FORCE_WEB_DOWNLOAD` to STREAMING.md's CLI table.
- **Verify**: a table row exists for each `env = "…"` in `cli.rs` (`grep -c 'env = "' src/bin/streaming_server/cli.rs`).

### [DOC-052] Stub docstrings and types (after DOC-039; after ARC-039's binding moves)
- **Files**: `scripts/generate_stubs.py`; `python/par_term_emu_core_rust/_native.pyi`
- **Steps**: In the generator, emit each method's `__doc__` as a stub docstring. Where `__text_signature__` is present, use its parameter names and defaults. Keep `-> Any` where no annotation is derivable. Regenerate with `make dev-streaming && make stubs`.
- **Verify**: `make stub-check`; pyright passes; `grep -c '"""' _native.pyi` > 1000.

### [DOC-053] Binding Example sections
- **Files**: `src/python_bindings/terminal/{bookmark,metrics,notification,scrollback,search,selection,text}_api.rs`; `pty.rs`; `streaming.rs`
- **Steps**: For each public `#[pymethods]` fn without `Example:`, add a 2-4 line Google-style Example. Prioritize the user-facing ones (search, selection, text, bookmark, and `pty.rs` spawn/read methods) and skip trivial getters. Keep the examples executable.
- **Verify**: `make dev-streaming`; `make checkall`.

### [DOC-054] Vendor the mux decision record
- **Files**: `docs/par-mux.md:11`; new `docs/MUX_DECISIONS.md`
- **Steps**: Collect the D-numbers cited in code (`grep -rhoE "par-mux.md[^)]*D[0-9]+" src Cargo.toml | sort -u`). If `~/Repos/par-agent-os/par-mux.md` is readable, write one line per D-number (the decision, one sentence). Point `docs/par-mux.md` at the new file. If the source is unreadable, list the D-numbers with the code comments' own summaries and flag the gap.
- **Verify**: every cited D-number appears in `docs/MUX_DECISIONS.md`.

### [DOC-055] Legacy-event deprecation
- **Files**: `src/python_bindings/terminal/mod.rs:1057,1224-1225`; `docs/API_REFERENCE.md:922,927`; `python/par_term_emu_core_rust/observers.py:14,39,56`
- **Steps**: In `poll_events_legacy` and `poll_subscribed_events_legacy`, emit `PyErr::warn(py, py.get_type::<PyDeprecationWarning>(), "…removed in 0.55.0; use poll_events()", 1)`. Fix the docs: introduced in 0.50.0, removal 0.55.0. Change observer callback hints to `dict[str, Any]`. Add a CHANGELOG `[Unreleased]` Deprecated entry. Add a Python test that uses `pytest.warns(DeprecationWarning)`.
- **Verify**: `uv run pytest -k legacy -v`; `make checkall`.

### [DOC-056] Fuzz docs
- **Files**: `CONTRIBUTING.md:75`; `docs/MUX.md:379-381`
- **Steps**: "Four targets" → "Six targets" (list them). MUX.md command → `cargo +nightly fuzz run mux_parse_command -- -max_total_time=60` (run from `fuzz/`). After ARC-057, mention `make fuzz-all`.
- **Verify**: `ls fuzz/fuzz_targets | wc -l` matches the doc.

### [DOC-057] ARCHITECTURE Terminal listing
- **Files**: `docs/ARCHITECTURE.md:344-410,813`
- **Steps**: Replace the field-by-field struct copy with a prose summary ("34 feature sub-structs; see `src/terminal/mod.rs`"). Add an "Extracted services" paragraph (`TerminalBenchmarks`, `screenshot::render_terminal`/`save_terminal`, plus MacroEngine/TriggerEngine if ARC-039 has landed). Make the Mermaid rendering diagram start at `screenshot::render_terminal`.
- **Verify**: a Mermaid render check (optional); `grep -n "Terminal.screenshot" docs/ARCHITECTURE.md` returns nothing.

### [DOC-058] Replay pseudo-code
- **Files**: `docs/ADVANCED_FEATURES.md:2436-2440`; reference `src/terminal/replay.rs:94`
- **Steps**: Use `ReplaySession::new(&manager)` and `current_frame()` (read the real signatures), and link `docs/INSTANT_REPLAY.md`.
- **Verify**: every call in the snippet exists (`find_symbol`).

### [DOC-059] Broken intra-doc links
- **Files**: `docs/API_REFERENCE.md:858`; `docs/SECURITY.md:40-41`
- **Steps**: `../CHANGELOG.md#0500---2026-09-23` (check the heading's actual date and slug). Fix the SECURITY.md TOC anchors to match the emoji-prefixed headings at `:140`/`:163` (GitHub slugs drop emoji and leave a leading `-`; compute them with parsight `find_broken_doc_links` output).
- **Verify**: parsight `find_broken_doc_links` no longer lists these.

### [DOC-060] Fence language tags
- **Files**: the eight docs listed in AUDIT.md
- **Steps**: For each bare opening fence, add `text`, `bash`, `python`, `rust` or `mermaid` by content. Find them with an awk script that tracks fence parity (opening fences only).
- **Verify**: the same script reports 0.

### [DOC-061] README What's New trim
- **Files**: `README.md:16-73`
- **Steps**: Keep the latest release's What's New (0.53.0, including the DOC-043 breaking note), replace the older sections with a link to `CHANGELOG.md`, and drop the dead `python_bindings/types.rs` reference.
- **Verify**: README still renders its TOC; `grep -n "types.rs" README.md` returns nothing.

### [DOC-062] CHANGELOG compare links
- **Files**: `CHANGELOG.md:1713+`
- **Steps**: Add `[x.y.z]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v<prev>...v<x.y.z>` for 0.38.0–0.53.0 and `[Unreleased]: …/compare/v0.53.0...HEAD`. First check the tag naming with `git tag | sort -V | tail`.
- **Verify**: the link count equals the number of release headings.

### [DOC-063] Missing rustdoc
- **Files**: `src/screenshot/mod.rs:1`; `src/grid/scroll.rs:259`; `src/mux/ipc.rs:157`; `src/python_bindings/observer.rs:437,477`
- **Steps**: Add a `//!` module doc to `screenshot/mod.rs` naming `render_terminal`/`save_terminal` and the `screenshot` feature, and add `///` to the four items. Do not add `#![warn(missing_docs)]` this cycle (parsight lists 8 undocumented public items today, but it would need a crate-wide sweep).
- **Verify**: parsight `list_symbols documented:false visibility:public` drops to ≤ 3 after a reindex; `make checkall`.
