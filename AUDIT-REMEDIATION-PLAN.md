# Audit Remediation Playbook

> Companion to `AUDIT.md` (2026-09-28, HEAD 6828c63, cycle tag `audit-2026-09-28`).
> Entries follow the `## Remediation Plan` phase order. Each one is written so a `/fix-audit`
> Opus 5 agent can execute it without re-deriving the analysis. Line numbers are from 6828c63.
> Earlier phases move lines, so re-read every file before editing (parsight `get_source_window`).
>
> **Standing gates** (run after each batch; details in CLAUDE.md):
> - Full gate: `make checkall`.
> - Mux: `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`.
> - Streaming: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::` and `make test-rust-streaming`.
> - Terminal core: `cargo test --lib --no-default-features --features pyo3/auto-initialize <filter>`.
> - FFI: `cargo test --lib --no-default-features --features pyo3/auto-initialize ffi` plus `make xcframework` on macOS. The xcframework build runs the header smoke-compile, link and Swift-import probes.
> - Windows-sensitive changes (mux, PTY, `cfg(windows)`): the Windows VM playbook in CLAUDE.md, run from the **main checkout**. Run both `cargo check --all-targets` and the explicit `--features rust-only,mux,serde` check.
> - Python stubs: build with `make dev-streaming` (never `make dev`) before `make stubs`.
>
> **Board**: the High findings have cards tagged `audit-2026-09-28`. Their ids are in the "Card" line of each High entry.

---

## Phase 1 — Security (sequential)

### [SEC-115] Host probe runs git in an OSC 7-controlled cwd (merges ARC-065, QA-155 git-flags half)
- **Card**: `01a0ea10df947292a78ceec11c67b970`
- **Files**:
  - `src/mux/host_probe.rs:133-172` (`run_git`), `:96-126` (`git_branch`, `git_dirty`), `:229-249` (`host_probe_sweep`)
  - `src/mux/pane.rs:159-168` (`PaneSnapshotParts::cwd`), `:658-690` (`process_cwd`, three cfg variants)
  - The call site where the sweep collects targets: find it with `get_symbol_context host_probe_sweep`
  - Tests: the `host_probe.rs` test module
- **Steps**:
  1. In `host_probe.rs`, add a `fn git_command(cwd: &Path) -> Command` builder, and make `run_git` use it. The builder applies:
     - `Command::new("git")`
     - `.args(["-c","core.fsmonitor=false","-c","core.hooksPath=/dev/null","-c","safe.bareRepository=explicit","-c","protocol.ext.allow=never","--no-optional-locks","-C"]).arg(cwd)`
     - `.env("GIT_OPTIONAL_LOCKS","0").env("GIT_TERMINAL_PROMPT","0").env_remove("GIT_DIR").env_remove("GIT_WORK_TREE")`
     - stdin null, stdout piped, stderr null (as today).
  2. Change the `git_dirty` tracked-changes call from `diff-index --quiet HEAD --` to `diff-index --cached --quiet HEAD --`. A worktree `diff-index` runs clean/smudge filters; `--cached` does not.
     - This narrows "dirty" to staged changes. To keep the unstaged signal without filters, compare stat info with `git status --porcelain=v1 -uno --no-renames` under the same hardened config. The fsmonitor hook is disabled by config, and `status` without `--ignore-submodules=none` does not run filters on unmodified files.
     - Pick one approach, document the choice in the function doc, and test it on a repo with an unstaged edit.
  3. Stop using the OSC 7 value as the probe target. The sweep target must come from the child's kernel-reported cwd.
     - Add a method alongside `PaneSnapshotParts::cwd` (keep `cwd()` for persistence, which legitimately prefers OSC 7): `pub(crate) fn probe_cwd(&self) -> Option<PathBuf> { self.child_pid.and_then(process_cwd) }`.
     - Use it in the sweep. On platforms where `process_cwd` returns `None` (the `_pid` stub at `pane.rs:688`), the probe serves absent. Do not fall back to OSC 7.
  4. Update the `host_probe.rs` module doc and the `run_git` doc to state the trust rule: the probe runs only in the child's own cwd, and git runs with hooks and fsmonitor disabled.
  5. Tests (Unix, `#[cfg(unix)]`, tempdir based):
     - `fsmonitor_hook_does_not_run`: `git init` a tempdir. Write `.git/config` with `[core] fsmonitor = "sh -c 'touch <tmp>/PWNED; exit 1'"`. Run `git_branch` and `git_dirty` on it. Assert `<tmp>/PWNED` does not exist. Skip if `git` is not on PATH (`which git`).
     - `probe_cwd_ignores_osc7`: build `PaneSnapshotParts` (or the smallest seam that exercises it) with a terminal whose `current_directory()` is `/nonexistent-osc7` and `child_pid: None`. Assert `probe_cwd()` is `None`, while `cwd()` still returns the OSC 7 value.
- **Method**:
  - Three layers, because each one alone is bypassable. Config flags stop the fsmonitor, hooks and ext transports. `--cached` avoids filters. The cwd-source change stops program output from choosing the directory at all.
  - The security agent verified `-c core.fsmonitor=false` plus `safe.bareRepository=explicit` against its reproduction, so keep both.
  - Pitfall: `process_cwd` on macOS uses `proc_pidinfo`, and on Linux `/proc/<pid>/cwd`. Both return the shell's cwd, which tracks `cd` just like OSC 7. So user-visible telemetry is unchanged for local panes, and SSH panes stop probing a remote path locally.
  - Pitfall: do not touch `MuxPane::persistence_cwd`/`cwd()` semantics (session restore depends on OSC 7 first).
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::host_probe`
  - The mux lib filter and `mux_daemon`.
  - Windows VM: the explicit mux feature check (`process_cwd` has a Windows variant).
  - `make checkall`

### [SEC-117] FFI title/cwd length vs NUL-truncated C string (merges QA-152)
- **Card**: `01a0ea10e1a87763875c57b738a2e6bc`
- **Files**:
  - `src/ffi.rs:124-139` (`SharedState::from_terminal` title/cwd block)
  - `src/terminal/sequences/osc/shell.rs:11-32` (OSC 7 handler), `:288-310` (`parse_osc7_url`)
  - `src/terminal/sequences/osc/iterm.rs:124-141` (iTerm2 `CurrentDir`)
  - `src/terminal/shell_integration.rs:160-185` (`record_cwd_change`)
  - `include/terminal_core.h:78-82` (comment only)
  - Tests: the `ffi.rs` test module and `src/terminal/sequences/osc/tests.rs`
- **Steps**:
  1. In `ffi.rs`, add `fn to_c_string(s: &str) -> (*mut c_char, u32)`. It replaces `'\0'` with `'\u{FFFD}'` when present, then builds the `CString` (infallible now; use `expect("interior NUL replaced")`). It returns `(cs.as_bytes().len() as u32, cs.into_raw())` in that order, adjusted to match the tuple you declare.
     - Use it for both title and cwd.
     - The length must always come from the `CString` bytes, never from the source string.
  2. In `parse_osc7_url`, after percent-decoding, return `None` when the decoded path contains any char with `c == '\0' || c.is_control()`. Do the same in the iTerm2 `CurrentDir` path.
     - Find the decoder with `find_symbol parse_osc7_url`, and the iterm handler by reading `iterm.rs:120-145`.
  3. Header comment at `:78-82`: "`title_len`/`cwd_len` equal `strlen(title)`/`strlen(cwd)`; interior NULs are replaced with U+FFFD".
  4. Tests:
     - FFI: create a terminal, feed `b"\x1b]7;file:///tmp/a%00b\x07"` plus an OSC 2 title containing `\u{0}`. Build the `SharedState` via the extern fn. Assert `CStr::from_ptr(cwd).to_bytes().len() == cwd_len` (or that cwd is null, because the OSC 7 was now rejected, which is the expected outcome). Assert the same for title.
     - OSC: `parse_osc7_url("file:///tmp/a%00b")` is `None`, and `file:///tmp/a%0Ab` (LF) is `None`.
- **Method**:
  - Fix it at both ends. The FFI invariant (`len == strlen`) must hold for any string, not just today's sources. The source-side reject removes control bytes from every downstream consumer: FFI, persistence, the host probe and future line-oriented sinks.
  - Enumerate `SharedState` readers with `get_symbol_context SharedState` (Python does not use it; only C and the bench).
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize ffi`; the same with the `osc` filter; `make xcframework` (macOS); `make checkall`.

### [SEC-116] Kitty zlib bomb; unbounded APC and chunk buffers (recurring: prior SEC-109)
- **Card**: `01a0ea10e3567a029905de379906f6da`
- **Files**:
  - `src/graphics/kitty.rs:505` (`data_chunks.push`), `:520-561` (`get_data`, `decompress_zlib`), the existing `MAX_IMAGE_PIXELS` use (grep it)
  - `src/terminal/apc_filter.rs:100-150` (`apc_buffer.push` sites)
  - `src/terminal/mod.rs:2799` (the caller)
  - `docs/SECURITY.md:897`
  - `fuzz/fuzz_targets/kitty.rs` and `apc_filter.rs`: unchanged, but re-run them
- **Steps** (the prior playbook's SEC-109 steps, refreshed):
  1. Add constants in `kitty.rs`, each with a `/// cap:` doc comment in the existing format (see any other `/// cap:` item, e.g. `grep -rn '/// cap:' src | head`):
     - `pub const MAX_KITTY_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;`
     - `pub const MAX_KITTY_DECOMPRESSED_BYTES: usize = MAX_IMAGE_PIXELS * 4;` (cast as needed).
  2. Keep a running `data_bytes: usize` on the parser, reset in every `reset()` path. In `parse_chunk`, before the push, if `data_bytes + decoded.len() > MAX_KITTY_PAYLOAD_BYTES`, call `self.reset()` and return `Err(GraphicsError::KittyError("payload exceeds limit".into()))`.
  3. Change `decompress_zlib(data, limit)`:
     - Keep the empty-input early return.
     - Read through `ZlibDecoder::new(data).take(limit as u64 + 1).read_to_end(&mut out)`, and return `Err` if `out.len() > limit`.
     - Choosing the limit: for `f=24`/`f=32` with `s` and `v` known, use `s.checked_mul(v)?.checked_mul(3|4)`, capped at `MAX_KITTY_DECOMPRESSED_BYTES`. PNG uses `MAX_KITTY_DECOMPRESSED_BYTES`.
  4. If `get_data` swallows decompression errors and falls back to raw bytes, change it to return `Result<Vec<u8>, GraphicsError>` and propagate the error to every caller (`get_symbol_context KittyParser::get_data`). A raw fallback on a limit error feeds compressed bytes to the decoder.
  5. In `apc_filter.rs`, add `/// cap:` `MAX_KITTY_APC_BYTES: usize = 96 * 1024 * 1024` (base64 of 64 MiB plus params).
     - Once `apc_buffer.len() >= MAX`, stop appending and set an overflow flag (an `ApcFilterState` variant, or a bool beside the buffer).
     - On termination, skip `on_kitty`, emit one `debug_error!`, and reset.
  6. `docs/SECURITY.md:897`: state that `MAX_DECOMPRESSED_SIZE` caps the streaming wire only. Add the kitty caps line, then run `make caps-table`.
  7. Tests:
     - A 512 MiB zero zlib stream (`ZlibEncoder` over `std::io::repeat(0).take(..)`) sent with `a=T,f=32,s=1,v=1,o=z` must return `Err` quickly.
     - A unit test: `decompress_zlib` with limit 16 on 1 KiB returns `Err`.
     - APC: `ESC _ G` plus `MAX+10` bytes plus `ESC \` must not call `on_kitty`, and buffer capacity must stay near MAX.
     - Chunk accumulation over 64 MiB must return `Err` and reset.
- **Method**: The bomb works because decompression completes before any size check. `take(limit+1)` makes the check streaming. The chunk and APC caps close the uncompressed paths. Pitfall: `data_bytes` must reset on every `reset()` path, including errors, or a later APC inherits the count.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize kitty`, and the same with the `apc` filter.
  - `make caps-table-check`
  - From `fuzz/`: `cargo +nightly fuzz run kitty -- -max_total_time=60 -rss_limit_mb=512` and `… apc_filter …`
  - `make checkall`

### [SEC-119] Kitty `t=t` parent-dir swap TOCTOU (recurring: prior SEC-111; after SEC-116)
- **Files**: `src/graphics/kitty.rs:1068-1088` (canonicalize, then `open_no_follow(path)`), `:1003-1010` (delete by `path`), and any other `remove_file` in the file-media path.
- **Steps**:
  1. Compute `canonical` before the open. Pass `&canonical` to `open_no_follow` and use it in error messages.
  2. After opening, capture `(dev, ino)` from `file.metadata()` via `std::os::unix::fs::MetadataExt`.
  3. At the delete site, `symlink_metadata(&canonical)`, compare `(dev, ino)`, and `remove_file(&canonical)` only on a match. On a mismatch, skip the delete and `debug_error!`.
     - Windows: delete `canonical` without the inode check, with a comment explaining why.
  4. Test (Unix): a `tty-graphics-protocol` PNG in a tempdir under an allowed temp root, loaded with `t=t`, is deleted. Also unit-test the `(dev, ino)` comparison helper.
- **Method**: Opening the canonical path means a later parent-dir swap cannot redirect the open. The dev/inode check keeps the delete on the same file that was read. Keep the rule "delete only after a successful decode".
- **Verify**: the `kitty` filter; Windows VM `cargo check --all-targets`; `make checkall`.

### [SEC-118] mux control socket unterminated-line growth (recurring: prior SEC-110; before QA-162)
- **Files**: `src/mux/server.rs:505-600` (the `read_line` loop in `handle_client`), `MAX_CONTROL_LINE_BYTES` (grep it), `tests/mux_daemon.rs` (the existing oversize test; grep `exceeds 1 MiB`).
- **Steps**:
  1. Replace `reader.read_line(&mut line)` with a byte loop over `let mut buf: Vec<u8>`:
     ```rust
     match reader.fill_buf() {
         Ok([]) => { /* EOF: same handling as Ok(0) today, via buf.is_empty() */ }
         Ok(chunk) => {
             let nl = chunk.iter().position(|&b| b == b'\n');
             let take = nl.map_or(chunk.len(), |i| i + 1);
             buf.extend_from_slice(&chunk[..take]);
             reader.consume(take);
             if buf.len() > MAX_CONTROL_LINE_BYTES { /* existing oversize arm */ }
             if nl.is_some() { /* line complete */ }
         }
         Err(e) if /* existing poll-wake kinds */ => { /* eviction check; keep buf */ }
         Err(e) => { /* existing error arm */ }
     }
     ```
  2. Convert with `String::from_utf8(buf)` after a line completes. On `Err`, set the existing `undecodable` flag. Timeout wakes no longer drop a split multibyte character, because the bytes stay in `buf`.
  3. Keep the oversize arm's reply and `break 'connection` exactly as they are. The check must run per chunk.
  4. Keep the wake-cadence logging for now; QA-170 trims it later.
  5. Tests in `tests/mux_daemon.rs`:
     - `oversize_unterminated_stream_closes_connection`: write more than 1 MiB with no newline in continuous 64 KiB writes from a thread. Assert the error block arrives and a second client can still `list-sessions`.
     - `send-keys -l` with a multibyte char split across two writes separated by more than 200 ms (`EVICTION_POLL`) delivers the full char.
- **Method**: `BufRead::read_line` loops internally until newline or EOF, so the length check never runs mid-stream. `fill_buf`/`consume` returns control after every chunk. Write the loop as a self-contained block so QA-162 can lift it into `read_control_line`.
- **Verify**: `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`; the mux lib filter; the Windows VM mux filter; `make checkall`.

### [SEC-124] Host probe / shutdown hang on a wedged filesystem (after SEC-115)
- **Files**: `src/mux/host_probe.rs:69-84` (`statvfs`), `:301-329` (worker), `src/mux/server.rs:360-366` (`probe_worker.join()`).
- **Steps**:
  1. Check the shutdown flag before each git invocation and before `statvfs`, not only between panes. Pass the flag or an `Instant` deadline down into `probe_one` (grep the per-pane function).
  2. Bound the shutdown join: replace the unconditional join with a join that waits up to `SWEEP_DEADLINE + GIT_TIMEOUT`, then detaches with a `debug_warn!`. Implement it with `JoinHandle::is_finished` polling at 50 ms. A thread stuck in uninterruptible sleep is abandoned, and the process exits anyway.
  3. Fix the worker doc so it describes the actual behavior: the sweep runs on the worker thread, and shutdown waits at most the bound.
- **Method**: `statvfs` on a hung NFS/autofs mount cannot be interrupted. The only defense is to not block shutdown on it.
- **Verify**: the mux lib filter; a unit test that the join helper returns within the bound for a thread that sleeps longer; `make checkall`.

### [SEC-123] `summarize_line` logs send-keys payloads
- **Files**: `src/mux/server.rs:636-642,715-727`.
- **Steps**:
  1. In `summarize_line`, when the command starts with `send-keys`, log the verb, the target and `payload_len=<n>`, and drop the payload bytes.
  2. Add a line to `docs/SECURITY.md` (debug logging section): `DEBUG_LEVEL>=3` records control-command summaries, and keystroke payloads are length-only.
- **Verify**: a unit test on `summarize_line("send-keys -t %1 -l secret")` asserts `!contains("secret")`; `make checkall`.

### [SEC-120] Harden the Python debug log (recurring: prior SEC-113; promoted to Phase 1: conflict file with QA-181)
- **Files**: `python/par_term_emu_core_rust/debug.py:25,60`; reference `src/debug.rs:66-80`.
- **Steps**: Replace `open(path, "w")` with `fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, "O_NOFOLLOW", 0), 0o600)`, then `os.fdopen(fd, "w")`. On Windows `O_NOFOLLOW` is absent, which is fine.
- **Verify**: `uv run pytest tests -k debug -v` (add a test that pre-plants a symlink and asserts `OSError` on POSIX); `make lint-python`.


---

## Phase 2 — Architecture (sequential)

### [ARC-058] RIS/DECSTR rebuild the whole Terminal
- **Card**: `01a0ea113d5d7b13b91bd7252d60931b`
- **Files**:
  - `src/terminal/mod.rs:3138-3150` (`reset`), `:1277-1355` (`with_scrollback`, for the field inventory), `:905-940` (`SecurityFlagsState`)
  - `src/terminal/sequences/csi/report.rs:93-95` (DECSTR)
  - `src/terminal/sequences/esc.rs:105-107` (RIS)
  - `src/python_bindings/terminal/mod.rs:182` (`PyTerminal.reset`, same semantics as RIS)
  - Tests: `src/terminal/tests/terminal_tests.rs`, `src/terminal/sequences/osc/tests.rs:1598` (`test_zones_cleared_on_reset` must keep passing)
- **Steps**:
  1. **Classify fields with this rule**: state set through an embedder API or setter survives RIS; state set by escape sequences resets.
     - Build the survive list from the `Terminal` fields in `with_scrollback` and the public `set_*`/`add_*` methods: `grep -n 'pub fn set_\|pub fn add_' src/terminal/*.rs`.
     - Survive (at minimum):
       - `security_state`: `accept_osc7`, `disable_insecure_sequences`, `max_osc_data_length`. Reset the in-flight `osc_*` guard fields.
       - `kitty_parser.allow_file_media`
       - `graphics.sixel_limits`, `graphics.cell_dimensions`, and the graphics store limits (`set_max_sixel_graphics`)
       - `title_state.answerback_string`
       - `clipboard_state.allow_clipboard_read`, `max_clipboard_history`, and clipboard sync limits
       - `theme`: the default fg/bg, cursor/link/bold/selection/badge/match colors, the `use_*` flags, `faint_text_alpha`, and `ansi_palette`, but only when set by `set_ansi_palette_color`. Note: OSC 4 also writes `ansi_palette`, and xterm RIS restores the palette to the configured one. So keep a `configured_palette` copy that `set_ansi_palette_color` writes, and restore from it.
       - `unicode_state` (width config, normalization)
       - `events.observers`, `events.next_observer_id`, `event_subscription`
       - `triggers` (the registry; clear the pending rows)
       - `macros`
       - `notifications_state.notification_config`, `max_notifications`, `custom_triggers`
       - `badge_state.badge_format`
       - `recording_state`: an active recording keeps recording
       - `tmux`: the control-mode flag and auto-detect
       - `pixel_width`/`pixel_height` and `profiling`
     - Reset: `conformance_level` (DECSCL sets it, so it is escape-sequence state), grids, cursors, SGR, modes, margins, charsets, keyboard state, hyperlinks, graphics store contents, dcs/apc/parser state, title, shell/zone state, selection, search, progress, color and palette stacks.
  2. Implement it with a `take`-and-restore: `let keep = self.take_host_config();` (a private struct holding the survive fields, moved out with `std::mem::take`/`replace`), then `*self = Self::with_scrollback(cols, rows, scrollback);`, then `self.restore_host_config(keep);`, then restore `tab_stops` (existing behavior; confirm xterm keeps them).
  3. After the restore, call `self.mark_rows_dirty(0, rows - 1)` (QA-150 relies on this).
  4. Add `fn soft_reset(&mut self)` for DECSTR. Per VT510 DECSTR:
     - Reset: cursor visible (DECTCEM on), IRM off, DECOM off, DECAWM on, DECKPAM → numeric, DECCKM off, margins full, charsets G0-G3 = ASCII with GL = G0, SGR default, saved cursor = home with default attributes.
     - Screen and scrollback are untouched.
     - Route `report.rs:93-95` to `soft_reset()`.
  5. Tests:
     - `ris_preserves_host_config`: call each setter from step 1 with a non-default value, add an observer (Rust `TerminalObserver` test double counting events) and a trigger, then `process(b"\x1bc")`. Assert every getter returns the set value, the observer count is 1, and the observer receives an event emitted after the reset (e.g. BEL). Also assert `get_dirty_rows().len() == rows`.
     - `decstr_keeps_screen_and_scrollback`: fill beyond the screen, then `CSI ! p`. Assert content and scrollback are unchanged, and SGR and modes reset.
     - Keep `test_zones_cleared_on_reset` green.
- **Method**:
  - The xterm/VT510 split is exactly the host/VT line: RIS resets the VT, not the embedder's configuration. This fix is the smallest one that restores the security guarantee. The full Host/Services/VT struct split is ARC-067 and must not be attempted here.
  - Pitfall: `with_scrollback` builds a fresh `vte::Parser`. That is correct for RIS, but the `reset()` call happens inside `esc_dispatch`, while the parser is mid-`advance`. Check with `get_symbol_context esc_dispatch_impl` whether `self.parser` is borrowed during dispatch. If it is (a `mem::take` pattern in `advance_parser`), the take-and-restore must not clobber the parser the caller will put back. Read `advance_parser` first.
  - Pitfall: `PyTerminal.reset()` should keep the same survive semantics. Document it in its docstring.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize reset`, and the same with the `decstr` filter; `make test-python`; `make checkall`.

### [ARC-060] list-agents roster grammar
- **Card**: `01a0ea113ede7673abf54e28ee3814bb`. Upstream par-term card: `01a0ea11f63b77e2859da204789c7aab`.
- **Files**: `src/mux/dispatch.rs:311-370` (`cmd_list_agents`), `src/mux/hooks.rs:225-236` (message normalization), `docs/MUX.md` (the roster grammar section, around :208; grep `list-agents`), `CHANGELOG.md:15`, `tests/mux_daemon.rs` (existing list-agents tests).
- **Steps**:
  1. Pick the reason encoding. Recommended: emit the reason as one `reason=<base64>` token, using the same base64 engine as telemetry (`crate::mux::hooks::fresh_telemetry_b64`'s engine).
     - A row becomes `%N agent state source [reason=<b64>] [telemetry=<b64>] [host_telemetry=<b64>]`: every field after `source` is a whitespace-free `key=value` token.
     - This changes the wire shape of rows with a reason, which is a behavior change. Record it in CHANGELOG.
  2. Update the `cmd_list_agents` wire-contract comment to the new grammar.
  3. Update `docs/MUX.md` roster grammar: positions 1-4 fixed, then zero or more `key=value` tokens, with unknown keys ignored.
  4. `CHANGELOG.md:15`: remove "Backward compatible by construction". In the `[Unreleased]` section (DOC-066), add a Changed entry covering the reason encoding and the fact that consumers must parse positionally.
  5. Conformance test in `tests/mux_daemon.rs` or the dispatch test module:
     - Report a state with message `"waiting on telemetry=x hook"`, plus telemetry.
     - Run `list-agents`, then parse the row the way the par-term fix will: tokens 1-4 positional, the rest split on the first `=`.
     - Assert agent, state and source are exact, `reason` decodes to the message, and `telemetry` is present.
  6. Check other in-repo consumers: `grep -rn "list-agents\|list_agents" src python web-terminal-frontend docs`. The Python `TmuxControlParser` does not parse list-agents rows (it is a request/reply body), but verify.
- **Method**:
  - The ambiguity exists only in the free-text reason. Base64 removes it without changing the positional contract.
  - Coordinate with the par-term card: its parser must read positions 1-4 and treat the rest as `key=value`.
  - Pitfall: the `ok` reply body is also consumed by `par-mux` CLI users. Grep `docs/par-mux.md` and any skill docs (`~/.claude/skills/par-mux`) for the roster format, and file a note if a skill documents the old grammar.
- **Verify**: `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde list_agents`; the mux lib filter; `make checkall`.

---

## Phase 3a — Security (remaining)

### [SEC-121] Track `paste` (recurring: prior SEC-114)
- **Files**: none. Tracking only.
- **Steps**: Run `cargo tree -i paste` and record which dependency pulls it. If a newer version of that parent drops `paste`, bump it. Otherwise leave it and note it in `docs/SECURITY.md`'s dependency section.
- **Verify**: `cargo audit` output recorded in the report.

---

## Phase 3b — Architecture (remaining)

> The FFI batch (ARC-062, ARC-063, ARC-077, ARC-087, ARC-085/ARC-088 via ENH-025/026) edits `src/ffi.rs` and `include/terminal_core.h`, the same files as Phase 1's SEC-117 and Phase 3c's QA-151/171/178. Run the FFI items as **one sequential agent**: SEC-117 → QA-151 → QA-178 → QA-171 → ARC-087 → ARC-063 → ARC-062 → ARC-077 → DOC-071. Bump nothing on the Rust version. Do add the ABI version constant in ARC-063.

### [ARC-063] C ABI version, event payload, re-entrancy
- **Files**: `include/terminal_core.h`; `src/ffi.rs:260-300` (observer vtable and dispatch), `:439-447` (`terminal_feed`); `docs/FFI_GUIDE.md` (DOC-065 follows).
- **Steps**:
  1. Add `pub const TERM_CORE_ABI_VERSION: u32 = 1;` and `#[no_mangle] pub extern "C" fn terminal_abi_version() -> u32`. Mirror them in the header (`#define TERM_CORE_ABI_VERSION 1u`). Bump the value whenever a `repr(C)` layout changes (ARC-062 will bump it to 2).
  2. Re-entrancy: document in the header and on `terminal_feed` that observer callbacks must not call any `terminal_*` function on the same handle.
     - Better: defer FFI observer dispatch. Have `terminal_feed` collect events (use the existing `process_deferred`/`ObserverDispatchBatch` path, `src/terminal/mod.rs:~3120-3136`) and dispatch them after `process()` returns and the `&mut` borrow ends.
     - Find the FFI observer registration with `get_symbol_context` on the FFI observer adapter.
  3. Event payload: keep the Debug string for ABI v1 compatibility, and add `kind: u32` plus the string. Alternatively, document the payload as unstable, human-readable text. Pick the smaller change and state it in the header. A full `repr(C)` event struct can wait for ENH-027.
- **Method**: With nothing in the binary to detect the ABI, any ARC-062 layout change silently corrupts ParDeck. Deferring dispatch removes the aliasing without an API change.
- **Verify**: the `ffi` filter; `make xcframework`; `make checkall`.

### [ARC-062] FFI readback: palette, theme, graphemes (after ARC-063)
- **Files**: `src/ffi.rs:20-60` (`SharedCell`), `:368-385` (`from_cell`); `src/color.rs:85-104`; `src/python_bindings/common.rs:2276-2294`; `src/screenshot/renderer.rs:196-230`; `include/terminal_core.h`.
- **Steps**:
  1. Add `impl Terminal { pub fn resolve_cell_colors(&self, cell: &Cell) -> (Rgb, Rgb) }`. Base it on the palette-aware Python resolver (`common.rs:2276`), and include bold-brightening when `bold_brightening` is set (renderer.rs:196). Make all three call sites use it. Diff the three existing implementations first and keep the union of behaviors.
  2. `SharedCell`: add `flags` bits `TERM_CELL_DEFAULT_FG`/`TERM_CELL_DEFAULT_BG`, a `text_len: u8` (the full grapheme byte length), and keep `text: [u8; 4]` holding the prefix. Add `terminal_read_cell_text(term, row, col, out, cap) -> u32`, which returns the full cluster.
  3. Update `offset_of!` tests and header `_Static_assert`s, and bump `TERM_CORE_ABI_VERSION` to 2.
  4. Tests: OSC 4 palette change reflected in readback; `x`+U+0301 returns `text_len` 3 and full text via `terminal_read_cell_text`; the default-fg bit is set for an untouched cell.
- **Verify**: the `ffi` filter; `make xcframework`; `make checkall`; screenshot tests (`make test-python -k screenshot`) are unchanged.

### [ARC-087] FFI panic policy
- **Files**: `src/ffi.rs` (every `#[no_mangle]` fn), `include/terminal_core.h`.
- **Steps**: Document "a Rust panic aborts the process" in the header's ownership section (DOC-071 writes the prose). Optionally wrap `terminal_feed` and `terminal_resize` in `std::panic::catch_unwind(AssertUnwindSafe(..))` and return 0/void on panic. `panic = "abort"` is not set in Cargo profiles (check `[profile.*]`), so `catch_unwind` works.
- **Verify**: the `ffi` filter; `make checkall`.

### [ARC-077] Opt-in `ffi` feature; prefixed symbols; deprecate `Broadcaster` (recurring: prior ARC-050)
- **Files**: `src/lib.rs:59`; `Cargo.toml [features]`; `scripts/build-xcframework.sh:31-34` (its cargo invocation); `include/terminal_core.h`; `benches/ffi_readback.rs` (`required-features`); `src/streaming/mod.rs:81-82`.
- **Steps**:
  1. Add `ffi = []` to `[features]`, gate with `#[cfg(feature = "ffi")] pub mod ffi;`, and do the same for `pub mod keyboard;` only if nothing else uses it (check with `get_symbol_context encode_key`; the Python bindings may). Add `--features ffi` to the xcframework script's cargo invocation and to the `ffi_readback` bench's `required-features`.
  2. Rename the exports from `terminal_*` to `ptec_*`. This is a breaking C ABI change, so fold it into the same ABI version bump as ARC-062 (v2). Keep `#define terminal_create ptec_create`-style compatibility macros in the header for one release.
  3. Add `#[deprecated(since = "0.56.0", note = "unused; removed in 0.57.0")]` to `Broadcaster`.
  4. Add a CHANGELOG Unreleased entry under Changed (breaking for C embedders) and Deprecated.
- **Method**: This removes 16 global symbols from every Python wheel (the `nm` check below confirms it). Pitfall: `make checkall`'s clippy line must include `ffi` or the module goes unlinted. Add it to the Makefile `lint` feature list.
- **Verify**: `nm -gU python/par_term_emu_core_rust/_native*.so | grep -c terminal_` returns 0 after `make dev-streaming`; `make xcframework`; `make checkall`.

### [ARC-064] Decouple triggers from render damage (after QA-150)
- **Files**: `src/terminal/mod.rs:3153-3163` (`mark_row_dirty`), `src/terminal/trigger.rs:176-220` (`process_trigger_scans`), `src/terminal/write.rs` (the character write path; grep `mark_row_dirty` there).
- **Steps**:
  1. Remove the `pending_trigger_rows.insert` from `mark_row_dirty`.
  2. Add `fn mark_row_written(&mut self, row: usize)`, which inserts into `pending_trigger_rows` when triggers are active. Call it only from the text-write path (`write.rs`, where printable characters land) and not from scroll, erase or alt-screen marking.
  3. Scroll-induced row shifts: a written row that scrolls before the scan should still be scanned once. Scanning happens at the end of `process()` (confirm in `trigger.rs:176`), and rows are indices, so on a scroll between write and scan, shift or clear pending indices. Simplest: scan pending rows before each scroll operation. Otherwise dedup on `(row content hash, trigger id)` with a small LRU. Pick one and document it.
  4. Tests: two `ERROR` lines followed by entering and exiting the alt screen produce exactly 2 matches. A line scrolled while visible does not re-match.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize trigger`; `make checkall`.

### [ARC-067] Terminal state split (recurring: prior ARC-039; after ARC-058)
- **Files**: `src/terminal/mod.rs`, `src/terminal/trigger.rs`, `src/terminal/macros.rs`.
- **Steps**: One slice only this cycle.
  1. Move the survive set from ARC-058 into an explicit `HostConfig` struct field on `Terminal`, replacing the take-and-restore with a struct that `reset()` simply does not rebuild.
  2. Make `TriggerEngine` own `TriggerState` (a field on `Terminal` typed `TriggerEngine`) instead of being a unit struct over `term.triggers`.
  3. Keep the 0.56.0-pinned forwarders until their scheduled removal.
- **Verify**: `make checkall`; parsight `find_god_objects` shows `terminal/mod.rs` total CC not higher than before.

### [ARC-068] Two-phase spawn loses early output (recurring: prior ARC-041)
- **Files**: `src/mux/pane.rs:335-342` (`on_output`), `PaneFactory::create_pane` and its implementors (`analyze_relationships query_type=overrides target=create_pane`); `src/pty_session.rs` (`set_output_callback`, `spawn_internal`, `start_reader_thread`); `src/mux/dispatch.rs:229-273,520-560,692-740`; `src/mux/server.rs` (`pane_output_sink`, `wire_all_pane_outputs`).
- **Steps** (prior ARC-041 playbook, still valid):
  1. Confirm whether the reader re-reads `output_callback` per chunk under its mutex. If it does, installing the callback before `spawn*` closes the window.
  2. Add `output_sink: Option<OutputCallback>` to the spawn context passed to `create_pane`. Each factory calls `session.set_output_callback(sink)` before `spawn*`.
  3. In dispatch, build the sink from the reserved pane id before dropping the tree lock, and remove the post-`complete_*` `on_output` loops.
  4. Extract `spawn_two_phase(ctx, begin, complete)` and rewrite `cmd_new_session`, `cmd_split_window` and `cmd_new_window` on it. Keep the rollback semantics.
  5. Test: a factory double emits `PROMPT$ ` immediately on spawn. Another thread holds the tree lock for 100 ms. The observing client still receives `%output` containing `PROMPT$`.
- **Verify**: the mux lib filter and `mux_daemon`; the Windows VM mux filter; `make checkall`.

### [ARC-069] Push-triggered cheap CI (recurring: prior ARC-040)
- **Files**: new `.github/workflows/ci-fast.yml`; `.github/workflows/bench.yml:31`.
- **Steps**:
  1. `ci-fast.yml` runs `on: push: branches: [main]` and `pull_request`, with one `ubuntu-latest` job:
     - checkout, `dtolnay/rust-toolchain@stable` with clippy and rustfmt, `Swatinem/rust-cache` (verify each tag resolves with `gh api repos/<o>/<r>/git/ref/tags/<tag>`)
     - `cargo fmt --check`
     - `cargo clippy --all-targets --no-default-features --features rust-only,mux,serde,streaming -- -D warnings`
     - `cargo test --lib --no-default-features --features pyo3/auto-initialize`
     - `make check-features`
  2. `bench.yml`: rotate the baseline tag only if the `ci-fast` check-run for `github.sha` concluded `success`, queried via `gh api .../commits/$SHA/check-runs`. Add `checks: read`.
- **Verify**: `actionlint` if installed. Pushing to test the workflow is outward-facing, so confirm with the user first.

### [ARC-070] Single-source wheel feature set (recurring: prior ARC-042)
- **Files**: `pyproject.toml:74-77`; `.github/workflows/deployment.yml:269,324,330,375`; `.github/workflows/publish-testpypi.yml`; `Makefile:120-127`.
- **Steps**:
  1. `[tool.maturin] features = ["pyo3/extension-module", "streaming"]`.
  2. Remove `--features streaming` from the maturin `args` in the workflows.
  3. Keep `dev-streaming` as an alias of `dev`. Add `dev-fast` (no streaming), and check its flag combination builds.
- **Verify**: `make dev && uv run python -c "import par_term_emu_core_rust as p; print(p.HAS_STREAMING)"` prints True; `make stubs && git diff --stat python/`; `make checkall`.

### [ARC-071] `mux-bin` feature for clap (recurring: prior ARC-044)
- **Files**: `Cargo.toml:60-63,255`; `Makefile` and `.github/workflows/*.yml` (every build of the `par-mux` binary: `grep -rn "par-mux\|--features.*mux" Makefile .github`); `scripts/check_features.sh:73-78`; docs build lines (DOC-067's par-mux install line uses `mux-bin` once this lands).
- **Steps**:
  1. Remove `clap` from `mux`, and add `mux-bin = ["mux", "dep:clap"]`.
  2. Set `required-features = ["mux-bin"]` on `[[bin]] par-mux`.
  3. Update binary build invocations and the clippy line.
  4. Turn the `check_features.sh` skip into an `assert_absent rust-only,mux clap`.
- **Verify**: `cargo tree --no-default-features --features rust-only,mux -i clap` finds nothing; `cargo build --no-default-features --features mux-bin --bin par-mux`; `make check-features`; `make checkall`.

### [ARC-072] Commit Cargo.lock (recurring: prior ARC-045; after ARC-069)
- **Files**: `.gitignore:4`, `Cargo.lock`, CI workflows, `CLAUDE.md` (Windows playbook step 4 says "no --locked", so update it).
- **Steps**:
  1. Remove the `.gitignore` line and `git add Cargo.lock`.
  2. Add `--locked` to CI cargo and maturin invocations.
  3. Update the CLAUDE.md note.
- **Verify**: `cargo build --locked --no-default-features --features rust-only`; `make checkall`.

### [ARC-073] Layering moves (recurring: prior ARC-046)
- **Steps**:
  1. Move `GridSnapshot` into `src/grid/snapshot.rs` and re-export it from `terminal::replay_snapshot`.
  2. Move `unix_millis` into `src/time.rs`, re-exported at its old path.
  3. Move `streaming/py_convert.rs` under `python_bindings/`.
  4. Use `get_symbol_context` for each moved symbol.
- **Verify**: `cargo test --no-default-features --features rust-only,mux,serde`; `cargo check --no-default-features --features sim`; `make checkall`.

### [ARC-074] Push-based events in the streaming binary (recurring: prior ARC-047)
- **Steps**:
  1. Add a `TerminalObserver` that forwards events into a tokio mpsc channel. Register it per session.
  2. Replace both 50 ms interval loops (`bootstrap.rs:218-226`, `:557-565`) with `rx.recv().await`.
  3. Drop the `Mutex` around `PtySession` where it only serialized `&self` calls.
- **Verify**: `cargo build --no-default-features --features streaming-bin`; the streaming filter; `make checkall`.

### [ARC-075] WsTransport unification (recurring: prior ARC-048; after QA-176, QA-175, QA-179)
- **Steps**:
  1. Add a `trait WsTransport` with send/recv, implemented for tungstenite and axum.
  2. Write one `run_session<T: WsTransport>`, and make both entry points thin wrappers.
  3. Then split `server.rs` into `server/{mod,accept,session,handlers,http}.rs` with no logic change.
- **Verify**: the streaming filter; `make test-rust-streaming`; `make checkall`.

### [ARC-076] `debug_*!` on the log facade (recurring: prior ARC-049)
- **Steps**:
  1. Re-implement the macros as `log::…!(target: $cat, …)`.
  2. Install the file logger as an optional `log::Log` from the binaries and the Python module init, so `DEBUG_LEVEL` behavior holds.
  3. Replace `grid/scroll.rs` `eprintln!` with `log::warn!`.
- **Verify**: `DEBUG_LEVEL=3 make test-python` still writes the log; `make checkall`.

### [ARC-086] Typed agent claim on MuxPane (after ARC-060)
- **Files**: `src/mux/pane.rs:124,317`; `src/mux/{hooks,scrape,host_probe,dispatch,persist}.rs`.
- **Steps**:
  1. Introduce `pub(crate) struct AgentClaim { agent, state, source, message, session_id, session_path, telemetry: Option<CanonicalTelemetry>, host: Option<HostProbe>, seq: HashMap<String,u64> }` as `Option<AgentClaim>` on `MuxPane`.
  2. Migrate readers and writers one module at a time, keeping `metadata` for non-agent keys.
  3. Persistence (`persist.rs`) keeps writing the same named identity keys, so the file format version does not change.
- **Verify**: the mux lib filter and `mux_daemon`; persistence round-trip tests; `make checkall`.

### Low architecture items
- **[ARC-078]** `Makefile`: add `lint-check` (clippy `-D warnings`, `cargo fmt --check`, `ruff format --check`, `ruff check`, `pyright`), and make `checkall` depend on it instead of `lint`/`lint-python`. Verify: `make checkall` on a clean tree leaves `git status` clean.
- **[ARC-079]** `Cargo.toml:88`: drop `"test-util"` from `[dependencies] tokio` (the dev-dependency keeps it). Check `serde_yaml_ng` reachability with `cargo tree -e features -i serde_yaml_ng --no-default-features --features sim`, and gate it only if something lets you. Verify: `cargo check --no-default-features --features streaming`.
- **[ARC-080]** `src/python_bindings/common.rs`: replace `#[macro_export]` with `macro_rules!` plus `pub(crate) use name;`. Verify: `cargo doc --no-deps` shows no root macros.
- **[ARC-082]** `src/mux/server.rs:202-204`: `let cap = collect_persist_capture(&self.tree.lock());` (the lock drops at the semicolon), then capture and write off-lock with the per-command helpers. Verify: the `mux_daemon` shutdown save round-trip.
- **[ARC-083]** `build.rs:128-151`: compare `git rev-parse --show-toplevel` (canonicalized) with `CARGO_MANIFEST_DIR`, and use the source-digest path on a mismatch.
- **[ARC-084]** `.github/workflows/fuzz.yml:18` and `Makefile:893`: add `mux_parse_command` and `mux_hook_report` to the matrix and to `fuzz-all`, plus per-target make targets. Verify: `make fuzz-mux_parse_command FUZZ_SECONDS=10`.
- **[ARC-085]**, **[ARC-088]**: implement via ENH-026 and ENH-025 respectively (see the plans in `docs/opus/`). No separate fix.
- **[ARC-081]** (run LAST): move `debug/` to `scripts/debug/` after grepping references. Leave `theme.css` if it is referenced. Do not archive the audit files, because `/fix-audit` handles them at wrap-up. Replace the 16-byte `AGENTS.md` with a pointer to CLAUDE.md only if its current content is not intentional (read it first).

---

## Phase 3c — Code Quality

> **FFI batch** (one agent, sequential after Phase 1's SEC-117): QA-151 → QA-178 → QA-171 → QA-150's FFI test frames. Then hand over to Phase 3b's ARC-087/063/062/077.
> **Streaming input batch**: QA-161 → QA-153 → QA-154 → QA-175 → QA-170 (streaming part).
> **Mux handle_client batch**: QA-162 → QA-170 (mux part). Both follow SEC-118.
> **Probe/telemetry batch**: QA-155 (after SEC-115 and SEC-124) → QA-156.

### [QA-151] TermKeyEvent enum UB (merges ARC-061, SEC-122)
- **Card**: `01a0ea1140d474d1b78000e808939bb5`
- **Files**: `src/keyboard.rs:36-85`; `src/ffi.rs:672-693`; `include/terminal_core.h:185-200` (comment only); `scripts/build-xcframework.sh:135` (the probe stays valid).
- **Steps**:
  1. In `keyboard.rs`, keep `enum TermKey` (`#[repr(u16)]` is fine for Rust use). Add `impl TermKey { pub fn from_raw(v: u16) -> TermKey { match v { x if x == TermKey::Up as u16 => TermKey::Up, … _ => TermKey::Unknown } } }`. Generate the match with a small `macro_rules!` over the variant list so the list cannot drift. If there is no `Unknown` variant, add one with a value unused by the header, and add it to the header too.
  2. Change `TermKeyEvent.key` to `pub key: u16`. Keep `TermKeyEvent::char_` and other constructors working by storing `TermKey::Char as u16`.
  3. `encode_key` (and `encode_legacy`/`encode_kitty`) match on `TermKey::from_raw(ev.key)`.
  4. Fix the `_pad` doc to "reserved; must be zero".
  5. Tests in the `ffi.rs` test module: build `TermKeyEvent { key: 2, .. }`, `{ key: 57388 }` and `{ key: 0xFFFF }`, call `terminal_encode_key`, and assert return 0. Keep the existing layout tests (`offset_of!`) green, since the field type size is unchanged.
- **Method**:
  - A reference to a `repr(C)` struct containing an enum field is only sound if the field holds a valid discriminant. `u16` is valid for every bit pattern.
  - Enumerate `TermKeyEvent` users with `get_symbol_context TermKeyEvent` (Python bindings may construct it).
  - Pitfall: `#[derive(PartialEq, Eq)]` stays valid.
- **Verify**: the `ffi` and `keyboard` filters; `make xcframework`; `make checkall`.

### [QA-150] Damage contract gaps (merges ARC-059; after ARC-058)
- **Card**: `01a0ea11bd0f7ab0848723b46cf88cbc`
- **Files**: `src/terminal/sequences/csi/edit.rs:46-71`; `src/terminal/sequences/csi/window.rs:17-190`; `src/terminal/replay_snapshot.rs:231-298`; `src/terminal/mod.rs:3153-3172` (helpers); `src/ffi.rs` (the `ffi_round_trip_matches_core_state` test, around `:830`).
- **Steps**:
  1. `edit.rs`: after `insert_characters(...)` and after `delete_characters(...)`, call `self.mark_row_dirty(cursor_row)`.
  2. `window.rs`: after each rectangle mutation call `self.mark_rows_dirty(top, bottom)`. The clamped helper handles bottom overflow. This covers DECFRA `$x` (after `fill_rectangle`), DECCRA `$v` (the **destination** rows `dt-1 ..= dt-1+(pb-pt)`, clamped), DECERA `$z`, DECCARA `$r` and DECRARA `$t`.
     - Skip the unreachable `'{'` arm, which QA-181 deletes.
     - Where `top > bottom` (invalid rect), the handler already no-ops, so don't mark.
  3. `replay_snapshot.rs`: at the end of `restore_from_snapshot` and `restore_for_new_process`, mark all rows: `let rows = self.active_grid().rows(); self.mark_rows_dirty(0, rows.saturating_sub(1));`. Also resize `dirty_rows` if the restore changed the row count. Check whether restore calls `resize`, which already resizes the bitset.
  4. RIS is handled by ARC-058's step 3. Verify the test.
  5. Add a unit test per sequence in `src/terminal/tests/terminal_tests.rs`: `mark_clean()`, process the sequence on a prefilled screen, and assert `get_dirty_rows()` contains the expected rows.
  6. Extend `ffi_round_trip_matches_core_state` with frames `\x1b[3@`, `\x1b[2P`, `\x1b[65;1;1;3;5$x`, `\x1b[1;1;3;5;4;1$v`, `\x1b[1;1;3;5$z`, `\x1b[1;1;2;5;7$r` and `\x1bc`.
- **Method**: Pitfall: DECCRA marks the destination, not the source (the source is unchanged). Enumerate other unmarked mutators while you are here with `grep -n "active_grid_mut()" src/terminal/sequences -r`, and confirm each call site has a mark. Report any extra gaps found instead of widening scope silently. ENH-025 makes this structural.
- **Verify**: `cargo test --lib --no-default-features --features pyo3/auto-initialize dirty`; the `ffi` filter; `make checkall`.

### [QA-178] SharedState `Box::into_raw` (recurring: prior QA-146)
- **Files**: `src/ffi.rs:142,162-165` and the matching free function.
- **Steps**: Replace `Vec` + `as_mut_ptr` + `mem::forget` with `let boxed = cells_vec.into_boxed_slice(); let len = boxed.len(); let ptr = Box::into_raw(boxed) as *mut SharedCell;`. Set `cell_count = len as u32`. The free function rebuilds with `Box::from_raw(ptr::slice_from_raw_parts_mut(ptr, len))`.
- **Verify**: the `ffi` filter under Miri if available (`cargo +nightly miri test ffi`, optional); `make checkall`.

### [QA-171] FFI duplicated copy loops
- **Files**: `src/ffi.rs:519-587`, `:115-121`, `:638-644`.
- **Steps**: Extract `fn copy_cells(row: &[Cell], out: *mut SharedCell, cap: u32, term: &Terminal) -> u32` and `fn mouse_mode_code(m: MouseMode) -> u8`, and use them at all four sites.
- **Verify**: the `ffi` filter; `make checkall`.

### [QA-161] Blocking I/O in tokio tasks; `--command` bypasses the queue (recurring: prior QA-135; before QA-153)
- **Files**: `src/streaming/mux_factory.rs:320-345`; `src/bin/streaming_server/main.rs:610-640`.
- **Steps**:
  1. Wrap the resize write (`writer.lock()`, `writeln!`, `flush`) in `tokio::task::spawn_blocking` with a cloned `Arc`.
  2. Initial command: look up the default session (`server.get_session("default")`, confirmed with `find_symbol get_session`) and call `enqueue_pty_input(format!("{cmd}\n").into_bytes())`. Keep any existing readiness delay.
- **Verify**: the streaming filter; `cargo build --no-default-features --features streaming-bin`; `make checkall`.

### [QA-153] PTY input drain (merges ARC-066; after QA-161)
- **Files**: `src/streaming/session.rs:363-497`; `src/pty_session.rs:1067-1087` (`PtySession::write`); `src/python_bindings/streaming.rs:535-555`; the tests `pty_input_queue_is_bounded_against_a_stalled_writer` and the stress repro in `session.rs` (both must stay).
- **Steps**:
  1. Replace the `try_recv` + `sleep(DRAIN_POLL)` loop with `match rx.recv_timeout(Duration::from_millis(250))`. `Ok(chunk)` writes, `Err(Timeout)` checks the shutdown/`Weak` upgrade and continues, and `Err(Disconnected)` exits.
  2. Replace the `try_lock` retry with `w.try_lock_for(Duration::from_secs(5))` (parking_lot). On timeout, `debug_error!` with the session id, count a drop in `dropped_messages`, and continue.
  3. Sole writer: while a `StreamSessionState` has the writer attached, route `PtySession::write` through it.
     - Minimal approach: `PtySession` gains an optional `input_sink: Arc<Mutex<Option<Box<dyn Fn(Vec<u8>) -> bool + Send + Sync>>>>` that the streaming layer installs when attaching the writer and clears on detach. `PtySession::write` calls the sink when set, else writes directly.
     - Check what other callers of `PtySession::write` exist (`get_symbol_context PtySession::write`: mux, Python) and confirm that ordering through the queue is acceptable for each.
  4. Keep the rate-limited drop logs. Trim the investigation narrative in the comments to the constraints, together with QA-170.
- **Method**:
  - 6828c63 proved the "drain deschedule" was the test itself holding the writer. With that gone, blocking receive is correct, and the timeout exists only for shutdown.
  - Pitfall: `recv_timeout` on `std::sync::mpsc::Receiver` is fine. Don't move to tokio channels, because e818a37's reason still holds: the drain is a plain thread.
- **Verify**: the streaming filter (three consecutive runs); `make test-rust-streaming`; `make checkall`.

### [QA-154] Writer-less input closes the WebSocket
- **Files**: `src/streaming/server.rs:1286-1316`, `:1964-1973`, `:2296-2304`; `src/bin/streaming_server/main.rs:265,344-354,446`; `src/python_bindings/streaming.rs:540-552`; `web-terminal-frontend/lib/terminal-connection.ts:157-167`.
- **Steps**:
  1. Change the guard so that Input or Paste on a session with no writer drops and counts (`dropped_messages`) without closing, logged rate-limited. Mouse and FocusChange are dropped silently and counted.
  2. Macro mode (`main.rs:344-354`): set `default_read_only = true` so the client knows up front.
  3. Remove the `break` at `:1964-1973` and `:2296-2304` for the writer-less case. Keep closing on genuine transport errors.
  4. Frontend: no change needed once the server stops closing. If a shutdown reason is still sent in some path, map it to no-retry in `terminal-connection.ts`.
  5. Tests: a session with no writer receives Input, and the connection stays open (the next server message is still received) while `dropped_messages` increments.
- **Method**: beffc93's intent was attributable drops, and counting keeps that. Closing turned a harmless drop into a reconnect storm.
- **Verify**: the streaming filter; `make test-rust-streaming`; `make test-web`; `make checkall`.

### [QA-155] `run_git` pipe deadlock; shutdown joins the sweep (after SEC-115, SEC-124)
- **Files**: `src/mux/host_probe.rs:136-178` (`run_git`), `:118-131` (`git_dirty`).
- **Steps**:
  1. In `run_git`, drain stdout on a helper thread spawned right after `spawn()`: `let reader = std::thread::spawn(move || { let mut out = Vec::new(); let _ = pipe.read_to_end(&mut out); out });`. Join it after `try_wait` returns `Some`, or after kill.
  2. `git_dirty` untracked check: append `-z` and stop reading at the first byte. Simplest is `ls-files --others --exclude-standard --directory --no-empty-directory -z` with stdout piped through `take(1)`, checking non-empty.
  3. Correct the worker doc. SEC-124 did the deadline and join parts.
  4. Test: a repo with 20k untracked files (generate them in a tempdir) yields `git_dirty == Some(true)` within `GIT_TIMEOUT`. Mark it `#[ignore]` if it takes more than 2 s on CI, and say so.
- **Verify**: `cargo test --lib --no-default-features --features rust-only,mux,serde host_probe`; `make checkall`.

### [QA-156] Future-dated telemetry
- **Files**: `src/mux/hooks.rs:520-575` (`parse_telemetry_object`, freshness check).
- **Steps**:
  1. In `parse_telemetry_object`, reject `sampled_at_unix_ms > now_ms + 300_000` with an error reply: "sampled_at_unix_ms is in the future". Validation happens before the lock, like the other checks.
  2. Test: a future timestamp gets an error reply and stores nothing. A later valid report is accepted.
- **Verify**: `cargo test --lib --no-default-features --features rust-only,mux,serde hooks`; `make checkall`.

### [QA-157] Windows `TOKEN_USER` alignment
- **Files**: `src/mux/ipc.rs:188-240`.
- **Steps**: Change `token_user_buffer` to return `Vec<u64>` sized `needed.div_ceil(8)`, passing `buf.as_mut_ptr().cast::<u8>()` and the byte length to `GetTokenInformation`. Cast from the `u64` buffer pointer. Update the SAFETY comment to cover alignment.
- **Verify**: the Windows VM (from the main checkout) runs `cargo check --lib --tests --no-default-features --features rust-only,mux,serde` and the mux lib filter; `make checkall` on macOS.

### [QA-158] Geometry mirror stale after bypass writes
- **Files**: `src/pty_session.rs:230-250` (mirror publish), `:1365-1375` (`terminal()`), `:1410-1420` (`with_terminal_mut`); `src/python_bindings/pty.rs:35-45` (`term_mut`); `src/mux/dispatch.rs:559,737`.
- **Steps**:
  1. Add `pub struct TerminalWriteGuard<'a> { guard: RwLockWriteGuard<'a, Terminal>, mirror: &'a GeometryMirror }` with `Deref`/`DerefMut`, where `Drop` publishes the mirror from `guard`. Add `pub fn terminal_write(&self) -> TerminalWriteGuard<'_>`.
  2. Python `term_mut` uses `terminal_write()`. Mux note writes use `with_terminal_mut`.
  3. `terminal()` still returns the raw `Arc` for readers. Document that writers must use `terminal_write`/`with_terminal_mut`. Consider deprecating direct writes later, but not now.
  4. Test: `flush_synchronized_updates` (or `process` via the Python binding path) moves the cursor, and `cursor_position()` reflects it immediately.
- **Verify**: `make test-pty`; `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session`; `make checkall`.

### [QA-159] Tests mutate process env
- **Files**: `src/mux/client.rs:580-630`; `src/pty_session.rs:3300-3440`; `tests/mux_nested.rs:348-355`; `tests/mux_reattach.rs:306`; `src/debug.rs:401-407`.
- **Steps**:
  1. `client.rs`: add `pub(crate) fn connect_or_spawn_at(socket: &Path, legacy: Option<&Path>, …)`, which the public fn wraps using env-derived paths. Tests call the seam with explicit paths.
  2. `pty_session.rs` tests: pass env through the spawn-with-env API rather than `set_var`.
  3. Move any test that must mutate process env into `tests/env_serial.rs`, a separate integration binary with one `#[test]` that runs its cases sequentially.
- **Verify**: `cargo test --lib ...` with default parallelism, run 3 times; `make checkall`.

### [QA-160] Trigger action caps
- **Files**: `src/terminal/trigger.rs:250-345`; `src/terminal/semantic_snapshot.rs:905-917`; `src/terminal/mod.rs:390-410`.
- **Steps**:
  1. Add `fn push_action_result(&mut self, r: ActionResult)`, which evicts the oldest when `len >= max_action_results`. Route every variant through it.
  2. Cap highlights at `max_action_results` with oldest-first eviction, and call `clear_expired_highlights` at the start of `process_trigger_scans`.
  3. Cap bookmarks at a `/// cap:` constant (e.g. 10_000) with oldest-first eviction, then run `make caps-table`.
- **Verify**: the `trigger` filter; `make caps-table-check`; `make checkall`.

### [QA-162] `handle_client` helpers (recurring: prior QA-134; after SEC-118)
- **Files**: `src/mux/server.rs:444-720`.
- **Steps**:
  1. Add `fn ensure_registered(registered: &mut bool, clients, client_id, tx, evicted, abort: &mut Option<ConnectionAbort>)`, holding the `expect` in one place, and replace the four copies.
  2. Lift SEC-118's byte loop into `fn read_control_line(reader, evicted) -> LineRead { Line(String), Oversize, Undecodable, Closed, Evicted }`.
- **Verify**: the mux filter and `mux_daemon`; parsight `calculate_cyclomatic_complexity handle_client` below 25 after a reindex; `make checkall`.

### [QA-163] Streamer `main` / `handle_csi_report` (recurring: prior QA-133; after ARC-058)
- **Files**: `src/bin/streaming_server/main.rs:96-143,167-470,587`; `src/terminal/sequences/csi/report.rs:7-250`.
- **Steps**:
  1. Route `run_mux_mode` through `serve_until_ctrl_c`.
  2. Extract `bootstrap::build_config(&args)`.
  3. Split `handle_csi_report` by final byte into `report_dsr`, `report_da`, `report_decrqm`, `report_decscl` and so on, behind a thin dispatcher. The DECSTR arm now calls `soft_reset()` from ARC-058.
- **Verify**: `cargo build --no-default-features --features streaming-bin`; the `csi` filter; `make checkall`.

### [QA-164] SAFETY comments + lint (recurring: prior QA-136)
- **Files**: `src/ffi.rs` (27 blocks + `unsafe impl Sync` `:253`); `src/mux/foreground.rs`, `pane.rs`, `host_probe.rs:69`, `client.rs`, `tree.rs`, `ipc.rs` (Windows); `src/bin/par_mux/main.rs`; `src/bin/streaming_server/cli.rs`; `src/lib.rs`.
- **Steps**:
  1. Add a `// SAFETY:` comment stating the invariant before every `unsafe` block and impl.
  2. Add `#![warn(clippy::undocumented_unsafe_blocks)]` to `lib.rs` and both binaries.
  3. Run clippy on macOS and on the Windows VM, which covers the `ipc.rs` Windows blocks.
- **Verify**: `make checkall` (clippy `-D warnings`); Windows VM `cargo clippy --no-default-features --features rust-only,mux,serde`, if clippy is installed there.

### [QA-166] Fixed sleeps in PTY tests (recurring: prior QA-140)
- **Files**: `src/pty_session.rs` tests at `:1888,2257,2357,2386,2685,2693,2709,2785`.
- **Steps**:
  1. Add `fn wait_until(deadline: Duration, pred: impl FnMut() -> bool) -> bool` to the test module.
  2. Replace each sleep-then-assert with a deadline poll, and delete trailing sleeps that assert nothing. Keep any sleep whose comment says it tests timing.
- **Verify**: the `pty_session` filter three times in a row; `make checkall`.

### [QA-167] Stub drift check (recurring: prior QA-138; after ARC-070)
- **Files**: `Makefile` (near `stub-check`).
- **Steps**: Add `stub-drift:` running `$(MAKE) dev-streaming && $(MAKE) stubs && git diff --exit-code python/par_term_emu_core_rust/_native.pyi`. Wire it into the CI Python job, not into `checkall`, because it rebuilds.
- **Verify**: `make stub-drift` exits 0 on a clean tree.

### Low code-quality items
- **[QA-168]** `src/python_bindings/pty.rs:708,840,865,934,944,954,964`: `.write()` becomes `.read()` for read-only methods. Fix the comment at `:299-301` to say size/cursor come from the mirror (`:55-71`). Verify: `make test-pty`.
- **[QA-169]** `src/terminal/metrics.rs:308`: return `self.dirty_rows.iter().map(|w| w.count_ones() as usize).sum()`. Implement the `:301,:303` placeholders from real data, or remove the fields (check the Python binding at `common.rs:2386` and API_REFERENCE). Assert the exact count in `terminal_tests.rs:2767`.
- **[QA-170]** `src/mux/server.rs:540-567,610-622`: drop the per-wake `debug_log!` and keep the line-completion log. In `src/streaming/session.rs:374-410,437-438` and `src/streaming/mux_factory.rs:760-775`, cut comments to the constraints they encode and delete the false "all input flows through this drain" claim unless QA-153 made it true. Run last in both batches.
- **[QA-172]** `src/keyboard.rs:273-277`: make the kitty `codepoint` a `u32` and format it as a decimal `u32`. Test Alt+U+1F600, which should give `CSI 128512;3u`.
- **[QA-173]** `src/python_bindings/observer.rs:447-448,487-488`: delete the impls. If compilation fails, report the offending field.
- **[QA-174]** Delete `web-terminal-frontend/components/TerminalDebug.tsx` after checking it has zero importers. Verify: `make test-web`, then `bun run lint` in the frontend.
- **[QA-175]** (after QA-154) `src/streaming/server.rs`: add a `ClientCtx<'a>` parameter object, and remove the five `too_many_arguments` allows.
- **[QA-176]** Add `enum MouseEventKind` parsed at the proto boundary. The wire format stays a string (`protocol.rs:541,812,1393,1609`, `server.rs:1570`, and the Python dict conversion).
- **[QA-177]** For each pair, diff the bodies and merge only if identical up to parameters. Confirm the current set first with `find_duplicate_code min_lines=8`, and skip pairs with semantic differences.
- **[QA-179]** `src/streaming/server.rs:2526`: use `HeaderValue::from_static` or `.expect("<invariant>")`. At `src/terminal/file_transfer.rs:166`, use `if let Some(..) = remove(..)`.
- **[QA-180]** `tests/test_terminal.py` and `tests/test_terminal_bindings.py`: add a value assertion after each standalone `is not None` (`grep -n "assert .* is not None$"`).
- **[QA-181]** Remove the DECSERA `'{'` arm in `csi/window.rs` after confirming `csi/mod.rs:106-110` routes `$ {` to `handle_decsera`. Remove `Grid::erase_rectangle` if it has no other callers, and `GraphicsStore::with_limits` (0 callers). Do not remove `PtySession::fire_output_callback`: it is public API, so check par-term first (`grep -rn fire_output_callback ~/Repos/par-term`). Remove the `debug.py:149-224` helpers only if they are unexported and unreferenced (check `__init__.py`).
- **[QA-165]** (run LAST) Split `src/mux/hooks.rs` into `hooks/{mod,report,telemetry,release}.rs` and move the PTY reader to `pty_session/reader.rs`, with no behavior change. Verify: `make checkall`; `make test-pty`; the mux filters.

---

## Phase 3d — Documentation

> Shared files take one agent each, running sequentially:
> - README.md: DOC-066/067/069/070/078/081/082/083/098
> - API_REFERENCE.md: DOC-068/070/073/090/093, then `make stub-check`
> - SECURITY.md: DOC-074 then DOC-084
> - CLAUDE.md: DOC-075/076/094
>
> DOC-071 lands inside the FFI batch (it edits `src/ffi.rs` and the header). DOC-065 and DOC-070 follow it.

### [DOC-066] Unreleased CHANGELOG + README xcframework claim (after ARC-060)
- **Card**: `01a0ea11bf337bb3aa5658ed14575d07`
- **Files**: `CHANGELOG.md:8`; `README.md:302`; `docs/MUX.md:92,114,242` (check they describe the `PAR_MUX_SOCKET` fallback; the audit found them already updated).
- **Steps**:
  1. Insert `## [Unreleased]` above `## [0.55.0]`, with these sections:
     - **Added**:
       - C FFI on-device embedding surface: `terminal_create`/`free`/`feed`/`resize`/`dirty_ranges`/`mark_clean`/`read_row`/`read_scrollback_row`/`scrollback_count`/`get_cursor`/`get_modes`/`encode_key` (9c2cf0b)
       - `keyboard::encode_key` shared encoder
       - `make xcframework` / `TerminalCore.xcframework` with a `Modules/module.modulemap` (c638c00, 610a030)
       - `ffi_readback` bench (99ebac8)
     - **Changed**:
       - **Behavior**: the `par-mux` CLI honors `$PAR_MUX_SOCKET` as the fallback socket, so `--stop`/`--restart` inside a pane target that pane's daemon (1367050).
       - **Behavior**: input for a session with no PTY writer is dropped and counted. Describe whatever QA-154 lands; before QA-154 it closes the connection (beffc93).
       - The `list-agents` roster reason encoding (ARC-060).
     - **Fixed**:
       - dirty-row damage for erase/resize/scroll/alt-screen/IL/DL (aa6edf1, 32b922f)
       - PTY input drain park and server-factory `Arc` cycle (932283a, 444be93, e818a37, 6828c63)
       - plus this remediation's fixes as they land.
  2. Change README:302 to: "`TerminalCore.xcframework` is attached to GitHub releases starting with 0.56.0; until then build it with `make xcframework`."
- **Verify**: `grep -n '## \[Unreleased\]' CHANGELOG.md`; every commit short hash listed exists (`git cat-file -t <h>`).

### [DOC-067] cargo install line (recurring: prior DOC-046)
- **Card**: `01a0ea11c12379e29e7814a1c2453386`
- **Files**: `README.md:281`; `QUICKSTART.md:160`.
- **Steps**:
  1. Replace the install line with `cargo install par-term-emu-core-rust --no-default-features --features streaming-bin --bin par-term-streamer`.
  2. Add `cargo install par-term-emu-core-rust --no-default-features --features mux --bin par-mux`. Use `mux-bin` if ARC-071 has landed.
- **Verify**: `cargo install --path . --no-default-features --features streaming-bin --bin par-term-streamer --root "$(mktemp -d)"` exits 0.

### [DOC-068] TmuxNotification type strings
- **Card**: `01a0ea11c36572d3b6389b99375a16b2`
- **Files**: `docs/API_REFERENCE.md:1815-1830`; source `src/tmux_control.rs:193` (`notification_type`).
- **Steps**:
  1. Read the full `notification_type` match and list every returned string verbatim.
  2. Fix :1825 to `agent-state-changed`.
  3. Document `name`/`value`/`source` for `agent-state-changed`, `agent-released` and `agent-telemetry-changed` by reading `src/python_bindings/types/notification.rs:128+`.
- **Verify**: a script that extracts the strings from `tmux_control.rs` and checks that each appears in API_REFERENCE; `make stub-check`.

### [DOC-069] README screenshot HTML example
- **Card**: `01a0ea11c5617e43bf7c6a45e06f6095`
- **Files**: `README.md:161,425`.
- **Steps**:
  1. Change :425 to `open("output.html", "w").write(term.export_html(include_styles=True))  # Styled HTML`. Check that `export_html` accepts `include_styles` (`grep -n "fn export_html" src/python_bindings -r`).
  2. Change :161 to "PNG, JPEG, BMP, SVG (vector); HTML via `export_html()`".
- **Verify**: run the README Quick Start snippet against `make dev`.

### [DOC-071] ffi.rs / header contract comments (inside the FFI batch)
- **Files**: `src/ffi.rs:404-408,461-469,514-520,550-557,587-592,662-672`; `include/terminal_core.h:9-15,203-235`.
- **Steps**:
  1. `terminal_create`: "returns NULL when `cols` or `rows` is 0; allocation failure aborts". State the same in the header. `terminal_resize` with a zero dimension is a no-op, so document it.
  2. `encode_key`, `read_row` and `read_scrollback_row`: "`out` must be non-NULL; there is no NULL sizing call". Alternatively make them accept `out == NULL, cap == 0` and return the total, as `dirty_ranges` does. The code change is preferred for consistency, with a test.
  3. Split the garbled `terminal_dirty_ranges` Safety sentence.
  4. Add to the header: rows and scrollback read the **active** grid, so the alternate screen has no scrollback. Add a threading note: no concurrent calls on one handle. Add a panic note (ARC-087).
- **Verify**: `make xcframework` (header smoke-compile); the `ffi` filter; `make checkall`.

### [DOC-065] FFI_GUIDE rewrite (after DOC-071, ARC-063)
- **Card**: `01a0ea11c6d97e90b0e11495fcd4a4a3`
- **Files**: `docs/FFI_GUIDE.md`.
- **Steps**:
  1. Restructure the guide into these sections:
     - Overview
     - Building: `make xcframework`, and `cargo rustc --crate-type staticlib` for other targets
     - Lifecycle: create/free
     - Render loop: `terminal_feed` → `terminal_dirty_ranges` (NULL sizing) → `terminal_read_row` per range → `terminal_mark_clean`
     - Scrollback
     - Cursor/modes
     - Key encoding: `TermKeyEvent`, the `TERM_KEY_*`/`TERM_MOD_*` constants, and the cap/return-total protocol
     - Snapshot (`SharedState`)
     - Observers: vtable, payload format, no re-entry
     - Swift via `import TerminalCore`
     - ABI version
  2. Every example must `#include "terminal_core.h"` and obtain a terminal with `terminal_create`.
  3. Fix the wrong claims at :78-86, :88, :212-215 and :259.
  4. Mermaid diagrams use `classDef` per `docs/DOCUMENTATION_STYLE_GUIDE.md`.
- **Verify**: extract each C example to a temp file and run `cc -fsyntax-only -Iinclude`; parsight `find_broken_doc_links` shows no new rows.

### [DOC-070] Other FFI references (after DOC-065)
- **Files**: `docs/RUST_USAGE.md:20,577-603`; `docs/API_REFERENCE.md:2462-2535`; `README.md:118`.
- **Steps**:
  1. Replace the RUST_USAGE "C FFI (Future)" section with a pointer to FFI_GUIDE and the header, and retitle the TOC entry.
  2. In API_REFERENCE:
     - "JSON-encoded" becomes "Debug-formatted text (unstable)", or whatever ARC-063 decided.
     - Rename `event_json` to `event_text`.
     - Move title to screen events.
     - Add `scrollback_lines`/`total_lines`.
     - Link FFI_GUIDE.
  3. Update README:118 to name the embedding API.
- **Verify**: parsight `find_broken_doc_links`.

### [DOC-072] MACROS.md
- **Files**: `docs/MACROS.md:294,699-732,813`.
- **Steps**:
  1. At :813, use `par_term_emu_core_rust::screenshot::render_terminal(&terminal, &config, 0)?`. Check the signature at `src/screenshot/mod.rs:93`.
  2. Make the forwarder calls `MacroEngine::load_macro(&mut terminal, …)` and so on. Check the signatures in `src/terminal/macros.rs:17-143`.
- **Verify**: `find_symbol` confirms every called function exists.

### [DOC-073] kitty_file_media StreamingConfig docs (recurring: prior DOC-041)
- **Files**: `docs/API_REFERENCE.md:2143-2180`; `docs/STREAMING.md:482-502`; source `src/python_bindings/streaming.rs:36,405,421`.
- **Steps**:
  1. Add the constructor parameter `kitty_file_media: str = "temp_only"`.
  2. Add the property: "`off` | `temp_only` | `all`; applied to each session's terminal".
  3. Add a STREAMING table row.
  4. Add one sentence: the `par-term-streamer` binary defaults `--input-rate-limit` to 1048576, while the library `StreamingConfig` default is 0. Do not change the library default rows.
- **Verify**: `make stub-check`.

### [DOC-074] SECURITY.md drift + caps (after SEC-115, SEC-116)
- **Files**: `docs/SECURITY.md:890,901,917-922,995,1023-1052,1091`; constants in `src/streaming/server.rs:45-46`, `src/mux/server.rs:77`, `src/streaming/session.rs:26`, `src/mux/host_probe.rs:42`, `src/mux/hooks.rs:609-610`.
- **Steps**:
  1. Fix :890: the default is 1048576 bytes/s per connection since 0.54.0, `0` means unlimited, and the library default is 0.
  2. Add `/// cap:` doc comments to `WS_MAX_MESSAGE_SIZE`, `WS_MAX_FRAME_SIZE`, `CLIENT_QUEUE_DEPTH`, `INPUT_QUEUE_MESSAGES` and `MAX_GIT_BRANCH_LEN`. Hoist the telemetry model/effort literals into named `/// cap:` constants in `hooks.rs`.
  3. Run `make caps-table`, and replace the prose numbers at :901/:995 with references to the table.
  4. Update the mux section header from "as of 0.52.0", and add `hooks.rs` (telemetry) and `host_probe.rs` to the verified-file list.
  5. Hook Reports: add a telemetry bullet list covering version 1, the model ≤128 and effort ≤32 limits, percents 0-100, the 55-min freshness window, the future-timestamp reject (QA-156), and that telemetry is display-only and never persisted.
  6. New "Host telemetry probe" subsection:
     - what it runs (the git subcommands, statvfs) and its cadence
     - that it probes only the child's kernel cwd
     - that git runs with hooks and fsmonitor disabled (SEC-115)
     - its deadlines
- **Verify**: `make caps-table-check`; parsight `find_broken_doc_links`.

### [DOC-075] Feature tables; CLAUDE.md:127/132 (recurring: prior DOC-044; after ARC-071/077)
- **Files**: `docs/RUST_USAGE.md:463-478`; `docs/ARCHITECTURE.md:955-1020`; `docs/BUILDING.md:70-83`; `CLAUDE.md:127,132`.
- **Steps**:
  1. Rebuild the RUST_USAGE table from `sed -n '/^\[features\]/,/^\[/p' Cargo.toml`. Add `screenshot`, `ffi` and `mux-bin` if they have landed.
  2. Replace ARCHITECTURE's pasted `[features]` block with a link to that table.
  3. Complete the BUILDING list.
  4. CLAUDE.md:127: "the former `Terminal::screenshot*` forwarders were removed in 0.55.0".
  5. CLAUDE.md:132: add `clap`/`windows-sys` to `mux`, or remove `clap` once ARC-071 lands.
- **Verify**: every name under `[features]` appears in the RUST_USAGE table (a script diff).

### [DOC-076] CLAUDE.md / CONTRIBUTING FFI sync rule
- **Files**: `CLAUDE.md:114-157` and the "Files that must stay in sync" list; `CONTRIBUTING.md:105-119`.
- **Steps**:
  1. Add artifacts: the `par-mux` binary (`mux` feature) and the iOS staticlib/xcframework (`make xcframework`).
  2. Add to the layout: `src/ffi.rs`, `include/terminal_core.h`, `src/keyboard.rs`, `scripts/build-xcframework.sh`.
  3. Add a sync rule: `src/ffi.rs` ↔ `include/terminal_core.h` (prototypes, `_Static_assert`s) ↔ the `offset_of!` tests ↔ `docs/FFI_GUIDE.md`, with any layout change bumping `TERM_CORE_ABI_VERSION`.
- **Verify**: read-back review only.

### Remaining documentation items (recurring unless noted; steps as in the prior cycle, refreshed)
- **[DOC-077]** In `docs/ARCHITECTURE.md`:
  - Add the missing modules: `keyboard.rs`, `streaming/mux_factory.rs`, `mux/host_probe.rs`, `mux/foreground.rs`, `mux/win_resume.rs`.
  - Add an "Extracted services" paragraph (MacroEngine, TriggerEngine, TerminalBenchmarks, `screenshot::render_terminal`/`save_terminal`).
  - At :813, `Terminal.screenshot` becomes `screenshot::render_terminal`.
  - Add an xcframework note to Build Process.
  - Verify: `grep -n "Terminal.screenshot" docs/ARCHITECTURE.md` returns nothing.
- **[DOC-078]** `README.md:16-76`: keep one paragraph each for 0.55.0, 0.54.0 and 0.53.0, restoring 0.53.0's file-media security change. Merge :24/:26, drop the 0.50.0 duplicate and anything older in favor of a CHANGELOG link, and remove the dead `python_bindings/types.rs` reference at :73.
- **[DOC-079]** Add `term.set_allow_file_media("all")` with a security callout before the `t=f` example (`docs/ADVANCED_FEATURES.md:1244-1248,1278`), and annotate the tables in `docs/VT_SEQUENCES.md:511` and `docs/VT_TECHNICAL_REFERENCE.md:1137` ("gated by `allow_file_media`, default `temp_only`; `t=f` needs `all`").
- **[DOC-080]** (after QA-154) `docs/STREAMING.md`: document `SessionMetrics::dropped_messages` (every drop reason) and the writer-less input rule as QA-154 lands it. Add a Connection Issues troubleshooting entry and a row in the Mux-Backed Sessions table.
- **[DOC-081]** `README.md:252-255`, `docs/RUST_USAGE.md:82-315`: `"0.50"` → `"0.55"`, plus one line recommending `cargo add par-term-emu-core-rust --no-default-features --features …`. Verify: `grep -rn '"0.50"' README.md docs/` returns nothing.
- **[DOC-082]** `README.md:663-672`: use `make test`, `make test-rust`, `make test-python` and `make test-pty`, and link `docs/BUILDING.md`.
- **[DOC-083]** `README.md:618-632`: use `make web-install`, `make web-dev` (bun, http://localhost:3000) and `make web-build-static`. Verify the port with `grep -n 3000 web-terminal-frontend/package.json`.
- **[DOC-084]** Copy the 12 `DROP_VARS` from `src/pty_session.rs:624-640`, plus the `PAR_MUX_*` prefix rule, into one place in `docs/SECURITY.md` and reference it from the other places, including `docs/CROSS_PLATFORM.md:82-83`.
- **[DOC-085]** `docs/CONFIG_REFERENCE.md:825-833`: add an env-var table built from `grep -rn 'env = "\|env::var\|var_os(' src --include=*.rs`. Add `--force-web-download` / `PAR_TERM_FORCE_WEB_DOWNLOAD` to the STREAMING CLI table.
- **[DOC-086]** `docs/MATURIN_BEST_PRACTICES.md`: replace the literal versions and config copies with references to `pyproject.toml` and `Cargo.toml`. Verify: `grep -n "0.45.0\|1.13.3"` returns nothing.
- **[DOC-087]** `scripts/generate_stubs.py`:
  - Emit `__doc__` docstrings and `__text_signature__` parameters, including class constructors, where available.
  - Regenerate with `make dev-streaming && make stubs`, then run `make stub-check`.
  - Extend `scripts/check_api_reference.py` to cover constructors once stubs carry them (ENH-030 covers the checker).
- **[DOC-088]** Add 2–4 line Google-style Examples to the user-facing `#[pymethods]` in `terminal/{bookmark,metrics,notification,scrollback,search,selection,text,image}_api.rs`, `pty.rs` and `streaming.rs`. Skip trivial getters. Verify: `make dev-streaming && make checkall`.
- **[DOC-089]** New `docs/MUX_DECISIONS.md`:
  - One line per D-number cited in code (`grep -rhoE "D[0-9]+" src/mux | sort -u`, cross-checked against the citing comments).
  - Take the text from `~/Repos/par-agent-os/par-mux.md` if it is readable; otherwise use the code comments' own summaries and flag the gap.
  - Point `docs/par-mux.md` and `docs/MUX.md:7` at the new file.
- **[DOC-090]** Name the removal version 0.57.0 for `poll_events_legacy`/`poll_subscribed_events_legacy`, emit a `DeprecationWarning` in `src/python_bindings/terminal/mod.rs`, and fix `docs/API_REFERENCE.md:922,927`. Add a CHANGELOG Deprecated entry and a `pytest.warns` test.
- **[DOC-091]** `docs/MUX.md:54`: "Same as par-mux default, unless `$PAR_MUX_SOCKET` is set (see Socket and State Paths)". At `:413-414`, add `-rss_limit_mb=512` to both fuzz commands.
- **[DOC-092]** `docs/ADVANCED_FEATURES.md:2438-2440`: use `ReplaySession::new(&manager)` and `current_frame()`, checked with `find_symbol`.
- **[DOC-093]** `docs/API_REFERENCE.md:858` becomes `../CHANGELOG.md#0500---2026-09-23` (check the heading slug). `docs/SECURITY.md`: drop the emoji from the headings at :141/:164 so the TOC anchors resolve.
- **[DOC-094]** Tag every bare opening fence (`text`/`bash`/`python`/`rust`/`mermaid`) in the eight docs plus `CLAUDE.md:141`, locating them with an awk fence-parity script.
- **[DOC-095]** `CHANGELOG.md`: add compare links for 0.38.0–0.55.0 and `[Unreleased]` (tags are `vX.Y.Z`).
- **[DOC-096]** Add a `//!` module doc to `src/screenshot/mod.rs`, and `///` docs to `src/grid/scroll.rs:259`, `src/python_bindings/observer.rs:437,477`, `src/mux/ipc.rs:275` and `src/keyboard.rs:25-30`. Do not add `#![warn(missing_docs)]` this cycle.
- **[DOC-097]** `Makefile` `help`: add `xcframework`, `caps-table` and `caps-table-check`.
- **[DOC-098]**
  - Link `docs/research/OSC-9-4-PROGRESS-BAR-IMPLEMENTATION.md` from ADVANCED_FEATURES (or delete it if superseded, after reading it).
  - Complete the README docs list (BENCHMARKING, MATURIN_BEST_PRACTICES, REGIONAL_FLAG_LIMITATION, TESTING_KITTY_ANIMATIONS, par-mux.md, DOCUMENTATION_STYLE_GUIDE) and the examples list (9 files).
  - Remove `docs/fable/ENH-001…015.md`, following the f14c76d precedent. Keep `docs/fable/BENCH-BASELINE-*.md` if they are referenced (grep first).

---

## Phase 4 — Verification

1. `make checkall` (full).
2. `make test-pty` and `make test-rust-streaming`.
3. The mux daemon suite, and the Windows VM (from the main checkout) with both feature sets.
4. `make xcframework` on macOS (header, link and Swift probes).
5. Fuzz smoke: `make fuzz-all FUZZ_SECONDS=30`.
6. Reindex parsight, then re-run `find_broken_doc_links`, `calculate_cyclomatic_complexity handle_client` and `find_god_objects` to confirm the numbers moved the expected way.
7. Check each board card's criteria one at a time before moving it to `done`.
