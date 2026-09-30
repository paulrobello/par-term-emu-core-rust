# Audit Remediation Playbook

> Companion to `AUDIT.md` (2026-09-29, HEAD f6535f2, cycle tag `audit-2026-09-29`).
> Entries follow the `## Remediation Plan` phase order. Each one is written so a `/fix-audit` Opus 5 agent can execute it without re-deriving the analysis.
> Line numbers are from f6535f2. Earlier phases move lines, so re-read every file before editing (parsight `get_source_window`, repository_id `par-term-emu-core-rust`).
> Items marked `verify:` were not confirmed while writing. Run the named query first.
>
> **AUDIT.md wins on conflict.** Some entries carry a "wrong AUDIT.md claim" note found while this playbook was written. Those corrections are already folded into AUDIT.md and appended to each card's notes.
>
> **Standing gates** (run after each batch; details in CLAUDE.md):
> - Full gate: `make checkall`.
> - Mux: `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`. If ARC-106 has landed, use `mux-bin` for the binary-driven tests.
> - Streaming: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::` and `make test-rust-streaming`.
> - Terminal core: `cargo test --lib --no-default-features --features pyo3/auto-initialize <filter>`.
> - FFI: `make ffi-header && make ffi-header-check ffi-surface-check`, `cargo test --lib --no-default-features --features pyo3/auto-initialize ffi`, and `make xcframework` on macOS.
> - Windows-sensitive changes (mux, PTY, `cfg(windows)`): run the Windows VM playbook in CLAUDE.md from the **main checkout**. Run both `cargo check --all-targets` and `cargo check --lib --tests --no-default-features --features rust-only,mux,serde`.
> - Python stubs: build with `make dev-streaming` (never `make dev`) before `make stubs`.
>
> **Decisions (AUDIT.md "Decisions Required"), resolved by the user 2026-09-29. These override any "gated" wording in the entries below:**
> - **D1 declined**: keep CI manual-dispatch. ARC-104 does the FFI-checks and xcframework half only, with no push/PR triggers.
> - **D2 approved**: commit `Cargo.lock`, add `--locked` (ARC-107, SEC-134). Also update CLAUDE.md's Windows VM playbook, which says "no --locked — Cargo.lock is not tracked".
> - **D3 approved**: pin `actions/*` to SHAs (SEC-135).
> - **D4 approved**: ship the breaking ABI revision (ARC-101, ARC-112, ARC-114). It is v4, or v5 if the additive ENH-038 ships first.
> - **D5 approved**: delete tracked `debug/` (ARC-118).
> - **D6 resolved: remove now.** DOC-120 is replaced by the removal entry below. Ignore any "gated" wording.
> - **ENH-039** (not a /fix-audit item): `--pane-endpoints` ships off by default, permanently. Pane-to-pane control through `$PAR_MUX_SOCKET` is a core feature.
> Never let a decision block Phase 1 or Phase 2.
> **CHANGELOG rule**: code fixes add bullets under `## [Unreleased]` only. DOC-099 edits only `## [0.57.0]`.
>
> **Board**: every finding has a card tagged `audit-2026-09-29`. Its id is on the entry's "Card" line.
> **Enhancements**: ENH-033…041 are separate `enhancement` cards with plans in `docs/opus/`. `/fix-audit` does not execute them.

---

## Phase 1 — Security (sequential)

> Line numbers were re-read at HEAD f6535f2. Every entry below edits files that earlier entries also edit, so re-read each file before editing (parsight `get_source_window`, `repository_id: "par-term-emu-core-rust"`). Code fixes add a bullet under `## [Unreleased]` in `CHANGELOG.md` only.

### [SEC-127] Mux control socket: a line with no newline grows without bound
- **Card**: `01a0ef5a8a537ac29331a53195cf8300`
- **Files**:
  - `src/mux/server.rs:85` (`MAX_CONTROL_LINE_BYTES`, with its `/// cap:` line at :84), `:450` (`handle_client`), `:509-585` (the per-line read loop), `:517` (`reader.read_line`), `:528-545` (the oversize arm), `:546-560` (the newline arm with wake-cadence logging), `:563-573` (the poll-wake arm), `:574-582` (the `InvalidData` arm), `:587-611` (the `undecodable` reply). AUDIT.md's numbers match HEAD.
  - `:766-771` (`is_poll_wake`).
  - Tests: `tests/mux_daemon.rs:1109-1175` (`oversized_control_line_gets_error_and_close`, which sends a complete line and so never exercised the unterminated case) and `tests/mux_end_to_end.rs:180-205` (`a_non_utf8_line_gets_an_error_reply_and_the_connection_survives`).
- **Steps**:
  1. Add a pure helper above `handle_client` so the bounded read is unit-testable and QA-199 can lift it into `read_control_line` later:
     ```rust
     /// How one bounded fill attempt ended (SEC-127).
     #[derive(Debug, PartialEq, Eq)]
     enum LineFill {
         /// `buf` ends in `\n`.
         Complete,
         /// `buf` grew past `max`; the caller answers %error and closes.
         Oversize,
         /// EOF; `buf` holds any unterminated final line.
         Eof,
     }

     /// Append bytes from `reader` to `buf` until a newline, EOF, or the
     /// budget trips. Errors (a recv-timeout poll wake above all) propagate
     /// with `buf` intact: the bytes stay raw until the line completes, so a
     /// wake that splits a multi-byte UTF-8 char loses nothing.
     fn fill_line_bounded<R: std::io::BufRead>(
         reader: &mut R,
         buf: &mut Vec<u8>,
         max: usize,
     ) -> std::io::Result<LineFill> {
         loop {
             let chunk = match reader.fill_buf() {
                 Ok(chunk) => chunk,
                 Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                 Err(err) => return Err(err),
             };
             if chunk.is_empty() {
                 return Ok(LineFill::Eof);
             }
             // max + 1 - len >= 1 while len <= max, and the Oversize return
             // below stops the loop the moment len exceeds max.
             let budget = max + 1 - buf.len();
             let newline = chunk.iter().position(|&b| b == b'\n');
             let take = newline.map_or(chunk.len(), |i| i + 1).min(budget);
             buf.extend_from_slice(&chunk[..take]);
             reader.consume(take);
             if buf.len() > max {
                 return Ok(LineFill::Oversize);
             }
             if buf.last() == Some(&b'\n') {
                 return Ok(LineFill::Complete);
             }
         }
     }
     ```
  2. In `handle_client`, replace `let mut line = String::new();` plus the inner `loop { match reader.read_line(&mut line) { … } }` (`:509-585`) with a `let mut buf: Vec<u8> = Vec::new();` loop over `fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES)`:
     - `Ok(LineFill::Eof)`: `if buf.is_empty() { break 'connection; }` else `break` (keeps the "unterminated final line is processed before EOF" behavior).
     - `Ok(LineFill::Oversize)`: the existing oversize body unchanged (register if needed, `command_number += 1`, send the `"line exceeds 1 MiB budget, closing connection"` block, `break 'connection`). The reply text must stay byte-identical: the existing test asserts `1 MiB`.
     - `Ok(LineFill::Complete)`: the existing wake-cadence `debug_log!` (use `buf.len()`), then `break`.
     - `Err(err) if is_poll_wake(&err)`: the existing wake arm unchanged (`wakes += 1`, log, `if evicted … break 'connection`). `buf` is kept, so the next call resumes the partial line.
     - `Err(_)`: `break 'connection` (this covers the Windows `CancelIoEx` abort, as today).
  3. After the loop, convert once: `let mut line = match String::from_utf8(buf) { Ok(s) => s, Err(_) => { undecodable = true; String::new() } };`. The existing `if undecodable { … }` block (`:587-611`) then runs unchanged. Delete the now-dead `Err(err) if err.kind() == InvalidData` arm. `read_line` was its only source.
  4. Rewrite the comment above the loop (`:510-514`). It claims a wake mid-line "retries with the partial preserved". That is true only when the partial is valid UTF-8 (see Method). State the new invariant: raw bytes accumulate, the budget is checked per chunk, and UTF-8 is decoded once per complete line.
  5. Unit tests in the `server.rs` `mod tests` (`:1143`), using a scripted `BufRead` (a `VecDeque<io::Result<Vec<u8>>>` behind a small struct that implements `Read` + `BufRead`):
     - `fill_line_bounded_trips_the_budget_without_a_newline`: feed 4 chunks of 64 KiB of `x`, `max = 100_000`. Assert `Ok(Oversize)` and `buf.len() == 100_001`. The budget clamp means nothing past `max + 1` is ever copied.
     - `fill_line_bounded_keeps_a_split_utf8_char_across_a_wake`: chunks `b"set-buffer caf\xc3"`, then `Err(TimedOut)`, then `b"\xa9\n"`. The first call returns `Err(TimedOut)`. The second returns `Ok(Complete)`, and `String::from_utf8(buf) == "set-buffer café\n"`.
     - `fill_line_bounded_reports_eof_with_a_partial`: `b"version"` then EOF gives `Ok(Eof)` with `buf == b"version"`.
  6. Daemon tests (these are the red-before-fix proofs):
     - `tests/mux_daemon.rs::an_unterminated_stream_over_budget_closes_the_connection`: bind a `MuxServer` as the existing oversize test does. Connect. Spawn a writer thread that writes `b"send-keys -l "` and then up to 8 MiB of `x` in 64 KiB `write_all` calls with **no newline**. It stops at the first write error and returns the total bytes written. On the main thread, read lines with a 10 s deadline until `%error` arrives, then until `Ok(0)` (EOF).
       - Assert that the reply contains `%error` and `1 MiB`, that EOF follows, and that the writer's total is `< 4 MiB` (the daemon stopped consuming near 1 MiB plus socket buffers).
       - Then connect a second client and assert `list-panes` still answers.
       - Red before the fix: `read_line` never returns, no `%error` arrives within the deadline, and the writer pushes the full 8 MiB into daemon memory.
     - `tests/mux_end_to_end.rs::a_multibyte_char_split_across_a_poll_wake_survives`: using the file's `spawn_server`/`connect`/`read_block_verdict` helpers, write `b"set-buffer caf\xc3"`, flush, sleep 450 ms (more than 2 × `EVICTION_POLL`, 200 ms), then write `b"\xa9\n"`. Assert the verdict is ok. Then `show-buffer` must return the body `["café"]`.
       - Red before the fix: std discards the partial (see Method), the tail `\xa9\n` decodes as invalid, and the daemon answers `%error`.
- **Method**:
  - Why `read_line` cannot be bounded in place: `BufRead::read_line` loops inside `read_until` until it sees a newline or EOF. A continuous no-newline stream therefore never returns to the length check. Wrapping it in `take(n)` would bound the length but keep the second defect below.
  - The UTF-8 defect, verified in `~/.rustup/toolchains/stable-*/lib/rustlib/src/rust/library/std/src/io/mod.rs:388-403` (`append_to_string`): when the call errors, including with a recv-timeout `WouldBlock`/`TimedOut`, and the bytes it appended are not valid UTF-8, the guard truncates `buf` back to its old length. Those bytes were already `consume`d from the `BufReader`. So a poll wake that splits a multi-byte char silently drops the whole partial chunk. The raw `Vec<u8>` accumulator removes this failure mode entirely.
  - Pitfall: do not decode per chunk. A chunk boundary can fall inside a code point, so decode only after `Complete`/`Eof`.
  - Pitfall: `BufReader`'s internal buffer is 8 KiB. Keep that; the bound comes from the budget clamp, not from the buffer size.
  - Windows named pipes reject recv timeouts, so `fill_buf` blocks until `ConnectionAbort::cancel_blocked_io` aborts it (memory note `interprocess-stream-eviction-teardown`). That surfaces as a non-wake `Err`, and `break 'connection` handles it exactly as before.
  - Blocks SEC-132 (same function and log sites), QA-199 and QA-207. Keep `fill_line_bounded` self-contained so QA-199 can wrap it.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::server`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `cargo test --test mux_end_to_end --no-default-features --features rust-only,mux,serde`
  - Windows VM (main checkout): `cargo check --all-targets` and `cargo check --lib --tests --no-default-features --features rust-only,mux,serde`, then the `mux::` lib filter run.
  - `make checkall`

### [SEC-132] The mux debug log records control-command payloads, including `send-keys` input
- **Card**: `01a0ef5ab6837711b077ce3ef2d0235c`
- **Files**:
  - `src/mux/server.rs:721-733` (`summarize_line`). There are three call sites, all verified: `:642-648` (`debug_log!`, every control command, `DEBUG_LEVEL>=3`), `:668-674` (`debug_error!`, rejected commands, `>=1`) and `:691-697` (`debug_error!`, unparseable lines, `>=1`). The non-UTF-8 log at `:597-603` records only a length and needs no change.
  - The existing test `server.rs:1169-1185` (`summarize_line_keeps_short_lines_and_cut_points_whole`).
  - `docs/SECURITY.md:918-1092` (the "Multiplexer Daemon Security" section; add a subsection after "Control-Connection Resource Bounds" at `:988-1003`).
- **Steps** (after SEC-127, which rewrote the same function):
  1. In `summarize_line`, classify by the first whitespace token before the length cut:
     ```rust
     /// Commands whose arguments carry user data (typed input, clipboard):
     /// logged as name, target and size only (SEC-132).
     const PAYLOAD_COMMANDS: &[&str] = &["send-keys", "set-buffer"];
     ```
     For a line whose first token is in the list, return `format!("{name}{target} [payload redacted, {} bytes]", line.len())`, where `target` is `" -t <value>"` when a `-t` token and a following token exist (`line.split_whitespace()`), and empty otherwise. Every other line keeps today's head-and-size behavior.
     - Optional, only with the orchestrator's approval: `set-environment` values and `new-session -e` values are the same CWE-532 class (API tokens attached to a session). If approved, add `"set-environment"` to the list, and redact `new-session` only when an `-e` token is present, keeping the name and target. Otherwise report them as a follow-up.
  2. Fix the existing test. `summarize_line_keeps_short_lines_and_cut_points_whole` asserts that `summarize_line("send-keys -t %1 -H 61")` is returned verbatim and that a long `send-keys -H` line keeps its head. Both assertions now fail by design. Move the head-and-cut assertions to a non-payload command, for example `capture-pane -t %1 -S -` padded to more than 120 bytes, and keep the multibyte-cut assertions.
  3. Add `summarize_line_redacts_payload_commands`:
     - `summarize_line("send-keys -t %1 -l hunter2")` does not contain `hunter2`, and does contain `send-keys`, `-t %1` and `bytes`.
     - `summarize_line("send-keys -t %1 -H 68 75 6e")` does not contain `68 75`.
     - `summarize_line("set-buffer topsecret")` does not contain `topsecret`.
     - Only if the optional `set-environment` scope is approved: `summarize_line("set-environment -t $0 TOKEN abc123")` does not contain `abc123`.
     - `summarize_line("list-panes")` equals `"list-panes"` (unchanged).
     - Red before the fix: the payloads appear verbatim.
  4. `docs/SECURITY.md`: add `### Debug Logging` after "Control-Connection Resource Bounds". It says:
     - `DEBUG_LEVEL>=1` logs a summary of rejected and unparseable control lines, and `>=3` logs every command.
     - `send-keys`, `set-buffer`, `set-environment` and `new-session -e` are logged as name, target and byte count, never their payload.
     - The log is `<temp>/par_term_emu_core_rust_debug_rust_<pid>.log`, created `0600` with `O_NOFOLLOW` (`src/debug.rs:66-79`), and it still records command names, targets and pane output metadata, so review it before attaching it to a bug report.
     - Do not edit the generated caps table (`make caps-table` owns it).
- **Method**:
  - Redaction by command name at the single summarizer covers all three log sites and the unparseable path, because every path passes the raw line through `summarize_line`.
  - Pitfall: do not use the parser here. An unparseable line never reaches a `MuxCommand`, and it is exactly the case logged at level 1.
  - Pitfall: QA-207 later removes the wake-cadence logs from the same loop. Do not touch them here.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::server::tests::summarize_line`
  - The mux lib filter and `mux_daemon`.
  - `make checkall`

### [SEC-125] A held (dead) pane keeps its reaped child's PID, and later kill and resize calls signal that PID
- **Card**: `01a0ef5aba4a7af1a375d19c5715d208`
- **Files**:
  - `src/pty_session.rs`:
    - `:107-156` (`PtySession` fields; `child_pid: Option<u32>` at :151), `:158-199` (`send_sigwinch` and its SAFETY comment), `:239` (constructor), `:745-756` (spawn stores `child_pid` and starts the reader).
    - `:816-828` and `:969-975`: the reader thread's alt-screen SIGWINCH pulse uses the **captured** `child_pid`. **AUDIT.md omits this site.**
    - `:1167-1174` (`resize` SIGWINCH), `:1246-1257` (`resize_with_pixels` SIGWINCH), `:1273-1285` (`poll_running`), `:1289-1291` (`child_pid`), `:1296-1308` (`try_wait`), `:1314-1322` (`wait`), `:1325-1346` (`kill`), `:1686-1695` (`Drop` calls `kill` when `is_running`), `:466-500` (`cleanup_previous_session`, which takes and reaps an old child).
  - portable-pty 0.9.0 `src/lib.rs:341-372` (`impl ChildKiller for std::process::Child`): a raw `libc::kill(self.id(), SIGHUP)` with no check of std's cached exit status, followed by 5 × 50 ms `try_wait` and then `std::process::Child::kill`, which *is* guarded by std.
  - `src/mux/pane.rs:186-206` (`PaneSnapshotParts::cwd`/`probe_cwd`), `:292-297` (`persistence_cwd`), `:315-322` (`snapshot_capture_parts`), `:340-342` (`poll_running`), `:356-359` (`mark_dead`), `:368-370` (`child_pid`), `:435-437` (`kill`).
  - `src/mux/server.rs:980-981` (the reaper: `poll_running` then `mark_dead`).
  - `src/mux/dispatch.rs:717,721-724` (`pane-info cmd=`).
  - `src/mux/scrape.rs:567` (agent liveness `table.agent_alive(child_pid, …)`). **AUDIT.md omits this reader.**
  - `src/mux/tree.rs:17-21,1047` (`begin_respawn` calls `poll_running`).
  - `src/python_bindings/pty.rs:241-253` (`child_pid` docstring), `:190-201` (`resize`), `:262-289` (`wait`, `try_wait`, `kill`).
  - `docs/API_REFERENCE.md:1139`.
- **Steps**:
  1. **The exit record.** Add a field to `PtySession`:
     ```rust
     /// Exit code once `try_wait`/`wait`/`kill` has reaped the child
     /// (SEC-125). `Some` means the PID is released to the OS and may be
     /// recycled, so nothing may signal it again. Every reap records under
     /// this lock and every signal is sent while holding it, so an
     /// "unreaped" check cannot go stale before the kill(2). Shared with
     /// the reader thread for its alt-screen pulse.
     reaped: Arc<parking_lot::Mutex<Option<i32>>>,
     ```
     Initialize it to `Arc::new(Mutex::new(None))` in `new`.
  2. **Reset on every spawn.** At `:753` (next to `self.child_pid = child_pid;`), assign a *fresh* `Arc` (`self.reaped = Arc::new(Mutex::new(None));`) before `start_reader_thread`. A previous reader thread that is still draining keeps the old child's record. Clearing the shared value instead would make the old reader think the new child is unreaped.
  3. **Record where the child is reaped.** Hold `self.reaped.lock()` across each `waitpid`-backed call:
     - `try_wait`: `let mut reaped = self.reaped.lock(); if let Some(code) = *reaped { return Ok(Some(code)); }`. On `Ok(Some(status))`, set `*reaped = Some(status.exit_code() as i32)` and store `running = false`.
     - `wait`: the same shape. Holding the lock through the blocking wait is intended. The reader uses `try_lock` (step 5), so it cannot deadlock against a child blocked on PTY output. Only `&mut self` callers contend, and they are serialized.
     - `kill`: take the lock. `if reaped.is_none() { child.kill()?; <existing 500 ms loop> }`. In that loop, on `Ok(Some(status))`, record `*reaped = Some(..)` before `break`. Always store `running = false` and return `Ok(())`. A reaped child is now a silent no-op.
     - portable-pty's own grace loop reaps inside `child.kill()` without telling us. Our loop's first `try_wait` then returns std's cached status (std's unix `Process::try_wait` short-circuits on its stored status), so the record is still written.
  4. **Queries.**
     - `child_pid()` returns `None` when `self.reaped.lock().is_some()`, and `self.child_pid` otherwise.
     - `poll_running()`: add `if self.reaped.lock().is_some() { return false; }` before the flag check.
  5. **Every signal goes through the record.**
     - Add a `#[cfg(unix)] fn signal_winch(&self, tag: &str, context: &str)` that takes `self.reaped.lock()`, returns if `Some`, and otherwise calls `send_sigwinch(pid, …)` with `self.child_pid` while still holding the guard.
     - Replace both `if let Some(ref child) = self.child { if let Some(pid) = child.process_id() { send_sigwinch(..) } }` blocks (`:1170-1174` and `:1249-1257`) with `self.signal_winch(..)`.
     - Pass `Arc::clone(&self.reaped)` into `start_reader_thread`. At `:969-975` use `if let Some(guard) = reaped.try_lock() { if guard.is_none() { send_sigwinch(pid, …) } }`. The pulse is best-effort, so skipping it under contention is correct.
     - Rewrite the SAFETY comment at `:168-172`: the caller holds the reap lock and has seen the child unreaped, so the PID (live or zombie) cannot have been recycled.
  6. **Other `child.kill()` sites.** `cleanup_previous_session` (`:471-491`) calls `child.kill()` only after `child.try_wait()` returned `Ok(None)` on the same thread, and std caches a prior reap. So no stale PID is reachable there. Leave it and add one comment saying why.
  7. **Mux side.**
     - `MuxPane::child_pid`, `persistence_cwd`, `snapshot_capture_parts` and `probe_cwd` inherit `None` after the reap with no code change. After the reaper's `poll_running`/`mark_dead`, `pane-info` serves no `cmd=` token, the host probe skips the pane, scrape's liveness check skips it (`scrape.rs:567` is `if let Some`), and `persistence_cwd` falls back to OSC 7 or `None`.
     - Confirm that `mark_dead` (`pane.rs:356-359`) still gets its code: its `try_wait` now returns the recorded value.
  8. **Python.**
     - Update the `child_pid` docstring (`python_bindings/pty.rs:241-253`): "`None` before spawn, and after `try_wait()`/`wait()`/`kill()` has observed the exit: the PID is released and may belong to another process."
     - Update the `kill` docstring: "A no-op once the exit has been observed."
     - Mirror both in `docs/API_REFERENCE.md:1139` (and the `kill` line nearby), plus a CHANGELOG `[Unreleased]` bullet flagged as a behavior change: `child_pid()` becomes `None` after exit, and `kill()` after exit returns instead of raising `OSError`/ESRCH.
  9. **Failing-first tests.** Write them first and run them red.
     - `src/pty_session.rs` tests, `#[cfg(unix)]`:
       - `reaped_child_reports_no_pid`: `spawn("/bin/sh", &["-c", "exit 3"])`. Poll `try_wait()` until `Ok(Some(3))` (10 s deadline). Assert `session.child_pid().is_none()`. Red now: it returns `Some(pid)`.
       - `kill_after_reap_is_a_silent_noop`: same setup, then assert `session.kill().is_ok()`. Red now: portable-pty sends a raw `kill(pid, SIGHUP)` to the reaped PID and gets `ESRCH`, which `PtySession::kill` returns as `Err(IoError)`.
       - `resize_after_reap_sends_no_signal`: same setup. Add a `#[cfg(test)] signals_sent: Arc<AtomicUsize>` counter incremented in `signal_winch` (and by the reader pulse) only when `send_sigwinch` is actually called. After the reap, call `resize(100, 30)` and `resize_with_pixels(100, 30, 1000, 600)`, and assert the counter did not move. This one is compile-red before the fix. The behavior-red proof is the two tests above.
       - `poll_running_after_reap_is_false_even_with_a_stale_flag`: extend the existing `poll_running_reports_exit_despite_a_stale_reader_flag` shape to assert `child_pid()` is `None` afterwards.
     - `src/mux/pane.rs` tests: `a_held_dead_pane_serves_no_child_pid`: `ShellPaneFactory::default().create_pane(PaneId(20), 80, 24, Some("exit 3"), &SpawnContext::default())`. Poll `poll_running()` false (10 s). `mark_dead()`. Assert `exit_code() == Some(3)`, `child_pid().is_none()` and `persistence_cwd()` is `None` (no OSC 7 was emitted). Red now on `child_pid`.
     - `tests/test_pty.py`, Unix-only: `test_child_pid_none_after_exit` and `test_kill_after_exit_is_noop` (spawn `/bin/sh -c "exit 0"`, `wait_for(lambda: term.try_wait() is not None)`, then assert `term.child_pid() is None`, and that `term.kill()` does not raise).
- **Method**:
  - Only this struct reaps its child: portable-pty's `Child` is a `std::process::Child`, and the daemon installs no SIGCHLD handler (grep of `src/` for `SIGCHLD|waitpid` is empty). So "recorded under our lock" is equivalent to "reaped". Before the reap the PID is at worst a zombie, which the kernel will not recycle. Signalling it is harmless.
  - Enumerate every consumer before editing:
    - `get_symbol_context` with `symbol: "child_pid"`, `file_path: "src/pty_session.rs"`. Callers: `pty::PyPtyTerminal::child_pid`, `pane::MuxPane::child_pid`, `persistence_cwd`, `snapshot_capture_parts`, plus a test.
    - `get_symbol_context` with `symbol: "pane::MuxPane::child_pid"`. Tests only, plus `dispatch.rs:717` and `scrape.rs:567` via `pane.child_pid()`.
    - `get_impact` on `pty_session::PtySession::kill` and `pty_session::PtySession::try_wait` (upstream, depth 3). Expect `kill_detached`, `MuxPane::kill`, `Drop`, the Python bindings, `mark_dead` and the reaper.
    - `process_id()` has only the three `pty_session.rs` call sites (`:745,1171,1250`). After this change only `:745` (spawn) remains.
  - Pitfall, the portable-pty bypass: never call `Child::kill` (portable-pty's `ChildKiller`) unless the record says unreaped. Its first action is a raw `libc::kill(pid, SIGHUP)` with no check of std's cached status. std's own `Child::kill` would refuse, portable-pty's does not.
  - Pitfall: `master.resize()` (TIOCSWINSZ) needs no guard. The kernel signals the tty's foreground process group by reference, not by numeric PID.
  - Pitfall: do not hold `reaped` while taking the terminal write lock. The reader takes the terminal lock and then `try_lock`s `reaped`, so the reverse order would invert the lock order.
  - Windows: `WinChild::kill` is `TerminateProcess` on an owned handle, so it is safe even today. The shared guard still applies for uniformity, and `child_pid() == None` after the exit matters there too.
  - Blocks SEC-128.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `make dev` then `uv run pytest tests/test_pty.py tests/test_pty_resize_sigwinch.py -v`
  - `make test-pty` (the longer per-test timeout family).
  - Windows VM (main checkout): `cargo check --all-targets`, `cargo check --lib --tests --no-default-features --features rust-only,mux,serde`, and the `mux::` lib test run.
  - `make checkall`

### [SEC-126] `respawn-pane` reads `-k`/`-c` anywhere on the line, including inside the command, so it kills live panes and runs a different command (merges QA-183)
- **Card**: `01a0ef5abd2c75c293b9c944f62eab89`
- **Files**:
  - `src/mux/command.rs`:
    - `:1200-1227` (`parse_respawn_pane`).
    - `Args` helpers: `:392-398` (`flag`), `:434-450` (`quoted_flag_allowing_empty`, which re-splits the **whole** line and finds the first `-c` anywhere), `:468-470` (`has_flag`, which scans every token), `:504-512` (`trailing_after`, which rejoins whitespace-split tokens with single spaces).
    - `:553-588` (`split_after_flag`, the raw-slice precedent) and `:597-633` (`shell_split`, the quoting grammar to mirror).
    - `:763` (`COMMANDS` table, `respawn-pane` at :789), `:800-842` (the `parse_command` doc comment that lists the trailing-text commands).
    - Existing tests `:2283-2311`.
  - `src/mux/tree.rs:1049-1051` (the `alive && !kill` guard the bug bypasses).
  - `src/mux/dispatch.rs:557-573` (`resolve_start_dir`, where a misparsed `-c` value lands) and `:849-905` (`cmd_respawn_pane`). Neither changes: `MuxCommand::RespawnPane { pane, kill, start_dir, command }` keeps its shape.
  - `docs/MUX.md:180` (command table row) and `:360` (respawn paragraph).
  - Fuzz target `fuzz/fuzz_targets/mux_parse_command.rs`.
- **Steps**:
  1. **Failing-first tests.** Add them, run them, and see them red.
     - In `command.rs` tests, a table test `respawn_pane_parses_only_leading_flags`, built as `Vec<(&str, Result<(Target<PaneId>, bool, Option<&str>, Option<&str>), ()>)>` asserting `parse_command(line)`:

       | # | Line | Expect |
       |---|------|--------|
       | 1 | `respawn-pane -t %0` | `%0`, kill false, dir None, cmd None |
       | 2 | `respawn-pane -t %0 -k -c /tmp sleep 60` | kill, dir `/tmp`, cmd `sleep 60` |
       | 3 | `respawn-pane -t %0 -k top` | kill, cmd `top` |
       | 4 | `respawn-pane -t %0 sh -c 'echo X; sort -k 1 /dev/null; sleep 600'` | kill **false**, dir None, cmd `sh -c 'echo X; sort -k 1 /dev/null; sleep 600'` (the reproduction) |
       | 5 | `respawn-pane -t %0 sh -c 'echo hi'` | dir None, cmd `sh -c 'echo hi'` |
       | 6 | `respawn-pane -c /tmp -t %0 top` | dir `/tmp`, cmd `top` (was `-t %0 top`) |
       | 7 | `respawn-pane -t %0 -c '/a b' sleep 5` | dir `/a b`, cmd `sleep 5` (was `b' sleep 5`) |
       | 8 | `respawn-pane -t %0 -c "/a b" sleep 5` | dir `/a b`, cmd `sleep 5` |
       | 9 | `respawn-pane -t %0 printf '%s\n'   'a    b'` | cmd exactly `printf '%s\n'   'a    b'` (runs of spaces and quotes preserved) |
       | 10 | `respawn-pane -t %0 -- -weird --flag` | cmd `-weird --flag` |
       | 11 | `respawn-pane -t %0 -k -- top -k` | kill, cmd `top -k` |
       | 12 | `respawn-pane -k -t %0` | kill, cmd None |
       | 13 | `respawn-pane -t %0 --` | cmd None |
       | 14 | `respawn-pane   -t   %0    -k    top  ` | kill, cmd `top` (whitespace runs between flags, trailing whitespace trimmed) |
       | 15 | `respawn-pane -t 'my pane' top` | `Target::Name("my pane")`, cmd `top` |
       | 16 | `respawn-pane -t %0 echo -k` | kill false, cmd `echo -k` |
       | 17 | `respawn-pane -t %0 echo -c /etc` | dir None, cmd `echo -c /etc` |
       | 18 | `respawn-pane -t %0 -x foo` | Err (unknown flag `-x`) |
       | 19 | `respawn-pane -t %0 -t %1` | Err (duplicate `-t`) |
       | 20 | `respawn-pane -t` | Err (`-t` requires a value) |
       | 21 | `respawn-pane -c /tmp top` | Err (`requires -t`) |
       | 22 | `respawn-pane -t %0 -c ''` | Err (empty start directory) |
       | 23 | `respawn-pane -t %0 -kt %1` | Err (unknown flag `-kt`; combined flags are not supported) |

       Rows 4, 6, 7, 9, 16, 17, 18, 19 and 23 are red against HEAD.
     - Daemon test `tests/mux_daemon.rs::respawn_pane_ignores_flags_inside_the_command`:
       - `new-session -s flags`. Then `respawn-pane -t %0 sh -c 'echo X; sort -k 1 /dev/null; sleep 600'` must reply `%error` containing `still running`. Red now: exit 0, and the live pane is killed.
       - Then `respawn-pane -t %0 -k sh -c 'echo RESP-$((6*7))'`. Wait for `%pane-respawned %0`, then an `%output %0` containing `RESP-42`, within 15 s, using the `next_notification`/`recv_timeout` pattern from `a_dead_pane_is_held_announced_and_respawnable` (`:371-450`). Red now: the inner `-c` is read as the start directory and `sh` runs the fragment `'echo RESP-$((6*7))'`.
  2. **The reusable helper.** Add it after `split_after_flag` (`:588`) and design it for QA-219:
     ```rust
     /// One flag a command accepts ahead of its free-text tail (SEC-126).
     #[derive(Clone, Copy, Debug, PartialEq, Eq)]
     enum LeadingFlag {
         /// Presence only (`-k`).
         Bare(&'static str),
         /// Exactly one value word (`-t v`); a second occurrence is an error.
         Valued(&'static str),
         /// A value word that may repeat (`-e NAME=V`), for QA-219's new-session.
         Repeated(&'static str),
     }

     /// The flags that lead a line, and the raw text after them.
     #[derive(Debug, Default, PartialEq, Eq)]
     struct LeadingFlags<'a> {
         /// Flags in line order; values unquoted by the `shell_split` grammar.
         flags: Vec<(&'static str, Option<String>)>,
         /// Raw slice after the last flag (or after `--`), leading whitespace
         /// dropped and trailing whitespace trimmed; empty when nothing follows.
         tail: &'a str,
     }

     impl LeadingFlags<'_> {
         fn value(&self, flag: &str) -> Option<&str>;   // last Valued/Repeated value
         fn values(&self, flag: &str) -> Vec<&str>;     // every Repeated value
         fn has(&self, flag: &str) -> bool;
     }

     fn split_leading_flags<'a>(
         line: &'a str,
         name: &str,
         spec: &[LeadingFlag],
     ) -> Result<LeadingFlags<'a>, String>;
     ```
     The grammar, as tokens consumed in order:
     - Start after the command name, stripped the way `parse_send_keys` does it: `line.trim_start().strip_prefix(name)`. A fuzz-found panic came from not trimming first.
     - Read the next word with a new `fn next_shell_word(s: &str, from: usize) -> Option<(usize, usize, String)>`. It returns the raw byte range and the unquoted text, following exactly `shell_split`'s rules: `'…'`/`"…"` literal, `\` escapes the next char, and unquoted Unicode whitespace separates. Walk with `char_indices` so every slice index is a char boundary.
     - If the word is `--`: consume it, and the tail is the rest of the line. Stop.
     - If the word matches a `Bare`: record it and continue.
     - If the word matches a `Valued`/`Repeated`: the next word is its value. Missing gives `Err("{name}: {flag} requires a value")`. A repeated `Valued` gives `Err("{name}: duplicate {flag}")`. Continue.
     - Else, if the word starts with `-`, is longer than 1 char and is not in the spec: `Err("{name}: unknown flag {word}")`. This matches tmux's getopt. A command starting with `-` must follow `--`.
     - Else it is the first non-flag word: the tail is the raw slice from that word's **start byte**, `trim_end()`. Stop.
     - Running out of words gives an empty tail.
  3. Rewrite `parse_respawn_pane`:
     ```rust
     fn parse_respawn_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
         use LeadingFlag::{Bare, Valued};
         let lead = split_leading_flags(a.line, a.name, &[Valued("-t"), Valued("-c"), Bare("-k")])?;
         let raw = lead.value("-t").ok_or_else(|| format!("{} requires -t", a.name))?;
         let pane = Target::parse(raw).map_err(|_| format!("invalid pane target: {raw}"))?;
         let start_dir = match lead.value("-c") {
             Some("") => return Err(format!("{}: -c requires a non-empty directory", a.name)),
             other => other.map(str::to_string),
         };
         let command = (!lead.tail.is_empty()).then(|| lead.tail.to_string());
         Ok(MuxCommand::RespawnPane { pane, kill: lead.has("-k"), start_dir, command })
     }
     ```
     Delete the "anchor on whichever value flag sits LAST" comment. It contradicted the code (which anchored on `-c` whenever present) and is now obsolete.
  4. Update the `parse_command` doc (`:800-830`): `respawn-pane` takes leading flags then a raw tail (`split_leading_flags`), and `rename-window` is the one remaining `trailing_after` user.
     - Do **not** change `Args::has_flag`, `quoted_flag` or `trailing_after`. Every other parser still uses them, and QA-219 and QA-187 are separate cards.
  5. `docs/MUX.md:180` and `:360`: flags must precede the command, `--` ends flag parsing, and the command text is passed verbatim to the pane's shell (`sh -c`, or `cmd /C` on Windows), quoting and whitespace included. CHANGELOG `[Unreleased]` gets a Fixed bullet naming the kill-without-`-k` behavior.
- **Method**:
  - Why a raw slice: the pane runs `sh -c <command>` (`pane.rs:604-613`), so the shell, not par-mux, must see the user's quoting. Unquoting and rejoining (`trailing_after`) is exactly what collapsed `'a    b'` and orphaned the quote in `b' sleep 5`.
  - Why stop at the first non-flag word: tmux's `args_parse` uses getopt semantics, which also stop at the first non-option. Every `-k`/`-c` after that point belongs to the command.
  - QA-219 reuse: `new-window` becomes `split_leading_flags(line, name, &[Valued("-t"), Valued("-n"), Valued("-c")])` with the tail as its command, and `new-session` becomes `&[Valued("-s"), Repeated("-e")]`. Keep the helper and its word scanner free of respawn specifics.
  - Pitfall: `quoted_flag`'s whole-line `shell_split` is the root cause, because it finds the `-c` inside `sh -c '…'`. Do not call any `Args` flag helper from the new parser.
  - Pitfall: byte-index slicing on a non-ASCII line. The fuzz target `mux_parse_command` exercises this, so run it (Verify).
  - Blocks SEC-128 (`begin_respawn`), QA-187 and QA-219.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::command`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - From `fuzz/`: `cargo +nightly fuzz run mux_parse_command -- -max_total_time=120 -rss_limit_mb=512`
  - The Windows VM is optional here (pure parser, no `cfg`), but the daemon test spawns `sh`. Keep the new daemon test `#[cfg(unix)]`, or give it a `cmd /C` variant.
  - `make checkall`

### [SEC-128] Respawn's default cwd trusts OSC 7 output, including a remote host's OSC 7
- **Card**: `01a0ef5ac044773097f838b5caf64908`
- **Files**:
  - `src/mux/tree.rs:1017-1068` (`begin_respawn`; `pane.persistence_cwd()` at `:1045`, the fallback chain at `:1062-1065`: `cwd.or(stored_cwd).or_else(current_dir)`).
  - `src/mux/pane.rs:288-297` (`persistence_cwd`, which is OSC 7 first and then `process_cwd(child_pid)`), `:720-755` (`process_cwd` per-OS).
  - `src/terminal/sequences/osc/shell.rs:288-315` (`parse_osc7_url`): the hostname is parsed, and empty or `localhost` maps to `None` (`:309-312`). It is stored by `record_cwd_change` (`src/terminal/shell_integration.rs:160-177`) and readable as `terminal.shell_integration().hostname()` (`src/shell_integration.rs:97`).
- **Steps** (after SEC-125 and SEC-126):
  1. Add `MuxPane::respawn_cwd(&self) -> Option<PathBuf>`, keeping `persistence_cwd` unchanged (restore relies on OSC 7 first):
     - Take `let term = self.terminal(); let term = term.read();`. If `term.current_directory()` is `Some(dir)`, `osc7_host_is_local(term.shell_integration().hostname())`, and `Path::new(dir).is_dir()`, return that path.
     - Otherwise return `self.session.child_pid().and_then(process_cwd)`. After SEC-125 this is `None` for a reaped child, and the kernel-reported cwd for a live one (the `-k` case).
  2. Add `fn osc7_host_is_local(host: Option<&str>) -> bool` in `pane.rs`:
     - `None` is local (the parser already folded empty and `localhost`).
     - Otherwise compare case-insensitively against the local hostname, both full and short form (the text before the first `.`).
     - Unix: `libc::gethostname` into a 256-byte buffer, with a `// SAFETY:` comment (QA-201 counts these).
     - Windows: `std::env::var("COMPUTERNAME")`.
     - If the local name is unknown, return `false`, so an unverifiable host is not trusted.
  3. In `begin_respawn`, replace `pane.persistence_cwd()` at `:1045` with `pane.respawn_cwd()`. Keep the explicit `-c` precedence and the final `current_dir` fallback as they are. `None` then lets the factory cwd apply; portable-pty falls back to `$HOME` when the directory is absent or not a dir (`portable-pty-0.9.0/src/cmdbuilder.rs:501-507`).
  4. Tests in the `tree.rs` tests, using `ShellPaneFactory` panes running `sleep 60`, with `begin_respawn(pane, true, None, None)`. Feed OSC 7 with `pane.terminal().write().process(format!("\x1b]7;file://{host}{dir}\x1b\\").as_bytes())`, the shape `persist.rs:1339` uses.
     - `respawn_ignores_a_remote_osc7_cwd`: host `remote.invalid`, dir a live tempdir. Assert `plan.cwd != Some(tempdir)`. Red now.
     - `respawn_uses_a_local_osc7_cwd`: host `localhost`, then again with the local `gethostname`. Assert `plan.cwd == Some(tempdir)`.
     - `respawn_ignores_an_osc7_dir_that_is_gone`: `localhost` plus a path that does not exist. Assert it is not used.
- **Method**:
  - This keeps respawn consistent with SEC-115's rule for the probe: program output may pick a directory only when it names this host and the directory exists.
  - Persistence restore keeps OSC 7 first (MUX.md documents it). That path is rare and was accepted in the prior cycle. Do not change it here.
  - Many shells emit `file://$HOSTNAME/path`, not `localhost`, so a `None`-only rule would break every local zsh/bash integration. The hostname comparison is required.
  - Pitfall: `current_directory()` returns `&str` borrowed from the read guard. Copy it before dropping the guard.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::tree`
  - The mux lib filter and `mux_daemon`.
  - Windows VM: both `cargo check` commands (the new `cfg(windows)` branch).
  - `make checkall`

### [SEC-129] The `pane-info cmd=` foreground name is program-controlled and unfiltered
- **Card**: `01a0ef5ac2f67a31b63548b5f01f80fe`
- **Files**:
  - `src/mux/foreground.rs:137-176` (`ProcessTable::foreground_command`; the basename is built at `:166-175`). Its tests are at `:561-577` (`foreground_command_names_the_deepest_descendant`).
  - `src/mux/dispatch.rs:700-728` (`cmd_pane_info`, the only caller; parsight `get_symbol_context foreground_command` confirms one production caller at `dispatch.rs:723`).
  - The rule it mirrors: `src/mux/host_probe.rs:98-112` (`git_branch`, which rejects empty, over-128 and control-char values).
  - `docs/MUX.md:173` (`pane-info` row).
- **Steps**:
  1. In `foreground.rs`, add `/// cap: Bytes of a pane's foreground command name served by pane-info.` followed by `const MAX_FOREGROUND_NAME_LEN: usize = 128;`.
  2. At the end of `foreground_command`, after computing the basename, return `None` when it is empty, `name.len() > MAX_FOREGROUND_NAME_LEN`, or `name.chars().any(char::is_control)`. Serve absent, exactly as `git_branch` does; do not truncate or strip, because a partial name is a different name.
  3. Extend the doc comment: the value is argv[0], which the program sets. It is a display hint, never an identity.
  4. Test `foreground_command_rejects_control_and_oversize_names` using `ProcessTable::with_fixed_argv`:
     - `argv(&["\x1b[31mvim"])` gives `None`.
     - `argv(&["/usr/bin/a\x07b"])` gives `None`.
     - `"a".repeat(129)` gives `None`.
     - `"a".repeat(128)` gives `Some`.
     - `"vim"` gives `Some("vim")`.
     - Red now for the first three.
  5. Run `make caps-table`. The new `/// cap:` constant must appear in the `docs/SECURITY.md` caps table, or `caps-table-check` in `checkall` fails.
  6. `docs/MUX.md:173`: append "a hint set by the program itself (argv[0]); names with control characters or over 128 bytes are served absent".
- **Method**: Filtering at the producer covers every consumer. Base64 already prevents line-framing injection, so this defends the client's display (ANSI escapes, spoofed or oversized names).
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::foreground`
  - The mux lib filter.
  - `make caps-table-check`
  - Windows VM: `cargo check --lib --tests --no-default-features --features rust-only,mux,serde` (Windows `read_argv` returns `None`, so compile only).
  - `make checkall`

### [SEC-133] Host probe: `run_git` pipe deadlock, and the probe and shutdown can hang on a wedged filesystem (merges QA-193)
- **Card**: `01a0ef5ac5af75418c80ae0af38961ec`
- **Files**:
  - `src/mux/host_probe.rs`:
    - `:6-11` (module doc: "up to three per pane, each under GIT_TIMEOUT"), `:36-45` (`HOST_PROBE_INTERVAL`, `GIT_TIMEOUT`, `SWEEP_DEADLINE`; the comment at `:43-45` wrongly says "one in-flight git").
    - `:59-65` (`probe_cwd`), `:72-88` (`disk_free_percent`, an unbounded `statvfs`), `:98-112` (`git_branch`), `:127-147` (`git_dirty`: `diff-index` at :128, `ls-files --others --directory` at :138-143).
    - `:185-218` (`run_git`: stdout is read only after `try_wait` reports exit, then `kill` plus an unbounded `wait` at :213-214).
    - `:263-305` (`host_probe_sweep`; the deadline is checked only between panes at :286-298), `:349-371` (`spawn_host_probe_worker`; the doc at :350-354 wrongly says "skipped, not awaited").
    - Test call sites of the sweep: `:557`, `:662`, `:672`.
  - `src/mux/server.rs:244` (`spawn_host_probe_worker` call) and `:367-371` (the unconditional `probe_worker.join()`, whose comment at `:369-370` claims a bound).
- **Steps**:
  1. **Drain stdout concurrently, and cap it** (the QA-155 pipe-drain half). Change the signature to `run_git(cwd, args, stdout_cap: usize, deadline: Instant, shutdown: &AtomicBool) -> Option<Output>`, built on a generic `fn run_bounded(command: Command, stdout_cap, deadline, shutdown) -> Option<Output>`, which the tests use:
     - Spawn with `stdout(Stdio::piped())` as today, then `let pipe = child.stdout.take()?`.
     - Start a reader thread: `let mut out = Vec::new(); let _ = pipe.take(stdout_cap as u64).read_to_end(&mut out); drop(pipe); let _ = tx.send(out);`. Dropping the read end after `stdout_cap` bytes makes a chatty git fail on EPIPE/SIGPIPE and exit, instead of blocking on a full pipe.
     - Poll `child.try_wait()` every 10 ms until exit, `Instant::now() >= deadline`, or `shutdown.load(Relaxed)`.
     - On deadline or shutdown: `let _ = child.kill();`, then hand the child to a detached reaper (`std::thread::spawn(move || { let _ = child.wait(); })`) and return `None`. Never block on `wait()` on the probe thread: git in uninterruptible I/O on a hung mount never returns.
     - On exit: `rx.recv_timeout(Duration::from_millis(250))` for the output. On a timeout, return `None` and leave the reader thread detached; it ends when the pipe closes.
  2. **Per-call caps.**
     - `git_branch`: `stdout_cap = MAX_GIT_BRANCH_LEN + 2`. Longer output is rejected anyway.
     - `diff-index --cached --quiet`: `0`. It prints nothing; only the status matters.
     - `ls-files --others --exclude-standard --directory`: `1`, because only emptiness matters. Treat "got at least 1 byte" as dirty **regardless of exit status** (git may die of SIGPIPE after we close).
  3. **Shutdown and deadline plumbing.**
     - `probe_cwd(cwd, deadline, &shutdown)` checks `shutdown` before each of `statvfs`, the branch call and the two dirty calls, and passes `min(deadline, now + GIT_TIMEOUT)` to each git call.
     - `host_probe_sweep(tree: &Arc<Mutex<MuxTree>>, shutdown: &Arc<AtomicBool>, state: &mut ProbeState)` checks `shutdown` before each pane.
     - `spawn_host_probe_worker` owns a `ProbeState` and passes its existing `shutdown` Arc.
     - Update the three test calls (`:557,662,672`) to pass `&Arc::new(AtomicBool::new(false))` and `&mut ProbeState::default()`.
  4. **Bound each pane's probe, `statvfs` included.** Add:
     ```rust
     /// Per-pane wall-clock budget (git runs are each capped by GIT_TIMEOUT).
     const PANE_PROBE_BUDGET: Duration = Duration::from_secs(6);
     /// Probe workers left running on a wedged filesystem before the sweep
     /// stops starting new ones.
     const MAX_ABANDONED_PROBES: usize = 4;

     #[derive(Default)]
     pub(crate) struct ProbeState {
         /// Panes whose last probe timed out, keyed to the cwd that wedged;
         /// skipped until the pane's cwd changes.
         timed_out: HashMap<PaneId, PathBuf>,
         abandoned: Arc<AtomicUsize>,
     }
     ```
     For each target in the sweep:
     - Skip it if `state.timed_out.get(&pane) == Some(&cwd)`, or if `abandoned >= MAX_ABANDONED_PROBES`.
     - Otherwise run `probe_cwd` on a fresh thread and receive its result with a `recv_timeout(100 ms)` loop until `min(sweep_deadline, now + PANE_PROBE_BUDGET)` or `shutdown`.
     - On timeout: record `timed_out.insert(pane, cwd)` and `abandoned.fetch_add(1)`. The worker thread does `abandoned.fetch_sub(1)` when it finally returns.
     - On success: `timed_out.remove(&pane)`.
  5. **Bounded shutdown join.** In `server.rs:367-371`, replace `let _ = probe_worker.join();` with a local `fn join_bounded(handle: JoinHandle<()>, bound: Duration) -> bool` that polls `handle.is_finished()` every 25 ms up to `PROBE_JOIN_BOUND = Duration::from_secs(1)`. Join if finished. Otherwise `debug_error!("MUX", "host probe still running at shutdown; detached")` and drop the handle.
  6. **Correct the four comments**, each describing the post-fix bound:
     - Module doc `:6-11`.
     - `SWEEP_DEADLINE` doc `:43-45`: bounds the sweep's wall clock; each pane's probe runs on a detached worker capped by `PANE_PROBE_BUDGET`.
     - Worker doc `:350-354`: exits within one 500 ms poll plus at most one 100 ms `recv_timeout` of the shutdown flag.
     - `server.rs:369-370`: at most `PROBE_JOIN_BOUND`, then detached.
  7. **Tests** in `host_probe.rs`, `#[cfg(unix)]` where a shell is used:
     - `git_dirty_survives_output_larger_than_the_pipe_buffer`: in the `git_repo()` fixture, create 3,000 untracked files with 40-char names (over 120 KB of `ls-files` output). Assert `git_dirty(..) == Some(true)` and that it took under 2 s. Red now: git blocks at the 64 KiB pipe until the 5 s kill, and `git_dirty` returns `None`.
     - `run_bounded_returns_promptly_on_shutdown`: `Command::new("sleep").arg("30")`. Set the flag from another thread after 100 ms. Assert `None` in under 500 ms.
     - `run_bounded_times_out_without_blocking_on_wait`: the same command with a 200 ms deadline. Assert `None` in under 600 ms.
     - `a_timed_out_pane_is_skipped_until_its_cwd_changes`: drive the per-pane runner through a generic `probe_pane_bounded<F: FnOnce() -> HostProbe + Send + 'static>(f, budget, &shutdown)` with `f` sleeping 2 s and a 100 ms budget. Assert `Err` in under 300 ms, then assert that `ProbeState` skips the same `(pane, cwd)` pair and resumes probing for a different cwd.
     - In `server.rs` tests: `join_bounded_detaches_a_stuck_thread` (a thread sleeping 5 s; bound 200 ms; returns `false` in under 400 ms).
  8. The existing `fsmonitor_hook_does_not_run` and `sweep_probes_child_cwd_not_osc7` tests must stay green. `git_command`'s hardening is unchanged.
- **Method**:
  - Draining at exit is the deadlock: once git has written about 64 KiB, it blocks in `write`, never exits, and the loop waits for an exit that never comes.
  - Closing the read end after a cap is the cheapest drain. The dirty check needs one byte, and the branch needs at most 130.
  - `statvfs` and a post-kill `wait` on a hung NFS/FUSE mount are uninterruptible. The only defense is not to wait on them: run them on a thread you can abandon, and never join such a thread at shutdown.
  - Pitfall: the per-pane worker must own its inputs (`PathBuf`, `Arc<AtomicBool>`), not borrow from the sweep.
  - Pitfall: do not hold the tree lock across any of this. The sweep already collects `PaneSnapshotParts` under the lock and probes after it drops (ARC-032); keep that shape.
  - Windows: `disk_free_percent` is `None`, and git runs normally. Closing the pipe makes git's next write fail with `ERROR_NO_DATA`, so it exits there too.
  - Blocks DOC-110 (SECURITY.md describes the new bounds).
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::host_probe` and `… mux::server`
  - The mux lib filter and `mux_daemon` (shutdown paths: `kill_server_stops_the_daemon_after_a_final_save`).
  - Windows VM: both `cargo check` commands, plus the `mux::` lib run.
  - `make checkall`

### [SEC-131] Python debug log: fixed shared-temp path, follows symlinks, default permissions
- **Card**: `01a0ef5ac85471e1a12a8be3fa040ad6`
- **Files**:
  - `python/par_term_emu_core_rust/debug.py:11-12` (module docstring naming the fixed file), `:25` (`DEBUG_FILE = Path(tempfile.gettempdir()) / "par_term_emu_debug_python.log"`), `:52-63` (`_initialize`; `open(DEBUG_FILE, "w", buffering=1)` at `:60`).
  - The reference implementation `src/debug.rs:61-79`: PID suffix, `mode(0o600)`, `O_NOFOLLOW`, and fail closed.
  - New test file `tests/test_debug_log.py`.
- **Steps**:
  1. `:25`: `DEBUG_FILE = Path(tempfile.gettempdir()) / f"par_term_emu_debug_python_{os.getpid()}.log"`. Keep the module-level name `DEBUG_FILE`; importers may read it.
  2. `:60`: replace the `open` with:
     ```python
     flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, "O_NOFOLLOW", 0)
     fd = os.open(DEBUG_FILE, flags, 0o600)
     self.file_handle = os.fdopen(fd, "w", buffering=1)
     ```
     The existing `except OSError` already disables logging, so a planted symlink fails closed, as the Rust side does. Keep the `# noqa: SIM115`, or drop it if ruff no longer flags `os.fdopen`.
  3. Update the module docstring (`:11-12`): the file name is `par_term_emu_debug_python_<pid>.log`, created `0600` and never through a symlink.
  4. `tests/test_debug_log.py` (Unix-only via `pytest.mark.skipif(sys.platform == "win32", …)`). Each test does `monkeypatch.setenv("DEBUG_LEVEL", "1")`, `monkeypatch.setattr(debug, "DEBUG_FILE", tmp_path / "log.txt")` and `monkeypatch.setattr(debug.DebugLogger, "_instance", None)`, then constructs `debug.DebugLogger()`. The class is a singleton, so reset `_instance` in teardown too.
     - `test_debug_log_is_owner_only`: assert `stat.S_IMODE(os.stat(path).st_mode) == 0o600`. Red now: 0o644 under umask 022.
     - `test_debug_log_refuses_a_planted_symlink`: create `victim = tmp_path / "victim"` with `"keep"`, and `os.symlink(victim, path)`. Construct the logger. Assert `victim.read_text() == "keep"` and `logger.level == debug.DebugLevel.OFF`. Red now: `open("w")` follows the link and truncates the victim.
     - `test_debug_log_name_carries_the_pid`: `str(os.getpid()) in debug.DEBUG_FILE.name`, checked after `importlib.reload(debug)`.
- **Method**:
  - Match the Rust side exactly, so both logs have the same threat model. On Linux's shared `/tmp`, `O_NOFOLLOW` plus `0o600` closes both the symlink redirect and the world-readable log.
  - Do **not** delete the `log_*` helpers at `:149-224` while in this file. `par-term-emu-tui-rust` imports `log_render_call`, `log_render_content`, `log_generation_check`, `log_widget_lifecycle` and `log_screen_corruption` (grep of `~/Repos/par-term-emu-tui-rust/src`), and `CHANGELOG.md:305` already records `debug.py` as kept for that consumer. QA-218's claim that they are unreferenced is wrong.
  - The sister project's `CLAUDE.md:95` names the old path. File that as a follow-up on its board from the orchestrator; do not edit another repo here.
- **Verify**:
  - `make dev` then `uv run pytest tests/test_debug_log.py -v`
  - `make lint-python`
  - `make checkall`

## Phase 2 — Architecture (sequential)

### [ARC-090] Zoom is enforced by convention; `resize-pane` while zoomed silently rewrites the hidden layout
- **Card**: `01a0ef5acb267bd0917f3a32d258e2c5`
- **Files**:
  - `src/mux/tree.rs`:
    - `:43-48` (`MuxWindow::zoomed` doc).
    - Every manual `window.zoomed = None` (verified): `:686` (`complete_split`), `:747` (`select_pane`, conditional), `:781` (`swap_panes`), `:887` (`break_pane`, source window), `:966` (`join_pane`, source window), `:1003` (`join_pane`, target window), `:1476` (`kill_pane`).
    - The two resize paths that never clear it: `:1174-1226` (`resize_pane`) and `:1237-1270` (`resize_pane_absolute`).
    - `:793-810` (`zoom_pane`), `:1281-1297` (`resize_window`), `:1371-1401` (`sync_pane_sizes`).
    - Zoom tests `:2312-2460`, including `layout_mutations_unzoom_the_window` at `:2396-2457`.
  - `src/mux/dispatch.rs:731-750` (`cmd_resize_pane`; no change needed).
  - `src/mux/server.rs:1048-1086` (`broadcast_layout_change` reads `zoomed` for the visible layout and the `Z` flag).
  - `docs/MUX.md` (the zoom description; verify the wording with `grep -n -i zoom docs/MUX.md`).
- **Steps**:
  1. **Failing-first tree tests**, each on an 80×24 window split vertically 40/40:
     - `absolute_resize_while_zoomed_unzooms_first`: zoom `first`, then `resize_pane_absolute(first, Some(79), None)`. Assert `window.zoomed == None`, `first` is 79 wide and `second` is 1 wide (terminal sizes equal the layout geometry). Red now: `zoomed` stays `Some(first)` and `first` stays 80×24, so the hidden layout silently became 79/1. This is the audit probe.
     - `relative_resize_while_zoomed_unzooms_first`: zoom `first`, then `resize_pane(first, ResizeDirection::Right, 5)`. Assert `zoomed == None` and `first` is 45 wide. Red now.
     - `a_rejected_resize_keeps_the_zoom`: zoom `first`, then `resize_pane(first, ResizeDirection::Up, 1)`, which is the wrong axis and returns `Err(PaneNotResizable)`. Assert `zoomed == Some(first)`. This documents the contract: a failed command changes nothing.
  2. **The choke point.** Add it to `MuxTree` next to `sync_pane_sizes`:
     ```rust
     /// The single entry for layout-shape mutations (ARC-090): run `f` on
     /// the window, then end any zoom and re-fit every pane terminal. On
     /// `Err` the window is untouched, so the zoom and sizes stay as they
     /// were.
     fn mutate_layout<R>(
         &mut self,
         window_id: WindowId,
         f: impl FnOnce(&mut MuxWindow) -> Result<R, MuxError>,
     ) -> Result<R, MuxError> {
         let window = self
             .windows
             .get_mut(&window_id)
             .ok_or(MuxError::NoSuchWindow(window_id))?;
         let out = f(window)?;
         window.zoomed = None;
         self.sync_pane_sizes(window_id);
         Ok(out)
     }

     /// The post-step alone, for mutations already applied outside a
     /// closure (the kill cascade's cross-window removal).
     fn end_layout_mutation(&mut self, window_id: WindowId) {
         if let Some(window) = self.windows.get_mut(&window_id) {
             window.zoomed = None;
         }
         self.sync_pane_sizes(window_id);
     }
     ```
  3. **Fold each site in:**
     - `resize_pane` (`:1183-1224`): move the block body into `self.mutate_layout(window_id, |window| { …; Ok(window_id) })`. The `return Err(PaneNotResizable)` lines become `return Err(..)` from the closure.
     - `resize_pane_absolute` (`:1246-1269`): the same, but make the closure transactional. Today `-x 40 -y 10`, where x adjusts and y spans the axis, leaves the x change applied and returns `Err` with no `sync_pane_sizes`. Apply both axes to `let mut next = window.layout.clone();` and assign `window.layout = next;` only when every requested axis returned `Adjusted`.
     - `complete_split` (`:683-689`): replace `window.active = pane_id; window.zoomed = None;` plus `sync_pane_sizes` with `self.mutate_layout(window_id, |w| { w.active = pane_id; Ok(()) })`. The layout split happens earlier in the function (`:660-677`); keep it there so the kill-on-failure path stays unchanged.
     - `swap_panes` (`:767-783`): the `layout.swap_pane` call goes inside `mutate_layout`, and `:781` and `:783` are deleted.
     - `break_pane`: delete `:887` (the zoom clear before `only_pane`). In the `!only_pane` branch (`:919-928`), wrap `remove_pane` plus the active fix in `mutate_layout(source_window, …)`. Keep `self.sync_pane_sizes(window_id)` for the **new** window: it is a fresh, unzoomed window, not a mutation.
     - `join_pane`: delete `:966`. Wrap the source `remove_pane` (`:991-1000`, when not closed) and the target `split_pane` (`:997-1005`) in `mutate_layout` calls on their respective windows. Delete the trailing `sync_pane_sizes` calls at `:1006-1009`.
     - `kill_pane`: replace `:1475-1478` with `self.end_layout_mutation(affected_window);`. The `iter_mut().find_map` removal stays; QA-187 rewrites that cascade later.
     - `complete_respawn` (`:1095`): **not** a layout mutation (same id, same layout). Keep the plain `sync_pane_sizes`.
     - **Stay outside the choke point:** `select_pane` (`:747`) is a focus rule, not a layout change, and keeps its conditional unzoom. `zoom_pane` sets the zoom. `resize_window` and `set_client_cell_pixels` change the window extent, not the layout, and the existing `window_resize_while_zoomed_follows_the_new_grid` test pins that the zoom survives them.
  4. **Docs.**
     - `MuxWindow::zoomed` doc (`:43-48`): "every layout mutation goes through `mutate_layout`, which ends the zoom".
     - `docs/MUX.md`: `resize-pane -x/-y/-L/-R/-U/-D` unzooms first, as tmux does (`~/Repos/tmux/cmd-resize-pane.c:94-95`, `server_unzoom_window`).
     - CHANGELOG `[Unreleased]` Fixed bullet.
  5. No dispatch change: `cmd_resize_pane` already returns `.with_layout(window_id)`, and `broadcast_layout_change` then emits the real layout with no `Z` flag.
- **Method**:
  - Enumerate before editing, and re-run after to confirm nothing writes `zoomed` outside `mutate_layout`/`end_layout_mutation`/`zoom_pane`/`select_pane`/construction:
    - `grep -n "zoomed" src/mux/*.rs`. Expected leftovers: `tree.rs` construction at `:417,541,903`, `persist.rs:542`, and the readers in `server.rs:1070` and `sync_pane_sizes`.
    - `get_symbol_context` with `symbol: "tree::MuxTree::sync_pane_sizes"`.
  - Why tmux semantics: par-term drives `resize-pane -x/-y` from divider drags (`par-term/src/app/tmux_handler/gateway.rs:533-600`). A zoomed window that silently takes a hidden resize breaks the exact-layout restore on the first drag.
  - Pitfall: `mutate_layout` borrows `self.windows` mutably inside the closure. The closure must not call other `&mut self` tree methods. Hoist any lookup (such as `window_of_pane`) before the call.
  - Blocks ARC-096 (it maintains its `pane_window` index at this choke point) and QA-188 (it logs resize errors in `sync_pane_sizes`).
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::tree` (all zoom tests, including `layout_mutations_unzoom_the_window` and the break/join unzoom test at `:2616-2637`).
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde` (the zoom wire test, fixture `zoomwire`, at `:590`).
  - Windows VM: both `cargo check` commands.
  - `make checkall`

### [ARC-103] Two-phase spawn installs the output sink after the reader starts; the wire-up block is now copied four times (merges QA-187 wiring half)
- **Card**: `01a0ef5ace3278e19b000bc7efd8b425`
- **Batch**: implement together with ARC-089 in one commit. Both edit `complete_respawn`, `kill_detached` and the wiring blocks.
- **Files**:
  - `src/mux/pane.rs`:
    - `:440-461` (`SpawnContext<'a>`: `#[derive(Debug, Clone, Copy, Default)]`, all `pub` fields).
    - `:519-567` (`ShellPaneFactory::configured_session`, the one place every shell and argv spawn builds its `PtySession`, before `spawn`).
    - `:593-620` (`create_pane`), `:630-645` (Windows `create_argv_pane`, which also uses `configured_session`), `:675-715` (`AgentPaneFactory`, which delegates), `:396-407` (`on_output`).
  - `src/pty_session.rs:61` (`pub type OutputCallback = Arc<dyn Fn(&[u8]) + Send + Sync>`), `:276-283` (`set_output_callback`/`clear_output_callback`), `:1036-1039` (the reader calls the sink while holding the callback mutex), `:1410-1416` (`with_terminal_mut`, which publishes the geometry mirror).
  - `src/mux/tree.rs`: the plans and their `context()` methods at `:75-212` (`SessionSpawn`, `RespawnSpawn` at :96-131, `WindowSpawn` at :144-167, `SplitSpawn` at :171-203); completions at `:392` (`complete_session`), `:510` (`complete_window`), `:637` (`complete_split`), `:1075` (`complete_respawn`).
  - `src/mux/dispatch.rs`, the four copies:
    - `cmd_new_session` `:249-300` (create `:265`, wiring loop `:270-285`).
    - `cmd_split_window` `:575-640` (create `:605`, wiring and raw-lock note `:615-622`).
    - `cmd_respawn_pane` `:849-905` (create `:870`, wiring and raw-lock note `:884-890`).
    - `cmd_new_window` (verify its start with `find_symbol cmd_new_window`; create `:971`, wiring and raw-lock note `:976-990`).
  - `src/mux/server.rs:996-1024` (`pane_output_sink`), `:1026-1046` (`wire_all_pane_outputs`, the restore path, called from `bind_with_tree` at `:161`).
  - Struct-literal constructions of `SpawnContext` that a new field breaks: `src/mux/persist.rs:470`, `src/mux/pane.rs:1197` (test), `src/mux/tree.rs:123,135,161,191`. The literal at `pane.rs:1148` uses `..SpawnContext::default()` and needs no change. par-term only receives `&SpawnContext` (`par-term/par-term-tmux/tests/layout_conformance.rs:229-240`) and constructs none.
- **Steps**:
  1. **Failing-first tests.**
     - `pane.rs` tests `spawn_context_output_sink_sees_the_first_byte` (unix): collect into `Arc<Mutex<Vec<u8>>>` through an `OutputCallback`. Call `ShellPaneFactory::default().create_pane(PaneId(12), 80, 24, Some("printf FIRST-BYTE-MARK; exec sleep 5"), &SpawnContext { output: Some(OutputSink(Arc::clone(&sink))), ..SpawnContext::default() })` and **never** call `on_output`. Assert the collector contains `FIRST-BYTE-MARK` within 5 s. Compile-red now (no field).
     - `tests/mux_end_to_end.rs::a_new_panes_first_output_is_pushed_even_when_wiring_lags` (behavior-red now):
       - Use a `LaggingFactory { inner: ShellPaneFactory::default() }` whose `create_pane` calls `self.inner.create_pane(id, cols, rows, Some("printf EARLY-MARK; exec sleep 30"), context)` and then sleeps 500 ms before returning. That keeps the dispatcher between spawn and wiring while the pane prints.
       - `bind_with_tree` it. Register an observer `MuxClient` first with `list-sessions` (clients join broadcasts on their first control command; memory note `mux-broadcast-registration`).
       - Send `new-session -s lag` from a second client, then scan the observer's notifications for an `Output` on `%0` containing `EARLY-MARK` within 10 s.
       - Red now: the bytes arrive while no sink is installed and are never pushed.
       - Do not assert the output's position relative to the reply (see Method).
  2. **Carry the sink in the context.** In `pane.rs`:
     ```rust
     /// A pane output sink carried into the spawn so it is installed before
     /// the reader thread starts (ARC-103). A newtype so `SpawnContext`
     /// keeps `Debug`.
     #[derive(Clone)]
     pub struct OutputSink(pub OutputCallback);
     impl std::fmt::Debug for OutputSink {
         fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
             f.write_str("OutputSink(..)")
         }
     }
     ```
     Add `pub output: Option<&'a OutputSink>` to `SpawnContext`, documented: "installed on the PTY before the child spawns; factories that build their own `PtySession` must do the same, or they lose the first output". The struct stays `Copy`, since the field is a reference.
     - Update the four `tree.rs` `context()` bodies and `persist.rs:470` with `output: None`, and the `pane.rs:1197` test literal.
     - In `configured_session`, after the env setup: `if let Some(OutputSink(sink)) = context.output { session.set_output_callback(Arc::clone(sink)); }`.
     - CHANGELOG `[Unreleased]`, marked **Breaking (Rust)**: `SpawnContext` gains `output`, so struct literals need `output: None` or `..Default::default()`.
  3. **One helper for all four commands.** In `tree.rs`:
     ```rust
     /// A reserved spawn the dispatcher can run through `spawn_and_wire`.
     pub(crate) trait SpawnPlan {
         type Done;
         fn pane_id(&self) -> PaneId;
         fn size(&self) -> (u16, u16);
         fn context(&self) -> SpawnContext<'_>;
         fn complete(self, tree: &mut MuxTree, pane: MuxPane) -> Result<Self::Done, MuxError>;
     }
     ```
     Implement it for `SessionSpawn` (`Done = SessionId`, `complete_session`), `WindowSpawn` (`WindowId`), `SplitSpawn` (`(PaneId, WindowId)`) and `RespawnSpawn` (`PaneId`). Each impl forwards to the existing inherent `context()` and `complete_*`, so the one-shot tree paths (`new_session_with_env`, `new_window_with_cwd`, `split_pane`) are untouched.

     In `dispatch.rs`:
     ```rust
     /// Phases 2 and 3 of a two-phase spawn (ARC-022/ARC-103): spawn OFF
     /// the tree lock with the output sink already in the context, re-lock
     /// and complete, re-install the sink (for factories that ignore
     /// `context.output`), and write the start-dir note through the
     /// geometry-publishing path (QA-195).
     fn spawn_and_wire<P: SpawnPlan>(
         ctx: &Ctx<'_>,
         factory: &dyn PaneFactory,
         plan: P,
         command: Option<&str>,
         note: Option<&str>,
     ) -> Result<P::Done, MuxError> {
         let pane_id = plan.pane_id();
         let (cols, rows) = plan.size();
         let sink = OutputSink(Arc::new(pane_output_sink(ctx.clients, pane_id)));
         let pane = {
             let mut context = plan.context();
             context.output = Some(&sink);
             factory.create_pane(pane_id, cols, rows, command, &context)?
         };
         let mut guard = ctx.tree.lock();
         let done = plan.complete(&mut guard, pane)?;
         if let Some(pane) = guard.pane_mut(pane_id) {
             pane.on_output_sink(sink);           // idempotent re-install
             if let Some(note) = note {
                 pane.write_note(note.as_bytes());
             }
         }
         Ok(done)
     }
     ```
     - `pane_output_sink` returns `impl Fn`, so wrap it in `Arc::new(..)` to get an `OutputCallback`.
     - Add `MuxPane::on_output_sink(&mut self, sink: OutputSink)` (it calls `set_output_callback(sink.0)`).
     - Add `MuxPane::write_note(&self, bytes: &[u8])` as `self.session.with_terminal_mut(|t| t.process(bytes))`. This fixes QA-195's three dispatch sites (`dispatch.rs:621,889,986`). `persist.rs:1339` is a test and stays.
  4. **Rewrite the four commands** to `begin_*` under the lock (unchanged), then `spawn_and_wire(ctx, &*factory, plan, cmd, note.as_deref())`, then build the same `Outcome` as today. `new-session` passes `note = None` and `command = None`. `respawn-pane` passes `plan.command.clone()`: clone it before the move, since `complete` consumes the plan. Delete the four inline wiring blocks. For respawn, see ARC-089 step 3 for the detach before the spawn.
  5. **Restore path** (`server.rs:1026-1046`): it keeps wiring after the tree is built. `bind_with_tree` builds `clients` after `persist` has already spawned every pane, and no client can connect before the accept loop starts, so no audience exists for the early bytes. They still land in the grid, and clients seed from it. Change its body to call `pane.on_output_sink(OutputSink(Arc::new(pane_output_sink(clients, pane_id))))`, so one wiring function exists. Add a doc line saying why restore is exempt from the context sink.
- **Method**:
  - `PtySession::set_output_callback` on a session that has not spawned yet is read by the reader thread from its first `read()`, because the reader clones the same `Arc<Mutex<Option<OutputCallback>>>` at `start_reader_thread` (`pty_session.rs:824`). That makes this ordering airtight with no timing assumption.
  - Every in-repo factory delegates to `ShellPaneFactory::create_pane` with the caller's context: `SilentPaneFactory` (`server.rs:1679`), `SleepingFactory` (`tests/mux_end_to_end.rs:225`), `RecordingFactory` (`persist.rs:970`), `ContextRecordingFactory` (`pane.rs:788`) and par-term's `QuietFactory`. They all inherit the fix. The re-install after `complete` keeps third-party factories correct, with today's early-output gap.
  - **Ordering (no gate needed):** with the sink live from the first byte, `%output %N` can reach clients *before* the command's reply and before `%window-add`/`%layout-change`. That is not a regression, and no `arm()` gate is needed:
    - Today those early bytes are lost outright, because no sink exists yet.
    - par-term drops output for a pane id it has not mapped (`par-term/src/app/tmux_handler/notifications/output.rs`, the "On miss: … dropped" branches), so a new-session or split pane loses at most what it loses today.
    - A respawned pane is reset (`ESC c`) and re-seeded from `refresh-client` after `%pane-respawned` (`par-term/src/app/tmux_handler/notifications/mux_pane_exit.rs:100-122`), so anything pushed early is replaced from the daemon grid.
    - The daemon grid always has the bytes, and the fix makes them available to any client already mapped (other daemon clients and the streaming bridge).
  - Lock-order pitfall: the reader invokes the sink while holding the callback mutex, and the sink takes `clients.lock()` (`push_to_clients`, `server.rs:886`). Never take `clients.lock()` and then the tree lock. Verify with `get_symbol_context` on `push_to_clients` and on `broadcast_notification`: every caller must compute under the tree lock, release it, then push (as `broadcast_layout_change` does).
  - Enumerate the call sites with `get_symbol_context` for `pane::MuxPane::on_output` and `server::pane_output_sink`. Expect the four dispatch functions, `wire_all_pane_outputs` and tests.
  - Blocks QA-195 (three of its sites are done here) and QA-187 (its wiring half is absorbed; the kill-cascade half remains).
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde` (the split-output test, fixture `splitout`, at `:503`, and respawn at `:371`).
  - `cargo test --test mux_end_to_end --no-default-features --features rust-only,mux,serde`
  - `cargo test --lib --no-default-features --features rust-only,streaming,mux,serde streaming::mux_factory` (the streaming bridge consumes `%output`; without `mux` the filter matches zero tests).
  - Windows VM: both `cargo check` commands, and the `mux::` lib run (`create_argv_pane` on Windows goes through `configured_session`).
  - `make checkall`

### [ARC-089] `respawn-pane -k` leaks the dying process's output into the respawned pane's id
- **Card**: `01a0ef5ad1167c91b7b5dd563023e14a`
- **Batch**: same commit as ARC-103.
- **Files**:
  - `src/mux/tree.rs:11-21` (`kill_detached`: it spawns a thread that calls `pane.kill()` with the sink still wired), `:1075-1097` (`complete_respawn`: inserts the new pane under the same id, then `kill_detached(old)`), `:527`, `:678`, `:1081`, `:1425`, `:1526`, `:1573` (the other `kill_detached` callers).
  - `src/mux/pane.rs:396-407` (`on_output`), `:434-437` (`MuxPane::kill`).
  - `src/pty_session.rs:281-283` (`clear_output_callback`; the only non-test callers are tests, so production never clears), `:1036-1039` (the callback mutex is held while the sink runs).
  - `src/mux/dispatch.rs:849-905` (`cmd_respawn_pane`), `src/mux/server.rs:996-1024` (`pane_output_sink`, keyed by `pane_id`).
- **Steps**:
  1. **Failing-first test.** `pane.rs` tests `a_killed_panes_late_output_is_not_forwarded` (unix):
     - Create a pane with command `trap 'echo OLD-PANE-BYE; exit 0' HUP; echo READY-MARK; while :; do sleep 1; done`, with a collecting sink via `on_output`.
     - Wait for `READY-MARK` in the collector (5 s). This avoids racing the trap install; see the comment at `tree.rs:1909-1914`.
     - Call `pane.kill()`, sleep 300 ms, and assert the collector does **not** contain `OLD-PANE-BYE`.
     - Red now: the audit probe reproduced exactly this delivery.
  2. Add `MuxPane::detach_output(&mut self)` (`self.session.clear_output_callback()`). Call it first in `MuxPane::kill`, and **synchronously** in `kill_detached` before the thread spawns:
     ```rust
     fn kill_detached(mut pane: MuxPane) {
         // Stop forwarding before the SIGHUP: the pane's id may already
         // belong to a replacement (respawn), and a dying TUI's exit
         // sequences must not reach it (ARC-089).
         pane.detach_output();
         std::thread::spawn(move || {
             let _ = pane.kill();
         });
     }
     ```
     This covers every `kill_detached` caller (`kill-pane`, `kill-window`, `kill-session`, failed completes, and respawn) with one edit.
  3. **Respawn of a live pane (`-k`).** With ARC-103, the replacement's sink is live during the off-lock spawn while the old process is still alive and emitting under the same id. So:
     - In `cmd_respawn_pane` phase 1, right after `begin_respawn` succeeds and while still under the lock, call `guard.pane_mut(pane).map(MuxPane::detach_output)`.
     - If phase 2 (the spawn) fails, re-lock and re-attach the old pane's sink with `on_output_sink(OutputSink(Arc::new(pane_output_sink(ctx.clients, pane))))`, so a failed respawn leaves the live pane forwarding as before.
     - In the ARC-103 helper, this pre-detach is respawn-specific. Keep it in `cmd_respawn_pane`, not in `spawn_and_wire`.
  4. Add a tree-level regression test `respawn_does_not_forward_the_old_processes_exit_output`, using the same trap command:
     - Tag the collector with the source generation. The old pane's sink pushes into `Vec<(u8 /*gen*/, Vec<u8>)>` with gen 0 via `on_output`; the replacement gets gen 1 via `on_output` after `complete_respawn`.
     - Run `begin_respawn(pane, true, None, None)`, `create_pane`, then `complete_respawn`.
     - Wait 700 ms (longer than portable-pty's ~250 ms grace plus the 500 ms reap). Assert no gen-0 chunk contains `OLD-PANE-BYE`.
- **Method**:
  - `clear_output_callback` takes the same mutex the reader holds while it invokes the sink (`pty_session.rs:1036-1039`). When `detach_output` returns, any in-flight callback has finished and no later one can start. That is a strict happens-before with no sleep.
  - It runs under the tree lock (inside `complete_*`/`kill_pane`). That is safe because the sink takes only `clients.lock()`. See ARC-103's lock-order pitfall.
  - Detaching in `kill_detached`, rather than only inside `MuxPane::kill` (AUDIT's remedy), matters: `MuxPane::kill` runs on the detached thread, and the reader can still forward bytes between `complete_respawn`'s insert and that thread's first instruction.
  - `kill-pane`'s stray bytes previously reached only a vanished id. They are now suppressed too, which the Impact line in AUDIT.md anticipated.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` (including `killing_a_hup_ignoring_pane_reaps_it_and_returns_promptly`, `tree.rs:1904`, whose 150 ms lock-hold bound the synchronous detach must not break).
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - Windows VM: both `cargo check` commands, and the `mux::` lib run.
  - `make checkall`

## Phase 3a — Security (remaining)

### [SEC-130] Kitty `t=t` TOCTOU through a swapped parent directory
- **Card**: `01a0ef5ad49970e0ba00ad5110afe88f`
- **Files**:
  - `src/graphics/kitty.rs`:
    - `:286-296` (`is_under_allowed_temp_root`), `:298-316` (`open_no_follow`; `O_NOFOLLOW` guards only the final component).
    - `:1036-1073` (`decode_payload`; the delete at `:1063-1068` is `fs::remove_file(path)` on the **original** path).
    - `:1079-1196` (`load_file_data`; the canonical check at `:1119-1141`, `open_no_follow(path)` of the **original** path at `:1145`, and `pending_delete = Some(path.to_path_buf())` at `:1188-1192`).
  - Existing tests from `:1478`, `:3227-3260`, and `:3398-3530` (`t=t` temp-file cases).
- **Steps**:
  1. **Open the checked path.** In `load_file_data`, keep `canonical` in scope for `TempFile`. Change the open to `open_no_follow(if self.medium == KittyMedium::TempFile { &canonical } else { path })`. `t=f` has no containment check, so a TOCTOU there changes nothing and it keeps the original path.
  2. **Re-verify the opened handle** (`TempFile` only), so a parent component swapped between `canonicalize` and `open` is caught:
     - Linux: `std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))`.
     - macOS: `libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr())` into a `[0u8; libc::PATH_MAX as usize]` buffer, with a `// SAFETY:` comment.
     - Re-run `is_under_allowed_temp_root` and the `tty-graphics-protocol` marker check on that path, and reject with the existing error texts on failure.
     - Other Unix and Windows: skip this step, with a comment. The dev/ino delete check below still holds.
  3. **Record identity.** Replace `Option<PathBuf>` with:
     ```rust
     /// A `t=t` file to unlink after decode, pinned to the inode that was read.
     struct PendingDelete { path: PathBuf, #[cfg(unix)] dev_ino: (u64, u64) }
     ```
     `path` is `canonical`. On Unix, take `dev_ino` from `metadata.dev()`/`metadata.ino()` of the already-open handle (`std::os::unix::fs::MetadataExt`).
  4. **Guarded delete.** Add `fn delete_if_same_file(pending: &PendingDelete) -> bool`:
     - Unix: `symlink_metadata(&pending.path)`. If it is a regular file with the same `(dev, ino)`, `remove_file` and return true. Otherwise `debug_error!("KITTY", …)` and return false.
     - Windows: `remove_file(&pending.path)`, with a comment that Windows has no inode check here.
     - Call it from `decode_payload` at `:1066`. Keep the rule that the file is deleted only after a successful decode.
     - Optional stronger form: open the canonical parent with `O_DIRECTORY|O_NOFOLLOW`, compare `fstatat(dirfd, name, AT_SYMLINK_NOFOLLOW)`, then `unlinkat`. It needs `libc` calls and SAFETY comments. Use it only if the reviewer asks.
  5. **Tests** (unix):
     - `temp_file_delete_skips_a_swapped_file`: write a valid 1×1 PNG as `tty-graphics-protocol-swap.png` in a tempdir under `std::env::temp_dir()`. Build a `PendingDelete` through `load_file_data`. Then `rename` the file away and write a different file at the same path. `delete_if_same_file` returns false, and the new file survives.
     - `temp_file_delete_removes_the_file_that_was_read`: the happy path deletes.
     - The existing `t=t` tests (`:3398-3530`) must still pass, including the case where `/tmp` canonicalizes to `/private/tmp` on macOS. `canonical` is now the delete path, so assert on existence via the canonical path.
- **Method**:
  - Opening `canonical` alone does not close the window: a parent component of the canonical path can still be swapped for a symlink before the open. Re-deriving the path of the open fd verifies what was actually opened. Pinning `(dev, ino)` makes the delete touch only the file that was read.
  - Do all of this as one batch across `load_file_data` and `decode_payload`, because the open, the identity record and the delete depend on each other.
  - In par-mux daemon terminals `retain_temp_files` is set (`pane.rs:572-578`). There `pending_delete` stays `None` and only step 2's read check applies.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize kitty`
  - From `fuzz/`: `cargo +nightly fuzz run kitty -- -max_total_time=60 -rss_limit_mb=512`
  - Windows VM: `cargo check --all-targets` (`cfg(unix)` fields and the Windows delete branch).
  - `make checkall`

### [SEC-134] `paste` 1.0.15 is unmaintained (RUSTSEC-2024-0436), pulled in transitively
- **Card**: `01a0ef5ad83f75b3bd090d057f50cda8`
- **Track-only.** Committing `Cargo.lock` is decision **D2** (ARC-107). Do not change the lockfile policy here.
- **Files**:
  - `Cargo.lock` (untracked, `.gitignore:4`), with `paste` at `:2110-2113`.
  - `Cargo.toml:139` (`image` features, including `"rayon"`) and `:140-147` (the comment claiming the AVIF/EXR exclusions remove the last `paste` edge).
- **Findings from verification** (these correct AUDIT.md):
  - `paste` is **never compiled**. `cargo tree --locked --offline -i paste -e all --all-features --target all` prints "nothing to print".
  - It is in `Cargo.lock` only. `cargo metadata --all-features` lists the edges `image 0.25.10 → exr 1.74.2 → pulp 0.22.3 → paste` and `image → ravif 0.13.0 → rav1e 0.8.1 → paste`.
  - Both edges come from `image`'s `rayon` feature, which declares weak dependency features: `rayon = ["dep:rayon", "ravif?/threading", "exr?/rayon"]` (`~/.cargo/registry/src/*/image-0.25.10/Cargo.toml:113-117`). Cargo records weakly referenced optional dependencies in the lockfile even when they are never activated, and `cargo audit` scans the lockfile.
  - This crate uses no `image` rayon API: grep of `src/` for `rayon|par_iter|par_chunks|ParallelIterator` is empty. `image`'s `rayon` feature only gates `images::buffer_par`.
- **Steps**:
  1. Default (track): leave the card open with the evidence above in its notes. Recheck each release with `cargo audit` (expect exactly one allowed warning, RUSTSEC-2024-0436).
  2. Optional local fix. It is reversible, but it is outside "track-only", so the orchestrator must approve it first:
     - Remove `"rayon"` from the `image` features at `Cargo.toml:139`.
     - Correct the comment at `:140-147` to say the lockfile edge came from the weak `rayon` features.
     - Regenerate the lockfile (`cargo update -p image --offline`, or delete the untracked `Cargo.lock` and `cargo generate-lockfile`).
     - Confirm with `grep -c 'name = "paste"' Cargo.lock` → `0` and `cargo audit` → no warnings. Run `make check-features` (a `[features]`/dependency edit triggers it per the Makefile note at `:368-371`).
- **Method**: The advisory is informational (unmaintained, not vulnerable), and the crate never builds `paste`. The only exposure is audit noise. That is why track-only is acceptable. Record the local fix so the next audit does not re-derive it.
- **Verify** (if step 2 is approved):
  - `cargo tree -i paste --all-features --target all` (still empty).
  - `grep -c 'name = "paste"' Cargo.lock` gives `0`.
  - `cargo audit` shows 0 warnings.
  - `make check-features`
  - `make checkall`

### [SEC-135] GitHub-owned actions are still on floating tags in secret-bearing jobs
- **Card**: `01a0ef5adba6716081dbd56233f3d2d9`
- **Decision-gated (D3).** ENH-032 deliberately left `actions/*` on tags. **Do not execute the Steps below without the user's decision on D3.** If D3 is not approved, report the item as skipped and leave the card open.
- **Files** (enumerated at HEAD with `grep -n "uses: actions/" .github/workflows/*.yml | grep -v "@[0-9a-f]\{40\}"`):
  - 56 floating refs across 9 files:

    | File | Refs |
    |------|-----:|
    | `deployment.yml` | 28 |
    | `ci.yml` | 12 |
    | `publish-testpypi.yml` | 9 |
    | `fuzz.yml` | 2 |
    | `bench.yml` | 1 |
    | `claude.yml` | 1 |
    | `claude-code-review.yml` | 1 |
    | `publish-crates.yml` | 1 |
    | `release.yml` | 1 |

  - Actions referenced: `actions/checkout@v7`, `actions/setup-python@v7`, `actions/upload-artifact@v7`, `actions/download-artifact@v8`, `actions/github-script@v9`.
  - Workflows that reference `secrets.`: `deployment.yml`, `publish-crates.yml`, `publish-testpypi.yml`, `release.yml`, `claude.yml`, `claude-code-review.yml`. AUDIT.md omits the last two, which carry `checkout@v7` next to the pinned `anthropics/claude-code-action`.
  - `.github/dependabot.yml` already watches `github-actions` weekly.
- **Steps** (only if D3 is approved):
  1. For each action and tag, resolve the commit the tag points to **now**, so pinning changes no behavior:
     - `gh api repos/actions/<name>/git/ref/tags/<tag> --jq '.object'`.
     - If `.type == "tag"` (an annotated tag), dereference with `gh api repos/actions/<name>/git/tags/<sha> --jq '.object.sha'`.
     - Find the exact semver tag on that commit: `gh api repos/actions/<name>/tags --paginate --jq '.[] | select(.commit.sha=="<sha>") | .name'`, and choose the `vX.Y.Z` form.
     - Verify each SHA resolves before committing (`~/.claude/guides/git-ci.md`: always verify a ref resolves).
  2. Replace every ref with `uses: actions/<name>@<40-hex-sha> # vX.Y.Z`. That is the existing house style (`claude.yml:35`, `ci.yml:19`), and Dependabot bumps both the SHA and the comment.
  3. Do all 9 files in one commit. Secret-bearing files first is not required, since one commit covers everything.
- **Method**:
  - Floating major tags on `actions/*` can be moved by anyone who controls those repos or a compromised maintainer token. A moved `download-artifact` tag would run inside `deployment.yml`'s publish job, which holds PyPI OIDC (`id-token: write`, `:16-18`).
  - Pitfall: `actions/checkout@v7` may be an annotated tag. Pinning the tag object's SHA instead of the commit's fails at `uses:` resolution. Step 1's dereference handles it.
- **Verify**:
  - `grep -n "uses: actions/" .github/workflows/*.yml | grep -v "@[0-9a-f]\{40\}"` returns nothing.
  - `actionlint .github/workflows/*.yml` if installed.
  - After pushing (standing-approved for the repo's own remote), trigger `ci.yml` (`workflow_dispatch`: `gh workflow run ci.yml`) and watch it pass with `gh run watch`. Do not dispatch `deployment.yml`/`publish-*` (they publish).
  - `make checkall`

---

## Phase 3b — Architecture (parallel domain; internal order as listed)

> Line numbers are from HEAD f6535f2. Phase 1 (SEC-125/126/127/128/129/133) and Phase 2 (ARC-090, ARC-103, ARC-089) land first and move lines in `src/mux/{tree,dispatch,server,pane,command}.rs` and `src/pty_session.rs`, so re-locate every mux symbol with parsight (`find_symbol` / `get_source_window`, `repository_id: "par-term-emu-core-rust"`) before editing.
>
> **CHANGELOG rule:** code fixes add bullets only under `## [Unreleased]` (currently empty at `CHANGELOG.md:8`). Create `### Added` / `### Changed` / `### Fixed` subsections as needed, and merge with bullets that other 3b/3c/3d agents have already added. The one exception in this section is the ARC-100 factual correction to the 0.56.0 entry at `CHANGELOG.md:57`.
>
> **Windows:** mux/PTY changes are Windows-sensitive. After merge, the orchestrator runs the CLAUDE.md Windows VM playbook from the **main checkout**. Worktree sessions cannot run `prlctl exec`.
>
> **Decision-gated items (D1, D2, D4, D5):** implement only the non-gated half, and report the gated half back to the orchestrator in your final message.

---

### [ARC-100] RIS still reverts host-set configuration outside a hand-maintained allowlist
- **Card**: `01a0ef5adf727a30a4fcc866ddcbf157`
- **Files**:
  - `src/terminal/mod.rs`:
    - `reset()` `:3164-3264`, with its doc comment `:3150-3163`.
    - Terminal fields `:1187-1201`: `window_position_x/y`, `window_iconified`, `conformance_level`, `warning_bell_volume`, `margin_bell_volume`.
    - `modes.bold_brightening`: field `:793`, default `true` `:823`, getter/setter `:1918-1924`.
    - `MouseHistoryState.max_mouse_history` `:322` (default 100 `:330`).
    - `InlineImageState.max_inline_images` `:345`.
    - `CommandHistoryState.max_command_history/max_cwd_history` `:872-874` (defaults 100/50 `:880-881`).
    - Constructor defaults `:1328-1334`.
  - `src/terminal/file_transfer.rs`: `FileTransferManager.max_transfer_size` `:83`, `DEFAULT_MAX_TRANSFER_SIZE` `:87`, manager setter `:277`, Terminal setter `:363`.
  - `src/mouse.rs:153` (`set_max_mouse_history`), `src/terminal/image.rs:137` (`set_max_inline_images`), `src/terminal/shell_integration.rs:264,274`.
  - Escape writers of the baselines:
    - DECSCL `src/terminal/sequences/csi/report.rs:79-92`.
    - DECSWBV `src/terminal/sequences/csi/window.rs:212-213`.
    - DECSMBV `src/terminal/sequences/csi/cursor.rs:197-213`.
  - Python setters that currently feed escapes instead of calling Rust: `src/python_bindings/terminal/mod.rs:250` (`set_conformance_level`), `:279` (`set_warning_bell_volume`), `:306` (`set_margin_bell_volume`).
  - `src/terminal/replay_snapshot.rs:267` restores `bold_brightening` from a snapshot.
  - `src/screenshot/mod.rs:81` reads it.
  - Test `src/terminal/tests/terminal_tests.rs:4140` (`ris_preserves_host_config`).
  - New `tests/test_ris_host_config.py`.
  - `CHANGELOG.md:57`, `docs/API_REFERENCE.md` (the `set_conformance_level` / bell-volume entries; find with `grep -n "set_conformance_level\|bell_volume" docs/API_REFERENCE.md`).
- **Setter classification.** This covers every `pub fn set_*` on `Terminal` (src/terminal/ plus `src/mouse.rs`) and every Python `Terminal.set_*`: 63 names in `_native.pyi:2135-2200`. "Carried" means `reset()` already copies or swaps the setting back. "LOST" means it reverts today.

  | Rust setter (file:line) | Python name | Today on RIS | Class |
  |---|---|---|---|
  | `set_max_transfer_size` file_transfer.rs:363 | same | **LOST** (50 MiB) | HostConfig |
  | `set_bold_brightening` mod.rs:1923 | same | **LOST** | HostConfig |
  | `set_window_position` mod.rs:1508 | same | **LOST** | HostConfig |
  | `set_window_iconified` mod.rs:1525 | same | **LOST** | HostConfig |
  | `set_max_mouse_history` mouse.rs:153 | same | **LOST** | HostConfig |
  | `set_max_inline_images` image.rs:137 | same | **LOST** | HostConfig |
  | `set_max_command_history` shell_integration.rs:264 | same | **LOST** | HostConfig |
  | `set_max_cwd_history` shell_integration.rs:274 | same | **LOST** | HostConfig |
  | *(none: Python feeds DECSCL)* | `set_conformance_level` | **LOST** | configured baseline (DECSCL) |
  | *(none: Python feeds `CSI n SP t`)* | `set_warning_bell_volume` | **LOST** | configured baseline (DECSWBV) |
  | *(none: Python feeds `CSI n SP u`)* | `set_margin_bell_volume` | **LOST** | configured baseline (DECSMBV) |
  | `set_accept_osc7` :2346, `set_disable_insecure_sequences` :2359, `set_max_osc_data_length` :2375 | same | carried :3175-3178 | HostConfig |
  | `set_allow_file_media` :1489 (+ `pub(crate) set_retain_kitty_temp_files` :1475) | `set_allow_file_media` | carried :3181-3182 | HostConfig |
  | `set_sixel_limits` :2190, `set_max_sixel_graphics` :2221, `set_cell_dimensions` :2207, `set_pixel_size` :1457 | `set_sixel_limits`, `set_sixel_graphics_limit` | carried :3185-3188, :3244-3245 | HostConfig |
  | `set_answerback_string` :2396 | same | carried :3191 | HostConfig |
  | `set_allow_clipboard_read` :2324; clipboard.rs `set_max_clipboard_sync_events` :283, `set_max_clipboard_event_bytes` :293, `set_remote_session_id` :298, `set_max_clipboard_sync_history` :303 | same | carried :3194-3199 | HostConfig |
  | colors.rs:46-208: `set_default_fg/bg`, `set_cursor_color`, `set_link_color`, `set_bold_color`, `set_cursor_guide_color`, `set_badge_color`, `set_match_color`, `set_selection_bg/fg_color`, `set_use_bold_color`, `set_use_underline_color`, `set_use_cursor_guide`, `set_use_selected_text_color`, `set_smart_cursor_color`, `set_faint_text_alpha`, `set_ansi_palette_color` (via `configured_palette`) | Python exposes 15 of these (no `use_cursor_guide`/`use_selected_text_color`/`smart_cursor_color`) | carried (theme swap :3204-3208) | HostConfig |
  | `set_width_config` :2417, `set_ambiguous_width` :2428, `set_unicode_version` :2439, `set_normalization_form` :2505 | same | carried (unicode swap :3210) | HostConfig |
  | `set_event_subscription` :3521, `set_notification_config` notification.rs:201, `set_max_notifications` :2549, `set_badge_format` :1654, `set_tmux_control_mode` :2740, `set_tmux_auto_detect` :2750, `set_profiling_enabled` metrics.rs:211 | same (no Python `set_profiling_enabled`; it has `enable/disable_profiling`) | carried | HostConfig |
  | `TriggerEngine::set_trigger_enabled` trigger.rs:155 | `set_trigger_enabled` | carried (registry swap :3220-3223) | HostConfig |
  | `set_tab_stop` :2471 | same | carried (xterm keeps tab stops, :3168/:3259) | survives by xterm rule; probe it |
  | `set_cursor_style` :1421 | same | reset | VT state (DECSCUSR) |
  | `set_title` :1634 | same | reset | VT state (OSC 0/2) |
  | `set_mouse_mode` :1857 (Rust only), `set_mouse_encoding` :1867, `set_focus_tracking` :1877, `set_bracketed_paste` :1907 | last three | reset | VT state (DECSET modes) |
  | `set_keyboard_flags` :2294, `set_modify_other_keys_mode` :2305 | same | reset | VT state (kitty flags / XTMODKEYS) |
  | `set_progress` :2601, `set_named_progress_bar` :2627 | same | reset | VT state (OSC 9;4 / OSC 934 content) |
  | `set_clipboard` clipboard.rs:194, `set_clipboard_with_slot` :155 | same | reset | VT state (OSC 52 content, not policy) |
  | `set_selection` screen.rs:407 | same | reset | VT/UI state over cleared screen content |
  | `set_user_var` :1713 (Rust only) | `set_badge_session_variable` (common.rs:1015) | reset | VT state (session variables, shared with OSC 1337 SetUserVar) |

  These setters belong to other types, so they are out of the `Terminal` harness:
  - `TriggerRegistry::set_enabled/set_max_matches` (trigger.rs:547,684)
  - `MacroEngine::set_macro_speed` (terminal/macros.rs:68; survives via the macros swap)
  - `MacroPlayback::set_speed` (src/macros.rs:368)
  - `SnapshotManager::set_*` (snapshot_manager.rs:149-165)
  - `FileTransferManager::set_max_transfer_size` (file_transfer.rs:277)
- **Steps**:
  1. Add a `HostConfig` sub-struct in `src/terminal/mod.rs`, after `CommandHistoryState` (~:885). Its doc: "Embedder configuration RIS must not touch (ARC-100)". Give it a `Default` that reproduces today's defaults exactly:
     - `bold_brightening: bool` (true)
     - `max_transfer_size: usize` (`DEFAULT_MAX_TRANSFER_SIZE`; make that const `pub(crate)`)
     - `window_position: (i32, i32)` ((0,0))
     - `window_iconified: bool` (false)
     - `max_mouse_history: usize` (100)
     - `max_inline_images: usize` (the current `InlineImageState` default; read it from its `Default` impl)
     - `max_command_history: usize` (100)
     - `max_cwd_history: usize` (50)
     - `configured_conformance_level: ConformanceLevel` (`ConformanceLevel::default()`)
     - `configured_warning_bell_volume: u8` (4)
     - `configured_margin_bell_volume: u8` (4)
  2. Add a `pub(crate) host: HostConfig` field to `Terminal`, and initialize it in the constructor (~:1328).
     - Delete `window_position_x/y`, `window_iconified`, `modes.bold_brightening`, `MouseHistoryState.max_mouse_history`, `InlineImageState.max_inline_images` and `CommandHistoryState.max_command_history/max_cwd_history`. Repoint every read to `self.host.*`.
     - The live `conformance_level`, `warning_bell_volume` and `margin_bell_volume` fields stay on `Terminal`, because programs change them.
     - Enumerate the read sites with `grep -rn '\.window_iconified\b\|window_position_[xy]\|\.bold_brightening\b\|max_mouse_history\|max_inline_images\|max_command_history\|max_cwd_history' src/`. Fields are not parsight call-graph symbols, so check with `find_symbol window_iconified` (kind field), and `get_impact bold_brightening` for the getter's callers.
     - At HEAD the counts are: window_iconified 18, max_mouse_history 10, max_inline_images 7, window_position 8, bold_brightening 5, the history caps 4 each.
  3. Repoint the setters to `self.host`: `set_bold_brightening`, `set_window_position`, `set_window_iconified`, `set_max_mouse_history`, `set_max_inline_images`, `set_max_command_history` and `set_max_cwd_history`. Keep each one's existing trim-on-lower behavior.
     - `Terminal::set_max_transfer_size` (file_transfer.rs:363) writes `self.host.max_transfer_size` **and** calls the manager's setter. The manager keeps its own field because `FileTransferManager` is a public type whose `append_data` (`:159`) enforces it.
  4. Add three Rust setters next to the getters (mod.rs:1426-1436). Each writes both the configured baseline and the live value:
     - `pub fn set_conformance_level(&mut self, level: ConformanceLevel)`
     - `pub fn set_warning_bell_volume(&mut self, v: u8)` (clamp to 8)
     - `pub fn set_margin_bell_volume(&mut self, v: u8)` (clamp to 8)

     DECSCL/DECSWBV/DECSMBV keep writing only the live field.
  5. Python bindings (`terminal/mod.rs:250,279,306`): keep the argument validation. Replace the `self.inner.process(format!(...))` escape feeding with the new Rust setters. `set_conformance_level` maps `level` through `ConformanceLevel::from_decscl_param(level)` (conformance_level.rs:84). `c1_mode` stays in the signature; the DECSCL handler never read it. Rewrite the three docstrings: drop the "Sends: CSI …" lines and say "Sets the configured level; a program's DECSCL changes the live level until the next RIS (`ESC c`), which restores this value". Add an Example section per CLAUDE.md.
  6. In `reset()`:
     - Add `std::mem::swap(&mut fresh.host, &mut self.host);`, then call a new private `fresh.apply_host_config()` that does three things:
       - `self.graphics.file_transfer_manager.set_max_transfer_size(self.host.max_transfer_size)`. This is the one cap enforced by a public sub-type; document that any future cap of that kind goes here.
       - Reset the live `conformance_level` / `warning_bell_volume` / `margin_bell_volume` from the `configured_*` fields.
       - Nothing else.
     - Leave the existing swap list intact. Moving those already-carried fields into `HostConfig` is the ARC-102 follow-on, and the new harness now guards them.
     - **Do not touch the `max_gen` / `raise_generation` block at `:3252-3263`.** ARC-092 rewrites it next.
     - Rewrite the doc comment at `:3150-3163` to name `HostConfig` and the configured baselines.
  7. Rust tests in `terminal_tests.rs`:
     - Extend `ris_preserves_host_config` (:4140) with every LOST row above: `set_max_transfer_size(1234)`, `set_bold_brightening(false)`, `set_window_position(5,6)`, `set_window_iconified(true)`, `set_max_mouse_history(3)`, `set_max_inline_images(2)` (assert `term.host.max_inline_images == 2`; there is no getter), and `set_max_command_history(2)` / `set_max_cwd_history(1)` (assert the retained counts after feeding 6 commands / 4 cwd changes post-RIS).
     - New `ris_restores_configured_conformance_level`: `set_conformance_level(VT220)`, then `process(b"\x1b[61\"p")`, and assert the live level is VT100. After `process(b"\x1bc")`, assert VT220. This proves a program's DECSCL does **not** survive RIS but the host's baseline does.
     - New `ris_restores_configured_bell_volumes`: `set_warning_bell_volume(1)`, `set_margin_bell_volume(2)`, then `process(b"\x1b[7 t\x1b[6 u")`. Assert 7/6, then after RIS assert 1/2. Verify the `CSI 6 SP u` routing first: the DECSMBV arm at cursor.rs:197 does not check intermediates.
  8. New Python test `tests/test_ris_host_config.py`:
     ```python
     """RIS (ESC c) keeps embedder configuration (audit ARC-100).

     Every public set_* on Terminal is classified: a probe proving it survives
     RIS, a Rust test that covers it (no Python getter), or a VT-state entry
     saying why RIS resets it. A new setter fails the classification test
     until someone decides.
     """

     import pytest
     from par_term_emu_core_rust import Terminal

     RIS = b"\x1bc"


     def _fill_commands(t: Terminal) -> int:
         for i in range(6):
             t.start_command_execution(f"c{i}")
             t.end_command_execution(0)
         return len(t.get_command_history())


     def _fill_cwds(t: Terminal) -> int:
         for i in range(4):
             t.record_cwd_change(f"/tmp/d{i}")
         return len(t.get_cwd_changes())


     # name -> (apply, read); read must differ from a fresh Terminal after apply
     PROBES = {
         "set_max_transfer_size": (
             lambda t: t.set_max_transfer_size(1234),
             lambda t: t.get_max_transfer_size(),
         ),
         "set_bold_brightening": (
             lambda t: t.set_bold_brightening(False),
             lambda t: t.bold_brightening(),
         ),
         "set_conformance_level": (
             lambda t: t.set_conformance_level(2),
             lambda t: t.conformance_level(),
         ),
         "set_warning_bell_volume": (
             lambda t: t.set_warning_bell_volume(1),
             lambda t: t.warning_bell_volume(),
         ),
         "set_margin_bell_volume": (
             lambda t: t.set_margin_bell_volume(1),
             lambda t: t.margin_bell_volume(),
         ),
         "set_max_mouse_history": (
             lambda t: t.set_max_mouse_history(3),
             lambda t: t.get_max_mouse_history(),
         ),
         "set_window_position": (
             lambda t: t.set_window_position(5, 6),
             lambda t: t.window_position(),
         ),
         "set_window_iconified": (
             lambda t: t.set_window_iconified(True),
             lambda t: t.window_iconified(),
         ),
         "set_max_command_history": (lambda t: t.set_max_command_history(2), _fill_commands),
         "set_max_cwd_history": (lambda t: t.set_max_cwd_history(1), _fill_cwds),
         # ...plus one entry per already-carried HostConfig row that has a
         # getter (accept_osc7, disable_insecure_sequences, max_osc_data_length,
         # get_allow_file_media, get_sixel_limits, get_sixel_graphics_limit,
         # answerback_string, allow_clipboard_read, get_max_clipboard_event_bytes,
         # get_max_clipboard_sync_events, remote_session_id, the 15 theme colors
         # and flags, faint_text_alpha, get_ansi_color, width_config (ambiguous
         # width, unicode version), normalization_form, get_max_notifications,
         # is_tmux_control_mode, is_tmux_auto_detect, get_tab_stops, badge_format
         # via evaluate_badge or its getter, get_notification_config).
     }
     # no Python getter; the named Rust test asserts it
     RUST_COVERED = {
         "set_max_inline_images": "ris_preserves_host_config",
         "set_max_clipboard_sync_history": "ris_preserves_host_config",
         "set_event_subscription": "ris_preserves_host_config",
         "set_trigger_enabled": "ris_preserves_host_config (registry swap)",
     }
     VT_STATE = {
         "set_title": "OSC 0/2 title",
         "set_cursor_style": "DECSCUSR",
         "set_bracketed_paste": "DECSET 2004",
         "set_focus_tracking": "DECSET 1004",
         "set_mouse_encoding": "DECSET 1005/1006/1015",
         "set_keyboard_flags": "kitty CSI = u",
         "set_modify_other_keys_mode": "XTMODKEYS",
         "set_progress": "OSC 9;4 content",
         "set_named_progress_bar": "OSC 934 content",
         "set_clipboard": "OSC 52 content",
         "set_clipboard_with_slot": "OSC 52 content",
         "set_selection": "selection over cleared screen",
         "set_badge_session_variable": "session variables (OSC 1337 SetUserVar)",
     }


     def test_every_setter_is_classified():
         setters = {n for n in dir(Terminal) if n.startswith("set_")}
         classified = PROBES.keys() | RUST_COVERED.keys() | VT_STATE.keys()
         assert setters - classified == set(), "classify the new setter(s)"
         assert classified - setters == set(), "stale classification entries"
         assert not (PROBES.keys() & VT_STATE.keys())


     @pytest.mark.parametrize("name", sorted(PROBES))
     def test_host_setting_survives_ris(name):
         apply, read = PROBES[name]
         baseline = read(Terminal(80, 24))
         t = Terminal(80, 24)
         apply(t)
         expected = read(t)
         assert expected != baseline, f"{name}: probe changes nothing"
         t.process(RIS)
         assert read(t) == expected


     def test_program_changes_reset_to_configured_baseline():
         t = Terminal(80, 24)
         t.set_warning_bell_volume(2)
         t.process(b"\x1b[7 t")
         assert t.warning_bell_volume() == 7
         t.process(RIS)
         assert t.warning_bell_volume() == 2
     ```
     Fill in the carried-row probes, one line each. Check `conformance_level()`'s return values before writing that probe: it returns `ConformanceLevel::level()`. Verify with `find_symbol level` scoped to `src/conformance_level.rs`.
  9. CHANGELOG:
     - Add an `[Unreleased]` → `### Fixed` bullet: "**RIS keeps every embedder setting** (audit ARC-100, `src/terminal/mod.rs`): the OSC 1337 transfer cap, bold brightening, window position/iconified state and the mouse/inline-image/command/cwd history caps now live in a `HostConfig` that `reset()` carries whole; the conformance level and bell volumes restore to the host-configured baseline (a program's DECSCL/DECSWBV/DECSMBV still resets). Python `set_conformance_level`/`set_warning_bell_volume`/`set_margin_bell_volume` now set that baseline instead of feeding the escape. A classification test fails on any new `set_*` until it is declared host config or VT state."
     - Correct `CHANGELOG.md:57`, the one allowed edit outside `[Unreleased]`. In the 0.56.0 "RIS resets the terminal, not the embedder's configuration" bullet, after the sentence ending "…pixel dimensions, and profiling." insert: "(Incomplete in 0.56.0: the OSC 1337 transfer cap, bold brightening, conformance level, bell volumes, window position/iconified state and the mouse, inline-image, command and cwd history caps still reverted — fixed in a later release, ARC-100.)"
  10. `docs/API_REFERENCE.md`: update the three setter entries (baseline semantics).
- **Method**:
  - One swap of an owned struct replaces a hand-kept list, so a field added to `HostConfig` is carried by construction. The Python classification test catches the other half of the risk, a new setter whose field sits outside `HostConfig`.
  - Configured-baseline settings must not copy the live value. Doing so would let a remote program's DECSCL survive RIS, which is the same reasoning as `configured_palette`.
  - **Pitfalls:**
    - (a) `TerminalSnapshot.bold_brightening` is part of the serde mux on-disk format. Keep the snapshot field and map it to `host.bold_brightening` in capture/restore (replay_snapshot.rs:267 and the capture site). Do not rename it.
    - (b) `set_pixel_size` also derives `cell_dimensions`. Leave it as is.
    - (c) **Observed, not in AUDIT: report it, do not fix here.** DECSCUSR (`CSI Ps SP q`, cursor.rs:172-192) also writes `warning_bell_volume = n.min(8)`. Any cursor-shape change moves the bell volume. DECSWBV is `CSI Ps SP t`.
    - (d) **Observed: report it.** OSC 10/11/12 write `theme.default_fg/bg/cursor_color` (osc/color.rs:100-119), and the theme swap carries that program-set drift across RIS. It is the same configured-baseline pattern as the palette.
  - Parsight queries:
    - `get_symbol_context reset`: callers include `PyTerminal.reset` and ESC c.
    - `get_impact set_max_transfer_size`
    - `get_impact bold_brightening`
    - `find_symbol HostConfig` must return nothing before you start.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize ris_`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize` (full lib: replay snapshot, screenshot, file transfer)
  - `cargo test --lib --no-default-features --features python-test` (the bindings compile only with a python feature)
  - `make dev` then `uv run pytest tests/test_ris_host_config.py tests/test_terminal.py tests/test_terminal_bindings.py tests/test_file_transfer.py -v`
  - Docstrings changed, so run `make dev-streaming && make stubs` and commit the regenerated `_native.pyi`.
  - `make checkall`

### [ARC-092] Damage generations rely on a manual `sync_damage_generations()` at each screen-switch site; snapshot restore misses it
- **Card**: `01a0ef5ae39c77808670ab2defbed11f`
- **Files**:
  - `src/terminal/replay_snapshot.rs:231-279` (`restore_from_snapshot`; the final mark is at `:275-278`) and `:287-311` (`restore_for_new_process`).
  - `src/grid/mod.rs:43-51` (`row_gen`, `gen`), `:78-104` (`mark_row_damage`, `generation`, `raise_generation`), `:303-320` (`Grid::restore_from_snapshot`).
  - `src/terminal/mod.rs`:
    - `resize` sync `:1545`
    - `enter_alt_screen` `:1754-1775`
    - `exit_alt_screen` `:1790-1808`
    - `reset` generation block `:3252-3263`
    - `sync_damage_generations` `:3380-3384`
    - `dirty_rows_since` `:3404`
  - Tests: `src/terminal/tests/replay_snapshot_tests.rs`, `src/terminal/tests/damage_props.rs`.
- **Decision (approach)**: **Make restore raise, through a single visible-screen invalidation funnel.** Do not build a shared Terminal-owned clock. Reasons:
  - `Grid` is a public type (`pub mod grid`) with independent users, for example the render view `Grid::new` at mod.rs:2699.
  - `mark_row_damage` runs on the hottest path. `get_mut`/`set`/`row_mut` stamp on every cell write (grid/mod.rs:159-191).
  - A shared clock means either an `Arc<AtomicU64>` on that path or a new clock parameter on every public `Grid` mutator, which breaks Rust embedders.
  - The per-grid design is already pinned by the ENH-025 property test. The actual defect is Terminal-level: `restore_from_snapshot` flips `alt_screen_active` without syncing the two grids' counters.
- **Steps**:
  1. In `src/terminal/mod.rs`, add `fn invalidate_visible_screen(&mut self)`. It calls `sync_damage_generations()` and then `mark_rows_dirty(0, active_grid().rows()-1)`. Its doc: "the only way to report a wholesale change of what the renderer sees; call it after the active grid or its content was replaced (ARC-092)".
  2. Route every wholesale-change site through it. Make `sync_damage_generations` private to the funnel, so the funnel is its only caller.
     - `resize`: `:1545` syncs before the regrid. Keep a sync there, because the ENH-025 comment explains that the marks must outrun earlier captures, and replace the later whole-screen mark with the funnel. Read `resize` whole first.
     - `enter_alt_screen` `:1773-1775` and `exit_alt_screen` `:1806-1808`: replace the sync-plus-mark pair.
     - `reset`: keep `max_gen` carry-over (the fresh terminal starts at 0), `*self = fresh`, then `raise_generation(max_gen)` on both grids, then call the funnel instead of `mark_rows_dirty` at `:3263`.
     - `restore_from_snapshot`: replace `:275-278` with the funnel. This is the missing sync.
     - `restore_for_new_process`: replace `:305-310` with the funnel.
  3. Leave `Grid::restore_from_snapshot` as it is. It already keeps `self.gen` and stamps above it. Add a comment that it never lowers `gen`.
  4. Regression tests in `replay_snapshot_tests.rs`:
     - `restore_onto_live_terminal_reports_every_row_dirty`:
       1. Source terminal `s = Terminal::new(80, 24)`. `s.process(b"\x1b[?1049hALT")`, then `snap = s.capture_snapshot()`. The alt screen is active and the alt generation is small.
       2. Live `l = Terminal::new(80, 24)`. Feed 300 lines (`"line {i}\r\n"`) so the primary generation is large, then `g = l.damage_generation()`.
       3. `l.restore_from_snapshot(snap)`.
       4. Assert `l.dirty_rows_since(g).count() == 24`. This is the audit probe, which returned `[]` at HEAD.
     - `restore_for_new_process_reports_every_row_dirty`: same shape, calling `restore_for_new_process`.
     - Run both against HEAD first. The first must fail (red), which proves the probe.
- **Method**:
  - Every path that swaps or replaces what the renderer sees now goes through one function, so a future path cannot forget the sync. `grep -n "sync_damage_generations" src/` must show only the definition and the funnel.
  - Consumers keep one remembered number (`damage_generation()` = max of both grids). The funnel keeps both grids' stamps above any captured value.
  - **Pitfall:** the FFI `terminal_dirty_ranges_since` and the Python `dirty_rows_since` both read `active_grid()`. Check both still pass `ffi_round_trip_matches_core_state`.
  - Parsight: `get_symbol_context sync_damage_generations` (expect 4 callers at HEAD), `get_symbol_context restore_from_snapshot`, `get_impact mark_rows_dirty`.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize replay_snapshot`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize damage`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize ffi`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` (mux restores snapshots)
  - `make dev` then `uv run pytest tests/test_damage_generation.py -v`
  - `make checkall`

### [ARC-091] Trigger scanning is lossy: a written row that scrolls out of the visible grid before the next scan is never matched
- **Card**: `01a0ef5ae71979b3a0dda3406a5d13e1`
- **Files**:
  - `src/terminal/mod.rs:3322-3368` (`mark_row_written`, `shift_pending_trigger_rows`, `flush_pending_trigger_rows`).
  - `src/terminal/trigger.rs:178-230` (`process_trigger_scans`).
  - Terminal-level scroll sites, with the grid call first and the shift after it:

    | Site | Grid call | Pending-row shift |
    |---|---|---|
    | `src/terminal/write.rs` | `:99` | `:104` |
    | `src/terminal/write.rs` | `:187` | `:190` |
    | `src/terminal/write.rs` (wide-char wrap) | **`:222`** | **none** |
    | `src/terminal/write.rs` | `:600` | `:603` |
    | `src/terminal/graphics.rs` | `:583` | `:594` |
    | `src/terminal/sequences/csi/scroll.rs` (SU) | `:25` | `:26` |
    | `src/terminal/sequences/csi/scroll.rs` (SD) | `:40` | `:41` |
    | `src/terminal/sequences/esc.rs` (RI) | `:47` | `:50` |
    | `src/terminal/sequences/esc.rs` (IND) | `:71` | `:74` |
    | `src/terminal/sequences/esc.rs` (NEL) | `:99` | `:102` |
    | `src/terminal/sequences/csi/edit.rs` (IL) | `:26` | `:27` |
    | `src/terminal/sequences/csi/edit.rs` (DL) | `:42` | `:43` |

  - `src/pty_session.rs:905`.
  - Tests: `src/terminal/tests/terminal_tests.rs:4288-4345` (ARC-064 tests).
- **Decision (approach)**: **Scan departing rows at scroll time, at a Terminal-level choke point.** Do not use absolute-line keys. Reasons:
  - The Grid scroll-into-history choke points are `Grid::push_rows_to_scrollback` (grid/scroll.rs:7) and `Grid::absorb_rows_into_scrollback` (:78). They are reached from `scroll_region_up` (:151, which calls `scroll_up` :48 for full-screen regions) (parsight `get_symbol_context scroll_region_up`: callees `push_rows_to_scrollback`, `scroll_up`).
  - Those Grid functions see only rows entering *history*. A row can also leave the screen in ways that never reach history:
    - a region scroll with `top > 0`, where rows are discarded
    - the alt screen, which has `max_scrollback = 0`
    - DL
    - SD/RI/IL pushing rows off the bottom
  - Absolute keys (`total_lines_scrolled + row`) can only re-find rows that reached scrollback, and `total_lines_scrolled` counts only scrollback pushes. They would still lose all four cases above.
  - `Grid` has no access to the trigger registry. So the choke point is a Terminal wrapper around every grid scroll, which scans pending rows that are about to leave the region *before* the grid moves them.
- **Steps**:
  1. In `trigger.rs`, split `process_trigger_scans` (:178). Its body after the `drain()` moves into `pub(crate) fn scan_rows(term: &mut Terminal, rows: Vec<usize>)`. `process_trigger_scans` becomes the `has_active_triggers` check, then `drain`, then `scan_rows`. The public signature does not change.
  2. In `mod.rs`, add `pub(crate) fn scan_departing_trigger_rows(&mut self, down: bool, n: usize, top: usize, bottom: usize)`.
     - If `pending_trigger_rows` is empty, return.
     - Departing rows are `pending ∩ [top, min(top+n-1, bottom)]` when `down == false` (content moving up leaves at the top), or `pending ∩ [max(bottom+1-n, top), bottom]` when `down == true`.
     - Remove them from the set and call `TriggerEngine::scan_rows(self, departing)`.
  3. Add Terminal wrappers that bundle the four operations each site repeats today:
     - `scroll_region_up_tracked(n, top, bottom)`: `scan_departing(false, …)`, then `active_grid_mut().scroll_region_up`, then `adjust_graphics_for_scroll_up`, then `shift_pending_trigger_rows(false, …)`.
     - `scroll_region_down_tracked` (the `down` mirror).
     - `delete_lines_tracked(n, row, bottom)`, which scans `[row, row+n)` first.
     - `insert_lines_tracked`, which scans rows pushed off the bottom.

     Keep `mark_rows_dirty` calls where the site had one (graphics.rs:592). Replace all 12 sites in the table above with the wrappers. The graphics adjust is not present at every site (IL/DL have none), so give the wrappers a `graphics: bool` parameter or keep that call at the site. Read each site before editing.
  4. **The wide-char wrap scroll at write.rs:218-224 calls `scroll_region_up` with no `shift_pending_trigger_rows`.** This is a second, unreported defect: pending indices desync after a wide-char wrap at the bottom. The wrapper fixes it. Name it in the commit.
  5. Tests in `terminal_tests.rs`, next to the ARC-064 block:
     - `trigger_matches_row_scrolled_out_before_scan` (the audit probe): `Terminal::new(80, 5)`, trigger `ERROR`. Feed `b"ERROR lost\r\n"` plus 10 × `b"line\r\n"` in **one** `process()`, then `process_trigger_scans`. Assert 1 match. At HEAD this gives 0.
     - `trigger_matches_row_discarded_by_region_scroll`: `\x1b[2;4r` (DECSTBM), cursor to row 2, write `ERROR r`, `\x1b[5S` (SU 5), then scan. Assert 1 match.
     - `trigger_matches_row_pushed_off_bottom_by_ri`: write `ERROR b` on the last row, cursor home, `\x1bM` × 3, then scan. Assert 1 match.
     - `wide_char_wrap_scroll_keeps_pending_rows`: write `ERROR w` at the bottom row, then enough text to trigger the wide-char wrap scroll (fill to col 79, then `"世"`), then scan. Assert one match whose `row` is the landing row.
     - Keep `alt_screen_switch_does_not_refire_triggers`, `scroll_does_not_refire_triggers` and `wrapped_line_is_scanned_where_the_scroll_left_it` green. Each row is still scanned exactly once.
- **Method**:
  - The ARC-064 "written text only" rule holds, because only rows already in the pending set are scanned. Early scanning only moves *when* a pending row is scanned, never *whether*.
  - **Pitfalls:**
    - (a) A match's `row` is the visible row at scan time (pre-scroll), which is the existing contract. `MarkLine` bookmarks and highlights for early-scanned rows therefore point at the pre-scroll index. Document this on `TriggerMatch.row`, and do not change `add_bookmark` semantics here.
    - (b) Actions now run inside `process()` rather than after it. Every action only pushes to buffers (`trigger_action_results`, `terminal_events`, notifications), so there is no re-entrancy risk. Confirm by reading `execute_trigger_actions` (trigger.rs:232-370).
    - (c) The cost is zero when no trigger is active, because `mark_row_written` never inserts.
    - (d) Sequence after ARC-100 and ARC-092 on `mod.rs`. ARC-102 later moves these helpers into `TriggerState`.
  - Parsight:
    - `get_symbol_context shift_pending_trigger_rows` should list the 11 callers above.
    - `get_symbol_context scroll_region_up` / `scroll_region_down` / `delete_lines` / `insert_lines` enumerate every grid-scroll caller, so none is missed.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize trigger`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize scroll`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize damage` (the damage proptest exercises every scroll path)
  - `cargo test --test test_triggers --no-default-features --features pyo3/auto-initialize`
  - `make dev` then `uv run pytest tests/test_triggers.py -v`
  - `make checkall`

### [ARC-093] Three named-key tables remain, and the public encoders disagree on option defaults (merges QA-189)
- **Card**: `01a0ef5aead170b3959bbdc608ea8ece`
- **Files**:
  - `src/keyboard.rs`: `KeyEncodeOptions::default` = ESC/ESC at `:82-92`; `encode_key_with` `:492-507`; `TermKeyEvent::char_` / `functional` constructors `:175-195`.
  - `src/mux/command.rs`:
    - `SendKeys` variant `:52-59`
    - `key_to_bytes` `:638-669`
    - `parse_send_keys_payload` `:694-735`
    - `parse_send_keys` `:932-951`
    - send-keys tests `:1630-1760`
  - `src/mux/dispatch.rs:427-440` (`cmd_send_keys`).
  - `src/streaming/mux_factory.rs:1036` (a test constructing `SendKeys { keys }`).
  - `src/macros.rs:192-270` (`KeyParser::parse_key`) and its test `:408-411`.
  - `src/terminal/macros.rs:109-133` (`tick_macro`).
  - `src/bin/streaming_server/main.rs:403-410`.
  - `src/python_bindings/terminal/input_api.rs:28-31,45`.
  - `tests/test_keyboard_encoding.py:1-6,73-74`.
  - `docs/API_REFERENCE.md:250,258`.
  - `docs/MUX.md:209` (send-keys key names).
  - `CHANGELOG.md`.
- **Steps**:
  1. **Python default.** In `input_api.rs:45`, change the signature to `left_option = 2, right_option = 2` (`option_modes::ESC`).
     - Docstring `:28-31`: "Default 2 (esc) — the same default as C `terminal_encode_key` and Rust `KeyEncodeOptions::default()`."
     - `tests/test_keyboard_encoding.py:73-74`: change to `# Defaults are ESC on both sides (same as C/Rust).` and `assert term.encode_key(CHAR, ALT, ord("a")) == b"\x1ba"`.
     - Add `test_python_default_matches_c_default`: `encode_key(CHAR, ALT, ord("f"))` equals `b"\x1bf"`.
     - `docs/API_REFERENCE.md:250`: signature `left_option: int = 2, right_option: int = 2`.
     - Regenerate the stub (`_native.pyi:1827-1834` shows `= 0`).
  2. **send-keys payload becomes key parts, encoded at dispatch.** In `command.rs`:
     - Add `#[derive(Debug, Clone, PartialEq, Eq)] pub enum SendKeysPart { Bytes(Vec<u8>), Key(crate::keyboard::TermKeyEvent) }` and `pub struct SendKeysPayload(pub Vec<SendKeysPart>)` with `pub fn encode(&self, term: &Terminal) -> Vec<u8>`. `Key` parts call `keyboard::encode_key_with(ev, term, &KeyEncodeOptions::default())`; `Bytes` parts are appended verbatim.
     - Change the variant to `SendKeys { pane, keys: SendKeysPayload }`.
     - `-l` and `-H` produce one `Bytes` part. `0xNN` tokens and literal text produce `Bytes`.
     - `key_to_bytes` becomes `fn key_part(name) -> Option<SendKeysPart>`, using this table:

     | Token(s) | Part | Bytes, legacy, DECCKM off | DECCKM on / kitty flags |
     |---|---|---|---|
     | `Enter` | `Bytes([0x0d])` | `\r` | unchanged (raw byte) |
     | `Escape`, `Esc` | `Bytes([0x1b])` | `\x1b` | unchanged |
     | `BSpace` | `Bytes([0x7f])` | `\x7f` | unchanged |
     | `Space` | `Bytes([0x20])` | ` ` | unchanged |
     | `Tab` (new) | `Bytes([0x09])` | `\t` | unchanged |
     | `C-Space` | `Bytes([0x00])` | NUL | unchanged |
     | `C-a` … `C-z` | `Bytes([0x01..=0x1a])` | control byte | unchanged |
     | `Up`/`Down`/`Right`/`Left` | `Key(functional(Up/Down/Right/Left, 0))` | `ESC [ A/B/C/D` | DECCKM: `ESC O A/B/C/D` |
     | `Home`, `End` (new) | `Key(functional(Home/End, 0))` | `ESC [ H` / `ESC [ F` | DECCKM: `ESC O H` / `ESC O F` |
     | `PageUp`, `PgUp`, `PPage` (new) | `Key(functional(PageUp, 0))` | `ESC [ 5 ~` | same |
     | `PageDown`, `PgDn`, `NPage` (new) | `Key(functional(PageDown, 0))` | `ESC [ 6 ~` | same |
     | `IC`, `Insert` (new) | `Key(functional(Insert, 0))` | `ESC [ 2 ~` | same |
     | `DC`, `Delete` (new) | `Key(functional(Delete, 0))` | `ESC [ 3 ~` | same |
     | `F1`…`F4` (new) | `Key(functional(F1..F4, 0))` | `ESC O P/Q/R/S` | kitty: encoder's CSI-u form |
     | `F5`…`F12` (new) | `Key(functional(F5..F12, 0))` | `ESC [ 15~ 17~ 18~ 19~ 20~ 21~ 23~ 24~` | kitty: CSI-u |
     | `BTab` (new) | `Key(functional(Tab, SHIFT))` | `ESC [ Z` | kitty: `CSI 9;2u` |

     The byte-class names stay raw bytes **on purpose**. par-term's `escape_keys_for_tmux` (`../par-term/par-term-tmux/src/session.rs:430`) forwards *already-encoded* keystroke bytes as `C-x` / `Escape` / `BSpace` / `Space` tokens. Re-encoding them against the pane's modes would double-encode par-term input: `C-c` under modifyOtherKeys 2 becomes `CSI 27;5;99~`, and the `Escape` of an arrow sequence becomes `CSI 27u` under kitty flags. Only names par-term never emits (navigation and function keys) resolve through the encoder. tmux key names are from `~/Repos/tmux/key-string.c:47-60`.
  3. `dispatch.rs cmd_send_keys` (:427): after resolving the pane, `let bytes = { let term = target.terminal(); let t = term.read(); keys.encode(&t) };`, then `target.write(&bytes)`. Drop the terminal read guard before `write`. Reading the terminal under the tree lock follows the existing pattern at dispatch.rs:716.
  4. Update the send-keys tests (`command.rs:1630-1760`) to compare `keys.encode(&Terminal::new(80, 24))` against the same bytes they assert today. Update `mux_factory.rs:1036` to `keys: SendKeysPayload(vec![SendKeysPart::Bytes(b"a\x1b\"'\n".to_vec())])`. New tests:
     - `send_keys_arrows_honor_application_cursor_mode`: `t.process(b"\x1b[?1h")`, then `Up` encodes as `\x1bOA`.
     - `send_keys_resolves_navigation_and_function_keys`: Home, End, PPage, NPage, IC, DC, F1, F12, BTab against a plain terminal, compared with the table.
     - `send_keys_byte_class_names_ignore_terminal_modes`: under `\x1b[>4;2m` and `\x1b[>1u`, `C-c` is still `0x03` and `Escape` is still `0x1b`.
     - `send_keys_round_trips_an_escape_keys_for_tmux_stream` (:1746) stays green.
  5. **Macros.** In `src/macros.rs`:
     - Add `pub fn encode_key(key: &str, term: &Terminal) -> Vec<u8>`. It parses the same lowercase `ctrl+`/`alt+`/`shift+` grammar into a `TermKeyEvent`:

       | Key names | Event |
       |---|---|
       | `f1`–`f12`, `up`/`down`/`left`/`right`, `home`, `end`, `pageup\|pgup`, `pagedown\|pgdn`, `insert\|ins`, `delete\|del`, `enter\|return`, `tab`, `backspace`, `escape\|esc` | `functional(key, mods)` |
       | `space` | `char_(' ', mods)` |
       | single character | `char_(c, mods)` |

       It encodes with `encode_key_with(…, &KeyEncodeOptions::default())`. Unknown names stay literal bytes.
     - Keep `pub fn parse_key(key)` as `encode_key(key, &Terminal::new(1, 1))`-equivalent behavior against default modes. It is public API, and the macros.rs:408-411 test pins it. Implement it without allocating a Terminal: a private `legacy_default` path calls `encode_key_with` on a static default terminal, or add `keyboard::encode_key_default(ev)`.
     - Macros replay *keystrokes*, so every key goes through the encoder, including Enter/Tab/Backspace/Escape. Under kitty flags, Enter correctly becomes `CSI 13u`, which is what a real keyboard would send through par-term.
     - `tick_macro` (terminal/macros.rs:109): take the event out first (`let event = term.macros.macro_playback.as_mut().and_then(|p| p.next_event());`), then call `KeyParser::encode_key(&key, term)`. This avoids the `&mut` borrow conflict.
     - `streaming_server/main.rs:405`: `let bytes = { let s = pty_session_clone.lock(); let t = s.terminal(); let g = t.read(); KeyParser::encode_key(&key, &g) };`.
     - Test in `macros.rs`: `macro_arrow_honors_application_cursor_mode`.
  6. Docs: `docs/MUX.md:209` send-keys key-name list, adding the new names and the rule "navigation/function keys follow the pane's DECCKM/kitty state; control-byte names are raw bytes". `API_REFERENCE.md:258` keeps the byte-identity note, which is now true.
  7. CHANGELOG `[Unreleased]` → `### Changed`:
     - "**Behavior: Python `Terminal.encode_key` option-key modes default to ESC on both sides** (audit ARC-093/QA-189, `src/python_bindings/terminal/input_api.rs`). `left_option`/`right_option` defaulted to 0 (Normal passthrough), so Alt+f encoded as `b'f'` from Python but `b'\x1bf'` from C `terminal_encode_key` and Rust `KeyEncodeOptions::default()`, contradicting the documented byte identity. The default is now 2 (ESC prefix); pass `left_option=0, right_option=0` for the old passthrough."
     - "**par-mux `send-keys` and macro playback encode keys through the shared encoder** (ARC-093): arrow, Home/End, PageUp/PageDown, Insert/Delete, F1–F12 and `BTab` key names honor the target pane's application-cursor (DECCKM) and kitty keyboard state; control-byte names (`C-x`, `Escape`, `BSpace`, `Space`, `Enter`, `Tab`) stay raw bytes. Rust: `MuxCommand::SendKeys.keys` is now a `SendKeysPayload`."
- **Method**:
  - One encoder (`keyboard::encode_key_with`) now decides every mode-dependent key. The daemon owns the pane's `Terminal`, so it can honor DECCKM the way tmux does (`input-keys.c:654-655`).
  - **Pitfalls:**
    - (a) `MuxCommand` is re-exported (`src/mux/mod.rs:33`), so the field type change is a Rust API change. par-term has no `MuxCommand` use (grep of `../par-term`).
    - (b) The fuzz target `fuzz/fuzz_targets/mux_parse_command.rs` only calls `parse_command`, so it still compiles.
    - (c) Same file as ARC-098 (`keyboard.rs`): land ARC-093 first.
  - Parsight:
    - `get_symbol_context parse_send_keys_payload`
    - `get_symbol_context parse_key` (callers: `tick_macro`, streamer main, test)
    - `find_code "MuxCommand::SendKeys"` to enumerate every constructor and match site.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `cargo test --lib --no-default-features --features rust-only,streaming,mux,serde streaming::mux_factory`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize keyboard`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize macros`
  - `make dev` then `uv run pytest tests/test_keyboard_encoding.py tests/test_macros.py tests/test_macros_extended.py -v`
  - `make dev-streaming && make stubs`
  - `make checkall`

### [ARC-094] Adding a mux command or notification takes edits in 5 places, with opposite exhaustiveness disciplines
- **Card**: `01a0ef5aef9c7830805b1ed4dc62757b`
- **Files**:
  - `src/mux/emit.rs:37-157`: `emit()`; the wildcard is at `:156`, with a stale comment at `:149-155`.
  - Test `src/mux/emit.rs:410-422`.
  - `src/tmux_control.rs:20-205` (`TmuxNotification`, **34** variants).
  - Already exhaustive, and must stay that way: `src/mux/command.rs:315-351` (`mutates()`, 33 arms, no wildcard); `src/mux/dispatch.rs:154-221` (`dispatch_command` match, 33 arms); `src/python_bindings/types/notification.rs:132-849` (34 arms); `tmux_control.rs:209-246` (`notification_type()`).
- **Steps**:
  1. Replace `_ => String::new(),` at `emit.rs:156` with an explicit arm listing the 13 variants the daemon never emits:
     ```rust
     TmuxNotification::UnlinkedWindowAdd { .. }
     | TmuxNotification::UnlinkedWindowRenamed { .. }
     | TmuxNotification::ClientSessionChanged { .. }
     | TmuxNotification::SessionWindowChanged { .. }
     | TmuxNotification::ClientDetached { .. }
     | TmuxNotification::Pause { .. }
     | TmuxNotification::ExtendedOutput { .. }
     | TmuxNotification::Continue
     | TmuxNotification::SubscriptionChanged { .. }
     | TmuxNotification::PasteBufferChanged { .. }
     | TmuxNotification::PasteBufferDeleted { .. }
     | TmuxNotification::Unknown { .. }
     | TmuxNotification::TerminalOutput { .. } => String::new(),
     ```
     Shapes are from `tmux_control.rs:71-204`. Re-read them in case Phase 1 or 2 added a variant.
  2. Rewrite the `:149-155` comment. The claim that "29 variants; Phase 1 emits the 9" is stale: 21 of 34 are emitted. New text: "Parser-only variants the daemon never constructs. Listed, not wildcarded: a new `TmuxNotification` variant fails to compile here until someone decides whether the daemon emits it (ARC-094)."
  3. Keep `unemitted_variants_produce_nothing_rather_than_panicking`, and update its comment ("catch-all arm" → "parser-only arm").
  4. **Do not** build the declarative `(name, parser, mutates, handler)` command table in this pass, and report it as deferred. Reasons:
     - `mutates()`, `dispatch_command` and the Python converter are already exhaustive.
     - `COMMANDS` (`command.rs:763-797`, 33 rows) is a `const` slice keyed by name, and a missing row surfaces as the existing `rejects_an_unknown_command` path.
     - `dispatch.rs` is the top churn hotspot and a QA-202 split target, so restructuring it here conflicts with that split.
  5. Add a guard test in `command.rs` tests: `every_command_table_name_parses_or_errors_on_its_own_grammar`. Iterate `COMMANDS` and assert `parse_command(name)` never returns the "unknown command" error. It may return a usage error. This catches a table row pointing at the wrong parser.
- **Method**: the compiler now enforces "a new notification is either emitted or explicitly parser-only". That is the same discipline the Python converter already has (commit 4dde9e5). Parsight: `get_symbol_context emit` for callers, and `find_symbol TmuxNotification` for the enum span.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `cargo test --lib --no-default-features --features python-test notification` (the Python converter still compiles)
  - `make checkall`

### [ARC-095] Held-dead pane state is push-only; a client that connects later cannot tell a pane has exited
- **Card**: `01a0ef5af3707142b234a2cd694f6c48`
- **Files**:
  - `src/mux/dispatch.rs:700-729` (`cmd_pane_info`): fields are read under the tree lock at `:707-718`; the reply is built at `:721-727`.
  - `src/mux/pane.rs`: `dead: bool` `:147`, `exit_code: Option<i32>` `:150`, `dead()` `:348`, `exit_code()` `:363`.
  - `docs/MUX.md:173` (the pane-info grammar row), `:233` (`%pane-exited`), `:358` (Pane Reaping).
  - Tests: `tests/mux_daemon.rs` and `tests/mux_targets.rs`, both of which already exercise pane-info.
- **Steps**:
  1. In `cmd_pane_info`, add `target.dead()` and `target.exit_code()` to the tuple read under the lock. After the optional `cmd=` token, append `exited=<code>` when the pane is dead with a code, or `exited=?` when it is dead without one (signal death, unreadable). Append nothing for a live pane.
     ```rust
     if dead {
         line.push_str(" exited=");
         match exit_code { Some(c) => line.push_str(&c.to_string()), None => line.push('?') }
     }
     ```
     Tokens are whitespace-free `key=value` (the ARC-060 tail grammar). par-term's parser (`../par-term/par-term-mux/src/resync.rs:204`, `split_whitespace().find_map(strip_prefix("cmd="))`) is order-independent, so older clients are unaffected.
  2. Update the doc comment at `:701-706` and the `docs/MUX.md:173` row to `%N @W COLSxROWS [cmd=<base64>] [exited=<code|?>]`. Add one sentence to `:358`: "a client that attaches after the exit reads it from `pane-info`'s `exited=` token". Read MUX.md first: DOC-101, DOC-102 and DOC-122 also edit it.
  3. Tests:
     - `tests/mux_daemon.rs::pane_info_reports_exit_for_held_pane`: start a daemon and create a pane running `sh -c 'exit 3'` (`cmd /c exit 3` on Windows, or `#[cfg(unix)]` if the harness lacks a Windows shell helper). Wait for `%pane-exited` using the existing bounded helper (find it with `grep -n "pane-exited" tests/mux_daemon.rs tests/common/mod.rs`). Open a **second** client connection, send `pane-info -t %N`, and assert the reply line ends with ` exited=3`.
     - `pane_info_live_pane_has_no_exited_token`.
  4. **Do not** implement "replay `%pane-exited` on registration" in this pass, and report it as optional. Clients join broadcasts on their first command, not on connect (see the project memory note on mux broadcast registration), so a replay design must choose its trigger point. The query token closes the gap on its own.
  5. CHANGELOG `[Unreleased]` → `### Added`: "**par-mux `pane-info` reports held panes**: an optional trailing `exited=<code|?>` token (ARC-095), so a client that connects after `%pane-exited` can render remain-on-exit state."
- **Method**:
  - Queries are the contract for late clients. The reply stays one line with an optional key=value tail.
  - After SEC-125 (Phase 1), `child_pid()` is `None` for a reaped pane, so a dead pane has no `cmd=` token and only `exited=`.
  - Parsight: `get_symbol_context cmd_pane_info`, `find_symbol exit_code` scoped to `src/mux/pane.rs`.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `cargo test --test mux_targets --no-default-features --features rust-only,mux,serde`
  - Windows VM (orchestrator, main checkout)
  - `make checkall`

### [ARC-101] FFI readback ignores the palette and truncates grapheme clusters
- **Card**: `01a0ef5af73a7e828cd36bc40b9772b6`
- **Decision**: **D4 (ABI v4).**
  - **Gated, do not implement:** changing `SharedCell` (palette-resolved colors, default-color bits, a grapheme side channel) and bumping `TERM_CORE_ABI_VERSION` to 4. It ships as one ABI revision with ARC-112's `ptec_*` rename and ARC-114's structured events.
  - **The additive `terminal_read_row_resolved`:** AUDIT.md gates it on D4 as well, because any new export bumps the ABI number. FFI_GUIDE:416 says the version is "bumped together on any layout or contract change", and v2 was an additive bump. Implement it **only** if the orchestrator's D4 answer is "additive first". Otherwise implement the non-gated prep below and report.
- **Files**:
  - `src/ffi.rs`:
    - `SharedCell` `:43-66` (`text: [u8; 4]`)
    - `SharedCell::from_cell` `:388-407`, which uses `cell.fg.to_rgb()` and ignores `cell.combining`
    - `blank()` `:410`
    - `terminal_read_row` `:621-653`, `terminal_read_scrollback_row` `:665-698`
    - `TERM_CORE_ABI_VERSION = 3` `:431`
  - `src/color.rs:45-51,85-126` (fixed `to_rgb` tables).
  - Palette: `src/terminal/mod.rs:687-742` (`ColorThemeState`: `default_fg`, `default_bg`, `ansi_palette`); `src/terminal/colors.rs:17-38,218`.
  - `src/python_bindings/common.rs:2275-2293`: the only palette-aware resolver, behind the python feature.
  - `src/cell.rs:243` (`combining: SmallVec<[char; 4]>`).
  - `include/terminal_core.h:70-111,449-470`, `include/terminal_core_layout.h:25,104-106`, `cbindgen.toml`, `docs/FFI_GUIDE.md:414-432`.
- **Steps**:
  1. **Non-gated prep, always.** Hoist the palette resolver into core:
     - Add `pub fn resolve_color(&self, color: &Color) -> (u8, u8, u8)` in `src/terminal/colors.rs`:
       - `Named(n)` → `theme.ansi_palette[n as usize]`
       - `Indexed(i < 16)` → `ansi_palette[i]`
       - anything else → `color.to_rgb()`
       - `.to_rgb()` on the palette entry, matching the Python closure at common.rs:2275-2293
     - Make that closure call it.
     - Unit test `resolve_color_follows_osc4`: `process(b"\x1b]4;1;rgb:00/00/ff\x07")`, then `resolve_color(&Color::Named(Red)) == (0, 0, 255)`.

     This is pure refactor, with no ABI or behavior change. It is the piece both v4 and the Python snapshot share.
  2. **Only if D4 is "additive first":**
     - Add `terminal_read_row_resolved(term, row, col_start, out: *mut SharedCell, cap) -> u32`. It has the same cap/return-total protocol as `terminal_read_row`, but fills colors through `resolve_color`, and the default fg/bg through `theme.default_fg/bg`.
     - Add it to `docs/FFI_GUIDE.md`, which `ffi-surface-check` requires.
     - Run `make ffi-header`.
     - Bump `TERM_CORE_ABI_VERSION` to 4 in `src/ffi.rs:431` and `include/terminal_core_layout.h:25` together. The tests `abi_version_matches_header_macro` (ffi.rs:1277) and `layout_header_defines_match_rust` (:1298) pin both.
     - Test `read_row_resolved_follows_palette`, replaying the audit probe via the extern fn.
  3. **Gated v4 plan, to report only:**
     - `SharedCell` gains `attrs` bits `TERM_ATTR_DEFAULT_FG` / `TERM_ATTR_DEFAULT_BG`.
       - `Cell::default` uses `Named(White)` / `Named(Black)` (cell.rs:261), so "default" needs an explicit representation. Verify whether `Color` has one: `find_symbol Color` in `src/color.rs`.
       - The default fg/bg are resolved through the palette.
     - Grapheme side channel: `text` keeps the base char, plus a `TERM_ATTR_HAS_COMBINING` bit, plus `terminal_read_cell_grapheme(term, row, col, out: *mut u8, cap) -> u32` returning the full UTF-8 cluster.
     - Layout asserts in `terminal_core_layout.h`, the Swift probe in `scripts/build-xcframework.sh`, and a DOC-103 v4 row.
- **Method**:
  - The resolver lives once in core, so the FFI and Python snapshots stay consistent.
  - **Pitfall:** `SharedCell` is 16 bytes and `_Static_assert`-pinned. Any field change is the D4 break, so never touch it without D4.
  - Parsight: `get_symbol_context from_cell`, `get_impact terminal_read_row`, `find_code "ansi_palette to_rgb resolve"`.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize ffi` (add `,ffi` once ARC-112 lands)
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize colors`
  - `make ffi-header`, then `make ffi-header-check ffi-surface-check`
  - `make xcframework` (macOS)
  - `make dev` then `uv run pytest tests/test_snapshot.py -v` (the Python snapshot path uses the hoisted resolver)
  - `make checkall`

### [ARC-102] `Terminal` is still a god object
- **Card**: `01a0ef5afaf17b7188775299f2157a03`
- **Files**:
  - `src/terminal/mod.rs`: 3,873 lines. `MacroState` is at `:366-370`, `TriggerState` at `:390-396`.
  - Engine-state reach-ins outside the engines: `mod.rs:3221-3225` (reset swaps), `:3328-3365` (pending-row helpers), `replay_snapshot.rs:234`.
  - `src/terminal/trigger.rs:134` (`pub struct TriggerEngine;`), `src/terminal/macros.rs` (`MacroEngine`).
  - Engine API callers: `src/python_bindings/pty.rs` (17), `src/python_bindings/terminal/trigger_api.rs` (10), `src/pty_session.rs` (1), `mod.rs` (1).
- **Scheduling**: execute after ARC-100, ARC-092 and ARC-091, all earlier in this list. The Remediation Plan's "run last across all domains" rule also puts it after QA-202, so the orchestrator may move it to the end of 3b.
- **Steps (first slice only; one PR per engine; do not attempt the full split):**
  1. **PR 1: `TriggerState` owns its invariants.** Move `mark_row_written`, `shift_pending_trigger_rows`, `flush_pending_trigger_rows` (mod.rs:3322-3368) and ARC-091's `scan_departing_trigger_rows` into `trigger.rs`, as methods on `TriggerState`:
     - `mark_written(row)`
     - `shift_pending(down, n, top, bottom)`
     - `take_departing(...) -> Vec<usize>`
     - `take_pending() -> Vec<usize>`
     - `clear_pending()`
     - `carry_registry_from(&mut self, old: &mut TriggerState)` (the reset swap)

     Then:
     - Make `TriggerState`'s fields private to `trigger.rs` (`pub(in crate::terminal::trigger)`).
     - Keep thin `Terminal` forwarders only where a call needs `&mut Terminal`, for scanning.
     - Replace `replay_snapshot.rs:234` with `self.triggers.clear_pending()`.
     - The `TriggerEngine` public signatures do not change. The 0.56.0 engine move is recent, so do not churn embedders.
  2. **PR 2: the same for `MacroState`.** `reset()` uses `carry_from`, and the `tick_macro` borrow dance from ARC-093 moves into a `MacroState::next_event()`.
  3. **PR 3 (optional follow-on from ARC-100):** migrate the already-carried scalar host settings (security flags, kitty policy, sixel/clipboard caps, answerback, pixel size) into `HostConfig`. Delete the matching `reset()` copy lines; the ARC-100 Python harness proves nothing regressed.
  4. Stop after these slices. Record the next candidates from `propose_decomposition` on `src/terminal/mod.rs` (parsight) in the final report. Do not implement them.
- **Method**:
  - Encapsulation first, then ownership. Each slice deletes field reach-ins from `mod.rs`, so the god-object metric (fields plus fan-in) drops without breaking the public API.
  - Parsight:
    - `find_god_objects` before and after, to report the delta
    - `get_symbol_context TriggerEngine` and `get_impact process_trigger_scans` for the callers
    - `propose_decomposition` scoped to `src/terminal/mod.rs` for the follow-on list
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize trigger`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize macros`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize ris_`
  - `cargo test --lib --no-default-features --features python-test`
  - `make dev` then `uv run pytest tests/test_triggers.py tests/test_macros.py tests/test_ris_host_config.py -v`
  - `make checkall`

### [ARC-104] CI is dispatch-only, and the FFI drift gates never run in CI
- **Card**: `01a0ef5afe5974819306f632b01b8440`
- **Decision**: **D1 gates the push/PR trigger half** (`ci.yml:3-4` is `workflow_dispatch` only, by design). Implement only the jobs, and report the trigger change. A `push: tags:` trigger for the xcframework job is also a trigger change, so it is D1-gated too.
- **Files**:
  - `.github/workflows/ci.yml`: `on:` at `:3-4`; jobs `test` `:50`, `mux-test` `:124`, `lint` `:173`, `build` `:213`, `web-term-drift` `:251`, `features` `:287`.
  - `Makefile:334-339` (`ffi-header`), `:344-359` (`ffi-header-check`), `:363-364` (`ffi-surface-check`, which runs `python3 scripts/check_ffi_surface.py` and needs only the stdlib), `:178-181` (`xcframework`).
  - `.github/workflows/deployment.yml:215-242` (the `ios-xcframework` job, release dispatch only).
- **Steps**:
  1. Add an `ffi-drift` job to `ci.yml`, modeled on `features` (:287-309):
     - ubuntu-latest, checkout, rust toolchain.
     - `cargo install cbindgen --version <X> --locked`, with the version **pinned**. The check diffs generated output, and output varies by cbindgen version. Take X from the version that produced the committed header: run `cbindgen --version` locally, because `terminal_core.h` carries no version stamp.
     - `make ffi-header-check ffi-surface-check`.
  2. Add an `xcframework` job to `ci.yml`: `runs-on: macos-latest`, add targets `aarch64-apple-ios,aarch64-apple-ios-sim` (copy deployment.yml:224-230), run `make xcframework`, with no upload step. It runs on the existing `workflow_dispatch` trigger. The "on tags" part waits for D1.
  3. **Observed, out of scope, report only:** other `make checkall` parts also have no CI equivalent. They are `caps-table-check`, `test-web`, and the `check_api_reference.py` step of `stub-check` (Makefile:321). Do not add them in this pass unless the orchestrator widens scope.
  4. If D1 is approved later: add `push: branches: [main]` and `pull_request:` to `ci.yml:3-4`.
- **Method**:
  - The drift gates then run in CI on every dispatch, and on every push once D1 lands.
  - Pinning cbindgen avoids false drift failures when a new cbindgen formats differently.
  - **Pitfall:** once ARC-112 lands, the xcframework build needs `rust-only,ffi` (`scripts/build-xcframework.sh:17`). If ARC-112 merges first, the job picks that up automatically through `make xcframework`.
- **Verify**:
  - `actionlint .github/workflows/ci.yml` if installed; otherwise `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"`.
  - Locally: `make ffi-header-check ffi-surface-check` and `make xcframework`.
  - After push (standing approval): `gh workflow run ci.yml`, then `gh run watch`.
  - `make checkall`

### [ARC-105] Wheel feature sets diverge
- **Card**: `01a0ef5b02127fe096076511d11706a0`
- **Files**:
  - `pyproject.toml:74-79` (`[tool.maturin] features = ["pyo3/extension-module"]` at `:75-77`).
  - `.github/workflows/deployment.yml:275,332,338,383`: `args: --release --features streaming --out dist …`.
  - `.github/workflows/ci.yml:238`: `maturin build --release`, no features.
  - `.github/workflows/publish-testpypi.yml:61`: no features, so TestPyPI wheels differ from PyPI. AUDIT.md does not list this one.
  - `Makefile`: `dev` `:123-130` (no features), `install` `:134`, `install-force` `:144`, `build` `:160`, `build-release` `:168`, `watch` `:211`, `build-streaming` `:176` (`--features streaming`), `dev-streaming` `:183-190` (`--features streaming`).
  - `CLAUDE.md` build-command block (`make dev`, `make dev-streaming`).
- **Steps**:
  1. `pyproject.toml:75-77` becomes `features = ["pyo3/extension-module", "streaming"]`. The pyproject is then the single source for what a wheel contains.
  2. Remove `--features streaming` from `deployment.yml:275,332,338,383`. Keep `--release --out dist --interpreter …`.
  3. `Makefile`:
     - `dev-streaming` (:190) and `build-streaming` (:176) drop `--features streaming` and become aliases of `dev`/`build`. Keep the targets so existing muscle memory, CLAUDE.md and the memory note still work, and give each help text "(alias: streaming is in pyproject features)".
     - No other target needs editing. They all inherit pyproject features.
  4. `CLAUDE.md`: `make dev` "Build library (release mode via maturin, streaming included)", and `make dev-streaming` "alias of make dev". Add one line to the stub note: "`make stubs` needs a streaming build, which `make dev` now is".
  5. CHANGELOG `[Unreleased]` → `### Changed`: "**Wheels have one feature set** (ARC-105): `streaming` moves into `pyproject.toml` `[tool.maturin] features`, so CI, TestPyPI and PyPI wheels and `make dev` builds all include the streaming classes (TestPyPI and CI wheels previously lacked them)."
- **Method**:
  - maturin reads `[tool.maturin] features` for every build/develop call, so per-command flags become redundant.
  - **Pitfall:** the `streaming` feature adds tokio/axum/rustls, so `make dev` takes longer. That is accepted as the cost of one wheel shape.
  - **Pitfall:** `scripts/generate_stubs.py` refuses non-streaming builds (CHANGELOG 0.57.0 ENH-030). After this change `make dev && make stubs` must work. Verify it.
  - Parsight is not applicable (config files).
- **Verify**:
  - `make dev && uv run python -c "import par_term_emu_core_rust as p; print(p.StreamingServer)"`
  - `make stubs && git diff --exit-code python/par_term_emu_core_rust/_native.pyi`
  - `make dev-streaming` (the alias still works)
  - `make checkall`

### [ARC-106] `mux` still pulls in `clap`
- **Card**: `01a0ef5b05b671c1811a06c28091f4f3`
- **Files**:
  - `Cargo.toml`: `[[bin]] par-mux` `:60-63` (`required-features = ["mux"]`), `mux` feature `:255`, `streaming-bin` `:246`, `clap` optional dependency `:104`.
  - `scripts/check_features.sh:71-76` (the skip guard) and `:29`.
  - Tests that use `env!("CARGO_BIN_EXE_par-mux")`: `tests/mux_cli.rs:27,276,445`, `tests/mux_daemon.rs`, `tests/mux_nested.rs`, `tests/common/mod.rs:186`. `common` is used by mux_agents, mux_cli, mux_daemon, mux_restart, mux_reattach, mux_nested, mux_hooks and mux_targets.
  - Invocations to update:
    - `.github/workflows/ci.yml:166,202`
    - `Makefile:227,278,298`
    - `.pre-commit-config.yaml:43`
    - `CLAUDE.md:84,104` and the CLAUDE.md feature table
    - `docs/BUILDING.md:211,214,222`
    - `docs/MUX.md:34,40`
    - `README.md:290`
    - `src/mux/client.rs:332` (a comment)
- **Steps**:
  1. `Cargo.toml:255`: remove `"clap"` from `mux`. Add `mux-bin = ["mux", "clap"]` after it. **Use `"clap"`, not `"dep:clap"` as AUDIT.md writes.** Cargo removes a dependency's implicit feature once any feature references it as `dep:clap`, which would break `streaming-bin = [... "clap" ...]` at `:246`.
  2. `[[bin]] par-mux` (`:63`): `required-features = ["mux-bin"]`.
  3. Every command that builds or tests the binary, or compiles an integration test using `CARGO_BIN_EXE_par-mux`, switches `mux` to `mux-bin`. `CARGO_BIN_EXE_*` exists only when the bin's required-features are met, so a missed one fails to compile rather than silently skipping.
     - `ci.yml:166` → `--features rust-only,mux-bin,serde`; `ci.yml:202` clippy → add `mux-bin`.
     - `Makefile:227` → `rust-only,mux-bin,serde`; `Makefile:278,298` clippy → add `mux-bin`.
     - `.pre-commit-config.yaml:43` → add `mux-bin`.
     - `CLAUDE.md:84` (`--lib --tests` compiles integration tests) → `rust-only,mux-bin,serde`. `CLAUDE.md:104` is `--lib` only, so leave it.
     - `docs/BUILDING.md:211,214,222`, `docs/MUX.md:34,40` and `README.md:290` → `--features mux-bin`.
     - `src/mux/client.rs:332`: fix the comment.
     - `--lib`-only invocations (`ci.yml:171`, `Makefile:229`, CLAUDE.md:104) and `fuzz/Cargo.toml:22` stay on `mux`. They only need the library.
     - Update the standard mux gate in this playbook too: `cargo test --test mux_daemon --no-default-features --features rust-only,mux-bin,serde`.
  4. `scripts/check_features.sh:71-76`: replace the guarded skip with an unconditional `assert_absent rust-only,mux clap`, and add `cargo check --no-default-features --features rust-only,mux-bin` next to `:29`.
  5. `CLAUDE.md` feature table: add a `mux-bin` row ("the `par-mux` binary: `mux` plus clap").
  6. CHANGELOG `[Unreleased]` → `### Changed` (**breaking for `cargo install`**): "**The `par-mux` binary needs the new `mux-bin` feature** (ARC-106): `mux` no longer pulls `clap`, so library embedders (par-term) stop compiling it. Install with `cargo install par-term-emu-core-rust --no-default-features --features mux-bin --bin par-mux`."
- **Method**:
  - This mirrors the existing `streaming` / `streaming-bin` split (`:242` / `:246`).
  - par-term depends on `features = ["par-term-emu-core-rust/mux"]` (`../par-term/par-term-mux/Cargo.toml:33`, `par-term-terminal/Cargo.toml:45`) for the library only, so it loses clap and gains nothing to fix.
  - Parsight is not applicable. Enumerate with `grep -rn "features.*mux\b\|--features mux" Makefile .github CLAUDE.md docs README.md scripts .pre-commit-config.yaml`.
- **Verify**:
  - `make check-features` (asserts clap is absent from `rust-only,mux`)
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux-bin,serde`
  - `cargo test --no-default-features --features rust-only,mux-bin,serde -- --test-threads=1`
  - `cargo tree --no-default-features --features rust-only,mux -e normal | grep -c clap` must print 0
  - `make checkall`
  - Windows VM (orchestrator), using the updated CLAUDE.md:84 line

### [ARC-107] `Cargo.lock` is untracked
- **Card**: `01a0ef5b092d74b1b4ad07003ae5f5b8`
- **Decision**: **D2 gates the whole remedy.** `.gitignore:4` untracks the lockfile deliberately. CLAUDE.md:78 says "no --locked — Cargo.lock is not tracked, cargo resolves fresh". Nothing is non-gated. Implement nothing, and report. It is also blocked by ARC-104 (CI must exist to verify `--locked`), and it relates to SEC-134 tracking.
- **Files**:
  - `.gitignore:4`.
  - `Cargo.lock` (108 KB on disk, untracked).
  - `--locked` targets: the `cargo test`/`clippy`/`build` lines in `ci.yml` (`:98,101,104,150,166,171,202`), deployment.yml cargo builds (`:186` and the xcframework job), `fuzz.yml`.
  - `CLAUDE.md:78` (Windows VM note).
  - `CONTRIBUTING.md`.
- **Steps**:
  1. **If D2 is approved:**
     - Delete `.gitignore:4` and `git add Cargo.lock`.
     - Add `--locked` to every `cargo build/test/clippy/check/rustc` line in the workflows. Tool installs already use `--locked`.
     - Rewrite `CLAUDE.md:78` to say `--locked` works, because `git archive` now ships the lockfile.
     - CHANGELOG `[Unreleased]` → `### Changed`: "`Cargo.lock` is committed; CI and release builds use `--locked` (ARC-107)."
  2. **If D2 is declined:**
     - Add one line to `CONTRIBUTING.md` stating the lockfile stays untracked, with the owner's rationale taken from the D2 answer. Do not invent a rationale.
     - Close the card as by-design.
- **Method**: the decision is the owner's. The lockfile is a reproducibility-versus-freshness tradeoff for a library crate.
- **Verify** (D2 approved):
  - `cargo test --locked --lib --no-default-features --features pyo3/auto-initialize`
  - `make check-features`
  - `make checkall`
  - A dispatched CI run

### [ARC-108] Layering inversions remain
- **Card**: `01a0ef5b0c4d7c709e03990a5c354528`
- **Files**:
  - (a) `src/grid/mod.rs:287-304`, where `capture_snapshot` / `restore_from_snapshot` name `crate::terminal::replay_snapshot::GridSnapshot`. `GridSnapshot` is defined at `src/terminal/replay_snapshot.rs:13-40` and `TerminalSnapshot` at `:41-80`, both with `#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]`. Users:
    - `src/mux/persist.rs:16,165,615,629,2440`
    - `src/mux/pane.rs:7,172,213,307`
    - `src/terminal/snapshot_manager.rs:8,27`
    - `src/python_bindings/types/screen.rs:90-91`
  - (b) `src/graphics/mod.rs:401,435` call `crate::terminal::unix_millis()`, which is defined at `src/terminal/mod.rs:169` and has 31 call sites.
  - (c) `src/streaming/protocol.rs:278-284,344-347,558-565,786-792`: `pydict(to_with/from_with = "crate::streaming::py_convert::…")` attributes, consumed by `derive/src/lib.rs:86` (`PyDictConvert`). `src/streaming/py_convert.rs`, declared at `src/streaming/mod.rs:74-75`.
- **Steps**:
  1. **(a)** Move the `GridSnapshot` struct, byte for byte including its serde `cfg_attr` and field order, to a new `src/grid/snapshot.rs`. Declare it with `mod snapshot; pub use snapshot::GridSnapshot;` in `grid/mod.rs`.
     - In `replay_snapshot.rs`, replace the definition with `pub use crate::grid::GridSnapshot;` so every existing path (`terminal::replay_snapshot::GridSnapshot`) still resolves.
     - `grid/mod.rs:287,304` use the local type.
     - `TerminalSnapshot` stays in `terminal`. terminal→grid is the correct direction.
  2. **(b)** Add a leaf module `src/time.rs` with `#[inline] pub fn unix_millis() -> u64`, moved from `terminal/mod.rs:169`. Declare `pub mod time;` in `lib.rs`.
     - In `terminal/mod.rs`, write `pub use crate::time::unix_millis;` so all 31 call sites and the public path stay valid.
     - Change `graphics/mod.rs:401,435` to `crate::time::unix_millis()`.
  3. **(c)** Move `src/streaming/py_convert.rs` to `src/python_bindings/streaming_convert.rs`, under the same `#[cfg(all(feature = "streaming", feature = "python"))]`. Update the four attribute paths in `protocol.rs` to `crate::python_bindings::streaming_convert::…`.
     - **Report the residual:** `protocol.rs` still names a bindings path inside `cfg_attr(python)`, because the `PyDictConvert` derive requires `to_with`/`from_with` paths on the type.
     - Removing the reference entirely would need the derive to accept a trait-based hook. That is a `derive/` crate change and a separate version bump (CLAUDE.md derive-crate rule), so it is out of scope here.
- **Method**:
  - `grid` and `graphics` no longer import `terminal`. Re-exports keep every public path, so the change is not breaking.
  - **Pitfall:** the `GridSnapshot` serde shape is the par-mux on-disk format (persist.rs). Move it verbatim, and run the mux persistence tests.
  - Parsight: `get_symbol_context GridSnapshot` (referenced_by), `get_impact unix_millis`, `find_dependency_path` from `grid` to `terminal`, which must be empty afterwards.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::persist`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::`
  - `make dev-streaming` then `uv run pytest tests/test_streaming_dict_api.py -v`
  - `make check-features`
  - `make checkall`

### [ARC-109] Streamer polls terminal events at 20 Hz through `Arc<Mutex<PtySession>>`
- **Card**: `01a0ef5b0f937d709e6938ef35b74239`
- **Files**:
  - `src/bin/streaming_server/bootstrap.rs`:
    - `ServerState::poll_terminal_events` `:217-226` (a 50 ms `interval`, then `pty_session.lock()` → `terminal().write()` → `poll_events()`)
    - `BinarySessionFactory::setup_session` poll task `:556-565`
    - `parking_lot::Mutex` fields `:38,69,86,96,315`
  - `src/pty_session.rs`: `update_signal` `:139`, `wait_for_update` `:1610`, `UpdateWaiter` `:1752` (with `wait_for_update(since, timeout)` `:1775`), `terminal()` `:1370`.
  - `src/terminal/mod.rs:3488-3510` (`poll_events`: drains `terminal_events` and is the **only** producer of `ZoneScrolledOut`).
- **Steps**:
  1. **Approach:** drive the existing `poll_events` from the PTY update condvar instead of a timer. Do not replace it with an observer.
     - An observer batch copies events without draining them (mod.rs:3138-3145). If polling stopped, `terminal_events` would grow to `MAX_TERMINAL_EVENTS` (10,000, `:161`), and `ZoneScrolledOut` would never be produced.
     - The condvar path keeps `poll_events` semantics exactly, but only wakes when output was applied.
  2. Obtain an `UpdateWaiter` from the session. Verify the accessor name with `get_symbol_context UpdateWaiter` (constructor/caller). If none is public, add `pub fn update_waiter(&self) -> UpdateWaiter` to `PtySession`.
  3. Replace each 50 ms loop with a named `std::thread` (not the tokio blocking pool; it is long-lived):
     ```rust
     let mut gen = 0;
     loop {
         if stop.load(Relaxed) { break }
         match waiter.wait_for_update(gen, Duration::from_secs(1)) {
             Some(g) => gen = g,
             None => continue,
         }
         let events = terminal.write().poll_events();   // Arc<RwLock<Terminal>>, no PtySession mutex
         if !events.is_empty() && tx.blocking_send(events).is_err() { break }
     }
     ```
     The async side `recv().await`s and runs today's `terminal_event_to_server_message` (protocol.rs:17) + broadcast code. Use a bounded `tokio::sync::mpsc::channel` (capacity 64; a `blocking_send` backpressures the thread, not the PTY reader).
  4. Stopping: a per-session `Arc<AtomicBool>` is set where the session tears down today. Find it with `get_symbol_context setup_session` and the session-close path. The 1 s timeout bounds the thread's exit.
  5. Leave the other `Arc<Mutex<PtySession>>` uses (writes, resize) alone. They are out of scope.
- **Method**:
  - An idle streamer now makes zero wakeups instead of 20 per second per session.
  - Event polling no longer takes the `PtySession` mutex, only the terminal write lock, as the reader thread does.
  - **Pitfall:** `wait_for_update`'s generation starts at 0. Seed it from the current generation, or the first call returns immediately. Read `:1775` first.
  - Parsight: `get_symbol_context poll_terminal_events`, `get_symbol_context wait_for_update`, `get_impact poll_events`.
- **Verify**:
  - `cargo build --no-default-features --features streaming-bin` (bin compiles)
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::`
  - `make test-rust-streaming`
  - `cargo test --test test_ws_smoke --no-default-features --features pyo3/auto-initialize,streaming`
  - Verify manually: `make streamer-run`, connect the web client, run `printf '\a'`, and check that a bell event arrives
  - `make checkall`

### [ARC-110] Accept loops are duplicated; the axum path keeps its own session loop
- **Card**: `01a0ef5b12ba7a5083dc7a012c2628f1`
- **Files**: `src/streaming/server.rs` (3,572 lines):
  - `start()` `:656`, which picks TLS `:666` or plain `:668`
  - `start_websocket_only` `:874` (loop `:886`), then `handle_connection_ws` `:1197`
  - `start_websocket_only_tls` `:993` (loop `:1014`, TLS handshake under `WS_HANDSHAKE_TIMEOUT`), then `handle_tls_connection_ws` `:1211`
  - `run_ws_session<S>` `:1883` (`select!` `:1935`), `Client<S>` at `src/streaming/client.rs:18`
  - `handle_axum_websocket` `:2187` (loop `:2255`, `select!` `:2256`, called at `:3023`)
- **Steps**:
  1. **One accept loop.** Extract `async fn accept_loop<F, Fut, S>(self: &Arc<Self>, listener: TcpListener, upgrade: F, label: &'static str)`, where `upgrade: Fn(TcpStream) -> Fut` returns `Result<S>`. Plain passes the stream through; TLS runs the `TlsAcceptor` handshake under `WS_HANDSHAKE_TIMEOUT`. The body is what both loops share today:
     - the client-cap check
     - `set_nodelay`
     - SEC-004 `try_add_client` plus `GlobalClientGuard`
     - header-callback auth via `accept_hdr_async_with_config`
     - `prepare_ws_session` → `run_ws_session`

     Merge `handle_connection_ws` and `handle_tls_connection_ws` (they differ only in the label) into one generic `handle_ws_connection<S>`. `start_websocket_only{,_tls}` shrink to listener setup plus `accept_loop`.
  2. **Axum feeds `run_ws_session`.** Introduce a private trait `WsTransport` with `async fn send_binary`, `async fn recv() -> Option<Result<WsFrame>>`, `async fn ping()` and `async fn close()`, where `enum WsFrame { Binary(Vec<u8>), Text, Ping, Pong, Close }`. Implement it for `Client<S>` and for a wrapper over axum's split `WebSocket`. Make `run_ws_session` generic over `T: WsTransport`. Preserve the axum-specific behavior:
     - The SEC-011 global slot is reserved before `prepare_ws_session`.
     - `Text` is rejected with the existing debug error (`:2311`).
     - `Ping`/`Pong` are ignored.
     - Close is `AxumMessage::Close(None)` with the `flush()` fallback (`:2368`).
  3. Leave `handle_stats_websocket` (`:3094`, the third `select!`) alone. Report it.
- **Method**:
  - One session loop means one place for keepalive, filtering (`should_send`) and input handling (`handle_client_message` `:1274`).
  - **Pitfall:** protobuf encoding lives in `Client<S>`. The axum path calls `encode_server_message` / `decode_client_message` by hand, so the trait's `send_binary`/`recv` must operate at the message level both implementations share. Read `client.rs` whole first.
  - **Pitfall:** QA-184, QA-192, QA-212, QA-213 and DOC-110 also edit `server.rs`. Read before editing.
  - Parsight: `get_symbol_context run_ws_session`, `get_symbol_context handle_axum_websocket`, `find_duplicate_code` scoped to `src/streaming/server.rs` (confirm the two loops are flagged, then not flagged afterwards).
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::`
  - `make test-rust-streaming`
  - `cargo test --test test_streaming --no-default-features --features pyo3/auto-initialize,streaming`
  - `cargo test --test test_ws_smoke --no-default-features --features pyo3/auto-initialize,streaming`
  - `make streamer-run-http`, then load `web_term/` in a browser and type (the axum path)
  - `make checkall`

### [ARC-111] The core library logs through a private file logger
- **Card**: `01a0ef5b16be7130838278dd978b996c`
- **Files**:
  - `src/debug.rs`:
    - `LOGGER` / `LOG_LEVEL` `:136-146`
    - `is_enabled` `:~169`, `log` `:~174`, `logf` `:~185` (verify: `find_symbol logf`)
    - macros `debug_error!` `:193`, `debug_info!` `:200`, `debug_log!` `:207`, `debug_trace!` `:214`
    - `src/lib.rs:57-58` (`#[macro_use] pub mod debug;`)
  - Call sites:
    - 133 macro calls across `src/` (59 in `streaming/server.rs`, 4 in `pty_session.rs`)
    - 53 direct `debug::log(`, 18 `debug::log_*` helpers (27 + 5 of them in `pty_session.rs`), 3 `debug::is_enabled(`
  - `log = "0.4"` at `Cargo.toml:66`, non-optional. It is already used in `src/mux/*`.
- **Steps**:
  1. Change **only** `src/debug.rs`, so there is no call-site churn. `log()` and `logf()` become the single funnel:
     - Map `DebugLevel` to `log::Level`: Error→Error, Info→Info, Debug→Debug, Trace→Trace.
     - Forward to `log::log!(target: "par_term_emu_core_rust", lvl, "[{category}] {args}")` when `log::log_enabled!(target: …, lvl)`.
     - Keep the existing file-sink write when the `DEBUG_LEVEL` file logger is enabled.
     - The four macros already expand to `$crate::debug::logf(...)`, so they follow automatically.
  2. `is_enabled(level)` returns `file_enabled(level) || log::log_enabled!(…)`, so guarded call sites build their messages when either sink wants them. Keep the QA-112 atomic fast path first: `LOG_LEVEL` is an `AtomicU8`, and `log::max_level()` is also an atomic load.
  3. The `debug::log_*` helpers (`log_scroll`, `log_screen_switch`, …) route through `log()` already. Verify each with `find_code "pub fn log_" file_path src/debug.rs`. Any helper that writes the file directly must go through the funnel.
  4. Add an integration test, `tests/debug_log_facade.rs`. It needs its own process, because the global logger can be set only once:
     - Install a minimal `log::Log` that pushes records into a `Mutex<Vec<String>>`, then `log::set_max_level(Trace)`.
     - Call `par_term_emu_core_rust::debug_error!("CAT", "boom {}", 1)`.
     - Assert that a record contains `[CAT] boom 1`.
     - Also test that nothing reaches the file sink while `DEBUG_LEVEL` is unset.
  5. Doc comment on `debug.rs`: "`log` facade first; the file logger is an optional sink enabled by `DEBUG_LEVEL`."
- **Method**:
  - Embedders (par-term, par-mux via its own `log` backend, pyo3-log users) now see core diagnostics in their own logging, and the private file stays for the existing debug workflow.
  - **Pitfall:** par-mux installs a `log` backend. Escape-level `debug_log!` noise now reaches it at Debug/Trace level. Confirm the par-mux default level filters Debug (verify: `grep -n "env_logger\|set_max_level\|LevelFilter" src/bin/par_mux/main.rs`).
  - **Pitfall:** the streamer binary uses `tracing` (`main.rs:91`, `bootstrap.rs:21`). Whether it bridges `log` records is unverified (verify: `grep -rn "tracing_log\|LogTracer" src/bin/`). Report either way. Do not add a bridge here.
  - Parsight: `get_symbol_context logf`, `get_impact is_enabled`.
- **Verify**:
  - `cargo test --test debug_log_facade --no-default-features --features pyo3/auto-initialize`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize debug`
  - `make test-rust-streaming`
  - `cargo bench --bench terminal_throughput -- --quick` if time allows. Compare interleaved against main per the memory note; the parser hot path calls `is_enabled`.
  - `make checkall`

### [ARC-112] C symbols are exported unprefixed from every Python wheel; `Broadcaster` is not deprecated
- **Card**: `01a0ef5b19ec744289d4478d7407ec8e`
- **Decision**: **D4 gates the `ptec_*` rename only.** Implement the `ffi` feature and the `Broadcaster` deprecation, and report the rename as part of the ABI v4 batch with ARC-101 and ARC-114.
- **Files**:
  - `src/lib.rs:59` (`pub mod ffi;`, unconditional). `Cargo.toml` has no `ffi` feature today (`[features]` `:221-284`, `crate-type = ["cdylib","rlib"]` `:51`).
  - 20 `#[no_mangle]` exports in `src/ffi.rs`; `src/keyboard.rs` has none.
  - Every consumer of `crate::ffi` needs the feature:
    - `scripts/build-xcframework.sh:17` (`FEATURES="${XCFRAMEWORK_FEATURES:-rust-only}"`)
    - `benches/ffi_readback.rs:18` (Cargo.toml `:212` `required-features = ["rust-only"]`)
    - `tests/ffi_dirty_ranges_alloc.rs:5`
    - `src/terminal/tests/ffi_tests.rs:1` (declared `src/terminal/tests/mod.rs:21-22`)
    - `docs/FFI_GUIDE.md:74-75` (build line)
    - the CLAUDE.md / playbook FFI gate
  - `Broadcaster`: `src/streaming/broadcaster.rs:15` (struct), module `src/streaming/mod.rs:52-53`, re-export `:81-82`. It has zero users outside its own tests (`:199-307`). Docs: `docs/STREAMING.md:86,1482,1485`.
  - `cbindgen.toml` (`[parse] parse_deps = false`, no `[defines]`).
- **Steps**:
  1. `Cargo.toml [features]`: add `ffi = []`, with the comment "C ABI surface (`terminal_*` exports) for the xcframework / C embedders; off in Python wheels (ARC-112)".
  2. `src/lib.rs:59`: `#[cfg(feature = "ffi")] pub mod ffi;`.
  3. Turn the feature on everywhere the ffi module is consumed:
     - `build-xcframework.sh:17`: default `rust-only,ffi`.
     - `Cargo.toml:212` bench: `required-features = ["rust-only", "ffi"]`.
     - `tests/ffi_dirty_ranges_alloc.rs`: add `#![cfg(feature = "ffi")]` at the top, or a `[[test]]` entry with `required-features = ["ffi"]`. Prefer the `[[test]]` entry, which is explicit.
     - `src/terminal/tests/mod.rs:21`: `#[cfg(all(test, feature = "ffi"))] mod ffi_tests;`.
     - `docs/FFI_GUIDE.md:74-75`: `--features rust-only,ffi`.
  4. Keep the FFI tests running in the standard gates. Otherwise the gate stops compiling `ffi.rs`:
     - `Makefile` `test-rust` (`:219-229`): change the `--lib --features rust-only,serde` line to `rust-only,serde,ffi`.
     - Clippy lines (`Makefile:278,298`, `ci.yml:202`, `.pre-commit-config.yaml:43`): add `ffi`.
     - CLAUDE.md "Running Tests": document the FFI filter as `cargo test --lib --no-default-features --features pyo3/auto-initialize,ffi ffi`.
     - `scripts/check_features.sh`: add `cargo check --no-default-features --features rust-only,ffi`.
  5. cbindgen: after gating, `make ffi-header` must produce a **byte-identical** header, because `ffi-header-check` diffs it. cbindgen may treat the `#[cfg(feature = "ffi")]` module as conditional and either `#if`-wrap its items or skip them. If the header changes, add a `[defines]` entry mapping `"feature = ffi"` to a macro that the header prose defines unconditionally, or equivalent. Verify with `cbindgen --help` and docs.rs cbindgen `[defines]`. Do not hand-edit the header.
  6. `Broadcaster`: add `#[deprecated(since = "<next version>", note = "unused by StreamingServer; clients are managed per session. Will be removed in a future release.")]` on the struct at `broadcaster.rs:15`. Get the version from `Cargo.toml` plus the orchestrator.
     - Add `#[allow(deprecated)]` on the `pub mod broadcaster;` (`mod.rs:52-53`) and on the `pub use` (`:81-82`), so the crate's own tests and re-export do not fail `-D warnings`.
     - Add a one-line deprecation note to `docs/STREAMING.md:86`.
  7. CHANGELOG `[Unreleased]`:
     - `### Changed`: "**The C ABI is behind a new `ffi` feature** (ARC-112): Python wheels no longer export the 20 `terminal_*` symbols; C/Swift embedders build with `--features rust-only,ffi` (`make xcframework` does)."
     - `### Deprecated`: "`streaming::Broadcaster` (unused)."
- **Method**:
  - Symbols leave the wheel without any ABI change. par-term does not use `par_term_emu_core_rust::ffi` (grep of `../par-term`), and its features (`rust-only, pty_session, screenshot`) never needed it.
  - **Pitfall:** any feature set used to run `ffi`-filtered tests must now include `ffi`, or the filter silently matches zero tests. Check that the test count is non-zero.
  - Parsight: `find_code "par_term_emu_core_rust::ffi"` and `get_symbol_context SharedState`, to find every consumer.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,ffi ffi`, and confirm it reports a non-zero test count
  - `cargo test --test ffi_dirty_ranges_alloc --no-default-features --features rust-only,ffi`
  - `make ffi-header`, then `make ffi-header-check ffi-surface-check`
  - `make xcframework` (macOS)
  - `make dev && nm -gU python/par_term_emu_core_rust/_native*.so | grep -c ' _terminal_'` must print 0 (it printed 20 at 0.57.0)
  - `make check-features`
  - `make checkall`

### [ARC-113] Pane metadata is a stringly-typed namespace, re-parsed as JSON on every roster read
- **Card**: `01a0ef5b1cf77460aa2bfabf8f6558dd`
- **Files**:
  - `src/mux/pane.rs`: `metadata: HashMap<String,String>` `:151`, `metadata()` `:373`, `set_metadata` `:378`, `clear_metadata` `:385`.
  - `src/mux/hooks.rs`:
    - `AGENT_CLAIM_KEYS` `:403-419`
    - `TELEMETRY_KEY = "agent_telemetry"` `:468`
    - `SEQ_STAMPS_KEY` `:880`
    - private `TelemetryV1` (Serialize only) `:584-607`, serialized `:716`
    - per-read parses: `stored_telemetry_sampled_at` `:789-800` (parse `:794`), called by `fresh_telemetry_b64` `:809-822` (at `:813`) and `handle_telemetry_report` `:538`; `seq_stamps` `:885-891` (parse `:890`), used by `is_stale` `:868` and `record_seq` `:900`
  - `src/mux/host_probe.rs`: `HOST_TELEMETRY_KEY` `:32`, `fresh_host_telemetry_b64` `:315-321`.
  - `src/mux/dispatch.rs:387-406` (`cmd_list_agents`, which reads keys under the tree lock).
- **Steps (first slice: the JSON-bearing keys; identity keys stay strings):**
  1. On `MuxPane`, add `pub(crate) telemetry: Option<StoredTelemetry>`, `pub(crate) host_telemetry: Option<StoredHostTelemetry>` and `pub(crate) seq_by_source: HashMap<String, u64>`.
     - `StoredTelemetry { sampled_at_unix_ms: u64, canonical_b64: String }` holds the fields the roster reads, plus the pre-encoded roster token. Encode once at write time (`hooks.rs:716`) with the same canonical JSON, so the wire bytes are unchanged.
     - `StoredHostTelemetry` works the same way. Read `host_probe.rs:300-330` to see which per-field `sampled_at` values the aging logic needs. Keep them typed, not re-parsed.
  2. Replace the three metadata keys' writes, reads and clears with the typed fields:
     - `TELEMETRY_KEY`, `HOST_TELEMETRY_KEY`, `SEQ_STAMPS_KEY`.
     - Clears happen at `pane.release_agent` and in the liveness sweep. Enumerate with `grep -n "TELEMETRY_KEY\|HOST_TELEMETRY_KEY\|SEQ_STAMPS_KEY" src/mux/*.rs`.
     - `fresh_telemetry_b64` and `fresh_host_telemetry_b64` become field reads plus the freshness check, with no `serde_json::from_str`.
  3. **Persistence check:** telemetry is display-only and never persisted (CHANGELOG 0.55.0). Confirm that `persist.rs` copies only identity keys (`grep -n "metadata" src/mux/persist.rs`), so the on-disk format is untouched. If any of the three keys is persisted, stop and report.
  4. Leave the `AGENT_CLAIM_KEYS` strings (`agent`, `agent_state`, `agent_state_source`, …) in the map for this slice. Report a typed `AgentClaim` as the follow-on.
  5. Tests: every existing hooks and `list-agents` roster test must pass unchanged, which proves byte-identical rows. Add `telemetry_token_is_encoded_once`: after a report, `fresh_telemetry_b64` returns the stored `canonical_b64` (pointer-equal or string-equal) without calling a parse. Assert through the public roster output plus a unit test on the helper.
- **Method**:
  - Parse on write (rare, from hooks) instead of on every roster read (frequent, under the tree lock).
  - **Pitfall:** `TelemetryV1` is `Serialize`-only and private. You may need `Deserialize` if `handle_telemetry_report` compares against the stored sample (`:538`). Read it first.
  - **Pitfall:** ARC-095 and ARC-113 both edit `dispatch.rs` near the roster grammar. Keep the ARC-060 tail grammar byte-identical.
  - Parsight: `get_symbol_context fresh_telemetry_b64`, `get_impact stored_telemetry_sampled_at`, `get_symbol_context seq_stamps`.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `cargo test --test mux_hooks --no-default-features --features rust-only,mux,serde -- --test-threads=1`
  - `cargo test --test mux_agents --no-default-features --features rust-only,mux,serde -- --test-threads=1`
  - Use `mux-bin` in place of `mux` for integration tests once ARC-106 lands
  - Windows VM (orchestrator)
  - `make checkall`

### [ARC-096] MuxTree reverse lookups are allocating linear scans
- **Card**: `01a0ef5b1fda7fb0a076e20299282fbc`
- **Files**:
  - `src/mux/tree.rs`:
    - `MuxTree` struct `:205-231`; `MuxSession.windows: Vec<WindowId>` `:66`
    - `session_of_window` `:715-721`; `window_of_pane` `:723-729`, which iterates every window and allocates `layout.pane_ids()` per window
    - inline scans at `:820` (`drop_empty_window`), `:870` (`break_pane`), `:1031` (`begin_respawn`), `:1108` (`move_window`), `:1140` (`swap_windows`), `:1426` and `:1454` (`kill_pane`), `:1494` (`select_window`), `:1530` (`kill_window`)
    - `window_of_pane` call sites: `:608, 735, 765, 795, 865, 956, 959, 1025, 1080, 1181, 1244, 1419`
  - Callers in `src/mux/dispatch.rs:472,713`.
  - `src/mux/persist.rs:409` (`from_persist_state`, which rebuilds the tree).
  - The **ARC-090 `mutate_layout` choke point** (Phase 2, landed before this).
- **Steps**:
  1. Re-locate everything. Phase 1 and 2 rewrote these functions. Run `get_symbol_context window_of_pane` and `find_symbol mutate_layout` (it must now exist). Read `mutate_layout` whole.
  2. Add `pane_window: HashMap<PaneId, WindowId>` and `window_session: HashMap<WindowId, SessionId>` to `MuxTree`, initialized empty in `MuxTree::new` (`:235`).
  3. **Pane→window, maintained at the choke point.** In `mutate_layout(window, f)`:
     - Capture `before = layout.pane_ids()`.
     - Run `f`.
     - Compute `after`.
     - For panes in `after`, insert `pane_window[p] = window`. For panes in `before` but not `after`, remove the entry only if it still maps to this window. A `join-pane`/`break-pane` move inserts into the new window first.
     - Window creation (new-session, new-window, break-pane, restore) seeds the entry for its first pane. Route those through `mutate_layout`, or add one `index_window_panes(window)` call where the window is created.
     - If ARC-090 left a layout mutation outside the choke point, route it through the choke point. **Do not** add a second maintenance site.
  4. **Window→session, maintained through two helpers.** `fn link_window(&mut self, session, window, index)` and `fn unlink_window(&mut self, window)` are the only code that edits `MuxSession.windows`. Replace the 13 `.windows.{push, insert, remove, retain, swap}` sites (tree.rs and persist.rs; `grep -n "\.windows\.\(push\|insert\|remove\|retain\|swap\)" src/mux/tree.rs src/mux/persist.rs`).
  5. `from_persist_state` (persist.rs:409) builds both maps after restore, through the same helpers.
  6. Rewrite `window_of_pane` and `session_of_window` as map lookups. Replace the 9 inline scans with the helpers.
     - `kill_pane` scans twice today (`:1419` and `:1426`). Collapse it to one lookup.
     - Leave the `.position()` calls on an already-found session's Vec. They are not reverse lookups.
  7. Tests in the `tree.rs` test module:
     - Add `#[cfg(test)] fn assert_indexes_consistent(&self)`, which brute-force scans and compares both maps.
     - `reverse_indexes_track_every_structural_command`: run split, kill-pane (including the last pane in a window), break-pane, join-pane, move-window, swap-window, respawn, kill-window, kill-session and new-window, calling `assert_indexes_consistent()` after each.
     - Add `debug_assert!(self.indexes_consistent())` at the end of `mutate_layout`, **cfg(test) only**, because it is O(n) and must stay out of release builds.
- **Method**:
  - Lookups become O(1) under the global tree mutex that the reap tick, scrape tick and every command share.
  - parsight `get_impact window_of_pane` at HEAD rates it **Critical**, with 13 direct and 138 transitive callers. AUDIT.md's "High / 56 transitive" came from an older depth/index. Every caller then benefits, and none needs editing.
  - **Pitfall:** the two maps become a second source of truth, which is what the consistency test and the single-maintenance-site rule guard against.
  - **Pitfall:** QA-188 and QA-202 also edit `tree.rs`. Read before editing.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde` (`mux-bin` after ARC-106)
  - `cargo test --test mux_restart --no-default-features --features rust-only,mux,serde -- --test-threads=1` (the restore rebuild)
  - Windows VM (orchestrator)
  - `make checkall`

### [ARC-097] remain-on-exit overloaded `SaveOrigin::ShutdownEmpty`
- **Card**: `01a0ef5b22cd70c39000d83627fb63e6`
- **Files**:
  - `src/mux/persist.rs:687-709` (the `SaveOrigin` enum; `ShutdownEmpty` doc `:703-708`), `:797-802` (`state_has_panes`), `:805-827` (`maintain_lastgood`), test near `:1715`.
  - `src/mux/server.rs:185-205` (`run_persisting`: `LoopExit::Empty` → `ShutdownEmpty` `:198`; doc `:192`), `:306-330` (idle exit: `sessions().is_empty() || all_panes_dead()` `:312-315`).
  - `docs/MUX.md:352,362`.
- **Decision (approach)**: **Fix the contract text, not the behavior.** Do not add a variant. Reasons:
  - MUX.md:362 documents the all-dead exit as intended: the final save restores every pane, respawned.
  - `maintain_lastgood` already branches on `state_has_panes`, not on the origin. An all-dead tree is pane-bearing, so the snapshot refreshes, and an empty tree clears it.
  - A `ShutdownAllDead` variant would take the identical branch.
  - What is wrong is the enum doc's promise of "not a resurrection".
- **Steps**:
  1. Rewrite the `ShutdownEmpty` doc (`persist.rs:703-708`): "The final save of an exit-when-empty daemon: no clients, and either zero sessions or every pane dead, held past the grace period with no shutdown signal received. An empty tree clears the snapshot (the user closed everything, so the next start is fresh). An all-dead tree is pane-bearing and refreshes it, so the next start respawns those panes (MUX.md, Pane Reaping)."
  2. Rewrite `server.rs:192` to match.
  3. Test in `persist.rs` next to `:1715`:
     - Read the existing test first.
     - Add `shutdown_empty_with_dead_panes_refreshes_lastgood`: a pane-bearing state saved with `SaveOrigin::ShutdownEmpty` leaves `.lastgood` present and updated.
     - Add or keep `shutdown_empty_with_no_panes_clears_lastgood`.
  4. **Docs observation, report to the DOC agents:** the MUX.md paragraph at `:364` is duplicated word for word at `:366`.
- **Method**: the code already implements the documented design. Only the enum's contract was stale. Parsight: `get_symbol_context maintain_lastgood`, `find_code "ShutdownEmpty"`.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::persist`
  - `cargo test --test mux_restart --no-default-features --features rust-only,mux,serde -- --test-threads=1`
  - `make checkall`

### [ARC-098] The kitty key encoder truncates non-BMP codepoints and encodes unknown keys (merges QA-209)
- **Card**: `01a0ef5b25d77e72887733e08c2e5f17`
- **Files**: `src/keyboard.rs:445-482` (`encode_kitty`; `let codepoint: u16` with `c as u16` / `functional as u16` at `:454-458`), tests module `:509+` (`kitty_disambiguate_mode` `:629`), `tests/test_keyboard_encoding.py`.
- **Steps**:
  1. At the top of `encode_kitty`, after the text-like early return, add `if matches!(ev.key(), TermKey::Unknown) { return; }`. A `Char` whose codepoint is not a valid `char` (`char::from_u32` → `None`) also returns empty, instead of today's `CSI 0;…u`.
  2. Change `let codepoint: u16` to `u32`, and use `c as u32` and `functional as u32`. The `format!` calls need no change.
  3. Tests in `keyboard.rs`:
     - `kitty_encodes_astral_codepoint_untruncated`: flags 1, `TermKeyEvent::char_('\u{1D54F}', CTRL)` → `b"\x1b[120143;5u"` (0x1D54F = 120143; the truncated value was 54607, matching the audit probe).
     - `kitty_unknown_key_encodes_nothing`: `TermKeyEvent { key: 0, .. }` and `key: 57437` (not a `TermKey` discriminant, so `from_raw` gives `Unknown`) → empty under flags 1, and also under flags 0.
     - `kitty_invalid_char_codepoint_encodes_nothing`: `codepoint: 0xD800`.
  4. Python test in `test_keyboard_encoding.py`: `test_kitty_astral_codepoint` via `term.process(b"\x1b[>1u")`, then `encode_key(CHAR, CTRL, 0x1D54F) == b"\x1b[120143;5u"`.
  5. CHANGELOG `[Unreleased]` → `### Fixed`: "Kitty key encoding reports non-BMP codepoints whole, and unknown keys encode to nothing in both regimes (ARC-098/QA-209)."
- **Method**:
  - Kitty CSI-u parameters are Unicode scalar values (up to 0x10FFFF), so `u16` was simply wrong.
  - Empty output for `Unknown` matches the documented "Empty output means the key has no encoding" contract (`encode_key` doc `:481-483`) and the legacy branch.
  - This is the same file as ARC-093, which lands first.
  - Parsight: `get_symbol_context encode_kitty`.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize keyboard`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,ffi ffi` (FFI encode path; drop `,ffi` if ARC-112 has not landed)
  - `make dev` then `uv run pytest tests/test_keyboard_encoding.py -v`
  - `make checkall`

### [ARC-114] FFI observer events are Debug-formatted text, so there is no structured C/Swift event channel
- **Card**: `01a0ef5b29377f91b40e528938871e13`
- **Decision**: **D4 gates the whole remedy (the payload half of prior ARC-063).** There is no non-gated half. The text payload is already documented as diagnostic (`docs/FFI_GUIDE.md:392`, header `:298`). Implement nothing, and report.
- **Files**:
  - `src/ffi.rs`:
    - `term_event_cb` `:242-243`
    - `TerminalObserverVtable` `:260-273`
    - `unsafe impl Send/Sync` `:278-279`
    - `FfiObserver` `:286`
    - `call_callback` `:298-307`: `format!("{:?}", event)`. An event whose text contains a NUL is **silently dropped** by `CString::new`; include this in the report.
    - `terminal_add_observer` `:874`, `terminal_remove_observer` `:893`
  - `include/terminal_core.h:284-335,562,572`, `docs/FFI_GUIDE.md:348-395`.
- **Steps (gated v4 plan, to report only):**
  1. Add a `repr(C)` `TermEvent { kind: u16, _pad: u16, payload_len: u32, payload: *const u8 }`. The kind is one `TERM_EVENT_*` constant per `TerminalEventKind`. The payload is a versioned, documented JSON object for events with fields, valid only during the callback.
  2. Add a new vtable slot `on_event_v2: Option<extern "C" fn(*mut c_void, *const TermEvent)>`, with the existing text slots kept as diagnostics.
  3. Batch it with ARC-101 and ARC-112 under one `TERM_CORE_ABI_VERSION = 4`, with layout asserts and a DOC-103 v4 row.
- **Method**: one breaking revision instead of three, per AUDIT.md's batch rule.
- **Verify** (when D4 is approved):
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,ffi ffi`
  - `make ffi-header`, then `make ffi-header-check ffi-surface-check`
  - `make xcframework`
  - `make checkall`

### [ARC-115] `checkall` runs the mutating `lint` target with `--fix`
- **Card**: `01a0ef5b2c1076b188a8bb56d543b421`
- **Files**:
  - `Makefile`:
    - `lint` `:276-279`: clippy `--fix --allow-dirty --allow-staged`, then `cargo fmt`
    - `lint-python` `:281-285`: `ruff format .`, `ruff check --fix .`, `pyright .`
    - check-only `clippy` `:296-298`
    - `checkall` `:366`
    - help text `:55`
- **Steps**:
  1. Add the target:
     ```make
     lint-check: ## Non-mutating lint gate (what checkall runs)
     	cargo fmt -- --check
     	cargo clippy --all-targets --features python,streaming,mux,serde,streaming-bin -- -D warnings
     	uv run ruff format --check .
     	uv run ruff check .
     	uv run pyright .
     ```
     Copy the clippy feature list from `:298` exactly, including any `mux-bin`/`ffi` additions from ARC-106/ARC-112 if they landed first.
  2. `checkall` (`:366`): replace `lint lint-python` with `lint-check`. Keep `lint` and `lint-python` as the auto-fix targets.
  3. Help text `:55`: `checkall` becomes "All quality checks (non-mutating; run `make lint lint-python` to auto-fix)". DOC-128 later adds help coverage.
  4. `CLAUDE.md` Code Quality block: add `make lint-check # non-mutating lint gate (used by checkall)`.
- **Method**:
  - A gate should report failures, not rewrite the tree. A mutating `checkall` also dirtied worktrees during verification and hid formatting drift in CI-equivalent runs.
  - **Pitfall:** a developer who relied on `checkall` auto-formatting now sees a failure. The help text names the fix command.
- **Verify**:
  - `make lint-check`
  - `make checkall && git status --porcelain`: must print nothing, which proves `checkall` is non-mutating
  - Red check: `echo 'fn  x(){}' >> /tmp/probe.rs` does not apply. Instead, confirm `cargo fmt -- --check` exits non-zero on a deliberately misformatted scratch copy.

### [ARC-116] tokio `test-util` is in `[dependencies]`, and `serde_yaml_ng` is unconditional
- **Card**: `01a0ef5b2ef57671b9fffad2a30d7936`
- **Files**:
  - `Cargo.toml`: `serde_yaml_ng = "0.10.0"` `:82` (non-optional); tokio `:88` with `"test-util"` in the main dependency's feature list; `[dev-dependencies]` `:188-195` (tokio already has `["full","test-util"]`).
  - `src/macros.rs:156-179`: `save_yaml`, `load_yaml`, `to_yaml`, `from_yaml` (public API); test `:415-423`.
  - YAML users: `src/python_bindings/types/recording.rs:365-389` (Python `Macro.save_yaml`/`load_yaml`/`to_yaml`/`from_yaml`), `src/bin/streaming_server/main.rs:377`, `tests/test_macros.py:62`.
  - tokio test-util API users: only `src/streaming/rate_limit.rs:91,95,107,109,119,124`, all inside `#[cfg(test)]`.
- **Steps**:
  1. `Cargo.toml:88`: remove `"test-util"` from the main tokio feature list. The dev-dependency already provides it for `cargo test`.
  2. `Cargo.toml:82`: `serde_yaml_ng = { version = "0.10.0", optional = true }`. Add the feature `macro-yaml = ["dep:serde_yaml_ng"]`.
     - `dep:` is safe here, because nothing references a `serde_yaml_ng` implicit feature (`grep -n serde_yaml_ng Cargo.toml`).
     - Add `"macro-yaml"` to `python` (`:223`), `python-test` (`:228`) and `streaming-bin` (`:246`).
  3. Gate the four YAML methods in `src/macros.rs` and the Rust test `test_yaml_serialization` with `#[cfg(feature = "macro-yaml")]`. The serde derives on `Macro` stay; `serde` is unconditional.
  4. `scripts/check_features.sh`: add `assert_absent rust-only serde_yaml_ng` and `assert_absent sim serde_yaml_ng`, using the existing helper.
  5. CLAUDE.md feature table: add a `macro-yaml` row. CHANGELOG `[Unreleased]` → `### Changed` (**breaking for Rust embedders on `rust-only`/`sim`**): "`Macro::{save_yaml, load_yaml, to_yaml, from_yaml}` need the new `macro-yaml` feature (enabled by `python` and `streaming-bin`); `rust-only`/`sim` builds no longer compile `serde_yaml_ng`. tokio's `test-util` is dev-only (ARC-116)."
- **Method**:
  - Test-only and feature-specific dependencies should not ship in every profile.
  - par-term (`rust-only, pty_session, screenshot`) does not call the YAML API (grep of `../par-term` found no `load_yaml`/`from_yaml` on core types).
  - Parsight: `get_symbol_context load_yaml` and `get_impact to_yaml`, to confirm the caller list above.
- **Verify**:
  - `make check-features`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming rate_limit` (test-util still available to tests)
  - `cargo test --lib --no-default-features --features python-test macros`
  - `cargo build --no-default-features --features streaming-bin`
  - `make dev` then `uv run pytest tests/test_macros.py -v`
  - `make checkall`

### [ARC-117] 17 `#[macro_export]` macros leak out of `src/python_bindings/common.rs`
- **Card**: `01a0ef5b324272e39c49750be04a19d6`
- **Files**:
  - `src/python_bindings/common.rs` (2,786 lines) has `#[macro_export]` at the line before each `macro_rules!`:

    | Line | Macro | Line | Macro |
    |---|---|---|---|
    | `:48` | `impl_terminal_simple_getters` | `:1264` | `impl_terminal_recording` |
    | `:103` | `impl_terminal_query_getters` | `:1358` | `impl_terminal_cell_line_queries` |
    | `:316` | `impl_terminal_color_setters` | `:1563` | `impl_terminal_content_misc` |
    | `:447` | `impl_terminal_state_setters` | `:1716` | `impl_terminal_search_select` |
    | `:685` | `impl_terminal_static_helpers` | `:1862` | `impl_terminal_debug_snapshots` |
    | `:742` | `impl_terminal_sixel_graphics` | `:1923` | `impl_terminal_file_transfer` |
    | `:886` | `impl_terminal_kitty_file_media` | `:2078` | `impl_terminal_exports` |
    | `:943` | `impl_terminal_badge_session` | `:2522` | `impl_terminal_screenshot_methods` |
    | `:1093` | `impl_terminal_progress_notifications` | | |

  - Invocations: `src/python_bindings/terminal/mod.rs:55,79-93` (16 macros for `PyTerminal`) and `src/python_bindings/pty.rs:47-48,72-86` (all 17 for `PyPtyTerminal`). All use the `crate::impl_terminal_X!(Ty);` form.
- **Steps**:
  1. For each of the 17 macros, remove `#[macro_export]`. After its `macro_rules!` block, add `pub(crate) use impl_terminal_X;`. Or collect all 17 in one `pub(crate) use { … };` list at the end of `common.rs`; path-based use of a re-exported `macro_rules!` does not depend on textual order.
  2. Rewrite the 33 invocations from `crate::impl_terminal_X!(Ty);` to `crate::python_bindings::common::impl_terminal_X!(Ty);`, or add `use crate::python_bindings::common as bindings_macros;` and call `bindings_macros::impl_terminal_X!(Ty);`.
  3. The 232 `$crate::…` paths inside the macro bodies are absolute and stay valid, so leave them.
  4. Fix the stale doc at `common.rs:1920-1921`, which says `pub(super)`. The item is `pub(crate)` (`terminal/mod.rs:1635`).
  5. CHANGELOG `[Unreleased]` → `### Changed`: "The 17 internal `impl_terminal_*!` binding macros are no longer exported at the crate root (they were unusable outside the crate) (ARC-117)."
- **Method**:
  - `#[macro_export]` puts macros in the public crate-root API and on docs.rs, even though they depend on `pub(crate)` items (`TerminalAccess` `common.rs:23`).
  - **Pitfall:** `python_bindings` compiles only with the `python`/`python-test` features. The terminal-core gate (`--features pyo3/auto-initialize`) does **not** compile these macros, so use the python-test gate below.
  - Parsight: `find_code "impl_terminal_ macro invocation"` scoped to `src/python_bindings`, to confirm there are 33 invocation sites and none outside `src/python_bindings/`.
- **Verify**:
  - `cargo test --lib --no-default-features --features python-test`
  - `make dev` then `uv run pytest tests/test_terminal.py tests/test_pty.py -v`
  - `cargo doc --no-deps --no-default-features --features python-test 2>&1 | grep -c impl_terminal_` must print 0 in the macro index (inspect `target/doc/par_term_emu_core_rust/index.html` for a Macros section)
  - `make checkall`

### [ARC-119] The shutdown save serializes under the tree lock
- **Card**: `01a0ef5b35107ff2bd86bed835507e0a`
- **Files**:
  - `src/mux/server.rs:185-205`, where `run_persisting` calls `crate::mux::persist::save_to_with_origin(&self.tree.lock(), &state_path, origin)`. Capture, cwd syscalls, `serde_json` serialization, fsync and rename all run while the tree lock is held.
  - The persist worker is joined before this, at `:357-368`.
  - `src/mux/persist.rs`: `collect_persist_capture` `:348` (under the lock), `PersistCapture::capture()` `:285` (off the lock), `save_to_with_origin` `:722-728`, `write_job` `:740-786`.
  - The off-lock pattern already exists in `dispatch.rs:234-246` and `server.rs:995-1002`.
- **Steps**:
  1. In `persist.rs`, add `pub fn save_off_lock(tree: &parking_lot::Mutex<MuxTree>, target: &Path, origin: SaveOrigin) -> io::Result<()>`. Check the Mutex type `server.rs` uses: `find_symbol tree` in `src/mux/server.rs`. The function:
     1. `let capture = tree.lock().collect_persist_capture();`. The guard drops at the end of the statement.
     2. `let state = capture.capture();`
     3. `write_job(origin, &state, target)`
  2. `server.rs:195-205`: call `save_off_lock(&self.tree, &state_path, origin)`.
  3. Keep `save_to_with_origin` if other callers exist (`get_symbol_context save_to_with_origin`). If this was its only production caller and only tests remain, keep it for the tests and document "holds the lock; prefer `save_off_lock`".
  4. Confirm equivalence. `save_to_with_origin` uses `tree.to_persist_state()`. Read `to_persist_state` and `collect_persist_capture().capture()`, and confirm they produce the same `PersistState`. If they diverge, for example if `to_persist_state` includes something the capture path omits, stop and report.
- **Method**:
  - Shutdown now follows the same capture-under-lock, work-off-lock discipline as every other save.
  - A wedged filesystem during the final cwd or fsync no longer holds the tree mutex. That mutex is shared with the reap tick and any late client command during teardown.
  - **Pitfall:** a command that lands between capture and write is not in the final save. That was already true of the periodic path, and at shutdown clients are disconnecting anyway.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::persist`
  - `cargo test --test mux_restart --no-default-features --features rust-only,mux,serde -- --test-threads=1`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde` (`mux-bin` after ARC-106)
  - Windows VM (orchestrator)
  - `make checkall`

### [ARC-120] The build stamp does not check that the git toplevel matches the manifest dir
- **Card**: `01a0ef5b380771e292822d293f090e2b`
- **Files**: `build.rs:51-76` (`emit_build_stamp`, which watches `.git/HEAD` only if it exists), `:125-151` (`git_short_sha`: `git rev-parse --short HEAD` and `git status --porcelain`, both run in the build-script cwd). `PAR_TERM_CORE_BUILD_SHA` is consumed at `src/mux/mod.rs:64`.
- **Steps**:
  1. At the top of `git_short_sha`, run `git rev-parse --show-toplevel`. Canonicalize it, and compare it with the canonicalized `std::env::var("CARGO_MANIFEST_DIR")`. If they are not equal (the crate is vendored inside another repo, or built from a crates.io tarball extracted in a repo), return `None`. The caller then falls back to `source_digest()` (`:66-73`), which is the correct identity for a non-repo build.
  2. Use `std::fs::canonicalize` on both paths, because macOS `/private/tmp` symlinks otherwise produce false mismatches.
  3. **Observed, report only:** in a *linked worktree*, `.git` is a file, so `Path::new(".git/HEAD").exists()` (`:52`) is false. No `rerun-if-changed` is emitted, and the stamp goes stale across commits in worktrees. The fix would resolve `git rev-parse --git-dir` and watch `<git-dir>/HEAD`. Do not widen ARC-120 to it without orchestrator approval.
- **Method**:
  - The git toplevel must equal the crate root for git identity to describe this crate.
  - Parsight is not applicable, because `build.rs` is not in the lib graph. Read the file.
- **Verify**:
  - `cargo build --no-default-features --features rust-only` (the stamp still emits in the repo)
  - Vendored scenario:
    1. `D=$(mktemp -d)`, then `git init "$D"`
    2. `git -C /Users/probello/Repos/par-term-emu-core-rust archive HEAD | tar -x -C "$D/vendor"` (after `mkdir "$D/vendor"`)
    3. `git -C "$D" add -A && git -C "$D" commit -qm x`
    4. `cargo build --manifest-path "$D/vendor/Cargo.toml" --no-default-features --features rust-only -vv 2>&1 | grep PAR_TERM_CORE_BUILD_SHA`
    5. The value must be the source digest, not the outer repo's short SHA
  - `make checkall`

### [ARC-121] The `mux_hook_report` and `mux_parse_command` fuzz targets are missing from CI and `make fuzz-all`
- **Card**: `01a0ef5b3bf879d3bafae8c5eeef0a1a`
- **Files**:
  - `.github/workflows/fuzz.yml:19`: `target: [terminal_process, sixel, kitty, apc_filter, tmux_control]`
  - `Makefile`: per-target recipes `:916-929` (`FUZZ_SECONDS ?= 60` `:914`), and `fuzz-all` `:931`, whose help text says "all five"
  - `fuzz/Cargo.toml`: 7 `[[bin]]` entries, including `mux_parse_command` `:64` and `mux_hook_report` `:71`, with features `["rust-only","mux"]` `:22`
  - `fuzz/fuzz_targets/mux_parse_command.rs`, `mux_hook_report.rs`
- **Steps**:
  1. `fuzz.yml:19`: append `mux_parse_command, mux_hook_report` to the matrix.
  2. `Makefile`: add `fuzz-mux_parse_command` and `fuzz-mux_hook_report` recipes identical in shape to `:916-929` (`cargo +nightly fuzz run <t> -- -max_total_time=$(FUZZ_SECONDS) -rss_limit_mb=512`), each with a `## …` help comment.
  3. Add both to `fuzz-all` (`:931`) and change its help text to "all seven fuzz targets".
  4. Keep `fuzz/Cargo.toml` on the `mux` feature. It needs only the library, so it is unaffected by ARC-106's `mux-bin`.
- **Method**:
  - The two targets exist (ENH-018) but never run, so parser regressions in the daemon's highest-churn input surface go unexercised.
  - This softly blocks DOC-121, which then references the new make targets.
  - **Pitfall:** the `fuzz/` crate must stay detached from the workspace (see the cargo-fuzz memory note: an empty `[workspace]` table in `fuzz/Cargo.toml`). Do not touch it.
- **Verify**:
  - `make -n fuzz-all` (dry-run lists 7 targets)
  - If nightly and cargo-fuzz are installed: `make fuzz-mux_parse_command FUZZ_SECONDS=15` and `make fuzz-mux_hook_report FUZZ_SECONDS=15`
  - Validate the workflow YAML
  - `make checkall`

### [ARC-118] Root clutter
- **Card**: `01a0ef5b3f0c70e3a83d551b87c23695`
- **Decision**: **D5 gates deleting the tracked `debug/` scripts.** Run this entry last across all domains. Nothing else in it changes the repository.
- **Files**:
  - Tracked `debug/diagnose_corruption.py`, `debug/test_debug_features.py`, `debug/test_wide_chars.py`. Nothing outside `debug/` references them: every `debug/` hit in the Makefile and docs is `target/debug/…`.
  - Untracked and gitignored: `.gitignore~` (`.gitignore:29` `*~`) and `.coverage` (`.gitignore:43`).
  - **Keep** these:
    - `theme.css`: `Makefile:739-740` copies it to `web_term/theme.css`, which is tracked and byte-identical.
    - `AGENTS.md`: 16 bytes, `read @CLAUDE.md`, a deliberate pointer file for other agents.
    - `AUDIT.md`, `AUDIT-REMEDIATION.md`, `AUDIT-REMEDIATION-PLAN.md`: owned by the audit pipeline.
- **Steps**:
  1. **Untracked files:** do **not** delete `.gitignore~` or `.coverage`. The fix agent did not create them, and deleting files it did not create needs the user's confirmation. List them in the report. They are gitignored, so leaving them changes no commit.
  2. **If D5 is approved:**
     - `git rm -r debug/`.
     - Check that `pyproject.toml`'s ruff and pyright include/exclude lists and `.pre-commit-config.yaml` do not name `debug/` (`grep -n "debug" pyproject.toml .pre-commit-config.yaml`). Drop any entry that becomes stale.
     - `Cargo.toml`'s `exclude` (`:22-47`) does not list `debug/`, so there is no Cargo change.
     - No CHANGELOG entry is needed; these are dev-only scripts.
  3. **If D5 is declined:** close the card with "debug/ kept by decision; theme.css, AGENTS.md and AUDIT-*.md are intentional".
- **Method**: AUDIT.md narrowed this finding to what is actually clutter. The three kept files each have a verified consumer.
- **Verify** (D5 approved): `git ls-files debug/` prints nothing; `make checkall` (proves nothing imported the scripts).

---

## Phase 3c — Code Quality (parallel domain; internal order as listed)

> Line numbers are from f6535f2. Phase 1 (SEC-125/126/127/…) and Phase 2 (ARC-090 `mutate_layout`, ARC-103 `spawn_and_wire`, ARC-089) rewrite `src/mux/{server,tree,pane,dispatch,command}.rs`, `src/pty_session.rs` and `src/python_bindings/pty.rs` before this phase. Re-read every file with parsight `get_source_window` (repository_id `par-term-emu-core-rust`) before editing, and re-locate symbols with `find_symbol` rather than trusting a line number.
>
> **Batches inside 3c** (one agent owns each chain; do not parallelize inside a chain):
> - QA-184 → QA-198 → QA-191 (same runtime concern, `mux_factory.rs`/`session.rs`/`main.rs`).
> - QA-185 → QA-186 (`tests/common/mod.rs`).
> - QA-199 + QA-207 in one commit series (`src/mux/server.rs` read loop), after SEC-127.
> - **QA-201 + QA-208 + QA-215 are one `src/ffi.rs` batch, executed at QA-201's slot, internal order QA-215 → QA-208 → QA-201.** QA-210 (4-line deletion) lands before QA-201's lint attribute so the python-feature clippy run stays clean.
> - QA-195 → QA-205 (`src/python_bindings/pty.rs`).
> - QA-202 runs **last**, after every other fix in every domain touching the split files.
>
> **Standing rules**: code fixes add CHANGELOG bullets under `## [Unreleased]` only. Any new `/// cap:` constant needs `make caps-table` in the same commit or `caps-table-check` in `checkall` fails. Any new Python-visible property needs a `docs/API_REFERENCE.md` bullet in the same commit or `stub-check` (`scripts/check_api_reference.py`, bidirectional Properties check) fails. Stubs regenerate only via `make dev-streaming && make stubs`.

---

### [QA-182] `refresh-client -C`/`-p` sizes have no upper bound: u16 pixel overflow (reproduced), and a grid allocation that can abort the daemon
- **Card**: `01a0ef5b422973a28b227c8befb9f128`
- **Files**:
  - `src/mux/command.rs:521-543` (`Args::size_pair`, rejects only 0), `:924-930` (`parse_refresh_client`), tests `:2314-2359` (`parses_refresh_client_with_and_without_a_size_report`, `refresh_client_size_report_rejects_malformed_values`).
  - `src/mux/pane.rs:422-432` (`MuxPane::resize_with_cell_pixels`, `cols * cell_w` in u16 at `:430`).
  - `src/pty_session.rs:559-560` (spawn `PtySize`), `:1126-1127` (`resize`), `:1183-1196` (`resize_with_pixels`, caches `pixel_width / cols`).
  - `src/python_bindings/pty.rs:196-201` (`resize`, `cols as u16`), `:212-229` (`resize_pixels`, four `as u16` casts). `:237` (`send_resize_pulse`) casts values read back from a u16 terminal and cannot truncate; leave it.
  - `src/mux/tree.rs:1281-1311` (`resize_window`, `set_client_cell_pixels`) and `:1341-1401` (`apply_cell_pixels`, `sync_pane_sizes`) are the unbounded consumers; no code change needed there once the parser bounds and the u32 math land.
  - `src/grid/scroll.rs:286` is where an oversized grid allocates (`cells.resize(cols * rows, …)`).
- **Decision on caps** (made here, not left open):
  - The streaming bounds are `MAX_COLS = 1000`, `MAX_ROWS = 500` at `src/streaming/server.rs:159-164`, enforced by `validate_terminal_size` (`:167`). They **cannot be reused by path**: `crate::streaming` is compiled only with `streaming`/`python`/`python-test` (`src/lib.rs:85`), so a `rust-only,mux` build has no `streaming::server`.
  - Add mux constants **with the same values**, so a mux-backed streamer (which forwards a viewer resize already bounded by `validate_terminal_size` at `server.rs:1518` as `refresh-client -C {cols}x{rows}` from `mux_factory.rs:357`) is never rejected by the daemon.
  - Pixel bound: `1..=512` per axis. par-term sends `-p` from `par-term-mux/src/client.rs:196`; 512 px cells cover any real font size.
- **Steps**:
  1. In `src/mux/command.rs`, above `impl Args` (near `:379`), add:
     ```rust
     /// cap: Columns a par-mux client may report for a window grid (`refresh-client -C`).
     pub(crate) const MAX_CLIENT_COLS: u16 = 1000;
     /// cap: Rows a par-mux client may report for a window grid (`refresh-client -C`).
     pub(crate) const MAX_CLIENT_ROWS: u16 = 500;
     /// cap: Pixels per cell axis a par-mux client may report (`refresh-client -p`).
     pub(crate) const MAX_CELL_PIXELS: u16 = 512;
     ```
     The names differ from the streaming `MAX_COLS`/`MAX_ROWS` so the generated caps table has no duplicate names.
  2. Change `size_pair(&self, flag_name: &str)` to `size_pair(&self, flag_name: &str, max: (u16, u16))`. After the zero check, reject `width > max.0 || height > max.1` with `format!("{}: {flag_name} size exceeds {}x{}: {raw}", self.name, max.0, max.1)`.
  3. In `parse_refresh_client`: `size: a.size_pair("-C", (MAX_CLIENT_COLS, MAX_CLIENT_ROWS))?`, `cell_pixels: a.size_pair("-p", (MAX_CELL_PIXELS, MAX_CELL_PIXELS))?`. `size_pair` has no other callers (verify: `get_symbol_context size_pair` → only `parse_refresh_client`).
  4. In `src/pty_session.rs`, add one free function next to `send_sigwinch` (`:167`):
     ```rust
     /// Pixel extent of `cells` cells at `cell_px` pixels each, saturated to the
     /// `u16` a `winsize`/`PtySize` field can hold.
     pub(crate) fn pixel_extent(cells: u16, cell_px: u16) -> u16 {
         u16::try_from(u32::from(cells) * u32::from(cell_px)).unwrap_or(u16::MAX)
     }
     ```
     Use it at `:559-560` (`pixel_extent(self.cols, self.cell_pixel_width)` / rows) and `:1126-1127`.
  5. In `src/mux/pane.rs:430`, replace `cols * cell_w, rows * cell_h` with `crate::pty_session::pixel_extent(cols, cell_w), crate::pty_session::pixel_extent(rows, cell_h)` (`mux` enables `pty_session`).
  6. In `src/python_bindings/pty.rs`, add a private helper and use it in `resize` and `resize_pixels`:
     ```rust
     fn to_u16(name: &str, value: usize) -> PyResult<u16> {
         u16::try_from(value)
             .map_err(|_| PyValueError::new_err(format!("{name} {value} exceeds {}", u16::MAX)))
     }
     ```
  7. Run `make caps-table` (three new `/// cap:` constants; `gen_caps_table.py` matches `pub(crate) const`).
  8. CHANGELOG `[Unreleased]` → `### Fixed`: "par-mux `refresh-client -C` is capped at 1000×500 and `-p` at 512×512; pane pixel extents saturate instead of overflowing (QA-182)."
  9. Tests:
     - `src/mux/command.rs` `refresh_client_rejects_sizes_over_the_caps`: `-C 1001x40`, `-C 120x501`, `-C 65535x65535`, `-p 513x20`, `-p 10x513` are `Err`; `-C 1000x500 -p 512x512` is `Ok` with those exact values.
     - `src/pty_session.rs` `pixel_extent_saturates_instead_of_overflowing`: `pixel_extent(80, 10) == 800`, `pixel_extent(2000, 40) == u16::MAX`, `pixel_extent(u16::MAX, u16::MAX) == u16::MAX`.
     - `src/mux/tree.rs` `oversized_cell_pixels_refit_does_not_panic`: the audit probe. Build a tree with the test pane factory the module already uses (verify: `get_source_window src/mux/tree.rs` around the `resize_window_refits_every_pane_terminal` test at `:2850`), create a session, `resize_window(w, 2000, 50)`, `set_client_cell_pixels(40, 40)`, then `resize_window(w, 2000, 50)` again. Assert no panic and the pane terminal reports `(2000, 50)`. This drives the tree API directly (below the parser cap) to prove the arithmetic, which is what panicked at `pane.rs:430:45`.
     - `tests/test_pty.py` `test_resize_rejects_dimensions_over_u16`: `PtyTerminal(80, 24).resize(70000, 24)` and `.resize_pixels(80, 24, 70000, 480)` raise `ValueError`.
- **Method**:
  - Two independent defects: the cap stops the allocation abort (uncatchable), and the u32 math stops the overflow (1000 cols × 512 px = 512 000 still exceeds u16, so the cap alone does not fix `pane.rs:430`).
  - Callers of the pixel math: `get_symbol_context resize_with_cell_pixels` → `MuxTree::apply_cell_pixels`, `MuxTree::sync_pane_sizes`; `get_symbol_context resize_with_pixels` (file `src/pty_session.rs`) → `PyPtyTerminal::resize_pixels`, `MuxPane::resize_with_cell_pixels`, and tests.
  - Pitfall: `resize_with_pixels` stores `pixel_width / cols` as the cached cell size. With a saturated extent the cached cell size becomes smaller than reported; that only affects the fallback path in plain `resize()`, and the mux always calls `resize_with_cell_pixels`. Acceptable; do not add more machinery.
  - Pitfall (report, do not fix here): `MuxTree::from_persist_state` (`src/mux/persist.rs:409`, spawns at `:476-490` with `window.cols/rows` from the state file) accepts an unbounded size from an on-disk file the code already treats as untrusted (`MAX_PERSISTED_SCROLLBACK_CELLS`). File a follow-up card rather than widening this fix.
  - ARC-090 (Phase 2) routed layout mutations through `mutate_layout`; confirm `sync_pane_sizes` is still the single re-fit site before editing (`get_symbol_context sync_pane_sizes`; at HEAD it has 13 callers, all in `tree.rs` plus `persist.rs:from_persist_state`).
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session::tests::pixel_extent`
  - `make dev && uv run pytest tests/test_pty.py -v`
  - Windows VM (mux + `pty_session.rs` changed), from the main checkout: both `cargo check --all-targets` and `cargo check --lib --tests --no-default-features --features rust-only,mux,serde`.
  - `make checkall` (includes `caps-table-check`).

### [QA-184] `MuxSessionFactory::create_session` does blocking socket I/O on a tokio worker with no reply deadline
- **Card**: `01a0ef5b454c74c08dc74f54af7dcb88`
- **Files**:
  - `src/streaming/mux_factory.rs:137-167` (`MuxLines`, `command()` loops on `reader.next()` with no timeout), `:113-133` (`pane_for`), `:256-321` (`create_session`: connect + up to three round trips), `:392-440` (`spawn_drain` consumes the same `MuxLines`).
  - `src/streaming/server.rs:569-642` (`resolve_session`, sync, calls `factory.create_session` at `:608`), `:1233-1252` (`prepare_ws_session`), callers `:1197-1206` (`handle_connection_ws`), `:1210-1220` (TLS), `:2190-2204` (axum), test `:3359`.
  - Test helpers: `src/streaming/mux_factory.rs:609-626` (`daemon()`), `:657-668` (`wait`, `std::thread::sleep`), `:677-689` (`type_line`, blocking lock+write), all inside `#[tokio::test(flavor = "multi_thread")]` bodies (`:704`, `:734`, `:779`, `:817`, `:850`, `:1003`).
  - Reference pattern: `src/mux/client.rs:18` (`REPLY_TIMEOUT = 10 s`), `:185-199` (`recv_timeout`).
- **Steps**:
  1. Reply deadline (portable). Named pipes reject I/O timeouts (`src/mux/server.rs:466-474` comment; interprocess 2.4 `set_recv_timeout` is `Unsupported` on Windows), so do **not** rely on `set_recv_timeout`. Adopt `MuxClient`'s shape:
     - Replace `type MuxLines = Lines<BufReader<LocalStream>>` with a reader thread started right after connect in `create_session`: it reads lines and forwards `io::Result<String>` into a `std::sync::mpsc::Receiver`. Name it `MuxLines` still (a struct wrapping the `Receiver`).
     - Add `const MUX_REPLY_TIMEOUT: Duration = Duration::from_secs(10);` (same value as `client.rs:18`).
     - `command()` takes `&MuxLines` and uses `recv_timeout` against one deadline computed at entry (`deadline.saturating_duration_since(Instant::now())` each iteration, so interleaved `%output` lines cannot extend it). Timeout → `io::ErrorKind::TimedOut` naming the command; `Disconnected` → the existing `ConnectionAborted`.
     - `spawn_drain` iterates the same receiver (`for line in lines.rx.iter()`), so the handshake and the live mirror share one reader thread and no line is lost at the hand-off.
  2. Off-runtime creation. Add `#[derive(Clone)]` to `ConnectionParams` (`server.rs:230`). Split `prepare_ws_session` into:
     - `async fn resolve_session_off_runtime(self: &Arc<Self>, params: &ConnectionParams) -> Result<Arc<StreamSessionState>>`: `let this = Arc::clone(self); let params = params.clone(); tokio::task::spawn_blocking(move || this.resolve_session(&params)).await.map_err(|e| StreamingError::ServerError(e.to_string()))?`
     - the existing sync slot reservation, taking the resolved session.
     The three transport callers `.await` the first half. Keep SEC-011's order: the global guard is reserved **before** resolve (the guard stays on the async side; only `Arc<Self>` moves into the closure).
  3. Leave `main.rs:455` (startup `resolve_session` for the `default` shell session) as is: it runs before any client is served.
  4. Test helpers: convert `wait` and `type_line` users to plain `#[test]` functions where the body has no `.await` (verify each of the six tests: `grep -n '\.await' src/streaming/mux_factory.rs` inside `mod tests`). Where an async body is needed, use `tokio::time::sleep` in `wait` and move `type_line`'s write into `tokio::task::spawn_blocking`.
  5. New test in `mux_factory.rs` tests: `create_session_times_out_against_a_silent_daemon`. Bind a raw `interprocess` listener at a temp socket (`crate::mux::ipc::bind_local_listener`, `src/mux/ipc.rs:37`) that accepts and never replies. Call `create_session` with `MuxPaneSelector::FromSessionId` and session id `"main"` (forces `list-panes`). Assert `Err` whose text contains `"no reply"`/`TimedOut`, and that elapsed is `< MUX_REPLY_TIMEOUT + 5 s`. Use a test-only shorter timeout if the module exposes one (`#[cfg(test)] const MUX_REPLY_TIMEOUT: Duration = Duration::from_millis(500)`), so the test runs in under a second.
  6. CHANGELOG `[Unreleased]` → `### Fixed`: "Mux-backed streaming sessions are created off the tokio runtime with a 10 s reply deadline; a wedged par-mux daemon no longer parks runtime workers (QA-184)."
- **Method**:
  - Enumerate `command` callers: `get_symbol_context command` (file `src/streaming/mux_factory.rs`) → `create_session`, `pane_for` only. Enumerate `resolve_session` callers: `server.rs:1243` (`prepare_ws_session`), `main.rs:455`, `mux_factory.rs:651` (test `mirror()`), tests `:3370,:3381`.
  - Pitfall: a timed-out handshake leaves the reader thread blocked on a wedged daemon until the socket closes. Drop the writer half on error so a live daemon sees EOF; a truly wedged daemon costs one parked OS thread, not a runtime worker, which is the defect being fixed.
  - Pitfall: `block_in_place` is not an option; some server tests run on the current-thread runtime (`#[tokio::test]` at `server.rs:3394`), where it panics.
  - Whether this causes the macOS CI hang is unproven (AUDIT); do not claim it in the commit message.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,streaming,mux,serde streaming::mux_factory -- --test-threads=1`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::`
  - `make test-rust-streaming`
  - Windows VM (the reader-thread change is exercised on named pipes): `cargo check --all-targets` and `cargo check --lib --tests --no-default-features --features rust-only,mux,serde`; also `cargo check --lib --tests --no-default-features --features rust-only,streaming,mux,serde`.
  - `make checkall`

### [QA-198] Blocking lock and I/O inside tokio tasks; `--command` bypasses the input queue
- **Card**: `01a0ef5b49887ac0beb832e302872a33`
- **Blocked by**: QA-184 (same file; lands on its restructured `setup_session`).
- **Files**:
  - `src/streaming/mux_factory.rs:352-364` (`setup_session`'s `tokio::spawn` loop: parking_lot `writer.lock()` + `writeln!` + `flush` on a runtime worker).
  - `src/bin/streaming_server/main.rs:615-638` (`serve_until_ctrl_c`: after a 1 s sleep, locks `factory.pty_sessions`, then the `PtySession`, then its writer, and writes directly).
  - `src/streaming/session.rs:363` (`StreamSessionState::enqueue_pty_input`, `pub(crate)`), `src/streaming/server.rs:539` (`get_session`, pub).
- **Steps**:
  1. In `setup_session`, keep the async `rx.recv().await`, but move each write off the runtime:
     ```rust
     let writer = Arc::clone(&writer);
     let sent = tokio::task::spawn_blocking(move || {
         let mut stream = writer.lock();
         writeln!(stream, "refresh-client -t %{pane} -C {cols}x{rows}").and_then(|()| stream.flush())
     })
     .await;
     if !matches!(sent, Ok(Ok(()))) { break; }
     ```
     Resizes are rare, so one blocking-pool hop per resize is fine.
  2. The binary cannot call `pub(crate) enqueue_pty_input`. Add one public method on `StreamSessionState` (`src/streaming/session.rs`, next to `enqueue_pty_input`):
     ```rust
     /// Queue `bytes` for the PTY through the session's serialized input path —
     /// the same path client `Input` messages take.
     pub fn send_input(self: &Arc<Self>, bytes: Vec<u8>) { self.enqueue_pty_input(bytes) }
     ```
  3. In `main.rs` replace the direct-write block with: `if let Some(session) = streaming_server.get_session("default") { session.send_input(format!("{command}\n").into_bytes()); }` after the existing 1 s settle sleep. Drop the now-unused `factory_ref` clone if nothing else uses it (orphan created by this change).
  4. CHANGELOG `[Unreleased]` → `### Changed`: "`StreamSessionState::send_input` queues bytes on the session input path; `par-term-streamer --command` uses it (QA-198)."
  5. Test: `src/streaming/session.rs` tests `send_input_is_written_through_the_drain`: `set_pty_writer` with a `Vec<u8>`-backed writer wrapped so the test can read it back (the existing `StalledWriter` pattern at `:883` shows how), call `send_input(b"echo hi\n".to_vec())`, poll with a 5 s deadline until the writer holds exactly those bytes.
- **Method**:
  - The only other runtime-blocking site in this path is the one QA-184 already moved. Confirm with `get_symbol_context setup_session` (file `src/streaming/mux_factory.rs`) that no other spawned task takes `writer.lock()`.
  - Pitfall: `--command` conflicts with `--mux-socket` at the CLI (`src/bin/streaming_server/cli.rs:194`), so this path only runs in shell mode with `BinarySessionFactory`; the `default` session exists because `main.rs:455` created it.
  - Ordering: QA-191 then changes the drain loop itself; do not touch `session.rs:395-460` here.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,streaming,mux,serde streaming::mux_factory -- --test-threads=1`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::session`
  - `make test-rust-streaming`
  - `cargo build --no-default-features --features streaming-bin --bin par-term-streamer` is a link step; use `cargo check --no-default-features --features streaming-bin --bin par-term-streamer` instead.
  - `make checkall`

### [QA-191] PTY input drain busy-polls and retries `try_lock` without bound; it is not the only writer
- **Card**: `01a0ef5b4d0878c3abcb05e067e94e04`
- **Blocked by**: QA-198.
- **Files**:
  - `src/streaming/session.rs:363-470` (`enqueue_pty_input`; drain thread body `:397-460`: `DRAIN_POLL` 10 ms `try_recv` + sleep at `:409-417`, unbounded `try_lock` spin at `:441-446`).
  - Bypass writers: `src/pty_session.rs:1067-1088` (`PtySession::write`, used by Python `PtyTerminal.write`/`write_str`, `src/python_bindings/pty.rs:175,184`), `src/python_bindings/streaming.rs:541-551` (`PyStreamingServer` shares the `PtyTerminal` writer mutex). The reader thread's device-query replies (`pty_session.rs:1021`) also take the writer mutex; that writer is legitimate and stays.
- **Steps**:
  1. Replace the `try_recv` + 10 ms sleep with a blocking wait that still wakes to notice a dropped session:
     ```rust
     const DRAIN_IDLE_WAKE: Duration = Duration::from_millis(250);
     const WRITER_LOCK_WAIT: Duration = Duration::from_secs(1);
     let bytes = match rx.recv_timeout(DRAIN_IDLE_WAKE) {
         Ok(bytes) => bytes,
         Err(RecvTimeoutError::Timeout) => { if weak.strong_count() == 0 { break } continue }
         Err(RecvTimeoutError::Disconnected) => break,
     };
     ```
  2. Replace the `try_lock` spin with `parking_lot::Mutex::try_lock_for(WRITER_LOCK_WAIT)` in a loop that logs each timeout once with `crate::debug_error!("STREAMING", "PTY writer for session {} held > {:?}; input waiting", session.id, WRITER_LOCK_WAIT)` and retries. Never drop the chunk: dropping would reorder or lose keystrokes.
  3. Remove the per-chunk `debug_log!("drain picked up {} B")` (`:418`) and replace the card-narrative comments at `:374-393` and `:398-408`, `:430-440` with constraint statements only: "std channel, not tokio: a parked `blocking_recv` can miss a `try_send` wakeup"; "bounded lock wait: a writer held elsewhere must not strand later chunks silently". (This is QA-207's `session.rs` half; doing it here avoids editing the block twice. QA-207 then skips `session.rs`.)
  4. The "only writer" half: `PtyTerminal.write` is public Python API and cannot be removed. Give Python a queue path instead: add `PyStreamingServer.send_input(data: bytes)` in `src/python_bindings/streaming.rs` that calls `session.send_input` (QA-198) on the `default` session. Google-style docstring with Args/Example. Document in its docstring that mixing `PtyTerminal.write` with client input has no cross-producer ordering, and that `send_input` shares the client queue. `docs/API_REFERENCE.md` gets the method (else `stub-check` fails once stubs regenerate).
  5. CHANGELOG `[Unreleased]`: `### Changed` "The streaming input drain blocks on its channel instead of polling every 10 ms and bounds its writer-lock wait with a logged timeout (QA-191)." `### Added` "`StreamingServer.send_input()` (Python) queues bytes on the client input path."
  6. Tests:
     - Existing: the stalled-writer test around `session.rs:883` must still pass (it proves a held writer does not strand later chunks).
     - New `drain_idles_without_polling`: enqueue nothing, sleep 600 ms, assert the drain thread is still alive and the session's `metrics.errors` is 0; then enqueue a chunk and assert it is written within 1 s (proves the recv_timeout path wakes on send).
     - Python: `tests/test_streaming.py::test_send_input_queues_bytes` only if the file already builds a `StreamingServer` over a `PtyTerminal` (verify: `grep -n 'StreamingServer(' tests/test_streaming.py`); otherwise cover via the Rust test.
- **Method**:
  - `recv_timeout` on a std channel uses a condvar; the file's own comment records that std's wakeup "has no runtime context to lose", so this does not reintroduce card 01a0e80db3e870e282af0cf84405043b's stall.
  - Idle cost drops from ~100 wakes/s to 4 wakes/s per session; keystroke latency loses the up-to-10 ms poll delay.
  - Enumerate writer-mutex users: `get_symbol_context set_pty_writer` (file `src/streaming/session.rs`) and `get_symbol_context get_pty_writer` (file `src/python_bindings/pty.rs`).
  - Pitfall: `weak.upgrade()` inside the timeout arm would keep the session alive for the check; use `weak.strong_count() == 0`.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::session`
  - `cargo test --lib --no-default-features --features rust-only,streaming,mux,serde streaming::mux_factory -- --test-threads=1` (the `stress_input_frames_of_varied_sizes_all_land_exactly_once` test at `mux_factory.rs:851` is the standing repro for this drain)
  - `make test-rust-streaming`
  - `make dev-streaming && make stubs && uv run pytest tests/test_streaming.py -v`
  - `make checkall`

### [QA-185] Mux test harness: unbounded waits, leaked daemons, duplicated helpers, and no per-test timeout in CI
- **Card**: `01a0ef5b50407a52ad9508188d424483`
- **Files**:
  - `tests/common/mod.rs:82-101` (`command()`, `read_line` with no deadline), `:203-208` (`wait_listening` returns silently).
  - Private copies: `tests/mux_agents.rs:24-41` and `tests/mux_hooks.rs:24-42` (`wait_listening`, `sigterm_clean`). `tests/mux_end_to_end.rs:30-50` has its own `connect()` retry loop.
  - `src/streaming/mux_factory.rs:607-626` (`daemon()` detaches `server.run()`).
  - `src/mux/server.rs:1487` and `:1643` (test `serving.join()` unbounded).
  - `.github/workflows/ci.yml:124-171` (Mux job: `timeout-minutes: 20` at `:127`, `cargo test … -- --test-threads=1` at `:166`, mux_factory run at `:171`).
  - New: `.config/nextest.toml`.
- **Steps**:
  1. `tests/common/mod.rs`:
     - Add `pub const REPLY_DEADLINE: Duration = Duration::from_secs(10);` and `pub fn connect(path: &Path) -> (LocalStream, BufReader<LocalStream>)` that retries `connect_local_stream` for 5 s (fold in `mux_end_to_end.rs:30`'s loop), then calls `stream.set_recv_timeout(Some(REPLY_DEADLINE))` (needs `use interprocess::local_socket::traits::Stream as _`). On Windows the call returns `Unsupported`; ignore that error only (`if e.kind() != io::ErrorKind::Unsupported { panic!(…) }`), because nextest's terminate-after (step 4) is the Windows backstop.
     - In `command()`, replace `.expect("read reply")` with `.unwrap_or_else(|e| panic!("no reply to {line:?} within {REPLY_DEADLINE:?}: {e}"))`, so a Unix timeout fails with the command's name.
     - Make `wait_listening` panic after its loop: `assert!(connect_local_stream(path).is_ok(), "daemon never listened on {}", path.display());`.
  2. Migrate test files that open control connections with bare `connect_local_stream` + `BufReader::new` (34 call sites; enumerate with `grep -n 'connect_local_stream(' tests/*.rs`) to `common::connect`. Delete `mux_end_to_end.rs`'s private `connect`.
  3. Delete the private `wait_listening` and `sigterm_clean` in `tests/mux_agents.rs:24-41` and `tests/mux_hooks.rs:24-42`; import `common::{wait_listening, sigterm_clean}` (both files are `#![cfg(all(feature = "mux", unix))]`, matching `sigterm_clean`'s `#[cfg(unix)]`). Their `Control` structs set a 5 s `UnixStream` read timeout already and may stay.
  4. `daemon()` Drop guard in `src/streaming/mux_factory.rs` tests:
     ```rust
     struct TestDaemon {
         shutdown: Arc<AtomicBool>,
         serving: Option<std::thread::JoinHandle<()>>,
         _dir: tempfile::TempDir, // dropped after the join, so the socket path outlives the loop
     }
     impl Drop for TestDaemon {
         fn drop(&mut self) {
             self.shutdown.store(true, Ordering::Relaxed);
             if let Some(serving) = self.serving.take() {
                 let (tx, rx) = std::sync::mpsc::channel();
                 std::thread::spawn(move || { let _ = serving.join(); let _ = tx.send(()); });
                 if rx.recv_timeout(Duration::from_secs(10)).is_err() {
                     eprintln!("mux test daemon did not stop within 10 s");
                 }
             }
         }
     }
     ```
     `daemon()` captures `server.shutdown_handle()` (`src/mux/server.rs:212`) before `std::thread::spawn(move || server.run())`, and returns `(PathBuf, MuxClient, String, TestDaemon)` with the guard **last**, so the control client drops before the join. Update the six `let (_dir, socket, …) = daemon();` call sites (`:705,:735,:780,:819,:1004` and the stress test).
     Fields drop in declaration order after `drop()` runs, so `_dir` is removed after the join.
  5. `src/mux/server.rs` tests: add `fn join_within(handle: JoinHandle<()>, limit: Duration, what: &str)` (same channel pattern, `panic!` naming `what` on timeout) and use it at `:1487` and `:1643`.
  6. nextest config, new file `.config/nextest.toml`:
     ```toml
     # QA-185: a wedged mux test fails by name in ~2 minutes instead of
     # consuming the job's 20-minute budget.
     [profile.default]
     slow-timeout = { period = "30s", terminate-after = 4 }

     [profile.ci]
     slow-timeout = { period = "30s", terminate-after = 4 }
     # PTY spawns contend under parallel threads (card 01a0c64fc43e7913b474c038db0635d5).
     test-threads = 1
     fail-fast = false
     failure-output = "immediate-final"
     ```
  7. `.github/workflows/ci.yml` Mux job (`mux-test`, after "Set up Rust"):
     - Add an install step pinned to a commit SHA per ENH-032 (the repo pins third-party actions): `uses: taiki-e/install-action@<sha> # v2` with `with: { tool: cargo-nextest }`. Resolve the SHA with `gh api repos/taiki-e/install-action/git/ref/tags/v2 --jq .object.sha` (dereference if it is an annotated tag) and verify it before committing.
     - Replace `:166` with `cargo nextest run --profile ci --no-default-features --features rust-only,mux,serde`.
     - Add `cargo test --doc --no-default-features --features rust-only,mux,serde` (nextest does not run doctests; the old `cargo test` did).
     - Replace `:171` with `cargo nextest run --profile ci --lib --no-default-features --features rust-only,streaming,mux,serde -E 'test(/^streaming::mux_factory::/)'`.
     - Keep `timeout-minutes: 20` as the outer bound.
     - Do not add push/PR triggers (that is D1/ARC-104). SEC-135/ARC-104/ARC-105 also edit `ci.yml`: re-read before editing.
  8. No CHANGELOG entry (test/CI only).
- **Method**:
  - Setting the recv timeout on the stream covers every `common::command` caller at once (`mux_daemon` 19, `mux_targets` 17, `mux_restart` 12, `mux_agents`/`mux_hooks` 1 each) without changing its signature.
  - nextest runs each test in its own process, so a hang is killed and named; it also isolates process-env mutation (QA-196), but local `cargo test` still needs QA-196.
  - Pitfall: `wait_listening` now panics; confirm every caller (33, `grep -n 'wait_listening(' tests/*.rs`) expects the daemon to come up. None tests a daemon that must fail to listen (verify while migrating; `mux_nested.rs` refusal tests use `par_mux_env` output, not `wait_listening`).
  - Pitfall: `server.run()` joins the host-probe worker on shutdown; before SEC-133 that can take ~25 s, after it the sweep checks the shutdown flag. The 10 s guard only logs, so a slow join never fails a test.
- **Verify**:
  - `cargo test --no-default-features --features rust-only,mux,serde -- --test-threads=1` (all mux integration targets compile and pass under plain cargo test)
  - `cargo nextest run --profile ci --no-default-features --features rust-only,mux,serde`
  - `cargo test --lib --no-default-features --features rust-only,streaming,mux,serde streaming::mux_factory -- --test-threads=1`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - Windows VM from the main checkout: both `cargo check` sets, then `cargo test --lib --no-default-features --features rust-only,mux,serde mux:: -- --test-threads=1` (the `Unsupported` branch runs there).
  - `make checkall`

### [QA-192] Writer-less sessions close the WebSocket on any input; the frontend reconnect-loops
- **Card**: `01a0ef5b537771808f20fa7e2d7341c5`
- **Files**:
  - `src/streaming/server.rs:1286-1316` (writer-less arm returns `close: true` for Input/Paste/Mouse/FocusChange), `:310-313` (`ClientMessageOutcome { replies, close }`), close handling at `:1964-1972` (tungstenite loop) and `:2296-2305` (axum loop), test `:3394-3430` (`input_without_pty_writer_is_counted_and_closes_the_connection`).
  - `src/bin/streaming_server/main.rs:265` (`default_read_only: false` for every mode, including macro mode, which has no PTY writer: `main.rs:448`).
  - `web-terminal-frontend/lib/terminal-connection.ts:157-168` (`onclose` → `scheduleRetry`); no change needed.
- **Steps**:
  1. In `handle_client_message`, keep the log and the `dropped_messages` increment, but return `ClientMessageOutcome { replies: Vec::new(), close: false }`. Update the comment to the new rule: "Input on a writer-less session is dropped and counted; the connection stays open."
  2. `close` is then never `true`. Remove the field and the two `if outcome.close { … break; }` blocks (orphans created by this change). If another variant still sets it after Phase 1–3b edits (verify: `grep -n 'close: true' src/streaming/server.rs`), keep the field.
  3. `main.rs:265`: `default_read_only: matches!(run_mode, bootstrap::RunMode::Macro { .. }),` (`run_mode` is resolved at `:230`, before the config at `:261`). Macro viewers become read-only, so their input is ignored before it reaches the writer check.
  4. Rename and rewrite the test: `input_without_pty_writer_is_counted_and_dropped`: Input, Paste, Mouse and FocusChange each leave `replies` empty and increment `dropped_messages` (final count 4); Ping on the same session still replies.
  5. CHANGELOG `[Unreleased]` → `### Changed`: "Input to a session with no PTY writer is dropped and counted instead of closing the WebSocket; macro-mode viewers connect read-only (QA-192)." DOC-112 documents the rule in `docs/STREAMING.md`.
- **Method**:
  - The close was meant to make a client "reconnect instead of typing into a void", but a writer-less session is still writer-less after reconnect, so the frontend's auto-retry turns it into a loop.
  - Enumerate other writer-less producers: `get_symbol_context session_has_writer` (file `src/streaming/server.rs`) → handlers at `:1479,:1563,:1599,:1667` already drop silently when no writer; this change aligns the top-level arm with them.
  - No frontend change: nothing in `web-terminal-frontend` reads a read-only flag today (verify: `grep -rn 'readOnly\|read_only' web-terminal-frontend/components web-terminal-frontend/lib`), and the server ignores read-only input.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::server`
  - `make test-rust-streaming`
  - `cargo check --no-default-features --features streaming-bin --bin par-term-streamer`
  - `make test-web` (unchanged frontend; confirms nothing depended on the close)
  - `make checkall`

### [QA-194] Windows `TOKEN_USER` view is misaligned
- **Card**: `01a0ef5b56c87ab19e6d7eb28d44910d`
- **Files**: `src/mux/ipc.rs:188-213` (`token_user_sids_equal`: `&*(server_buf.as_ptr().cast::<TOKEN_USER>())` at `:207-208`), `:215-243` (`token_user_buffer` returns `Vec<u8>`; `vec![0u8; needed]` at `:233`). Also `:160-185` (caller) — all `#[cfg(windows)]`.
- **Steps**:
  1. `token_user_buffer` returns `Option<Vec<u64>>`: allocate `vec![0u64; (needed as usize).div_ceil(8)]`, pass `buffer.as_mut_ptr().cast()` and the byte length `needed` (≤ capacity bytes by construction).
  2. Add `const _: () = assert!(std::mem::align_of::<TOKEN_USER>() <= std::mem::align_of::<u64>());` inside the `#[cfg(windows)]` fn or next to it.
  3. `token_user_sids_equal` keeps the `.cast::<TOKEN_USER>()` view, now over an 8-aligned buffer.
  4. Replace the comments above the three `unsafe` blocks (`:167`, `:206`, `:222`) with `// SAFETY:` comments that state the invariant (handles null-checked, buffer sized by the sizing call and 8-aligned, SIDs point into buffers that outlive the borrow). This also satisfies QA-201's lint on Windows.
  5. Test (`#[cfg(windows)]`, in `ipc.rs` tests): `token_user_buffer_is_aligned_and_self_equal`: open the current process token (`OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, …)`), assert `token_user_buffer` returns a buffer with `as_ptr() as usize % align_of::<TOKEN_USER>() == 0`, and `token_user_sids_equal(token, token)` is `true`. Close the handle.
- **Method**:
  - `TOKEN_USER` contains a pointer, so its alignment is 8 on 64-bit; a `Vec<u8>` allocation guarantees alignment 1 only. `read_unaligned` would copy the struct, but its `Sid` pointer still points into the buffer, so the `Vec<u64>` fix is the cleaner one.
  - Callers: `get_symbol_context token_user_sids_equal` (file `src/mux/ipc.rs`) → the peer-identity check at `:160-185`, used on every Windows connect.
- **Verify**:
  - macOS: `cargo check --lib --no-default-features --features rust-only,mux,serde` (the code is cfg'd out; confirms nothing else moved).
  - Windows VM playbook from the **main checkout** (CLAUDE.md "Local Windows VM"): `cargo check --all-targets`, `cargo check --lib --tests --no-default-features --features rust-only,mux,serde`, then `cargo test --lib --no-default-features --features rust-only,mux,serde mux:: -- --test-threads=1`.
  - `make checkall`

### [QA-195] Raw terminal-lock writes leave the geometry mirror stale
- **Card**: `01a0ef5b5a467f029b422c30c2e3280d`
- **Blocked by**: ARC-103 (its `spawn_and_wire` writes notes through `with_terminal_mut`, removing `src/mux/dispatch.rs:621,889,986`). First confirm those three sites are gone: `grep -n 'terminal().write()' src/mux/dispatch.rs` must be empty.
- **Files** (remaining after ARC-103):
  - `src/mux/pane.rs:283-285` (`MuxPane::terminal()` hands out the raw `Arc<RwLock<Terminal>>`).
  - Mutating raw-lock users in production: `src/mux/tree.rs:1326-1334` (`set_client_colors`), `:1349-1357` (`apply_cell_pixels` colors), `src/mux/persist.rs:522-528` (restore: `restore_for_new_process` + cwd note).
  - `src/pty_session.rs:1370-1372` (`PtySession::terminal()`), `:1410-1417` (`with_terminal_mut`, publishes the mirror), `:71-90` (`GeometryMirror`).
  - `src/python_bindings/pty.rs:41-43` (`term_mut` returns a raw write guard; every macro-generated mutating method goes through it), plus direct `terminal.write()` mutators at `:562, :825, :855, :877, :889, :899, :909, :922, :977, :995, :1230, :1277, :1309, :1340, :1352`.
- **Steps**:
  1. In `src/pty_session.rs` add a guard that publishes on drop:
     ```rust
     /// Exclusive terminal access that republishes the wait-free geometry
     /// mirror when dropped, so `size()`/`cursor_position()` never go stale.
     pub struct TerminalWriteGuard<'a> {
         guard: parking_lot::RwLockWriteGuard<'a, Terminal>,
         geometry: &'a GeometryMirror,
     }
     impl Deref/DerefMut<Target = Terminal> for TerminalWriteGuard<'_> { … }
     impl Drop for TerminalWriteGuard<'_> { fn drop(&mut self) { self.geometry.publish(&self.guard) } }
     impl PtySession { pub fn terminal_write(&self) -> TerminalWriteGuard<'_> { … } }
     ```
     `GeometryMirror` stays private (a private field type in a pub struct is fine).
  2. `src/python_bindings/pty.rs:41-43`: `term_mut` returns `self.inner.terminal_write()`. Convert the direct mutating `let mut term = terminal.write();` sites listed above to `let mut term = self.inner.terminal_write();` (drop the `let terminal = self.inner.terminal();` line where it becomes unused).
  3. `src/mux/pane.rs`: add `pub fn with_terminal_mut<R>(&self, f: impl FnOnce(&mut Terminal) -> R) -> R { self.session.with_terminal_mut(f) }` if ARC-103 did not already add it (verify: `find_symbol with_terminal_mut` with `file_path: src/mux/pane.rs`). Use it in `tree.rs` `set_client_colors`, `apply_cell_pixels` and in `persist.rs:522-528`.
  4. Leave test-only raw writes alone (`src/mux/persist.rs:1339+`, `tree.rs:1690`, `scrape.rs:664` are inside `mod tests`).
  5. CHANGELOG `[Unreleased]` → `### Fixed`: "`PtyTerminal` methods that mutate the terminal from Python and par-mux restores now republish the geometry mirror, so `cursor_position()`/`size()` are current without waiting for PTY output (QA-195)." `### Added`: "`PtySession::terminal_write()`."
  6. Tests:
     - `src/pty_session.rs` `terminal_write_guard_publishes_geometry`: `PtySession::new(80, 24, 100)`; `{ let mut t = session.terminal_write(); t.process(b"\x1b[5;10H"); }`; `assert_eq!(session.cursor_position(), (9, 4))`. Without spawning.
     - `src/mux/persist.rs` tests: after a restore with a cwd-fallback note, the pane's `session.cursor_position()` equals the terminal's cursor (read through `with_terminal`). Find the existing restore test that emits the note (verify: `grep -n 'cwd_fallback\|no longer exists' src/mux/persist.rs` in tests) and add the assertion there.
- **Method**:
  - `with_terminal_mut` (`pty_session.rs:1410`) is already the publishing path; the guard makes the macro layer (`term_mut`) publish without touching 62 macro call sites in `common.rs`.
  - Enumerate raw writers: `grep -n 'terminal().write()\|terminal.write()\|terminal_ref().write()' src/mux src/python_bindings/pty.rs src/pty_session.rs` and `get_symbol_context terminal` (file `src/mux/pane.rs`).
  - Pitfall: the guard holds the write lock until drop, exactly as before; never hold it across a Python call (CLAUDE.md threading rule).
  - Pitfall: QA-205 then turns the eight read-only `terminal.write()` sites into `read()`; do not convert those here.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `make dev && uv run pytest tests/test_pty.py tests/test_macros.py -v`
  - Windows VM (both `cargo check` sets; `pty_session.rs` changed).
  - `make checkall`

### [QA-196] Tests mutate the process environment while running in parallel
- **Card**: `01a0ef5b5d287ae3bef6b8f3e8c040b9`
- **Files**:
  - `src/mux/client.rs:56-71` (`connect_or_spawn`: env-derived paths), `:578-594` (`EnvGuard`), `:599-630+` (`a_legacy_path_daemon_is_attached_not_replaced_by_a_second_spawn` sets `TMPDIR`, removes `XDG_RUNTIME_DIR`).
  - `src/mux/ipc.rs:514-527` (`default_socket_path` reads `XDG_RUNTIME_DIR`), `:582-584` (`legacy_socket_path` reads the temp dir).
  - `src/pty_session.rs:600-671` (spawn env drop: `DROP_VARS`, `std::env::var_os`, `std::env::vars()` twice), tests `:3301-3346` (`par_mux_env_does_not_leak_into_spawned_ptys`), `:3355-3437` (`outer_agent_identity_env_does_not_leak_into_spawned_ptys`).
  - `tests/mux_nested.rs:345-356` (`auto_spawn_inside_a_pane_is_refused_fast`), `tests/mux_reattach.rs:306` (`remove_var("PAR_MUX_ENV")`).
  - `src/debug.rs:36-49` (`DebugLevel::from_env`), test `:399-409`.
- **Steps**:
  1. `client.rs`: extract the body of `connect_or_spawn` into `fn connect_or_spawn_paths(path: &Path, legacy: Option<&Path>) -> io::Result<Self>`; `connect_or_spawn(name)` computes `default_socket_path(name)` and (Unix, `XDG_RUNTIME_DIR` unset) `legacy_socket_path(name)` and calls it. The test calls `connect_or_spawn_paths(&sandbox_default, Some(&legacy))` directly. Delete `EnvGuard` and the env writes.
  2. `pty_session.rs`: make the parent environment an input. Add `fn parent_env(&self) -> Vec<(OsString, OsString)>` that returns `std::env::vars_os().collect()` in production, or a `#[cfg(test)] parent_env_override: Option<Vec<(OsString, OsString)>>` field when set. When the override is set, call `cmd.env_clear()` first (CommandBuilder::new preloads the real process env via `get_base_env`). Rewrite the drop loop to iterate `parent_env()` once: `env_remove` every dropped name, then `cmd.env(k, v)` for the rest. Tests set the override to `[("PATH", real PATH), ("PAR_MUX_LEAK_PROBE", "stale-outer-value"), …]` instead of `set_var`.
  3. `tests/mux_nested.rs:345` and `tests/mux_reattach.rs:306`: re-exec the test in a child process with the env it needs, instead of mutating this process. Add to `tests/common/mod.rs`:
     ```rust
     /// Run `test` (exact name) in a fresh copy of this test binary with `set`/`unset`
     /// applied, and return whether it passed. The child sees `PAR_TEST_REEXEC=1`.
     pub fn rerun_isolated(test: &str, set: &[(&str, &str)], unset: &[&str]) -> bool
     ```
     using `std::env::current_exe()` with `--exact <test> --nocapture`. Each test body starts with `if std::env::var_os("PAR_TEST_REEXEC").is_none() { assert!(rerun_isolated(…)); return; }`.
  4. `debug.rs`: extract `fn from_value(raw: Option<&str>) -> Self`; `from_env` calls it with `std::env::var("DEBUG_LEVEL").ok().as_deref()`. Rewrite `test_debug_level_parsing` against `from_value(Some("3"))`, `Some("0")`, `None`, `Some("junk")`.
  5. Add a guard so this does not recur: `grep -rn 'env::set_var\|env::remove_var' src tests` must return only the re-exec child branches (document the exception in the helper's doc comment). Optionally add `clippy.toml` `disallowed-methods` for `std::env::set_var`/`remove_var` with `#[allow]` at the two helper sites (verify the project has no `clippy.toml` first).
  6. No CHANGELOG entry (tests only), unless step 1's `connect_or_spawn_paths` is made `pub` (keep it private).
- **Method**:
  - Process env is global to the test binary. `tests/mux_nested.rs`'s other tests spawn `par-mux` children that inherit the env; a concurrent `set_var("PAR_MUX_ENV", "1")` makes the nesting guard refuse them. That race is real under `cargo test`, and QA-185's nextest covers it only in CI.
  - `set_var`/`remove_var` become `unsafe` in edition 2024 (`Cargo.toml:11` is 2021), so this blocks that migration.
  - Enumerate readers of the env: `grep -rn 'env::var' src/mux src/pty_session.rs src/debug.rs`; `get_symbol_context nested_daemon_refusal` (`src/mux/mod.rs:76`).
  - Pitfall: on Windows the PTY env tests use `cmd.exe`; the override must carry `SystemRoot`/`PATH`/`COMSPEC` or `cmd.exe` fails to start. Copy them from the real env into the override.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::client`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize debug::`
  - `cargo test --test mux_nested --test mux_reattach --no-default-features --features rust-only,mux,serde`
  - `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - Windows VM (PTY env code has `cfg(windows)` branches): both `cargo check` sets.
  - `make checkall`

### [QA-197] Trigger action results and bookmarks are capped inconsistently
- **Card**: `01a0ef5b602c7000a7e288da41a2f064`
- **Files**: `src/terminal/trigger.rs:243-260` (Highlight push, `duration_ms == 0` → `u64::MAX`), `:262-278` (Notify), `:280-296` (MarkLine), `:305-344` (the three capped pushes), `:346-360` (SplitPane), `:384-392` (`clear_trigger_highlights`, `clear_expired_highlights`), tests `:838+`; `src/terminal/mod.rs:388-408` (`TriggerState`, `max_action_results: 100`), `:531-536` (`BookmarksState`); `src/terminal/semantic_snapshot.rs:905-917` (`add_bookmark`); `src/terminal/tests/bookmarks.rs`.
- **Steps**:
  1. In `trigger.rs` add:
     ```rust
     /// Queue an action result for host polling, dropping it once
     /// `max_action_results` are pending (the policy the capped arms already use).
     fn push_action_result(term: &mut Terminal, result: ActionResult) {
         if term.triggers.trigger_action_results.len() < term.triggers.max_action_results {
             term.triggers.trigger_action_results.push(result);
         }
     }
     ```
     Route all six pushes (Notify, MarkLine, RunCommand, PlaySound, SendText, SplitPane) through it.
  2. Highlights: add `/// cap: Trigger highlights retained from matched terminal output.` `pub(crate) const MAX_TRIGGER_HIGHLIGHTS: usize = 1000;` in `trigger.rs`. Before pushing, drop expired entries (`retain(|h| h.expiry > now)`, `now` is already computed at `:239`), then if still at the cap `remove(0)` (evict oldest).
  3. Bookmarks: add `/// cap: Bookmarks retained per terminal from triggers and API calls.` `pub(crate) const MAX_BOOKMARKS: usize = 1000;` near `BookmarksState` in `mod.rs`; in `add_bookmark` evict the oldest (`remove(0)`) when at the cap. IDs keep incrementing.
  4. `make caps-table`.
  5. CHANGELOG `[Unreleased]` → `### Fixed`: "Trigger Notify/MarkLine/SplitPane results honor `max_action_results`; trigger highlights and bookmarks are capped at 1000, evicting the oldest (QA-197)."
  6. Tests:
     - `trigger.rs` tests `every_action_result_respects_max_action_results`: register a trigger with Notify + MarkLine + SplitPane actions, set `max_action_results = 2` (it is `pub(crate)`), process 5 matching lines, assert `poll_action_results` returns 2 entries.
     - `trigger_highlights_are_capped_evicting_oldest`: a Highlight trigger with `duration_ms: 0`, feed `MAX_TRIGGER_HIGHLIGHTS + 5` matches (scrolling), assert `get_trigger_highlights().len() == MAX_TRIGGER_HIGHLIGHTS` and the first retained row is not the first matched row.
     - `src/terminal/tests/bookmarks.rs` `bookmarks_are_capped_evicting_oldest`: add `MAX_BOOKMARKS + 1`, assert len is the cap and bookmark id 0 is gone.
- **Method**:
  - Keep the existing drop-newest policy for action results: `poll_action_results` drains them (`trigger.rs:395`), and changing which results survive would change host-visible behavior for the three already-capped actions.
  - ARC-091 and ARC-102 (3b) also edit `trigger.rs`/`mod.rs`; re-read before editing.
  - Enumerate result producers: `grep -n 'trigger_action_results' src/terminal/*.rs`.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize trigger`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize bookmark`
  - `make dev && uv run pytest tests/ -k "trigger or bookmark" -v`
  - `make checkall`

### [QA-199] Client-registration block copied four times in `handle_client`
- **Card**: `01a0ef5b630d76c197311bd21fa6959d`
- **Blocked by**: SEC-127 (bounded read) and SEC-132 (log redaction) — both rewrite this loop in Phase 1. **Batch with QA-207** (same function, same commit series).
- **Files**: `src/mux/server.rs:450-700` (`handle_client`, CC 41, hotspot #2); the four registration copies at HEAD `:529-536`, `:587-594`, `:632-639`, `:681-688`; the read loop `:505-583`. Line numbers will have moved after SEC-127; re-read.
- **Steps**:
  1. Add a small registration holder:
     ```rust
     struct Registration { done: bool, abort: Option<ConnectionAbort> }
     impl Registration {
         fn ensure(&mut self, clients: &Clients, client_id: u64, tx: &SyncSender<String>, evicted: &Arc<AtomicBool>) {
             if !self.done {
                 clients.lock().push((client_id, tx.clone(), Arc::clone(evicted),
                     self.abort.take().expect("abort is registered once")));
                 self.done = true;
             }
         }
     }
     ```
     Replace all four copies with `registration.ensure(&clients, client_id, &tx, &evicted)`. Match `client_id`'s real type (verify: `CLIENT_SEQ` declaration).
  2. Extract the read loop into a function generic over the reader so it is unit-testable:
     ```rust
     enum ControlLine { Line(String), Oversize, Undecodable(usize), Closed }
     fn read_control_line<R: BufRead>(reader: &mut R, evicted: &AtomicBool) -> ControlLine
     ```
     It keeps SEC-127's bounded accumulation, the poll-wake arm (`is_poll_wake`), the "unterminated final line is processed before EOF" behavior, and the eviction check. `handle_client` becomes a loop over `read_control_line` plus the three-way dispatch.
  3. Tests (in `server.rs` tests):
     - `read_control_line_classifies_lines`: `Cursor::new(b"a\nb".to_vec())` → `Line("a\n")`, then `Line("b")`, then `Closed`.
     - `read_control_line_rejects_oversize`: `MAX_CONTROL_LINE_BYTES + 10` bytes with no newline → `Oversize`.
     - `read_control_line_flags_invalid_utf8`: `b"\xff\n"` → `Undecodable(_)`, and the next call on `b"\xff\nok\n"` returns `Line("ok\n")` (framing survives).
  4. No CHANGELOG entry (refactor).
- **Method**:
  - Target CC ≤ 15 for `handle_client`; measure before and after with parsight `calculate_cyclomatic_complexity` (or `find_most_complex_functions`, repository `par-term-emu-core-rust`, `file_path: src/mux/server.rs`).
  - Callers: `get_symbol_context handle_client` → the accept loop only (`server.rs:286`). Existing oversize/undecodable/eviction tests in `server.rs` tests must pass unchanged; they are the behavioral contract.
  - Pitfall: a `Cursor` never produces `WouldBlock`, so the poll-wake arm is covered only by the existing socket tests; do not delete those.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::server`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - Windows VM: both `cargo check` sets (the read loop has the `ConnectionAbort` Windows path).
  - `make checkall`

### [QA-200] Oversized streamer `main` and `handle_csi_report`; `run_mux_mode` re-implements the serve loop
- **Card**: `01a0ef5b667a7210bdee98f5e95f2a59`
- **Files**: `src/bin/streaming_server/main.rs:94-145` (`run_mux_mode`, `#[cfg(feature = "mux")]`; its own spawn/ctrl_c/shutdown/abort), `:147-157` (non-mux stub), `:167-…` (`main`, CC 37), `:584-685` (`serve_until_ctrl_c`); `src/terminal/sequences/csi/report.rs:7-247` (`handle_csi_report`, CC 54; arms `'n'` `:16`, `'c'` `:52`, `'q'` `:66`, `'p'` `:79-225` with the DECRQM mode table `:97-222`, `'x'` `:226`).
- **Steps**:
  1. `main.rs`: extract two helpers used by both modes:
     ```rust
     fn spawn_server_task(server: Arc<StreamingServer>) -> tokio::task::JoinHandle<()>
     async fn stop_server(server: &StreamingServer, handle: tokio::task::JoinHandle<()>)
     ```
     `run_mux_mode` becomes: build factory/server → `spawn_server_task` → `signal::ctrl_c().await` → `stop_server`. `serve_until_ctrl_c` uses the same two helpers around its macro/shell branches (keep its factory teardown and macro `exit\n` step between them).
  2. Split `main` (CC 37) by concern: `fn build_config(args: &Args, run_mode: &RunMode, tls: Option<TlsConfig>, auth, presets, cols, rows) -> StreamingConfig` (move the literal at `:261-285`), and `fn resolve_size(args) -> (u16, u16)` if the size logic is inline. Keep `main` as orchestration.
  3. `report.rs`: one private method per report family, `handle_csi_report` becomes a 5-arm dispatch:
     - `fn report_dsr(&mut self, params: &Params, private: bool)` (`'n'`)
     - `fn report_device_attributes(&mut self, intermediates: &[u8])` (`'c'`)
     - `fn report_xtversion(&mut self, intermediates: &[u8], private: bool)` (`'q'`)
     - `fn handle_csi_p(&mut self, params: &Params, intermediates: &[u8])` (`'p'`: DECSCL, DECSTR, DECRQM) with `fn decrqm_status(&self, mode: u16, private: bool) -> (u8, &'static str)` holding the mode table
     - `fn report_decreqtparm(&mut self, params: &Params)` (`'x'`)
  4. No CHANGELOG entry (refactor).
- **Method**:
  - Pure moves; byte-identical responses. The response tests already exist: `src/terminal/tests/terminal_tests.rs:698-790` (`test_da_primary`…`test_decreqtparm_*`), `src/terminal/tests/modes.rs:207` (`test_decrqm_reports_mode_9`) and the other DECRQM tests (verify: `grep -rn 'decrqm' src/terminal/tests`).
  - Measure CC before/after with parsight `calculate_cyclomatic_complexity` on `handle_csi_report` and `main` (file `src/bin/streaming_server/main.rs`).
  - Pitfall: `run_mux_mode` exists twice (mux / not(mux)); only the mux variant changes.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize terminal::tests`
  - `cargo check --no-default-features --features streaming-bin,mux --bin par-term-streamer` and `cargo check --no-default-features --features streaming-bin --bin par-term-streamer` (both `run_mux_mode` variants)
  - `make test-rust-streaming`
  - `make checkall`

### [QA-201] `unsafe` without SAFETY comments has grown to 44+
- **Card**: `01a0ef5b6a017a719e1b04a78240904f`
- **Batch**: **one `src/ffi.rs` batch with QA-208 and QA-215, executed here. Order: QA-215 → QA-208 → QA-201.** SAFETY comments go on the final blocks, and QA-208's dedup removes several blocks first. Land QA-210 (removes the four `unsafe impl` in `src/python_bindings/observer.rs`) before step 3 below. Read `src/ffi.rs` fully before starting (ARC-101/ARC-112/ARC-114 in 3b may have changed it; if the D4 ABI-v4 batch is in flight, sequence after it).
- **Files** (production counts from `cargo clippy --no-default-features --features rust-only,streaming,streaming-bin,mux,serde -- -W clippy::undocumented_unsafe_blocks`, measured at f6535f2):
  - `src/ffi.rs`: 32 (e.g. `:206,:214,:222` Drop; `:279` `unsafe impl Sync` — the SAFETY comment at `:276` covers only `:278`; `:302`; the extern fns `:469-898`).
  - `src/mux/foreground.rs`: 4. `src/bin/par_mux/main.rs`: 4 (`:298,:313,:354,:367`). `src/mux/pane.rs`: 1. `src/mux/host_probe.rs`: 1 (`:75/:79`). `src/mux/client.rs`: 1. `src/bin/streaming_server/cli.rs`: 1 (`:23`).
  - Not in that count: `src/python_bindings/observer.rs:447-448,487-488` (QA-210 deletes them), Windows `src/mux/ipc.rs:167,206,222` (QA-194 adds their SAFETY comments), `src/pty_session.rs:173` (has a constraint comment; check clippy accepts it).
  - Test code (`--all-targets`, measured): `src/ffi.rs` tests 65 more (97 total), `src/mux/server.rs` tests 9, `src/mux/tree.rs:1961` 1, `src/terminal/tests/ffi_tests.rs` 2, plus separate crates `benches/ffi_readback.rs` 16, `tests/ffi_dirty_ranges_alloc.rs` 7, `tests/mux_daemon.rs` 1.
- **Steps**:
  1. Write a `// SAFETY:` comment directly above every production `unsafe` block/impl listed, stating the invariant that makes it sound (for FFI entry points: "`term` is non-null (checked above) and, per this fn's `# Safety` contract, points to a live `Terminal` not aliased mutably elsewhere"; for `out.add(i).write`: "`i < cap` and the caller guarantees `out` is valid for `cap` writes"). Reuse each fn's `# Safety` doc rather than restating it at length.
  2. Add a second comment line for `ffi.rs:279` (`// SAFETY:` for `Sync`, same reasoning as `Send`).
  3. Add `#![cfg_attr(not(test), warn(clippy::undocumented_unsafe_blocks))]` at the top of `src/lib.rs`, `src/bin/par_mux/main.rs` and `src/bin/streaming_server/main.rs`. `cfg_attr(not(test))` keeps the ~75 test-module blocks out of `make lint` (which runs `--all-targets -D warnings`); documenting test FFI calls is noise. Benches and `tests/` are separate crates and are unaffected.
  4. No CHANGELOG entry.
- **Method**:
  - Count before and after: `cargo clippy --no-default-features --features rust-only,streaming,streaming-bin,mux,serde -- -A clippy::all -W clippy::undocumented_unsafe_blocks 2>&1 | grep -c 'missing a safety comment'` must reach 0, and the same with `--features python,streaming,mux,serde,streaming-bin` (what `make lint` uses) must be 0.
  - Pitfall: clippy recognizes `// SAFETY:` (upper case). The observer comments say `// Safety:`; QA-210 removes them, which is why it goes first.
  - Pitfall: the Windows `ipc.rs` blocks are only linted on Windows; QA-194 fixes them. The CI lint job runs on ubuntu, so they cannot fail CI, but the VM check will show them.
- **Verify**:
  - The two clippy counts above (0 each).
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize ffi`
  - `make xcframework` (macOS; header smoke-compile, link and Swift import) and `make ffi-header-check ffi-surface-check`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::`
  - Windows VM: both `cargo check` sets.
  - `make checkall`

### [QA-203] Fixed sleeps in PTY tests
- **Card**: `01a0ef5b6d9a717099523b6c67ec6270`
- **Files**: `src/pty_session.rs` tests: `:2257` (`test_spawn_with_env`, trailing 100 ms), `:2357` (`test_env_not_leaked_to_parent`, 100 ms before a parent-env assert), `:2386` (`test_spawn_with_env_and_set_env_combined`, trailing 100 ms), `:2685/:2693/:2709` (`test_generation_counter_after_ctrl_c`, 500/300/500 ms), `:2785` (`test_observer_dispatch_does_not_hold_write_lock`, 500 ms). Helpers: `UpdateWaiter::wait_for_update` (`:1775`), `UpdateWaiter::wait_until` (`:1804`), obtained via `PtySession::update_waiter()` (`:1599`).
- **Steps**:
  1. `:2257` and `:2386`: replace the trailing sleep with `assert!(session.update_waiter().wait_until(Duration::from_secs(5), |t| t.content().contains("hello")))` (use each test's echoed text; `:2386` echoes whatever its command prints — read it). If the test asserts nothing about output, delete the sleep only.
  2. `:2357`: delete the sleep; the parent-env assertion does not depend on the child.
  3. `test_generation_counter_after_ctrl_c`:
     - `:2685` → `let gen0 = session.update_generation(); session.update_waiter().wait_for_update(gen0, Duration::from_secs(5)).expect("shell printed a prompt");`
     - `:2693` → delete; the following `>=` assertion is always true, so remove it too (it asserts nothing).
     - `:2709` → `assert!(session.update_waiter().wait_until(Duration::from_secs(5), |t| t.content().contains("TEST_GENERATION")))`, then keep the `>`/`has_updates_since` assertions.
  4. `:2785`: poll the probe flag: `let deadline = Instant::now() + Duration::from_secs(5); while !probe.saw_any_invocation.load(SeqCst) { assert!(Instant::now() < deadline, "BEL never dispatched"); std::thread::sleep(Duration::from_millis(10)); }`.
  5. No CHANGELOG entry.
- **Method**:
  - Deadline polling passes as soon as the condition holds and fails with a message instead of racing a fixed sleep on a slow CI runner.
  - SEC-125 (Phase 1) and ARC-089 edited this file; re-read the test bodies first.
  - Windows variants exist (`#[cfg(windows)]` spawns `cmd.exe`); the waits apply to both.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session::tests`
  - Windows VM: `cargo check --all-targets`.
  - `make checkall`

### [QA-204] No stub drift check
- **Card**: `01a0ef5b72547cd1b00ded2a62b0f295`
- **Files**: `Makefile:9` (`.PHONY` list with `stubs stub-check`), `:50-51` (help lines), `:300-321` (`stubs`, `stub-check`), `:366` (`checkall`).
- **Steps**:
  1. After `stub-check`, add:
     ```make
     # QA-204: regenerate the stub from a streaming build and fail on drift.
     # Not in checkall: it rebuilds the extension. A `make dev` build drops the
     # streaming classes, so this depends on dev-streaming, never dev.
     stub-drift: dev-streaming stubs
     	git diff --exit-code -- python/par_term_emu_core_rust/_native.pyi
     ```
  2. Add `stub-drift` to `.PHONY` (`:9`) and a help line under `:51`: `@echo "  stub-drift      - Rebuild (dev-streaming), regenerate the stub, and fail if it differs from the committed file"`.
  3. Leave `checkall` unchanged.
- **Method**:
  - `make stubs` runs `ruff format` on the output, so a clean regeneration is byte-identical to a committed stub produced the same way.
  - Pitfall: `git diff` compares against the index; a developer who regenerated and staged the stub sees a clean result, which is correct.
  - The typing half of the old finding is DOC-116; do not change `scripts/generate_stubs.py` here.
- **Verify**:
  - `make stub-drift` on a clean tree exits 0 (if it exits 1 at HEAD, the committed stub is already stale: commit the regenerated stub in the same change and say so).
  - `make help | grep stub-drift`
  - `make checkall`

### [QA-202] Large files keep growing
- **Card**: `01a0ef5b762f70d1bbf31511786f3f9b`
- **Runs LAST** in Phase 3 (after every QA/ARC/SEC/DOC fix that touches these files: SEC-125/126/127/128, ARC-089/090/094/095/096/103/113, QA-182/185/187/188/191/195/196/199/203/207/218). **Re-read every file in full immediately before splitting; do not reuse any line number below.**
- **Files and HEAD boundaries** (production = before `mod tests`):
  - `src/mux/hooks.rs`: 2226 lines, `mod tests` at `:934`. Items: `handle_report` `:44`, `ReportHeader` `:69`, `MAX_REPORT_VALUE_LEN` `:82-83` (`/// cap:`), `check_value_len` `:85`, `parse_header` `:95`, `handle_state_report` `:148`, `handle_session_report` `:261`, `AGENT_CLAIM_KEYS` `:403`, `handle_release_report` `:433`, telemetry `:468-829` (`TELEMETRY_KEY`, `TELEMETRY_FRESHNESS_MS`, `handle_telemetry_report`, `TelemetryV1`, parsers, `fresh_telemetry_b64` `:809`), `parse_resume_argv` `:831`, `is_stale` `:863`, `SEQ_STAMPS_KEY`/`seq_stamps`/`record_seq` `:880-907`, `ok_reply`/`error_reply` `:909-932`.
  - `src/pty_session.rs`: 3439 lines, tests at `:1829`; `start_reader_thread` `:811-~1060`.
  - `src/mux/tree.rs`: 3139 lines, tests at `:1604`.
- **Steps** (one commit per file, `git mv` first so history follows):
  1. **hooks** → `src/mux/hooks/`:
     - `mod.rs`: `handle_report`, `ReportHeader`, `parse_header`, `check_value_len`, `MAX_REPORT_VALUE_LEN`, `is_stale`, seq stamps, `ok_reply`/`error_reply`; `mod report; mod telemetry; mod release;` and `pub(crate) use` re-exports so existing paths still resolve: `crate::mux::hooks::{handle_report, fresh_telemetry_b64, TELEMETRY_FRESHNESS_MS, AGENT_CLAIM_KEYS}` (callers: `dispatch.rs:405`, `persist.rs:1864`, `host_probe.rs:324`, `server.rs:623`, `scrape.rs:20`).
     - `report.rs`: `handle_state_report`, `handle_session_report`, `AGENT_CLAIM_KEYS`, `parse_resume_argv`.
     - `telemetry.rs`: everything from `TELEMETRY_KEY` through `fresh_telemetry_b64`.
     - `release.rs`: `handle_release_report`.
     - Tests: move to `hooks/tests.rs` unchanged (`#[cfg(test)] mod tests;` in `mod.rs`).
  2. **PTY reader** → convert `src/pty_session.rs` to `src/pty_session/mod.rs` (`git mv`) and move `start_reader_thread` plus the private items only it uses into `src/pty_session/reader.rs` as `impl PtySession { pub(super) fn start_reader_thread(…) }`. Child modules can read the parent's private fields, so no visibility changes are needed.
  3. **tree** → `src/mux/tree/`:
     - `mod.rs`: types (`MuxTree`, `MuxWindow`, `MuxSession`, spawn plans), constructors, accessors, target resolvers, `window_of_pane`, `kill_detached`.
     - `lifecycle.rs`: `new_session`…`complete_session`, `spawn_from`, `new_window`…`complete_window`, `begin_respawn`/`complete_respawn`, `kill_pane`, `kill_window`, `kill_session`, `drop_empty_window`, `all_panes_dead`, `rename_*`, `set_session_env`.
     - `layout_ops.rs` (not `layout.rs`: `src/mux/layout.rs` already holds `LayoutTree`): `split_pane*`, `begin_split`/`complete_split`, `select_pane`, `swap_panes`, `zoom_pane`, `break_pane`, `join_pane`, `move_window`, `swap_windows`, `resize_pane`, `resize_pane_absolute`, `resize_window`, `set_client_cell_pixels`, `set_client_colors`, `apply_cell_pixels`, `sync_pane_sizes`, and ARC-090's `mutate_layout`.
     - Tests: `tree/tests.rs`.
     - Confirm the grouping with parsight `propose_decomposition` on `src/mux/tree.rs` before moving; follow its clusters where they disagree with this list only if they keep lifecycle and geometry separate.
  4. Update path references: `docs/MUX.md:263,446,453` (`src/mux/hooks.rs`, `src/mux/tree.rs`), `docs/SECURITY.md:261,530` (`src/pty_session.rs`, "`start_reader_thread` in `src/pty_session.rs`"), `:927` (`src/mux/hooks.rs`), `CLAUDE.md:160-170` Key Source Layout (`src/pty_session.rs`, add the new dirs), and run `make caps-table` (`MAX_REPORT_VALUE_LEN`'s location changes).
  5. Out of scope: `src/terminal/mod.rs` (ARC-102), `src/streaming/server.rs`, `dispatch.rs`, `command.rs`, `ffi.rs`. Record their post-phase sizes in the commit message.
- **Method**:
  - Pure moves: `git diff -M --stat` should show renames with small deltas; no function body changes. Run `get_impact` on each moved `pub(crate)` symbol (`handle_report`, `fresh_telemetry_b64`, `start_reader_thread`, `sync_pane_sizes`) before and after to confirm the caller set is unchanged.
  - Pitfall: `#[cfg(test)]` helpers used across the split (e.g. `first_read_delay` in `start_reader_thread`) must stay reachable; `cargo test` catches it.
  - Pitfall: the doc/CLAUDE.md edits touch files 3d owns; do them here because the paths change here.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `cargo test --no-default-features --features rust-only,mux,serde -- --test-threads=1` (all mux integration targets)
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session`
  - `make test-rust-streaming`
  - Windows VM from the main checkout: both `cargo check` sets, then `cargo test --lib --no-default-features --features rust-only,mux,serde mux:: -- --test-threads=1`.
  - `make checkall` (includes `caps-table-check`)

### [QA-186] Wall-clock and sleep-selected assertions in mux tests
- **Card**: `01a0ef5b7b2a7a618738f55ed34b9a42`
- **Blocked by**: QA-185 (`tests/common/mod.rs` helpers).
- **Files**: `tests/mux_end_to_end.rs:214-280` (`a_slow_spawn_does_not_stall_other_clients`: `SleepingFactory` with a 1.5 s stall at `:225-237`, `sleep(300 ms)` at `:260`, `elapsed < 800 ms` at `:265-268`); `tests/mux_restart.rs:261-325` (`a_shutdown_race_restores_the_pre_exit_layout`: `sleep(600 ms)` at `:306`).
- **Steps**:
  1. `mux_end_to_end.rs`: replace the timed stall with gates. `SleepingFactory` becomes `GatedFactory { inner, entered: Mutex<Option<mpsc::Sender<()>>>, release: Mutex<mpsc::Receiver<()>> }`: `create_pane` sends on `entered`, blocks on `release.recv()`, then delegates. The test:
     - waits `entered_rx.recv_timeout(5 s)` (the spawn is in flight, tree lock released);
     - sends B's `list-panes` and reads the reply through `common::connect`/`common::command` (deadline-bounded);
     - asserts the body is empty (two-phase semantics) and `!session_a.is_finished()`;
     - sends on `release_tx`, joins A, asserts A's reply names a session.
     No elapsed-time assertion remains.
  2. `mux_restart.rs:306`: replace the 600 ms sleep with an ordering signal. The reaper broadcasts `%pane-exited` before it sends the persist capture (`src/mux/server.rs`, `reap_dead_panes`). Read the control stream until both `%pane-exited {left}` and `%pane-exited {right}` lines arrive (10 s deadline), then poll `std::fs::metadata(fixture.state_path()).modified()` until it is later than a timestamp taken just before the two `send-keys 'exit 1'` commands (5 s deadline). Verify first that a reap persist rewrites the state file (read `reap_dead_panes`); if it does not, the metadata poll is unnecessary and the `%pane-exited` wait alone selects the interleaving the comment describes.
  3. No CHANGELOG entry.
- **Method**:
  - Gates make the interleaving deterministic, so the test proves "B is not blocked by A's spawn" by ordering instead of latency.
  - Pitfall: `%pane-exited` arrives only on a registered client; the test's control connection has issued commands, so it is registered (MEMORY: clients join broadcasts on the first control command).
- **Verify**:
  - `cargo test --test mux_end_to_end --test mux_restart --no-default-features --features rust-only,mux,serde -- --test-threads=1`
  - `cargo nextest run --profile ci --no-default-features --features rust-only,mux,serde -E 'binary(mux_end_to_end) | binary(mux_restart)'`
  - `make checkall`

### [QA-187] Remaining duplication in the new mux code (kill cascade, split geometry)
- **Card**: `01a0ef5b7f947e20a2289f4b27700350`
- **Blocked by**: ARC-103 (wiring half, done in Phase 2) and SEC-126 (`Args` flag/trailing split). Only the kill-cascade and split-geometry halves remain.
- **Files**: `src/mux/tree.rs:818-835` (`drop_empty_window`; callers today are only `break_pane` and `join_pane`), inline copies in `kill_pane` `:1453-1470` and `kill_window` `:1519-1545`; `src/mux/command.rs:1030-1057` (`parse_split_window` `-h`/`-p`), `:1152-1177` (`parse_join_pane`, identical block). Tests: `tree.rs:2919-2953` (`kill_window_*`), `command.rs:1983-2036` (`parses_split_window_*`, `split_window_rejects_out_of_range_percent`).
- **Steps**:
  1. `kill_pane`: replace the inline block (`self.windows.remove(&window_id)` + session `find_map` + `sessions.remove`) with `removed_session = self.drop_empty_window(window_id);`.
  2. `kill_window`: collect `window.panes()` via `self.windows.get(&window_id).ok_or(MuxError::NoSuchWindow(window_id))?`, kill each removed pane with `kill_detached`, then `Ok(self.drop_empty_window(window_id))`. `drop_empty_window` removes the window itself.
  3. `command.rs`: add
     ```rust
     /// `-h` (beside, Vertical) vs `-v`/default (below, Horizontal), and `-p` 1-99 (default 50).
     fn parse_split_geometry(a: &Args<'_>) -> Result<(SplitDirection, u32), String>
     ```
     and use it in `parse_split_window` and `parse_join_pane`. Keep the error strings byte-identical (`invalid percentage: {raw}`, `percentage must be 1-99: {raw}`).
  4. Tests: add `join_pane_rejects_out_of_range_percent` (`join-pane -s %1 -t %0 -p 0` and `-p 100` are `Err`) if no join-pane percent test exists (verify: `grep -n 'join_pane\|join-pane' src/mux/command.rs` in tests). The existing kill tests cover the cascade.
  5. No CHANGELOG entry.
- **Method**:
  - Callers: `get_symbol_context drop_empty_window` → `break_pane`, `join_pane` (after this: plus `kill_pane`, `kill_window`). `kill_pane`'s `%window-close`/`%sessions-changed` return contract (the `(affected_window, removed_session)` tuple) is unchanged.
  - Pitfall: `drop_empty_window`'s session fix-up (`active` clamp) is identical to both inline copies; diff them once more after Phase 2 in case ARC-090 changed `kill_pane`.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `make checkall`

### [QA-188] Resize errors are swallowed
- **Card**: `01a0ef5b83507c42ae64b2e21d8ab34e`
- **Blocked by**: QA-182 (edits the same resize path) and ARC-090 (`mutate_layout`/`sync_pane_sizes` choke point).
- **Files**: `src/mux/tree.rs:1392-1398` (`sync_pane_sizes`: `let _ = resized;`), `:1344` (`apply_cell_pixels`: `let _ = pane.resize_with_cell_pixels(…)`); `src/mux/pane.rs:348` (`MuxPane::dead()`).
- **Steps**:
  1. In `sync_pane_sizes` replace `let _ = resized;` with:
     ```rust
     if let Err(err) = resized {
         if !pane.dead() {
             log::warn!("par-mux: resize of pane {} to {width}x{height} failed: {err}", pane_geometry.pane);
         }
     }
     ```
     `PaneId` displays as `%N` (`src/mux/ids.rs:34-36`).
  2. Same pattern at `:1344` in `apply_cell_pixels`.
  3. Keep the best-effort semantics documented at `:1364-1370`: the structural mutation still succeeds.
  4. No test: a PTY resize failure has no deterministic seam (a never-spawned `PtySession` resizes `Ok` because `pty_master` is `None`). State this in the commit message.
  5. No CHANGELOG entry.
- **Method**:
  - The daemon installs a stderr `log` logger at Info (`src/bin/par_mux/main.rs:157-158`), and `src/mux/server.rs` already logs with `log::warn!`/`log::error!`, so `log::warn!` reaches the daemon's stderr.
  - Skip dead panes: remain-on-exit panes are re-fit on every layout change, and their resize outcome is irrelevant (SEC-125 made signals to them no-ops).
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `make checkall`

### [QA-190] `%pane-exited` puts its exit code in the `name` field
- **Card**: `01a0ef5b87b77732a6c6f73d7edfba14`
- **Blocks**: DOC-100 (documents the new field).
- **Files**: `src/python_bindings/types/notification.rs:7-67` (`PyTmuxNotification`, 19 fields), `:128-852` (`From<&TmuxNotification>`, 38 struct literals), `:620-640` (`PaneExited` arm: `name: exit_code.map(|code| code.to_string())`); `src/tmux_control.rs:186-190` (`PaneExited { pane_id, exit_code: Option<i32> }`), `:645-659` (parser); `python/par_term_emu_core_rust/_native.pyi:2231+` (`class TmuxNotification`); `docs/API_REFERENCE.md:1833-1856` (TmuxNotification Properties).
- **Steps**:
  1. Add the field after `name`:
     ```rust
     /// Exit code of the pane's process (for `pane-exited`); None on signal
     /// death, when unreadable, and for every other notification type
     pub exit_code: Option<i32>,
     ```
  2. `PaneExited` arm: `exit_code: *exit_code`, and **keep** `name: exit_code.map(|code| code.to_string())` for one release (back-compat for clients that read it today). Mark it with a comment: `// Deprecated: remove in 0.59.0; clients read exit_code.`
  3. Every other struct literal: `exit_code: None` (37 literals; the compiler lists each).
  4. `docs/API_REFERENCE.md`: add `- \`exit_code: int | None\`: For \`pane-exited\`, the process exit code; \`None\` on signal death and for other types` to the Properties list. Required in this commit: `scripts/check_api_reference.py` fails `stub-check` when a stub property is missing from a class's Properties section. DOC-100 later adds the two `notification_type` strings and the deprecation wording for `name`.
  5. Regenerate the stub: `make dev-streaming && make stubs` (never `make dev` → drops streaming classes). Commit the `_native.pyi` diff (a new `exit_code` property on `TmuxNotification`).
  6. CHANGELOG `[Unreleased]`: `### Added` "`TmuxNotification.exit_code` for `pane-exited` (QA-190)." `### Deprecated` "`TmuxNotification.name` carrying the `pane-exited` exit code; removed in 0.59.0."
  7. Test, new `tests/test_tmux_pane_exited.py`:
     - `term = Terminal(80, 24); term.set_tmux_control_mode(True); term.process_str("%pane-exited %3 7\n"); n = term.drain_tmux_notifications()`; assert one notification with `notification_type == "pane-exited"`, `pane_id == "%3"`, `exit_code == 7`, `name == "7"`.
     - `"%pane-exited %3\n"` → `exit_code is None`, `name is None`.
     - Verify the control-mode feed path first with the Rust test in `src/terminal/tests/tmux.rs` (`set_tmux_control_mode(true)` at `:9,:19`), and mirror its input format.
- **Method**:
  - Enumerate every constructor: `grep -n 'PyTmuxNotification {' src/python_bindings/types/notification.rs` (38) plus `grep -rn 'PyTmuxNotification {' src` outside the file (none at HEAD). ARC-094 (3b) also edits this file; re-read first.
  - External consumers of `name` for `pane-exited`: par-term parses `%pane-exited` natively in Rust (`par-term/src/app/tmux_handler/notifications/mux_pane_exit.rs`), not through the Python type, so the one-release window is for Python clients (par-term-emu-tui-rust has no `pane-exited` handling at HEAD).
- **Verify**:
  - `make dev-streaming && make stubs && git diff --stat python/par_term_emu_core_rust/_native.pyi` (only the `exit_code` property added)
  - `uv run pytest tests/test_tmux_pane_exited.py -v`
  - `make stub-check`
  - `cargo test --lib --no-default-features --features python-test`
  - `make checkall`

### [QA-205] Read-only `PtyTerminal` methods take the write lock
- **Card**: `01a0ef5b8c0b7ad28af4fa8367b783f0`
- **After**: QA-195 (same file).
- **Files**: `src/python_bindings/pty.rs:707-708` (`debug_info`), `:839-840` (`get_macro`), `:864-865` (`list_macros`), `:933-934` (`is_macro_playing`), `:943-944` (`is_macro_paused`), `:953-954` (`get_macro_progress`), `:963-964` (`get_current_macro_name`), `:1012-1013` (`recording_to_macro`); stale comment `:296-302`. Candidate ninth: `:641-644` (`paste`: `let term = terminal.write();` only reads `bracketed_paste_start/end`; verify the guard is not used mutably later in the block).
- **Steps**:
  1. Change each `let term = terminal.write();` above to `let term = terminal.read();`. Every callee takes `&Terminal`: `MacroEngine::get_macro`/`list_macros`/`is_macro_playing`/`is_macro_paused`/`get_macro_progress`/`get_current_macro_name`/`recording_to_macro` (`src/terminal/macros.rs:23,33,75,84,93,98,142`), and `debug_info` only reads.
  2. Fix the comment at `:296-302`: `size` and `cursor_position` are served from the geometry mirror block at `:50-70`, not by `impl_terminal_query_getters!`.
  3. No CHANGELOG entry.
- **Method**:
  - A read lock lets these run concurrently with other readers and never blocks behind the PTY reader longer than a read does.
  - The compiler proves correctness: if any of these needs `&mut`, `read()` will not compile.
- **Verify**:
  - `make dev && uv run pytest tests/test_macros.py tests/test_macros_extended.py tests/test_pty.py -v`
  - `cargo test --lib --no-default-features --features python-test`
  - `make checkall`

### [QA-206] `estimated_memory_bytes: 0 // Should be calculated` placeholders
- **Card**: `01a0ef5b8f8a7f7399d2ec2ce088e178`
- **Files**: `src/terminal/metrics.rs:288-310` (`get_stats`; placeholders at `:301` and `:303`), `:331` (field doc); `src/terminal/tests/terminal_tests.rs:2863-2876` (`test_enhanced_stats`); Python surface `src/python_bindings/common.rs:2350-2370` (`get_stats` dict); `docs/API_REFERENCE.md:1001`. Reference estimate: `src/terminal/replay_snapshot.rs:152-154` ("per-cell overhead as `size_of::<Cell>()` + 24 bytes").
- **Steps**:
  1. `estimated_memory_bytes`: `total_cells * (std::mem::size_of::<Cell>() + 24)`, reusing the replay-snapshot per-cell estimate (extract a shared `pub(crate) const CELL_OVERHEAD_ESTIMATE: usize = 24;` in `replay_snapshot.rs` or `metrics.rs` and use it in both, so the two estimates cannot drift).
  2. `hyperlink_memory_bytes`: `self.hyperlink_state.hyperlinks.values().map(|url| url.len() + std::mem::size_of::<u32>() + std::mem::size_of::<String>()).sum()`.
  3. Field docs at `:331` and the hyperlink field: "Estimate (lower bound): grid cells only / URL bytes plus map overhead".
  4. Test: in `test_enhanced_stats` assert `stats.estimated_memory_bytes >= stats.total_cells * std::mem::size_of::<Cell>()` and `stats.hyperlink_memory_bytes >= "https://example.com".len()`.
  5. CHANGELOG `[Unreleased]` → `### Fixed`: "`get_stats()` reports non-zero `estimated_memory_bytes` and `hyperlink_memory_bytes` estimates (QA-206)."
- **Method**: callers of the stats: `get_symbol_context get_stats` (file `src/terminal/metrics.rs`) → the Python dict builder only. The values are documented as estimates; do not try to count heap in combining-character vectors.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize test_enhanced_stats`
  - `make dev && uv run pytest tests/ -k stats -v`
  - `make checkall`

### [QA-207] Stall-hunt diagnostics remain in hot paths
- **Card**: `01a0ef5b93de7c219d412c988a9831f2`
- **Executed with QA-199** (same `handle_client` rewrite, after SEC-127/SEC-132). The `src/streaming/session.rs` half was done in QA-191 step 3; verify it and skip.
- **Files**: `src/mux/server.rs:510-567` at HEAD (`pickup_started`, `wakes`, the wake-cadence log at `:547-558`, the per-wake log at `:563-568`), `:640-662` (dispatch timing and rejection logs, already edited by SEC-132); `src/streaming/mux_factory.rs:833-845` (narrative doc on `stress_input_frames_of_varied_sizes_all_land_exactly_once`, `:851`).
- **Steps**:
  1. In the extracted `read_control_line` (QA-199), drop `pickup_started`, `wakes` and both `debug_log!` calls. Nothing else reads them.
  2. Keep the rejection `debug_error!` (a rejected command otherwise has no trace; its comment says so) and the `>100 ms` dispatch-timing log (it fires only on slow dispatch). Keep whatever SEC-132 decided about `summarize_line`.
  3. `mux_factory.rs`: reduce the test doc to what it asserts: "Every typed frame, alternating the queued path and the direct writer across resizes and a second viewer, lands exactly once in the pane-side file." Remove the card narrative.
  4. `session.rs:374-421`: confirm QA-191 left only constraint comments and no per-chunk log; otherwise apply QA-191 step 3 here.
  5. No CHANGELOG entry.
- **Method**: `debug_log!` expands to `logf` (`src/debug.rs:207-211`); the per-wake call runs on every 500 ms eviction poll of every idle client. Removing it removes work, not behavior.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::server`
  - `cargo test --lib --no-default-features --features rust-only,streaming,mux,serde streaming::mux_factory -- --test-threads=1`
  - `make checkall`

### [QA-208] Duplicated FFI code
- **Card**: `01a0ef5b96ac7a52a9842f39600e3ef6`
- **Executed inside the QA-201 batch** (order QA-215 → **QA-208** → QA-201). This entry is the step list for that batch; verify after the batch.
- **Files**: `src/ffi.rs:136-142` (MouseMode map in `SharedState::from_terminal`) and `:751-757` (`terminal_get_modes`); `:621-661` (`terminal_read_row`) and `:665-700` (`terminal_read_scrollback_row`), identical copy loops; `:533-551` (`terminal_dirty_ranges`) and `:579-598` (`terminal_dirty_ranges_since`), identical writers.
- **Steps**:
  1. `fn mouse_mode_code(mode: MouseMode) -> u8` (0 Off, 1 X10, 2 Normal, 3 ButtonEvent, 4 AnyEvent) used by both sites.
  2. ```rust
     /// Copy `cells[col_start..]` into `out` (up to `cap`), padding short rows
     /// with blanks to `cols`. Returns cells written.
     unsafe fn copy_row(cells: &[Cell], cols: u32, col_start: u32, out: *mut SharedCell, cap: u32) -> u32
     ```
     Both readers keep their own lookup (`grid.row` vs `grid.scrollback_line`, both `Option<&[Cell]>`, `src/grid/mod.rs:177,261`) and the null-`out` sizing answer, then call `copy_row`.
  3. ```rust
     /// Write ranges visited by `each` into `out` (up to `cap`) and return the total count.
     unsafe fn write_ranges(out: *mut TermRowRange, cap: u32, each: impl FnOnce(&mut dyn FnMut(u32, u32))) -> u32
     ```
     Match the real closure signature of `for_each_dirty_range`/`for_each_dirty_range_since` (verify: `find_symbol for_each_dirty_range` with `repository_id`, then read the signature).
  4. SAFETY comments on the new helpers are part of QA-201.
- **Method**: exported signatures and the header are unchanged, so `make ffi-header-check` stays clean. The round-trip and dirty-range tests in `ffi.rs` tests (`:903+`) and `tests/ffi_dirty_ranges_alloc.rs` are the contract; the latter asserts zero allocation, so the helpers must not allocate (no `Vec` in `write_ranges`).
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize ffi`
  - `cargo test --test ffi_dirty_ranges_alloc --no-default-features --features pyo3/auto-initialize`
  - `make ffi-header-check ffi-surface-check` and `make xcframework` (macOS)
  - `make checkall`

### [QA-210] Unneeded `unsafe impl Send/Sync`
- **Card**: `01a0ef5b99dc7513b0b932ea3e25adfd`
- **Land before QA-201's lint attribute** (the `// Safety:` comments here would otherwise be flagged).
- **Files**: `src/python_bindings/observer.rs:445-448` (`PyCallbackObserver`), `:485-488` (`PyQueueObserver`).
- **Steps**:
  1. Delete the four `unsafe impl` lines and the two `// Safety:` comments above them.
  2. Add a compile-time check in the file's tests (or a `const _` block): `fn assert_send_sync<T: Send + Sync>() {}` invoked for both types.
  3. No CHANGELOG entry.
- **Method**: both structs hold `Py<PyAny>` and `Option<HashSet<TerminalEventKind>>`. pyo3 0.29.2 implements `Send` and `Sync` for `Py<T>` unconditionally (`pyo3-0.29.2/src/instance.rs:1498,1505`), so the auto traits already apply; `TerminalObserver: Send + Sync` (`src/observer.rs:27`) would fail to compile if they did not.
- **Verify**:
  - `cargo test --lib --no-default-features --features python-test`
  - `make dev && uv run pytest tests/ -k observer -v`
  - `make checkall`

### [QA-211] Dead `web-terminal-frontend/components/TerminalDebug.tsx`
- **Card**: `01a0ef5b9d4f75e3a7a48ed1721a0488`
- **Files**: `web-terminal-frontend/components/TerminalDebug.tsx` (286 lines; ungated `console.log` at `:87`). Zero importers (`grep -rln TerminalDebug web-terminal-frontend --include=*.ts --include=*.tsx` → only itself).
- **Steps**:
  1. `git rm web-terminal-frontend/components/TerminalDebug.tsx`.
  2. `make test-web`.
  3. `make web-build-static` and commit any change under the tracked `web_term/` (32 tracked files). Since nothing imported the component, the bundle is likely unchanged; commit whatever the build produces.
  4. No CHANGELOG entry.
- **Method**: re-run the importer grep after Phase 3 web edits (QA-192 does not touch the frontend) before deleting.
- **Verify**: `make test-web`, `make web-build-static`, `git status --short web_term`, `make checkall`.

### [QA-212] 21 `too_many_arguments` suppressions
- **Card**: `01a0ef5ba0b97043bb82ce12ec0c0d28`
- **Files** (21, counted with `grep -rn 'too_many_arguments' src`): `src/streaming/server.rs` 5 (`handle_client_message` `:1274`, `handle_mouse` `:1549`, `handle_selection_request` `:1716`, `handle_clipboard_request` `:1821`, `send_trigger_matched` `:2091`); `src/streaming/protocol.rs` 4 (`connected_full` `:1143`, `selection_changed` `:1349`, `system_stats` `:1410`, `mouse` `:1602`); `src/python_bindings/common.rs` 2 (`screenshot` `:2573`, `screenshot_to_file` `:2693`); `src/python_bindings/streaming.rs` 2 (`new` `:38`, `send_trigger_matched` `:974`); `src/screenshot/renderer.rs` 2 (`render_char` `:404`, `render_shaped_glyph` `:686`); one each: `src/ansi_utils.rs:68` (`generate_sgr`), `src/graphics/mod.rs:329` (`sample_half_block_in`), `src/python_bindings/screenshot_config.rs:85` (`new`), `src/python_bindings/terminal/color_api.rs:278` (`add_rendering_hint`), `src/python_bindings/terminal/mouse_api.rs:46` (`record_mouse_event`), `src/streaming/py_convert.rs:106` (`system_stats_to_py_dict`).
- **Steps** (convert where the call sites allow; keep the rest with a reason):
  1. Convert (internal signatures):
     - `server.rs`: introduce `struct ConnCtx<'a> { transport_label: &'a str, client_id: uuid::Uuid, read_only: bool }` passed to `handle_client_message` and its per-variant handlers; per-connection mutable state (`subscriptions`, `rate_limiter`) stays as two `&mut` params. `handle_mouse` takes a `MouseInput { col, row, button, shift, ctrl, alt, event_type }` (shares QA-213's enum).
     - `renderer.rs`: `struct GlyphDraw { … }` for `render_char`/`render_shaped_glyph` (read both signatures and their callers first).
     - `graphics/mod.rs:329` `sample_half_block_in` (`pub(crate)`): a params struct.
  2. Keep with a one-line reason comment above the `#[allow]`:
     - PyO3 `#[new]`/`#[pymethods]` (`python_bindings/streaming.rs:38,:974`, `screenshot_config.rs:85`, `color_api.rs:278`, `mouse_api.rs:46`, `common.rs:2573,:2693`): "Python keyword arguments; a struct would change the Python API."
     - Public Rust API (`ansi_utils::generate_sgr`, `py_convert::system_stats_to_py_dict`, `protocol.rs` constructors, `server.rs:2091 send_trigger_matched`): "public API; changing it breaks embedders." `protocol.rs::connected_full` has only a test caller (`protocol.rs:1833`) and CLAUDE.md names `ConnectedBuilder` as the single edit site: mark it `#[deprecated(note = "use ConnectedBuilder")]` instead of refactoring.
  3. One commit per file group; CHANGELOG only for the `connected_full` deprecation.
- **Method**: before each conversion run `get_impact <fn>` (repository `par-term-emu-core-rust`, direction upstream) to confirm callers are internal. Expect ~9 of 21 removed; the remaining allows gain a reason.
- **Verify**:
  - `make test-rust-streaming`
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize screenshot` and `… graphics`
  - `make dev-streaming && uv run pytest tests/test_streaming.py -v`
  - `make checkall`

### [QA-213] Mouse `event_type` is still a `String`
- **Card**: `01a0ef5ba3a376b3ad827a11d3b44d8a`
- **Files** (corrected; AUDIT's `protocol.rs:541` and `:1393` are `ShellIntegrationEvent.event_type`, which is out of scope): `src/streaming/protocol.rs:800-813` (`ClientMessage::Mouse`, `event_type: String` at `:812` with `pydict(default = "press".to_string())` at `:811`), `:1602-1621` (`ClientMessage::mouse` constructor, param at `:1609`); `src/streaming/proto.rs:589-604` (app → pb), `:1009-1017` (pb → app); `src/streaming/server.rs:1343-1354` (dispatch), `:1549-1575` (`handle_mouse`, `event_type != "release"` at `:1570`). Wire: `terminal.pb.rs:659` stays `string`. Frontend sends `'press'`/`'release'` (`web-terminal-frontend/components/Terminal.tsx:778,786`).
- **Steps**:
  1. In `protocol.rs` add:
     ```rust
     #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
     #[serde(rename_all = "lowercase")]
     pub enum MouseEventType { Press, Release, Move, Scroll }
     impl MouseEventType {
         pub fn as_str(self) -> &'static str { … }
         pub fn parse(s: &str) -> Option<Self> { … }
     }
     ```
  2. `ClientMessage::Mouse.event_type: MouseEventType`; `mouse()` takes it. For the Python dict codec use the derive's escape hatches (`derive/src/lib.rs:75-80`: `to_with`/`from_with`, `default`) with `default = MouseEventType::Press` and converters to/from `str`.
  3. `proto.rs`: app → pb `event_type.as_str().to_string()`; pb → app `MouseEventType::parse(&mouse.event_type).unwrap_or(MouseEventType::Press)` (matches today's behavior: anything but `"release"` counts as pressed). Decide explicitly whether unknown strings become `Press` (compatible) or an error; choose `Press` and log at debug.
  4. `server.rs:1570`: `let pressed = event_type != MouseEventType::Release;`.
  5. CHANGELOG `[Unreleased]` → `### Changed` (breaking for Rust embedders constructing `ClientMessage::Mouse`): "`ClientMessage::Mouse.event_type` is a `MouseEventType` enum; the wire format is unchanged (QA-213)."
  6. Tests: `proto.rs` round trip for all four variants plus `"bogus"` → `Press`; the Python dict round trip in `tests/test_streaming.py` if it covers mouse messages (verify: `grep -n mouse tests/test_streaming.py`).
- **Method**: `get_impact mouse` (file `src/streaming/protocol.rs`) and `grep -rn 'ClientMessage::Mouse' src tests` enumerate sites. par-term does not construct `ClientMessage::Mouse` (grep of `~/Repos/par-term` found none). No frontend or `.proto` change, so no `make proto-*` or `web-build-static`.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::`
  - `make test-rust-streaming`
  - `make dev-streaming && make stubs && uv run pytest tests/test_streaming.py -v`
  - `make checkall`

### [QA-214] Near-duplicates persist
- **Card**: `01a0ef5ba6ef71b383ace69341001d65`
- **Files** (parsight `find_duplicate_code`, `min_lines: 12`, `similarity: 0.90`, confirmed at f6535f2; all report distinct AST shapes, so diff each pair before merging):
  - `sample_half_block`: `src/graphics/mod.rs:521-538` / `src/python_bindings/types/graphics.rs:167-184` (0.99)
  - `src/python_bindings/enums.rs` `From` pairs `:256-271`/`:275-290` and `:492-506`/`:510-524` (0.98)
  - `spawn_login_shell` `src/pty_session.rs:772-794`/`:798-809` (0.97) — `#[cfg(unix)]`/`#[cfg(not(unix))]` variants: **genuine, skip**.
  - `resize_pixels` `src/python_bindings/pty.rs:212-229` / `src/python_bindings/terminal/mod.rs:167-180` (0.97) — different backends; share only validation.
  - `src/streaming/proto.rs` `From` `:648-677`/`:1048-1078` (0.96)
  - `erase_rectangle`/`_unconditional` `src/grid/rect.rs:85-104`/`:107-130` (0.96) — **resolved by QA-218** (removes `erase_rectangle`); skip here.
  - `encode_server_message`/`encode_client_message` `src/python_bindings/streaming.rs:1128-1147`/`:1201-1220` (0.95)
  - `export_visible_screen_styled`/`_lines` `src/grid/export.rs:287-327`/`:338-374` (0.95)
  - `trigger_notification`/`trigger_custom_notification` `src/python_bindings/terminal/notification_api.rs:45-90`/`:172-193` (0.93)
  - Underline renderers `src/screenshot/renderer.rs:831-845`/`:848-862`/`:892-910` (0.91)
  - `create_pane`/`create_argv_pane` ×4 `src/mux/pane.rs:487-499,:676-692,:697-713,:789-805` (0.85; ARC-103/ARC-113 edit this file first).
- **Steps** (one commit each):
  1. `types/graphics.rs::sample_half_block` delegates to the core `graphics::sample_half_block` (or `sample_half_block_in`).
  2. `enums.rs`: collapse each From pair with a small `macro_rules!` that emits both directions from one variant list.
  3. `resize_pixels`: extract `fn check_positive_dims(cols: usize, rows: usize) -> PyResult<()>` shared by both bindings (after QA-182's `to_u16`).
  4. `proto.rs` pair, `encode_*_message` pair, `trigger_notification` pair: extract the shared body into a private helper parameterized by the differing part (read each pair's diff first).
  5. `export_visible_screen_styled` = `export_visible_screen_styled_lines(...).join("\n")` if the diff shows that is the only difference (verify trailing-newline behavior with the existing export tests).
  6. Underlines: one `fn render_underline(&mut self, …, style: UnderlineStyle)` with the per-style row pattern.
  7. `pane.rs` factories: after ARC-103 re-measure; if still ≥0.85 similar, extract the shared "new PtySession + apply context + spawn" body into one private fn taking a spawn closure.
  8. No CHANGELOG entries (internal).
- **Method**: re-run `find_duplicate_code` (repository `par-term-emu-core-rust`, same parameters) after the batch; each resolved pair should drop out. For each helper run `get_impact` on the kept function.
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize graphics` / `grid` / `screenshot`
  - `make test-rust-streaming`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` (if step 7 ran)
  - `make dev-streaming && uv run pytest tests/ -v`
  - `make checkall`

### [QA-215] `SharedState` keeps a separate `cell_count` and uses `as_mut_ptr` plus `mem::forget`
- **Card**: `01a0ef5ba98e7cd2a97aaaec99133240`
- **Executed inside the QA-201 batch, first** (order **QA-215** → QA-208 → QA-201).
- **Files**: `src/ffi.rs:100-103` (`cells: *mut SharedCell`, `cell_count: u32` — both in the C ABI, `include/terminal_core.h:267-273`), `:155-177` (`cell_count = (cols * rows) as u32` computed separately from the vec; `into_boxed_slice` + `as_mut_ptr` + `mem::forget`), `:218-225` (Drop: `from_raw_parts_mut` + `Box::from_raw`, guarded by `cell_count > 0`).
- **Steps**:
  1. Build: `let cells: Box<[SharedCell]> = cells_vec.into_boxed_slice(); let cell_count = u32::try_from(cells.len()).expect("grid cells fit in u32"); let cells = Box::into_raw(cells).cast::<SharedCell>();`. `cell_count` now comes from the allocation itself.
  2. Drop: `if !self.cells.is_null() { unsafe { drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.cells, self.cell_count as usize))) } }`. Remove the `cell_count > 0` guard: a zero-length `Box<[T]>` round-trips through `into_raw`/`from_raw` (dangling non-null pointer, never dereferenced).
  3. Keep both C fields; the ABI and header do not change (`make ffi-header-check` must stay clean). The finding's "separate `cell_count`" is fixed by deriving it, not removing it.
  4. Test (in `ffi.rs` tests): `shared_state_cell_count_matches_grid`: create 7×3, `terminal_get_state`, assert `cell_count == 21`, read the last cell through the pointer, `terminal_free_state`. Run under Miri if available (`cargo +nightly miri test --lib … ffi::tests::shared_state` — optional; say whether it ran).
  5. No CHANGELOG entry.
- **Method**: `get_symbol_context SharedState` (file `src/ffi.rs`) → `terminal_get_state`, `terminal_free_state`, tests, `benches/ffi_readback.rs`. Python does not use it.
- **Verify**: the QA-201 batch gates (`cargo test --lib --no-default-features --features pyo3/auto-initialize ffi`, `make ffi-header-check`, `make xcframework`, `make checkall`).

### [QA-216] Production unwraps
- **Card**: `01a0ef5bac3e7592bbff55bd4cfb0301`
- **Files**: `src/streaming/server.rs:2519-2527` (`WWW_AUTHENTICATE` pushed as a `&str`, then `value.parse().unwrap()` at `:2526`); `src/terminal/file_transfer.rs:152-172` (`append_data`: `get_mut(&id)` at `:153`, then `self.active_transfers.remove(&id).unwrap()` at `:167`); `src/mux/foreground.rs:311,313` (inside a closure returning `Option`), `:355-362` (loop over the kinfo buffer); `src/macros.rs:215-216,262` (`k.chars().next().unwrap()` after `k.len() == 1`); `src/grid/export.rs:428,448` (`writeln!(String).unwrap()` in `debug_snapshot`).
- **Steps**:
  1. `server.rs`: build the header directly, `response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"Terminal Server\""))`, and drop the `Vec<(HeaderName, &str)>` + parse loop.
  2. `file_transfer.rs`: restructure so no lookup can fail: check the size limit with `get(&id)` (early `Err` when absent), and on overflow `let Some(mut transfer) = self.active_transfers.remove(&id) else { return Err(…) }`, set its status/`completed_at`, `push_completed`, return the error. The happy path keeps `get_mut`.
  3. `foreground.rs`: replace `buf[a..a + 4].try_into().unwrap()` with `*buf[a..].first_chunk::<4>()?` (the enclosing closure/fn returns `Option`; verify the second site's return type before using `?`, else `.expect("length checked above")`).
  4. `macros.rs`: in the guards, bind the single char without unwrap: match arm `k if k.chars().count() == 1 && has_ctrl && …` → restructure as `let mut chars = k.chars(); if let (Some(ch), None) = (chars.next(), chars.next()) { … }` at both sites. Note `k.len() == 1` is a byte check; the new form is char-correct and keeps ASCII behavior.
  5. `export.rs`: `writeln!(output, …).expect("writing to a String cannot fail")` (or `let _ =`); keep behavior.
  6. No CHANGELOG entry (no behavior change).
- **Method**: AUDIT says the first two "can fail"; at HEAD neither can (a static, valid header literal; a key just found by `get_mut`). The fix is still worth making: it removes the unwraps clippy flags and makes the invariants structural. Count check: `cargo clippy --no-default-features --features rust-only,streaming,streaming-bin,mux,serde -- -A clippy::all -W clippy::unwrap_used 2>&1 | grep -c 'unwrap()'` drops by 11 (the sites listed).
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize file_transfer` and `… macros` and `… grid`
  - `make test-rust-streaming`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::foreground`
  - `make checkall`

### [QA-217] Weak `assert … is not None` checks
- **Card**: `01a0ef5baf667e519dd42a71e7d5955b`
- **Files** (re-counted: 25 in `tests/test_terminal.py`, 12 in `tests/test_terminal_bindings.py`; most are pyright type-narrowing followed by value asserts and must stay). The asserts with **no value check after them**:
  - `tests/test_terminal.py:584` (`\x1b[42m` bg), `:593` (`\x1b[38;5;196m` fg), `:602` (`\x1b[48;5;21m` bg).
  - `tests/test_terminal_bindings.py:23` (`term is not None`, redundant), `:222` (`\x1b[31m` fg), `:233` (`\x1b[42m` bg), `:244` (`\x1b[38;2;255;128;64m`), `:254` (`\x1b[38;5;123m`), `:263` (`\x1b[31;42m` fg and bg), `:288`/`:290` (bold, `hasattr` check), `:299` (italic), `:308` (underline), `:317` (bold+italic+underline).
- **Steps** — assert the values (`get_fg_color`/`get_bg_color` return `cell.fg.to_rgb()`, `src/python_bindings/common.rs:1370-1392`; named palette `src/color.rs:85-100`; indexed `src/color.rs:108-125`):
  - `test_terminal.py:584`: `assert bg_color == (0, 128, 0)`; `:593`: `assert fg_color == (255, 0, 0)` (index 196); `:602`: `assert bg_color == (0, 0, 255)` (index 21).
  - `test_terminal_bindings.py:23`: delete the line. `:222`: `== (128, 0, 0)`; `:233`: `== (0, 128, 0)`; `:244`: `== (255, 128, 64)`; `:254`: `== (102, 255, 255)` (index 123); `:263`: `fg == (128, 0, 0) and bg == (0, 128, 0)`; `:288-290`: `assert attrs.bold is True` (drop the `hasattr`); `:299`: `attrs.italic is True`; `:308`: `attrs.underline is True`; `:317`: all three.
  - Keep the `is not None` line where pyright needs the narrowing, and add the value assert after it.
  - Leave the 21 narrowing asserts in `test_terminal.py` that are already followed by value asserts.
- **Method**: compute each expected RGB from `src/color.rs` rather than trusting the test's comment; run once and fix any mismatch by re-deriving, never by loosening.
- **Verify**: `make dev && uv run pytest tests/test_terminal.py tests/test_terminal_bindings.py -v`, then `make lint-python`, then `make checkall`.

### [QA-218] Dead code (DECSERA arm, `Grid::erase_rectangle`, `debug.py` helpers, `fire_output_callback`)
- **Card**: `01a0ef5bb1ed7b63a3f581b8b1766274`
- **Files and verified status**:
  - `src/terminal/sequences/csi/window.rs:104-127` (`'{'` arm): **dead**. `csi/mod.rs:106-111` routes `'{'` with `$` to `handle_decsera` (`src/terminal/sequences/csi/erase.rs:140`); `handle_csi_window` is called only for `'t'|'r'`, `'s'`, `'x'`, `'v'|'z'` (`csi/mod.rs:74-104`).
  - `src/grid/rect.rs:85-104` (`Grid::erase_rectangle`): production-reachable only from that arm; also exercised by `src/grid/tests.rs:673` (`test_erase_rectangle`). `Terminal::erase_rectangle` (`src/terminal/mod.rs:3699`) goes through `fill_rectangle`, not this. No caller in `~/Repos/par-term` (grep, excluding `.claude/worktrees` and `target`).
  - `python/par_term_emu_core_rust/debug.py:149-224`: **AUDIT is wrong that these are unreferenced.** `~/Repos/par-term-emu-tui-rust` imports five of the nine: `log_render_call`, `log_render_content`, `log_screen_corruption` (`src/par_term_emu_tui_rust/terminal_widget/rendering.py:9-17`), `log_generation_check`, `log_widget_lifecycle` (`terminal_widget.py:13-18`; also stubbed in `tests/conftest.py`). Unused in that repo: `log_snapshot`, `log_terminal_state`, `log_textual_event`, `log_get_line_cells_call`.
  - `src/pty_session.rs:290` `PtySession::fire_output_callback`: **used by par-term** — `par-term-terminal/src/terminal/spawn.rs:234` (`pty.fire_output_callback(data)` in `process_mux_output`, last touched in par-term 757bf9b7). **Keep it; drop it from this finding.**
- **Steps**:
  1. Delete the `'{'` arm in `window.rs:104-127`.
  2. Delete `Grid::erase_rectangle` (`rect.rs:85-104`) and `test_erase_rectangle` (`grid/tests.rs:673-~695`). This also resolves QA-214's `erase_rectangle`/`_unconditional` pair. CHANGELOG `[Unreleased]` → `### Removed`: "`Grid::erase_rectangle` (unreachable; DECSERA uses the terminal-level handler) (QA-218)" — breaking for Rust embedders, none known.
  3. `debug.py`: re-read first (SEC-131 rewrote its logger in Phase 1). Keep the five helpers the TUI imports. Remove `log_snapshot`, `log_terminal_state`, `log_textual_event`, `log_get_line_cells_call` only after re-running `grep -rn '<name>' ~/Repos/par-term-emu-tui-rust --include=*.py` for each (excluding `.venv`). CHANGELOG `### Removed` for the four. If `debug.py` is re-exported or listed in `docs/API_REFERENCE.md`, update it (verify: `grep -n 'debug\.' docs/API_REFERENCE.md`).
  4. `fire_output_callback`: add a doc line stating the constraint so the next dead-code sweep does not flag it: "Called by embedders that feed daemon-sourced bytes into a mirror session (par-term's mux mirror)."
- **Method**: parsight `find_dead_code` misses cross-repo callers; for any public symbol, grep the sister repos (`~/Repos/par-term`, `~/Repos/par-term-emu-tui-rust`) before deleting. DECSERA behavior is covered by `src/terminal/tests/terminal_tests.rs:1940` (`test_decsera_erase_rectangle`), which must still pass (it goes through `handle_decsera`).
- **Verify**:
  - `cargo test --lib --no-default-features --features pyo3/auto-initialize decsera` and `… grid::`
  - `make dev && uv run pytest tests/ -v`
  - `cd ~/Repos/par-term-emu-tui-rust && uv run pytest -q` against a locally installed build if its tests import `debug` (optional; say whether it ran)
  - `make checkall`

### [QA-219] `new-window` and `new-session` silently ignore trailing command text
- **Card**: `01a0ef5bb5587b93951d0f46a9c26166`
- **Blocked by**: SEC-126 (introduces the leading-flag grammar in `src/mux/command.rs`; find its helper with `find_symbol` on the name SEC-126 used, e.g. `leading_flags`, `repository_id: par-term-emu-core-rust`, `file_path: src/mux/command.rs`).
- **Files**: `src/mux/command.rs:844-860` (`parse_new_session`: reads `-e`, `-s` only), `:953-959` (`parse_new_window`: `-t`, `-n`, `-c` only); `MuxCommand::NewSession`/`NewWindow` `:40-46`, `:87-100`; tests `:1403-1629` (`parses_new_session_*`, `new_window_keeps_a_quoted_name_whole`, `quoted_names_may_look_like_flags` `:1578`, `new_session_env_*`); `docs/MUX.md:163-164`.
- **Decision**: reject, do not implement. Neither known sender passes a command (par-term sends `new-window` bare, `par-term-tmux/src/prefix.rs:166`, and `new-session -s {name}{env_args}`, `par-term-mux/src/resync.rs:149`); implementing it needs new plan fields in `tree.rs` and dispatch plumbing. Rejecting turns a silent success into an error, which is the defect.
- **Steps**:
  1. With SEC-126's helper, parse the leading flags of each command against an explicit table of value-taking flags, and return an error for the first positional token:
     - `new-session` value flags: `-s`, `-e`, `-c`, `-n`, `-t`, `-x`, `-y`, `-F`, `-f` (tmux's set, so a tmux-shaped sender's `-x 80` is not misread as a positional); boolean: `-A`, `-d`, `-D`, `-E`, `-P`, `-X`.
     - `new-window` value flags: `-t`, `-n`, `-c`, `-e`, `-F`; boolean: `-a`, `-b`, `-d`, `-k`, `-P`, `-S`.
     - Error text: `format!("{}: unexpected argument {tok:?}: a start command is not supported; use respawn-pane", a.name)`.
  2. Quoted values must stay one token (`-n 'my win'`, `-e 'X=a b'`, `-s "\"x\""`); SEC-126's helper already handles quoting for `-c '/a b'`.
  3. `docs/MUX.md:163-164`: add to the Arguments column or a note under the table: "A trailing command is rejected; start the pane's program with `respawn-pane -t %N <command>`."
  4. CHANGELOG `[Unreleased]` → `### Changed`: "`new-window`/`new-session` reject trailing command text instead of ignoring it (QA-219)."
  5. Tests (`command.rs`):
     - `new_window_rejects_a_trailing_command`: `new-window sleep 5` and `new-window -n w sleep 5` are `Err` containing `"unexpected argument"`.
     - `new_session_rejects_a_trailing_command`: `new-session -s a top` is `Err`.
     - `new_session_accepts_tmux_value_flags`: `new-session -s a -x 80 -y 24 -d` is `Ok` (values not taken as positionals).
     - Existing quoting tests (`:1415`, `:1508-1629`) must pass unchanged.
- **Method**:
  - Enumerate senders before shipping: `grep -rn 'new-window\|new-session' ~/Repos/par-term --include=*.rs` (excluding `.claude/worktrees`, `target`) and the `par-mux` skill (`~/.claude/skills/par-mux/SKILL.md:65-66`, documents no command). Tests in this repo send only flag forms (`grep -rhno '"new-window[^"]*"\|"new-session[^"]*"' tests src`).
  - Pitfall: unknown flags stay tolerated as today; only positionals are rejected, so the value-flag table is what prevents false rejections.
- **Verify**:
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::command`
  - `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` and `cargo test --test mux_daemon --no-default-features --features rust-only,mux,serde`
  - `cargo test --no-default-features --features rust-only,mux,serde --test mux_targets --test mux_cli`
  - `make checkall`

---

## Phase 3d — Documentation (parallel domain; internal order as listed)

All paths are relative to `/Users/probello/Repos/par-term-emu-core-rust`. Line numbers were read at HEAD f6535f2. Phases 1, 2 and 3a–3c touch several of these files first, so re-read each file before editing and locate text by the quoted anchor, not the number.

**Ordering constraints inside 3d:**
- DOC-099 before DOC-109 (README What's New).
- DOC-122 before DOC-129 (MUX.md:422 links a fable plan that DOC-129 deletes).
- DOC-107, DOC-108 and DOC-133 all edit `docs/ARCHITECTURE.md`. Run them in that order, one at a time.
- DOC-108, DOC-130 and DOC-125 all edit `CLAUDE.md`. Run them in that order.
- DOC-111, DOC-123 and DOC-125 edit `docs/ADVANCED_FEATURES.md`. DOC-106, DOC-112 and DOC-125 edit `docs/STREAMING.md`. DOC-118 and DOC-125 edit `docs/MATURIN_BEST_PRACTICES.md`. Run DOC-125 last among these, and recompute its fence list at execution time.
- DOC-110, DOC-114 and DOC-124 edit `docs/SECURITY.md`. Run them one at a time.
- DOC-128 runs after the Makefile changes from QA-204, ARC-115 and ARC-121.
- DOC-116 regenerates the stub. Run it after every change to a binding docstring (DOC-100 case B, DOC-117, DOC-120) and after QA-190, or regenerate again at the end of 3d.

**CHANGELOG rule:** DOC-099 edits only `## [0.57.0]`. DOC-126 edits only the link-reference block at the bottom of the file. DOC-116 adds one `## [Unreleased]` bullet because the wheel ships the stub. No other DOC entry touches CHANGELOG.

**Board rule:** fix agents run no `kanban` command. The orchestrator closes the cards.

---

### [DOC-099] 0.57.0 CHANGELOG and README omit `split-window -b` and the `pane-info cmd=` reply token
- **Card**: `01a0ef5bb7cd7bb091e7ac8b150c1beb`
- **Files**:
  - `CHANGELOG.md:10-27` (`## [0.57.0] - 2026-09-29`, `### Added` runs from :12 to :15)
  - `README.md:22` (the 0.57.0 What's New paragraph)
  - Evidence: commit `1c4c479` ("feat(mux): split-window -b and pane-info cmd= foreground token") is an ancestor of the remote `v0.57.0` tag (`f6535f2`) and not of `v0.56.0`. `src/mux/dispatch.rs:700-730` (`cmd_pane_info`) appends ` cmd=<base64>` only when the name is known.
- **Steps**:
  1. In `CHANGELOG.md`, under `## [0.57.0]` → `### Added`, insert this bullet after the one starting `- **par-mux dead-pane hold and respawn**` and before `### Changed`:
     ```markdown
     - **par-mux `split-window -b` and a `pane-info` foreground-command token** (`1c4c479`): `split-window -b` places the new pane before its target — left of it under `-h`, above it in the default direction — keeping exactly its `-p` share. `pane-info` replies append an optional last `cmd=<base64>` token (standard base64 of the pane's foreground command name, tmux `#{pane_current_command}`), absent on Windows or when the process argv is unreadable. The fixed `%N @W COLSxROWS` prefix is unchanged, so existing parsers keep working.
     ```
  2. Optional, same section: in the `### Fixed` ENH-028 bullet, the C type is `TermKeyOptions` (`include/terminal_core.h:526`), while `KeyEncodeOptions` is the Rust name. Replace `FFI \`terminal_encode_key_ex\` + \`KeyEncodeOptions\`` with `FFI \`terminal_encode_key_ex\` + \`TermKeyOptions\` (Rust \`KeyEncodeOptions\`)`.
  3. In `README.md:22`, replace the substring
     `and brought back with **\`respawn-pane\`**. The shared key encoder`
     with
     `and brought back with **\`respawn-pane\`**; \`split-window -b\` places a new pane before its target, and \`pane-info\` replies gain an optional trailing \`cmd=<base64>\` foreground-command token. The shared key encoder`
- **Method**: This records what v0.57.0 shipped, so edit only `[0.57.0]`. SEC-129 may add filtering to the `cmd=` name. That is a post-release change, and its own bullet goes under `[Unreleased]`. Do not describe it here.
- **Verify**:
  ```bash
  awk '/^## \[0.57.0\]/,/^## \[0.56.0\]/' CHANGELOG.md | grep -c 'split-window -b'   # 1
  awk '/^## \[0.57.0\]/,/^## \[0.56.0\]/' CHANGELOG.md | grep -c 'cmd=<base64>'     # 1
  sed -n '22p' README.md | grep -c 'split-window -b'                                  # 1
  awk '/^## \[Unreleased\]/,/^## \[0.57.0\]/' CHANGELOG.md | grep -c 'split-window -b'  # 0
  ```

### [DOC-100] `TmuxNotification.notification_type` omits `pane-exited`/`pane-respawned`; the exit code's location is undocumented
- **Card**: `01a0ef5bbae37933ac709072e2ee27ad`
- **Blocked by**: QA-190.
- **Files**:
  - `docs/API_REFERENCE.md:1833` (`### TmuxNotification`), `:1838` (`notification_type` list, 32 strings), `:1840` (`pane_id`), `:1843` (`name`). The section sits under `## Data Classes` (:1275), so `scripts/check_api_reference.py` checks its Properties bidirectionally against the stub.
  - Code: `src/tmux_control.rs:229-245` (`notification_type()`, 34 arms, :241-242 add the two new ones); `src/python_bindings/types/notification.rs:620-640` (the `PaneExited` arm sets `name: exit_code.map(|code| code.to_string())`; the `PaneRespawned` arm sets only `pane_id`).
- **Detect QA-190**: `grep -n 'pub exit_code' src/python_bindings/types/notification.rs`. A hit means QA-190 landed (case A). No hit means case B.
- **Steps**:
  1. Both cases: in the `:1838` list, insert `` `pane-exited`, `pane-respawned`, `` after `` `pane-title-changed`, ``. Replace the trailing sentence `The agent and pane-title types are par-mux-fed; compare against the hyphenated string exactly.` with:
     `The agent, pane-title, pane-exited and pane-respawned types are par-mux-fed; compare against the hyphenated string exactly.`
  2. Both cases: replace the `:1840` bullet with:
     ``- `pane_id: str | None`: Pane identifier (`%N`); set for every pane-scoped notification, including `pane-exited` (the held pane) and `pane-respawned` (the restarted pane)``
  3. **Case A (QA-190 landed)**:
     - If QA-190 did not already add it, add this property bullet directly after `name`:
       ``- `exit_code: int | None`: For `pane-exited`, the held pane's exit code; `None` when it was unreadable (signal death, or reaped before the daemon read it) and for every other notification type``
     - Read the `PaneExited` arm in `notification.rs`. If `name` still carries the code (AUDIT's QA-190 remedy keeps it for one release), replace the `:1843` bullet with:
       ``- `name: str | None`: Session/window name; for `agent-state-changed`, `agent-released` and `agent-telemetry-changed`, the agent label; for `pane-exited`, the exit code as a decimal string (deprecated — read `exit_code`; kept for one release)``
     - If `name` is now `None` for `pane-exited`, leave the `:1843` bullet unchanged.
  4. **Case B (QA-190 not landed)**: replace the `:1843` bullet with:
     ``- `name: str | None`: Session/window name; for `agent-state-changed`, `agent-released` and `agent-telemetry-changed`, the agent label; for `pane-exited`, the pane's exit code as a decimal string, `None` when it was unreadable (signal death, or reaped before the daemon read it)``
     Optionally, and only if QA-190 is not in flight on `notification.rs`, change the Rust field doc at `notification.rs:29` from `/// Name (for window/session rename notifications)` to `/// Name: session/window name, agent label, or (pane-exited) the exit code as a decimal string`.
- **Method**:
  - The list must equal the `notification_type()` arms exactly. Clients switch on these strings, and the checker only compares property names.
  - Pitfall: in case A, `check_api_reference.py` fails `stub-check` if the stub gains `exit_code` and this section does not document it (empty `UNDOCUMENTED_PROPERTIES` allowlist).
  - Pitfall: a Rust doc edit changes `__doc__`, and DOC-116 copies `__doc__` into the stub. Regenerate the stub after this entry.
- **Verify**:
  ```bash
  python3 - <<'EOF'
  import re
  src = open('src/tmux_control.rs').read()
  body = src[src.index('pub fn notification_type'):]
  body = body[:body.index('\n    }\n')]
  code = set(re.findall(r'=> "([a-z-]+)"', body))
  line = next(l for l in open('docs/API_REFERENCE.md') if l.startswith('- `notification_type: str`'))
  doc = set(re.findall(r'`([a-z-]+)`', line.split('runtime strings:')[1]))
  print('missing:', sorted(code - doc), 'extra:', sorted(doc - code))
  assert code == doc
  EOF
  make checkall   # stub-check runs check_api_reference.py (Properties both ways)
  ```

### [DOC-101] MUX.md says `kill-pane` is refused for a window's last pane; the code closes the window (and session)
- **Card**: `01a0ef5bbd8f7203bf5bab8202fbed76`
- **Files**:
  - `docs/MUX.md:216` (the wrong sentence), `:181` (the `kill-pane` row in the command table, which also omits the close notifications)
  - Code: `src/mux/tree.rs:1441-1470` (`Err(_)` on the window's only pane → `Some(*id)`, window removed, session removed when empty); `src/mux/dispatch.rs:508-545` (`cmd_kill_pane`: survivor → `%layout-change` + `%window-pane-changed`; otherwise `%window-close`, then `%sessions-changed` when `removed_session.is_some()`)
  - Tests pinning it: `tree.rs:1974,1988`, `server.rs:3080` (AUDIT locations; re-grep `killing_the_last_pane` if they moved)
- **Steps**:
  1. Replace the whole `:216` bullet with:
     ```markdown
     - **`kill-pane`** of a window's last pane closes the window (`%window-close`), and of a session's last window closes the session (`%sessions-changed`). A pane whose child process exits on its own (rather than via `kill-pane`) is instead HELD with its exit code (remain-on-exit) and announced — see [Pane Reaping](#pane-reaping) below; `respawn-pane` restarts it in place.
     ```
  2. Replace the `:181` table row with:
     ```markdown
     | `kill-pane` | `-t <pane>` | empty | `%layout-change`, `%window-pane-changed`; `%window-close` instead when it was the window's last pane, then `%sessions-changed` when that emptied the session |
     ```
- **Method**: The rewrite matches the tests and MUX.md:352 ("an explicit `kill-pane` of the last pane"). Leave the remain-on-exit sentence intact. If ARC-090 or QA-187 changed `kill_pane`, re-read `cmd_kill_pane` before writing the row.
- **Verify**:
  ```bash
  grep -c 'is refused for a window' docs/MUX.md          # 0
  grep -n '^| `kill-pane`' docs/MUX.md | grep -c 'window-close'   # 1
  ```

### [DOC-102] MUX.md save list and Notifications table disagree with `mutates()` and `emit.rs`
- **Card**: `01a0ef5bc0057b81a3aa564c1b44be27`
- **Files**:
  - `docs/MUX.md:350` (the "When it saves" list), `:221-241` (the Notifications table; `%session-changed` row at :235, `%sessions-changed` row at :236), `:162` and `:166` (the `new-session` and `kill-window` command rows)
  - Code, derived at HEAD:
    - `src/mux/command.rs:315-351` `mutates()`: true for `NewSession`, `KillPane`, `NewWindow`, `SelectWindow`, `KillWindow`, `RenameWindow`, `SplitWindow`, `SelectPane`, `ResizePane`, `SwapPanes`, `BreakPane`, `JoinPane`, `MoveWindow`, `SwapWindows`, `RespawnPane`, `SetBuffer`, `SetEnvironment`, `RenameSession`, `KillSession`, and `RefreshClient` only with `-C`.
    - `src/mux/emit.rs:65-67` emits `%session-renamed {session_id} {name}` (name = rest of line, no quoting).
    - `TmuxNotification::SessionsChanged` is emitted in `src/mux/dispatch.rs` by `cmd_new_session` (:303, always), `cmd_kill_pane` (:542, when the session emptied), `cmd_join_pane` (:836, when the source session emptied), `cmd_move_window` (:919, always), `cmd_swap_windows` (:936, always), `cmd_kill_window` (:1050, when the session emptied) and `cmd_kill_session` (:1139, always). `cmd_break_pane` never emits it. `SessionRenamed` comes only from `cmd_rename_session` (:1113).
- **Steps**:
  1. In `:350`, change `(the structural set: \`new-session\`, \`set-environment\`,` to `(the structural set: \`new-session\`, \`rename-session\`, \`kill-session\`, \`set-environment\`,`.
  2. Insert this row directly after the `%session-changed $N <name>` row:
     ```markdown
     | `%session-renamed $N <name>` | A session was renamed (`rename-session`). The name is the rest of the line, spaces included |
     ```
  3. In the `%sessions-changed` row, replace the first sentence `The session set changed — a session was created (\`new-session\`) or destroyed (a \`kill-window\`/\`kill-pane\`/reaper cascade emptied it).` with:
     `The session set changed — a session was created (\`new-session\`), destroyed (\`kill-session\`, or a \`kill-window\`/\`kill-pane\`/\`join-pane\` that emptied it), or its windows were reordered (\`move-window\`/\`swap-window\`).`
     Then replace `**Client contract:** on receiving it, re-run \`list-sessions\`;` with `**Client contract:** on receiving it, re-run \`list-sessions\` (and \`list-windows\` for the new order after a reorder);`. The rest of the cell stays.
  4. Command table rows:
     - `:162` `new-session` Broadcasts cell → `` `%window-add` per window, `%sessions-changed`; `%session-changed` to the issuer ``
     - `:166` `kill-window` Broadcasts cell → `` `%window-close`; `%sessions-changed` when it emptied the session ``
- **Method**:
  - The emitter list comes from the `SessionsChanged` call sites above. It matches AUDIT's proposed rewrite.
  - Steps 4 and the `list-windows` clause go beyond AUDIT, but they belong to the same contract drift. The reorder comment at `dispatch.rs:917-918` says clients re-query `list-windows`.
  - Held panes mean the reaper never removes panes (MUX.md:358), so "reaper" is dropped.
  - Optional: the `emit.rs:68-70` comment "(one created or destroyed)" is stale in the same way. It is a code comment, so leave it for ARC-094, which rewrites `emit.rs`.
- **Verify**:
  ```bash
  grep -c 'reaper cascade' docs/MUX.md                      # 0
  grep -c '^| `%session-renamed' docs/MUX.md                # 1
  sed -n '/When it saves/p' docs/MUX.md | grep -c 'rename-session`, `kill-session'   # 1
  # every mutates()==true command appears in the save list:
  python3 - <<'EOF'
  import re
  src = open('src/mux/command.rs').read()
  body = src[src.index('pub fn mutates'):]; body = body[:body.index('=> true')]
  variants = re.findall(r'MuxCommand::(\w+)', body)
  kebab = {re.sub(r'(?<!^)([A-Z])', r'-\1', v).lower().replace('swap-panes','swap-pane').replace('swap-windows','swap-window') for v in variants}
  line = next(l for l in open('docs/MUX.md') if 'When it saves' in l)
  missing = [k for k in sorted(kebab) if f'`{k}`' not in line]
  print('missing:', missing); assert not missing
  EOF
  ```

### [DOC-103] FFI_GUIDE ABI history stops at v2; the shipped ABI is v3
- **Card**: `01a0ef5bc3497233912d697327b94938`
- **Files**:
  - `docs/FFI_GUIDE.md:414-416` (`## ABI Version`, the sentence `Version 2 added ...`)
  - Truth: `src/ffi.rs:431` (`TERM_CORE_ABI_VERSION: u32 = 3`), `include/terminal_core_layout.h:20-25`
  - History (git): `4650702` set v1, `0f7d591` bumped to v2, `2626707` bumped to v3. `4650702` and `0f7d591` are both after `v0.55.0` and before `v0.56.0`, so v0.56.0 shipped **v2**, and v1 never appeared in a tagged release. `2626707` is after `v0.56.0` and in v0.57.0.
- **Steps**:
  1. In `:416`, replace `Version 2 added \`terminal_damage_generation\` and \`terminal_dirty_ranges_since\` (per-consumer damage). Guard at startup:` with:
     ```markdown
     Each version keeps the previous surface and adds to it:

     | Version | First release | Adds |
     |---------|---------------|------|
     | v1 | none (development builds only; 0.56.0 shipped v2) | The initial embedding surface: `terminal_abi_version`, lifecycle, `terminal_feed`, `terminal_dirty_ranges`/`terminal_mark_clean`, row and scrollback readback, cursor and mode state, `terminal_encode_key`, snapshots, and observers |
     | v2 | 0.56.0 | Per-consumer damage: `terminal_damage_generation`, `terminal_dirty_ranges_since` |
     | v3 | 0.57.0 | Per-side Option-key modes: `TermKeyOptions`, `terminal_encode_key_ex`, `TERM_MOD_ALT_RIGHT`, `TERM_OPTION_MODE_*` |

     Guard at startup:
     ```
  2. Add a v4 row only if D4 was approved and the ABI v4 batch (ARC-101/ARC-112/ARC-114) landed. Detect it with `grep -n 'TERM_CORE_ABI_VERSION: u32 = 4' src/ffi.rs`, and take the row's content from that batch's CHANGELOG `[Unreleased]` bullets.
- **Method**: AUDIT's table labels v1 as 0.56.0. The git history shows 0.56.0 shipped v2, so an embedder on 0.56.0 always reads `2`. The table above states that.
- **Verify**:
  ```bash
  grep -c '| v3 | 0.57.0' docs/FFI_GUIDE.md          # 1
  grep -n 'TERM_CORE_ABI_VERSION: u32 = ' src/ffi.rs   # the value must match the highest row
  make checkall                                        # ffi-surface-check, ffi-header-check
  ```

### [DOC-104] API_REFERENCE and RUST_USAGE call `terminal_core.h` hand-written; it is cbindgen-generated
- **Card**: `01a0ef5bc6427890ad92322703a7b67e`
- **Files**: `docs/API_REFERENCE.md:2480`, `docs/RUST_USAGE.md:579`. Truth: the `include/terminal_core.h:1-6` banner ("GENERATED by cbindgen from src/ffi.rs (ENH-027); do not edit — regenerate with `make ffi-header`").
- **Steps**:
  1. `docs/API_REFERENCE.md:2480`: replace `The authoritative reference is the hand-written header \`include/terminal_core.h\` —` with:
     `The authoritative reference is the cbindgen-generated header \`include/terminal_core.h\` (regenerate with \`make ffi-header\`; the \`TERM_*\` constants and layout asserts live in the hand-written \`terminal_core_layout.h\`) —`
  2. `docs/RUST_USAGE.md:579`: replace `hand-written header [\`include/terminal_core.h\`](../include/terminal_core.h) mirroring \`src/ffi.rs\`,` with:
     `the cbindgen-generated header [\`include/terminal_core.h\`](../include/terminal_core.h) (generated from \`src/ffi.rs\` by \`make ffi-header\`; the \`TERM_*\` constants and layout asserts live in the hand-written \`terminal_core_layout.h\`),`
- **Method**: This uses FFI_GUIDE.md:3 as the model wording. `terminal_core_layout.h` is still correctly called hand-written.
- **Verify**:
  ```bash
  grep -rn 'hand-written header' docs/ README.md     # no output
  grep -c 'make ffi-header' docs/API_REFERENCE.md docs/RUST_USAGE.md   # >=1 each
  ```

### [DOC-105] README "Running Tests" and web-frontend build commands fail
- **Card**: `01a0ef5bc9eb7601a5ac9f7895642f2f`
- **Files**:
  - `README.md:672-681` (`## Running Tests`), `README.md:623-639` (the "Building from Source" block under Web frontend)
  - Truth: `Makefile:217` (`test: test-rust test-rust-streaming test-python`), `:219-229` (`test-rust`), `:235` (`test-python: dev`), `:679-733` (`web-install` = `bun install`, `web-dev` prints `http://localhost:3000`, `web-build-static` copies `out/` to `web_term/`). `web-terminal-frontend/package.json:6` is `"dev": "next dev -H 0.0.0.0"` (no port flag, so Next's default 3000).
- **Steps**:
  1. Replace the Running Tests fenced block with:
     ````markdown
     ```bash
     # All tests: Rust, Rust streaming, and Python (rebuilds the extension first)
     make test

     # Rust tests only (plain `cargo test` fails to link under the default `python` feature)
     make test-rust

     # Python tests only (runs `make dev` first)
     make test-python
     ```
     ````
  2. Replace the "Building from Source" fenced block (from `cd web-terminal-frontend` through `cp -r out/* ../web_term/`) with:
     ````markdown
     ```bash
     # Install dependencies (bun)
     make web-install

     # Development server on http://localhost:3000
     make web-dev

     # Static build copied to web_term/ for par-term-streamer --web-root
     make web-build-static
     ```
     ````
- **Method**: The make targets carry the feature flags and the package manager, so the README stops duplicating them. `web-terminal-frontend/README.md:42-61` also shows `npm install`/`npm run dev`. It is outside this finding, so report it rather than edit it.
- **Verify**:
  ```bash
  awk '/^## Running Tests/,/^## Performance/' README.md | grep -v '^#' | grep -c '^cargo test'   # 0 (only the explanatory comment mentions it)
  grep -c 'port 8030' README.md          # 0
  grep -c 'npm run' README.md            # 0
  make -n test-rust >/dev/null && make -n web-build-static >/dev/null && echo targets-ok
  ```

### [DOC-106] STREAMING.md says a reaped pane ends the mux-backed session; held panes never end it
- **Card**: `01a0ef5bcd5e7d93b0abf7cd45f14f65`
- **Files**:
  - `docs/STREAMING.md:1563` (the `Pane closes` row of the Mux-Backed Sessions table)
  - Code: `src/streaming/mux_factory.rs:447-485`. The session ends on a layout that no longer holds the pane, on `WindowClose` for the pane's window, or on `Exit`. `PaneExited`/`PaneRespawned` fall to `_ => {}`.
- **Detect a behavior change**: `grep -n 'PaneExited\|PaneRespawned' src/streaming/mux_factory.rs`. At HEAD there is no hit. If a later change handles them, describe that behavior instead of step 2.
- **Steps**:
  1. Replace the row with:
     ```markdown
     | Pane closes | A pane killed (`kill-pane`, `kill-window`, `kill-session`) or missing from its window's new layout (moved away by `break-pane`/`join-pane`), a closed window, or a daemon shutdown ends the session. Closing a streaming session never kills the pane. |
     ```
  2. Add this row directly after it:
     ```markdown
     | Process exits | The pane is held (remain-on-exit): the session stays open on the frozen screen, and viewers get no exit cue. `respawn-pane` restarts the process in the same pane, and its output reaches the same session. |
     ```
- **Method**: This follows the `mux_factory` match arms. A moved pane leaves its original window's layout, which triggers the layout-miss arm. That is why `break-pane`/`join-pane` are named.
- **Verify**:
  ```bash
  grep -c 'killed, reaped' docs/STREAMING.md     # 0
  grep -c '^| Process exits |' docs/STREAMING.md # 1
  ```

### [DOC-107] README C-surface list and ARCHITECTURE omit the 0.56/0.57 FFI and damage additions
- **Card**: `01a0ef5bd0b174308ef5b8d467d0a9ef`
- **Files**:
  - `README.md:311-318`
  - `docs/ARCHITECTURE.md`:
    - :147-161 (the `Grid` struct block, which lacks `row_gen`/`gen`)
    - :231-242 (Triggers & Automation; the Multiplexer block at :234-237 was inserted in the middle of the trigger bullets, which resume at :238)
    - :236 (mux key submodules)
    - :256-273 (streaming submodules)
    - :276-278 (Utility Modules, `ffi.rs`)
    - :367 (`dirty_rows: HashSet<usize>,`)
    - :813 (`A[Terminal.screenshot]`)
  - Truth: the `src/ffi.rs` exports (21 `extern "C"` fns, listed in FFI_GUIDE.md:5-8); `src/grid/mod.rs:43-51` (`row_gen`, `gen`); `src/terminal/mod.rs:1110-1113` (`default_consumer_gen` on `Terminal`); `src/mux/mod.rs:11-30` (modules incl. `foreground`, `host_probe`, `ids`, `win_resume`); `src/streaming/mod.rs:68` (`mux_factory`); `src/terminal/{macros.rs:14,trigger.rs:134,benchmarks.rs:16}` (`MacroEngine`, `TriggerEngine`, `TerminalBenchmarks`); `src/screenshot/mod.rs:93,117` (`render_terminal`, `save_terminal`).
- **Steps**:
  1. `README.md:311`: replace `is attached to GitHub releases starting with 0.56.0 — until then, build it locally with \`make xcframework\`.` with `is attached to GitHub releases from 0.56.0 on; build it locally with \`make xcframework\` for unreleased commits.`
  2. `README.md:313-318`: replace the intro line and all four bullets with:
     ```markdown
     The C surface is a full embedding API, not just snapshots (contracts and examples in the [FFI Guide](docs/FFI_GUIDE.md)):

     - **Version check**: `terminal_abi_version` — compare against the header's `TERM_CORE_ABI_VERSION` at startup
     - **Lifecycle/input**: `terminal_create` / `terminal_free` / `terminal_feed` (VT bytes) / `terminal_resize`
     - **Damage**: `terminal_dirty_ranges` returns coalesced inclusive dirty-row ranges and `terminal_mark_clean` consumes them; independent renderers use `terminal_damage_generation` + `terminal_dirty_ranges_since`, so one consumer never hides damage from another
     - **Pinned readback**: `terminal_read_row` / `terminal_read_scrollback_row` / `terminal_scrollback_count` copy cells into caller-owned buffers — no allocation, no full-grid copy per frame; `terminal_get_cursor` / `terminal_get_modes` carry per-frame state
     - **Key encoding**: `terminal_encode_key` turns key events into PTY bytes (xterm legacy, kitty level-1 disambiguate, and modifyOtherKeys, honoring application cursor keys and the negotiated kitty flags); `terminal_encode_key_ex` adds per-side macOS Option-key modes through `TermKeyOptions`
     - **Snapshots and observers**: `terminal_get_state` / `terminal_free_state`, `terminal_add_observer` / `terminal_remove_observer`
     ```
  3. ARCHITECTURE `:367`: replace `    dirty_rows: HashSet<usize>,` with `    default_consumer_gen: u64,        // Damage generation seen by the built-in get_dirty_rows/mark_clean consumer`.
  4. ARCHITECTURE Grid block (`:147-161`): before the closing `}` add
     ```rust
         row_gen: Vec<u64>,             // Per-row damage generation (ENH-025)
         gen: u64,                      // Monotonic damage counter
     ```
     After that code block's closing fence, add this paragraph:
     ```markdown
     **Damage tracking (ENH-025).** The grid owns damage. Every mutator stamps the rows it changes with a fresh value from the grid's monotonic `gen` counter (`row_gen`), so a mutation cannot forget to mark damage. A consumer remembers `damage_generation()` and asks `dirty_rows_since(gen)`, and independent consumers never clear each other's damage. The built-in consumer behind `get_dirty_rows`/`mark_clean` keeps its own generation in `Terminal.default_consumer_gen`. The C equivalents are described in [FFI_GUIDE.md](FFI_GUIDE.md#per-consumer-damage).
     ```
  5. ARCHITECTURE `:813`: replace `    A[Terminal.screenshot]` with `    A["screenshot::render_terminal"]`.
  6. ARCHITECTURE `:231-242`: move the five trigger bullets at `:238-242` (`- \`TriggerRegistry\` with \`RegexSet\`…` through `- Character-to-grid-column mapping…`) up so they sit directly under `- Regex-based pattern matching on terminal output` (:232), before the `**Multiplexer Daemon**` heading line. Then add after them:
     ```markdown
     **Terminal Services** (`src/terminal/macros.rs`, `src/terminal/trigger.rs`, `src/terminal/benchmarks.rs`)
     - `MacroEngine`, `TriggerEngine` and `TerminalBenchmarks` are stateless services over a borrowed `Terminal`; they replaced the `Terminal` forwarding methods removed in 0.55.0 and 0.56.0
     ```
  7. ARCHITECTURE `:236` (mux key submodules): after `` `ipc.rs` (Unix socket / Windows named pipe transport), `` insert `` `ids.rs` (`$N`/`@N`/`%N` ids and allocation), `host_probe.rs` (30 s disk and git host-telemetry sweep, hardened git), `foreground.rs` (process-table snapshot for `pane-info cmd=` and hook-claim liveness), `win_resume.rs` (Windows resume transport), ``.
  8. ARCHITECTURE streaming submodules: after the `rate_limit.rs` bullet add ``  - `mux_factory.rs` - `MuxSessionFactory`: streaming sessions that mirror a par-mux pane (`--mux-socket`; needs `streaming` + `mux`)``.
  9. ARCHITECTURE Utility Modules: add as the first bullet `- \`keyboard.rs\` - Shared key-event encoder (xterm legacy, kitty level 1, modifyOtherKeys, macOS Option-key modes) behind the FFI \`terminal_encode_key*\` functions and Python \`Terminal.encode_key\``. In the `ffi.rs` bullet, after `C/C++` add `; the header is generated by cbindgen (\`make ffi-header\`) and drift-gated in \`checkall\``.
- **Method**:
  - The README list is built from the 21 exports in `src/ffi.rs`.
  - The damage paragraph states only fields that exist (`grid/mod.rs:43-51`, `terminal/mod.rs:1113`).
  - Step 6 fixes the structural damage the Multiplexer insertion left in the trigger list. It is in scope because it corrects the module listing this finding covers.
  - Mermaid needs quotes around a label that contains `::`.
- **Verify**:
  ```bash
  for f in $(grep -o 'extern "C" fn [a-z_]*' src/ffi.rs | awk '{print $3}'); do grep -q "\`$f\`" README.md || echo "README missing $f"; done   # no output
  grep -c 'dirty_rows: HashSet' docs/ARCHITECTURE.md        # 0
  grep -c 'Terminal.screenshot' docs/ARCHITECTURE.md        # 0
  for m in host_probe foreground win_resume mux_factory keyboard MacroEngine TriggerEngine TerminalBenchmarks; do grep -q "$m" docs/ARCHITECTURE.md || echo "missing $m"; done   # no output
  ```
  Then run parsight `find_broken_doc_links` (repository_id `par-term-emu-core-rust`) and confirm there is no new row for `docs/ARCHITECTURE.md` or `README.md`.

### [DOC-108] Feature tables omit `screenshot` and misstate their Includes columns
- **Card**: `01a0ef5bd326794288e7fbbd5f456b37`
- **Files**:
  - `docs/RUST_USAGE.md:463-478` (`## Feature Flags` table)
  - `docs/BUILDING.md:81-93` (the feature bullets; `pty_session` :86 and `sim` :92 **already exist**)
  - `docs/ARCHITECTURE.md:955-1033` (`**Cargo.toml features:**` plus the verbatim `toml` block, which has drifted)
  - `CLAUDE.md:143` (the `mux` row)
  - Truth, `Cargo.toml` `[features]`:
    - `python`/`python-test` include `screenshot`
    - `screenshot = ["dep:swash"]`
    - `mux = ["pty_session", "interprocess", "widestring", "windows-sys", "serde", "dirs", "dep:toml", "clap"]`
    - `widestring`/`windows-sys` are `[target.'cfg(windows)'.dependencies]`
    - `[[bin]] par-mux` `required-features = ["mux"]`
- **Detect ARC-106**: `grep -n '^mux-bin' Cargo.toml`. A hit means `mux` no longer pulls `clap`, and the binary needs `mux-bin`.
- **Steps**:
  1. RUST_USAGE table:
     - `python` row Includes → `` `pyo3`, `pyo3/extension-module`, `par-term-emu-derive`, `pty_session`, `screenshot` ``
     - `python-test` row Includes → `` `pyo3`, `pyo3/auto-initialize`, `par-term-emu-derive`, `pty_session`, `screenshot` ``
     - Insert after `python-test`:
       ``| `screenshot` | Terminal-to-image renderer (`screenshot::render_terminal` / `save_terminal`, embedded fonts). Enabled by `python` and `python-test`; opt-in for `sim` (`features = ["sim", "screenshot"]`) | `swash` |``
     - `mux` row Includes (no ARC-106) → `` `pty_session`, `interprocess`, `widestring` and `windows-sys` (Windows only), `serde`, `dirs`, `toml`, `clap` ``
     - With ARC-106: drop `clap` from `mux`, and insert after it ``| `mux-bin` | The `par-mux` daemon binary: `mux` plus its CLI parser | `mux`, `clap` |``. Copy the exact deps from `Cargo.toml`.
  2. BUILDING.md: insert after the `python` bullet (:85):
     ```markdown
     - **`screenshot`** - Terminal-to-image renderer (`screenshot::render_terminal` / `save_terminal`, embedded fonts via `swash`). Enabled by `python`; opt-in for `sim` (`features = ["sim", "screenshot"]`)
     ```
     Insert after the `streaming-bin` bullet (:88):
     ```markdown
     - **`mux`** - The par-mux multiplexer daemon (Rust only; not in the default build or the Python wheel). Enables `pty_session` and `serde`. Build the binary with `cargo build --bin par-mux --no-default-features --features mux`
     - **`serde`** - Serde derives on the replay-snapshot types, which are the par-mux persistence format. Enabled by `mux`
     ```
     With ARC-106, the `mux` bullet's build command uses `--features mux-bin` and says "the binary needs `mux-bin`". Do not add `sim` or `pty_session`, because BUILDING already documents both.
  3. ARCHITECTURE: replace everything from `**Cargo.toml features:**` (:955) through the closing fence after `required-features = ["mux"]` (:1033) with:
     ```markdown
     **Features:** the authoritative list is `[features]` in [`Cargo.toml`](../Cargo.toml), and [RUST_USAGE.md](RUST_USAGE.md#feature-flags) describes each feature. The PyO3 split this section is about:

     - `python` (default) enables `pyo3` with `pyo3/extension-module`, which wheels need: the extension must not link libpython.
     - `python-test` enables the same bindings with `pyo3/auto-initialize` instead, so `cargo test` can link a real interpreter.
     - The `pyo3` dev-dependency also enables `auto-initialize` for Rust tests.
     - Binary targets are gated by `required-features`: `par-term-streamer` needs `streaming-bin`, and `par-mux` needs `mux`.
     ```
     With ARC-106, write `` `par-mux` needs `mux-bin` ``. Keep the `**Build commands:**` block and everything after it.
  4. `CLAUDE.md:143` `mux` row → ``| `mux` | `par-mux` multiplexer daemon: PTYs, session tree, control-mode socket, on-disk persistence. Enables `pty_session`, `interprocess`, `serde`, `dirs`, `toml`, `clap`, and on Windows `widestring`/`windows-sys` |``. With ARC-106, drop `clap` and add a `mux-bin` row. `CLAUDE.md` already has a `screenshot` row.
- **Method**:
  - Every Includes cell is copied from `Cargo.toml`.
  - The ARCHITECTURE verbatim block is replaced by a link, per the style guide ("Avoid duplicating dependency versions"). That also removes a stale `par-term-emu-derive` `version = "0.45.0"` copy.
  - AUDIT asks to add `sim` to BUILDING, but it is already at BUILDING.md:92.
- **Verify**:
  ```bash
  grep -c '^| `screenshot`' docs/RUST_USAGE.md          # 1
  grep -c '\*\*`screenshot`\*\*\|\*\*`mux`\*\*\|\*\*`serde`\*\*' docs/BUILDING.md   # 3
  grep -c '^\[features\]' docs/ARCHITECTURE.md           # 0
  grep -n '^| `mux`' CLAUDE.md docs/RUST_USAGE.md | grep -c clap   # 2 without ARC-106, 0 with it
  ```

### [DOC-109] README What's New duplicates 0.54.0 and 0.50.0 and lacks 0.53.0
- **Card**: `01a0ef5bd6427110a1444f50fb505f83`
- **Blocked by**: DOC-099.
- **Files**:
  - `README.md:16-80`:
    - :22/:24/:26 are the 0.57.0/0.56.0/0.55.0 paragraphs
    - :28 and :30 both cover 0.54.0
    - :32 is 0.52.0, :34 is 0.51.0, :36 is 0.50.0
    - `## What's New in 0.50.0` through `## What's New in 0.44.0` run from :38 to :77
    - :78-79 are the "Full history … through 0.43.0" note
  - Truth: `CHANGELOG.md:96-120` (`[0.53.0]`: Kitty file media gated; `set_allow_file_media` added)
- **Steps**:
  1. Confirm that DOC-099 landed (`sed -n 22p README.md | grep -c 'split-window -b'` returns 1).
  2. Delete lines :28 through :77: the two 0.54.0 paragraphs, 0.52.0, 0.51.0, 0.50.0, and every `## What's New in 0.4x/0.50.0` section. The only inbound link to those anchors is README:36, which is deleted with them.
  3. After the 0.55.0 paragraph, insert:
     ```markdown
     Version 0.53.0 changed a **security default**: Kitty graphics file media is gated. `t=f` file reads are refused unless the embedder calls `set_allow_file_media("all")`, and `t=t` temp files load only when spec-named under an allowed temp root (`"temp_only"`, the default). See [CHANGELOG.md](CHANGELOG.md) for complete release notes.
     ```
  4. Replace the `Full history: …` two-line note (old :78-79) with `Every other release is documented in [CHANGELOG.md](CHANGELOG.md).`
- **Method**: The note at :18-20 already promises "most recent releases". The deleted sections include README:77's broken `python_bindings/types.rs` reference, which parsight flags as high confidence.
- **Verify**:
  ```bash
  grep -c '^Version 0.54.0\|^## What.s New in 0\.' README.md   # 0
  grep -c '^Version 0.53.0' README.md                          # 1
  grep -c 'whats-new-in-0500' README.md                        # 0
  ```
  Then run parsight `find_broken_doc_links` (repository_id `par-term-emu-core-rust`): the `README.md` `python_bindings/types.rs` row is gone.

### [DOC-110] SECURITY.md drift: rate-limit default, "as of 0.52.0" framing, no telemetry/host-probe/respawn threat model, uncapped constants
- **Card**: `01a0ef5bd9697352a7b18827822be030`
- **Blocked by**: SEC-133 and QA-182. Also depends on SEC-125, SEC-126 and SEC-128 for the respawn text. All are Phase 1 or 3c.
- **Files**:
  - `docs/SECURITY.md`:
    - :890 (`Client input is rate-limited (\`--input-rate-limit\`, default 0 = unlimited).`)
    - :906-910 (WebSocket 16 MiB prose, "0.43.1")
    - :917-927 (mux intro "as of 0.52.0")
    - :992-1003 (Control-Connection Resource Bounds)
    - :1027-1057 (`### Hook Reports`), followed by `### Spawn Quoting on Restore`
    - the caps table between `<!-- caps-table:start -->`/`<!-- caps-table:end -->`
  - Constants without `/// cap:`:
    - `src/streaming/server.rs:45-46` (`WS_MAX_MESSAGE_SIZE`, `WS_MAX_FRAME_SIZE`)
    - `src/mux/server.rs:70-76` (`CLIENT_QUEUE_DEPTH`)
    - `src/streaming/session.rs:25-26` (`INPUT_QUEUE_MESSAGES`)
    - `src/mux/host_probe.rs:47-48` (`MAX_GIT_BRANCH_LEN`)
  - Defaults: the CLI is `src/bin/streaming_server/cli.rs:296` (`default_value = "1048576"`). The library is `src/streaming/config.rs:422` (`input_rate_limit_bytes_per_sec: 0`).
  - Annotation format (`scripts/gen_caps_table.py:30-36`): a `/// cap: <text>` doc line, optionally followed by more `///` lines or attributes, then `const NAME`. The existing convention is one sentence ending in a period, placed as the last doc line directly above the `const` (e.g. `src/streaming/server.rs:51`: `/// cap: Bytes accepted in one Input message payload from a streaming client.`).
- **Detect blockers**:
  - SEC-133: `grep -n 'recv_timeout\|shutdown' src/mux/host_probe.rs` shows a bounded, shutdown-aware sweep, and `grep -n 'probe_worker' src/mux/server.rs` shows no unconditional `.join()`.
  - QA-182: `grep -rn '/// cap:' src/mux/command.rs src/mux/tree.rs src/mux/pane.rs` shows a new mux size cap.
  - SEC-126: `parse_respawn_pane` in `src/mux/command.rs` no longer calls `a.has_flag("-k")` over every token.
  - SEC-125: `PtySession` records reap state, and `child_pid()` returns `None` after reap. Read `poll_running`/`try_wait`/`kill` in `src/pty_session.rs`.
  - SEC-128: `begin_respawn` in `src/mux/tree.rs` checks the OSC 7 host and `is_dir()` before using it.
- **Steps**:
  1. `:890` → `- Client input is rate-limited per connection. \`par-term-streamer --input-rate-limit\` defaults to 1048576 bytes/s (1 MiB/s; \`0\` = unlimited). The library \`StreamingConfig.input_rate_limit_bytes_per_sec\` defaults to \`0\` (unlimited), so embedders set it explicitly.`
  2. `:906-910` → `- Inbound WebSocket frames and messages are capped (\`WS_MAX_MESSAGE_SIZE\` / \`WS_MAX_FRAME_SIZE\` on the \`WebSocketConfig\`, values in the [Resource Limits Reference](#resource-limits-reference)). This bounds per-connection memory from one oversized frame independently of the protobuf-level caps above.`
  3. `:919-925`: replace `This section describes the daemon's security posture as of 0.52.0, after the 2026-09-26 security pass landed (socket ownership hardening, the 1 MiB control-line budget, and the 4 KiB hook-value caps — those budgets' current values live in the [Resource Limits Reference](#resource-limits-reference) table); every statement is verified against` with:
     `This section describes the daemon's security posture: socket ownership hardening, a per-line control budget, per-value hook caps, the host probe, and pane respawn. The budgets' current values live in the [Resource Limits Reference](#resource-limits-reference) table. Every statement is verified against`
     Then add `` `src/mux/host_probe.rs`, `` to the file list that follows.
  4. `:992-1003` Control-Connection Resource Bounds: confirm that the "complete or unterminated" 1 MiB bullet matches the post-SEC-127 read loop. At HEAD it does not (SEC-127). If SEC-127 changed the wording or behavior, align the bullet with the code, and replace the literal "1 MiB"/"4096 lines" with the constant names `MAX_CONTROL_LINE_BYTES`/`CLIENT_QUEUE_DEPTH` (both are in the caps table after step 7).
  5. Insert a new subsection after `### Hook Reports`, before `### Spawn Quoting on Restore`:
     ```markdown
     ### Agent Telemetry and Host Probe

     `pane.report_agent_telemetry` lets a pane's hook attach display-only telemetry (model, effort, context and rate-limit percents) to its agent claim. Validation is bounded like the other hook reports: strings are length-capped (`model` 128, `effort` 32) and free of control characters, and percents must be 0-100. A sample dated in the future is rejected with an error. A sample older than 55 minutes, older than the stored sample, at or below the last accepted `seq`, or reported for an agent that is not the pane's current label is dropped silently. Telemetry is never persisted and clears with the claim.

     The host probe is a daemon thread that measures each rostered pane's working directory every 30 s: disk-free percent and git branch and dirty state. That directory can be one a cloned repository or pane output controls, so the probe:

     - probes only the pane child's kernel-reported cwd, never an OSC 7 value, and skips a pane whose cwd cannot be read
     - runs every git command with `core.fsmonitor=false`, `core.hooksPath=/dev/null`, `safe.bareRepository=explicit`, `protocol.ext.allow=never`, `--no-optional-locks`, `GIT_OPTIONAL_LOCKS=0` and `GIT_TERMINAL_PROMPT=0`, with `GIT_DIR`/`GIT_WORK_TREE` cleared, so no repo-configured hook, filter or transport executes
     - checks dirty state with `diff-index --cached` (index against HEAD), which runs no clean filter; unstaged-only edits to tracked files therefore do not read as dirty
     - serves a branch name only when it is under `MAX_GIT_BRANCH_LEN` and free of control characters
     ```
     After SEC-133 lands, append one bullet that states the post-fix bounds exactly as implemented. Read `host_probe.rs` first. The template for AUDIT's remedy: `- bounds each git call by a deadline while draining its output, runs each pane's probe on a worker read with a timeout (a pane whose last probe timed out is skipped), and checks the shutdown flag before each git call, so a hung filesystem cannot stall daemon shutdown`. If SEC-133 has not landed, add no bounds bullet and leave this card open. Do not document bounds that do not exist.
  6. Insert a second subsection directly after it:
     ```markdown
     ### Pane Respawn

     `respawn-pane -t %N [-k] [-c dir] [command]` runs a new process in an existing pane, so any client of the control socket can replace a pane's program. That is the same same-user power `split-window` and `send-keys` already grant: the command runs through `sh -c` like every other spawn, behind the socket's peer-UID check. A running pane refuses to respawn without `-k`.
     ```
     Then append one sentence per landed Phase 1 fix, each matching the code:
     - SEC-126: `Only leading flags are parsed (\`-t\`, \`-c\`, \`-k\`, up to the first non-flag token or \`--\`), and the command is taken verbatim from there, so a \`-k\` or \`-c\` inside the command (\`sh -c '…'\`, \`sort -k 1\`) is never read as a flag.`
     - SEC-125: `After a pane's child is reaped, the pane no longer holds its PID, so a later \`kill-pane\`, \`respawn-pane -k\`, or resize cannot signal a recycled PID.`
     - SEC-128: `The default start directory uses the pane's OSC 7 directory only when it names an existing local directory; otherwise the factory default or \`$HOME\`.`
     Omit the sentence for any fix that has not landed.
  7. Add `/// cap:` annotations as the last doc line directly above each `const`:
     - `src/streaming/server.rs`, above `const WS_MAX_MESSAGE_SIZE`:
       `/// cap: Bytes accepted in one inbound WebSocket message from a streaming client.`
     - `src/streaming/server.rs`, above `const WS_MAX_FRAME_SIZE`:
       `/// cap: Bytes accepted in one inbound WebSocket frame from a streaming client.`
     - `src/mux/server.rs`, above `const CLIENT_QUEUE_DEPTH`:
       `/// cap: Broadcast lines queued per control-socket client before the daemon evicts it.`
     - `src/streaming/session.rs`, above `const INPUT_QUEUE_MESSAGES`:
       `/// cap: Client input chunks queued per session pending write to the PTY.`
     - `src/mux/host_probe.rs`, above `const MAX_GIT_BRANCH_LEN`:
       `/// cap: Bytes of git branch name the host probe serves for one pane cwd.`
     - If QA-182 added a mux size constant without a `/// cap:` line, add one in the same format.
  8. Run `make caps-table` last, after every constant edit from Phase 1 and 3c, then `make caps-table-check`.
- **Method**:
  - AUDIT says the default is 1048576. That is only the CLI default. The library default is 0, so the text states both.
  - All telemetry facts come from `src/mux/hooks.rs:470-545,609-640` and MUX.md:312-316.
  - All host-probe facts come from `host_probe.rs:1-21,96-178`. SEC-115 already landed.
  - Pitfall: `WS_MAX_MESSAGE_SIZE` inherits an orphaned "TLS/SSL configuration" doc block (`server.rs:22-44`; the real `TlsConfig` lives in `config.rs:19`). Only add the `cap:` line and leave that block alone. The generator's regex tolerates the preceding `///` lines.
  - Pitfall: the generator renders byte units only for names matching `BYTES|SIZE|LENGTH|…`. `MAX_GIT_BRANCH_LEN` and the queue depths therefore render as plain counts, which is correct for them.
- **Verify**:
  ```bash
  for c in WS_MAX_MESSAGE_SIZE WS_MAX_FRAME_SIZE CLIENT_QUEUE_DEPTH INPUT_QUEUE_MESSAGES MAX_GIT_BRANCH_LEN; do grep -c "^| \`$c\`" docs/SECURITY.md; done   # 1 each
  make caps-table-check                                  # exit 0
  grep -c 'default 0 = unlimited' docs/SECURITY.md       # 0
  grep -c 'as of 0.52.0' docs/SECURITY.md                # 0
  grep -c '^### Agent Telemetry and Host Probe\|^### Pane Respawn' docs/SECURITY.md   # 2
  make checkall
  ```

### [DOC-111] Kitty `t=f` examples ignore the default file-media gate
- **Card**: `01a0ef5bdca87c02850138af06bc91f3`
- **Files**:
  - `docs/ADVANCED_FEATURES.md:1241-1255` (File Transmission example), `:1278` (Key Parameters `Transmission` row)
  - `docs/VT_SEQUENCES.md:511-512`, `docs/VT_TECHNICAL_REFERENCE.md:1137-1138`
  - Truth: `src/graphics/kitty.rs:108-138` (`FileMediaMode`, default `TempOnly`), `docs/SECURITY.md:573`, `docs/API_REFERENCE.md:705`
- **Steps**:
  1. In the ADVANCED_FEATURES example, insert this line before `term.process_str(f"\x1b_Ga=T,f=100,t=f;{file_path_b64}\x1b\\")`:
     `term.set_allow_file_media("all")  # t=f is refused by default ("temp_only")`
  2. After that example's closing fence, add:
     ```markdown
     > **Note:** `t=f` loads only after the embedder opts in with `set_allow_file_media("all")`. The default `"temp_only"` mode refuses `t=f` and loads `t=t` only for spec-named temp files under an allowed temp root. See [SECURITY.md](SECURITY.md#kitty-graphics-protocol-file-transmission).
     ```
  3. ADVANCED_FEATURES `:1278`: change `` `t=d` (direct), `t=f` (file) `` to `` `t=d` (direct), `t=f` (file; requires `set_allow_file_media("all")`) ``.
  4. VT_SEQUENCES `:511`: `` - `t=f` - Read from file (requires the file-media mode `all`, Python `set_allow_file_media("all")`; refused by default) ``. `:512`: `` - `t=t` - Read from temp file and delete (default `temp_only` mode: spec-named files under an allowed temp root only) ``.
  5. VT_TECHNICAL_REFERENCE `:1137`: ``| File | `t=f` | Load from file path (requires file-media mode `all`; refused by default) |``. `:1138`: ``| Temporary File | `t=t` | Load from temporary file (default mode: spec-named temp files only) |``.
- **Method**:
  - With the opt-in line, the example works as copied.
  - The example's base64 path encoding is correct: `parse_chunk` base64-decodes every payload (`kitty.rs:506-518`), and `load_file_data` then reads those bytes as a UTF-8 path.
  - Related, outside this card: SECURITY.md:582-583 say the path is "NOT base64-encoded", which is true only after decoding. Report it rather than edit it here.
- **Verify**:
  ```bash
  grep -c 'set_allow_file_media("all")' docs/ADVANCED_FEATURES.md   # >=2
  grep -n '`t=f`' docs/VT_SEQUENCES.md docs/VT_TECHNICAL_REFERENCE.md | grep -vc 'all'   # 0
  ```
  Run parsight `find_broken_doc_links` to confirm that the `SECURITY.md#kitty-graphics-protocol-file-transmission` anchor resolves.

### [DOC-112] STREAMING.md does not document input drops or the writer-less close
- **Card**: `01a0ef5be0277ee0bc37880f1e0f0b69`
- **Blocked by**: QA-192.
- **Files**:
  - `docs/STREAMING.md:1346-1370` (`### Sessions Endpoint (/sessions)`; its JSON example lacks the metric fields), `:1874+` (`## Troubleshooting`). There is no metrics section. AUDIT's `~:1753-1790` is `## Security Considerations`.
  - Code at HEAD:
    - `src/streaming/server.rs:1286-1316`: no-writer guard. Input, Paste, Mouse or FocusChange from a non-read-only client increments `dropped_messages` and returns `close: true`.
    - `:1445-1500`, `:1636-1710`: oversize Input/Paste and rate-limit drops are counted.
    - `src/streaming/session.rs:25-35,497-560`: queue-full and byte-budget drops, logged at most once per second.
    - `src/streaming/session.rs:717-744`: `SessionInfo` fields `id, created, clients, idle_seconds, cols, rows, cwd, messages_sent, bytes_sent, input_bytes, errors, dropped_messages`.
    - `src/bin/streaming_server/main.rs:265`: `default_read_only: false`.
    - `?readonly=true`: `src/streaming/server.rs:272`.
- **Detect QA-192**: re-read the guard in `handle_client_message` (`grep -n 'session_has_writer(session)' src/streaming/server.rs`). If it still returns `close: true`, use variant A. If it drops and counts without closing, use variant B, and check `main.rs` for macro mode's `default_read_only`.
- **Steps**:
  1. Replace the `/sessions` JSON example with the full `SessionInfo` shape:
     ```json
     {
       "sessions": [
         {
           "id": "default",
           "created": 1707840000,
           "clients": 1,
           "idle_seconds": 0,
           "cols": 120,
           "rows": 40,
           "cwd": "/home/user",
           "messages_sent": 1532,
           "bytes_sent": 482113,
           "input_bytes": 2048,
           "errors": 0,
           "dropped_messages": 0
         }
       ],
       "max_sessions": 10,
       "available": 9
     }
     ```
  2. Add after that subsection:
     ```markdown
     ### Input Drops

     Client input can be dropped before it reaches the PTY. Every drop increments the session's `dropped_messages` counter, which `/sessions` reports, and is logged. Queue drops are logged at most once per second.

     | Cause | Behavior |
     |-------|----------|
     | Input or Paste payload over its size cap (`MAX_INPUT_PAYLOAD_BYTES`, `MAX_PASTE_PAYLOAD_BYTES`) | That message is dropped |
     | Client over its `--input-rate-limit` budget | That message is dropped |
     | Session input queue full (`INPUT_QUEUE_MESSAGES` chunks) or over its byte budget (`MAX_QUEUED_INPUT_BYTES`), because the child is not reading stdin | The chunk is dropped |
     | Session has no PTY writer | See below |

     Cap values are in the [SECURITY.md Resource Limits Reference](SECURITY.md#resource-limits-reference).
     ```
     Then add the writer-less paragraph:
     - **Variant A** (QA-192 not landed): `A session with no PTY writer (macro playback, or a session whose PTY was detached) drops Input, Paste, Mouse and FocusChange from a non-read-only client, counts it, and closes that WebSocket. Read-only viewers stay connected. The bundled frontend reconnects, so a viewer that sends input to a writer-less session reconnects repeatedly. Connect such viewers with \`?readonly=true\`.`
     - **Variant B** (landed): describe the post-fix guard exactly. For the full AUDIT remedy: `Input on a session with no PTY writer is dropped and counted in \`dropped_messages\`; the connection stays open.` If macro mode now defaults to read-only, add `\`--macro\` sessions are read-only by default.`
  3. In `## Troubleshooting` → `### Connection Issues`, add:
     ```markdown
     **Problem:** Keystrokes have no effect, or the WebSocket closes when you type

     **Solutions:**
     - Check `dropped_messages` for the session: `curl http://localhost:8099/sessions`
     - A writer-less session (macro playback) does not accept input; connect viewers with `?readonly=true`
     - A rising count while typing means the rate limit or the input queue is dropping input; raise `--input-rate-limit`, or check that the child process reads stdin
     ```
     For variant B, drop "or the WebSocket closes when you type" from the Problem line.
- **Method**:
  - Every row maps to a `dropped_messages.fetch_add` site. Constant names instead of numbers keep the prose from drifting; DOC-110 adds `INPUT_QUEUE_MESSAGES` to the caps table.
  - Pitfall: `INPUT_QUEUE_MESSAGES` appears in the caps table only after DOC-110 step 7. Run DOC-110 first, or link to `src/streaming/session.rs`.
- **Verify**:
  ```bash
  grep -c 'dropped_messages' docs/STREAMING.md     # >=3
  grep -c '^### Input Drops' docs/STREAMING.md     # 1
  ```
  Run parsight `find_broken_doc_links`, and check that no new `STREAMING.md` row appears.

### [DOC-113] Rust dependency snippets pinned to `0.50`
- **Card**: `01a0ef5be3817c92b4b1171d7f0ec033`
- **Files**: `README.md:252-259` (table with a `Cargo.toml` column), `docs/RUST_USAGE.md:80-116` (four `toml` blocks: :82, :92, :100/:102, :111/:114), `docs/RUST_USAGE.md:313-320` (Basic Streaming Server block, :315)
- **Steps** (use `cargo add`, which resolves the current version and needs no pin):
  1. README table: rename the column header `Cargo.toml` to `Command`, and set the cells to:
     - Rust Only: `` `cargo add par-term-emu-core-rust --no-default-features --features pty_session` ``
     - Rust + Streaming: `` `cargo add par-term-emu-core-rust --no-default-features --features streaming,pty_session` ``
     - Python Only: `` `cargo add par-term-emu-core-rust` ``
     - Everything: `` `cargo add par-term-emu-core-rust --features full` ``
  2. RUST_USAGE: replace each `toml` block with a `bash` block of the matching command:
     - Rust Only: `cargo add par-term-emu-core-rust --no-default-features --features pty_session`
     - Rust with Streaming: `cargo add par-term-emu-core-rust --no-default-features --features streaming,pty_session`
     - Python Only: `cargo add par-term-emu-core-rust` plus a comment line `# default features include python`
     - Python with Streaming: `cargo add par-term-emu-core-rust --features python,streaming`, and `# or: cargo add par-term-emu-core-rust --features full  (also pulls the par-term-streamer CLI deps)`
  3. RUST_USAGE :313-320: replace the block with
     ```bash
     cargo add par-term-emu-core-rust --no-default-features --features streaming,pty_session
     cargo add tokio --features macros,rt-multi-thread
     # The crate uses parking_lot internally; PtySession::get_writer() and
     # terminal() return parking_lot locks, so reuse it for your own wrappers.
     cargo add parking_lot
     ```
- **Method**:
  - The style guide says to avoid duplicated versions. A placeholder breaks copy-paste, and `cargo add` avoids both problems.
  - Fallback if the maintainer prefers TOML: replace `"0.50"` with `"0.57"` everywhere, and add "check crates.io for the current version". Expect this finding to recur on the next minor release if you do.
- **Verify**:
  ```bash
  grep -n '0\.50' README.md docs/RUST_USAGE.md | grep -v "What's New\|0.50.0"   # no version pins left
  grep -c 'cargo add par-term-emu-core-rust' README.md docs/RUST_USAGE.md          # 4 and >=5
  ```

### [DOC-114] The inherited-env drop list shows 6 of 13 entries
- **Card**: `01a0ef5be62d709093bcb1c51d43fdf8`
- **Files**:
  - `docs/SECURITY.md`: :23, :95 (Mermaid edge label), :124, :226-233 (`**Automatic Environment Filtering**` under `### Inherited Environment`), :261-263, :276, :321
  - `docs/CROSS_PLATFORM.md:82-83`
  - Truth: `src/pty_session.rs:600-660`. `DROP_VARS` = `COLUMNS`, `LINES`, `TMUX`, `TMUX_PANE`, `STY`, `WINDOW`, `CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_MESSAGING_TOKEN`, `OMPCODE`, `CODEX_THREAD_ID`, plus every `PAR_MUX_*` by prefix. `set_env`/`additional_env` are applied after the drop (:681-692).
- **Steps**:
  1. Replace the bullets under `**Automatic Environment Filtering**` (:226-231, keep the `PAR_TERM_REPLY_XTWINOPS` bullet at :232) with:
     ```markdown
     - Every spawn drops these inherited variables (`DROP_VARS` in `src/pty_session.rs`, plus a prefix match):
       - **Size hints** — `COLUMNS`, `LINES`. They are static and do not update on resize. Many libraries (Python's `shutil.get_terminal_size()`, some TUIs) prefer them over `ioctl(TIOCGWINSZ)` and would stay stuck at the parent terminal's size.
       - **Parent multiplexer** — `TMUX`, `TMUX_PANE`, `STY`, `WINDOW`. The child runs in a new PTY, not the parent's tmux or screen pane; tools like fzf would otherwise render in the parent pane.
       - **par-mux pane identity** — every `PAR_MUX_*` variable. A PTY spawned inside a mux pane must not report agents to the outer daemon; mux panes re-add their own values.
       - **Outer agent-session identity** — `CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_MESSAGING_TOKEN`, `OMPCODE`, `CODEX_THREAD_ID`. A spawned shell is not a child agent of whatever started this process.
     - `set_env()` or the `env` argument re-adds any of them for an intentional child session.
     ```
  2. `:23` → ``- The system automatically drops size, multiplexer, par-mux, and agent-session variables from the inherited environment — see [Inherited Environment](#inherited-environment)``
  3. `:95` → `    EnvFilter -->|Filtered env: DROP_VARS and PAR_MUX_* removed| PTY`
  4. `:124` → ``- Automatic filtering of inherited size, multiplexer, par-mux, and agent-session variables ([full list](#inherited-environment))``
  5. `:262-263` (code comment) → `# 1. Inherit all parent env vars except the dropped set (see Inherited Environment:` / `#    COLUMNS/LINES, TMUX/TMUX_PANE/STY/WINDOW, PAR_MUX_*, agent-session vars)`
  6. `:276` → `term.spawn_shell()  # Includes all parent env vars (except the dropped set)!`
  7. `:321` → ``1. All parent environment variables are inherited, except the ones listed under [Inherited Environment](#inherited-environment), which are filtered out automatically``
  8. `docs/CROSS_PLATFORM.md:82-83` → one bullet: ``- Drops inherited size, multiplexer, par-mux, and agent-session variables (`COLUMNS`/`LINES`, `TMUX`/`STY`, `PAR_MUX_*`, …) — full list in [SECURITY.md](SECURITY.md#inherited-environment)``
- **Method**: One authoritative list, with the other mentions linking to it. The grouping and reasons come from the code comment at `pty_session.rs:600-620`.
- **Verify**:
  ```bash
  for v in COLUMNS LINES TMUX TMUX_PANE STY WINDOW CLAUDECODE CLAUDE_CODE_SESSION_ID CLAUDE_CODE_CHILD_SESSION CLAUDE_CODE_MESSAGING_TOKEN OMPCODE CODEX_THREAD_ID 'PAR_MUX_\*'; do awk '/^### Inherited Environment/,/^### Secure Environment/' docs/SECURITY.md | grep -q "$v" || echo "missing $v"; done   # no output
  grep -c 'except COLUMNS/LINES)' docs/SECURITY.md      # 0
  ```
  Then run parsight `find_broken_doc_links` to confirm that the `#inherited-environment` anchors resolve.

### [DOC-115] CONFIG_REFERENCE says the emulator reads no environment variables
- **Card**: `01a0ef5be8c77683bdf10828b21c9537`
- **Files**:
  - `docs/CONFIG_REFERENCE.md:825-834` (`## Environment Variables`)
  - Truth: the non-test `env::var`/`var_os` reads in `src/`:
    - `DEBUG_LEVEL` (`src/debug.rs:37`, values 0-4, anything else is off; also `python/par_term_emu_core_rust/debug.py:69`)
    - `PAR_TERM_REPLY_XTWINOPS` (`src/pty_session.rs:210`, off only for `0`/`false`)
    - `SHELL`/`COMSPEC` (`pty_session.rs:381-388`, fallbacks `/bin/bash`/`cmd.exe`)
    - `PAR_MUX_SOCKET` (`src/bin/par_mux/main.rs:170`, lowest precedence after `--socket` and a name, `src/mux/ipc.rs:536-551`)
    - `PAR_MUX_ENV` and `PAR_MUX_ALLOW_NESTED` (`src/mux/mod.rs:77-78`)
    - `XDG_RUNTIME_DIR` (`src/mux/ipc.rs:517`, `client.rs:62`)
    - `PATH` (`src/mux/client.rs:348`, `win_resume.rs:60`)
    - Indirect: `TMPDIR` via `std::env::temp_dir()` (`ipc.rs:525`, `debug.rs:66`); `HOME` via `dirs::home_dir()` (`dispatch.rs:565`, `persist.rs:457`)
    - Nothing sets `TERM_PROGRAM_VERSION`. Spawned children get `TERM`, `COLORTERM`, `TERM_PROGRAM`, `KITTY_WINDOW_ID` and `KITTY_PID` (`pty_session.rs:676-681`).
- **Steps**: Replace everything between `## Environment Variables` and the next `---` with:
  ```markdown
  The library reads the environment variables below. The streaming server binary reads additional `PAR_TERM_*` variables, listed in [STREAMING.md](STREAMING.md#command-line-options-and-environment-variables).

  | Variable | Read by | Default | Purpose |
  |----------|---------|---------|---------|
  | `DEBUG_LEVEL` | Rust debug logger and the Python `debug` module | `0` (off) | Log verbosity `0`-`4` (off, error, info, debug, trace); logs go to the system temp directory |
  | `PAR_TERM_REPLY_XTWINOPS` | `PtyTerminal`/`PtySession` construction (read once) | replies on | `0` or `false` suppresses XTWINOPS (`CSI t`) query replies |
  | `SHELL` | Default shell lookup (Unix) | `/bin/bash` | Shell for `spawn_shell()` |
  | `COMSPEC` | Default shell lookup (Windows) | `cmd.exe` | Shell for `spawn_shell()` on Windows |
  | `PAR_MUX_SOCKET` | `par-mux` binary | unset | Socket path when neither `--socket` nor a name is given; set inside every mux pane |
  | `PAR_MUX_ENV` | par-mux nested-daemon guard | unset | Marks a process as running inside a mux pane; serve mode and auto-spawn refuse to start a nested daemon |
  | `PAR_MUX_ALLOW_NESTED` | par-mux nested-daemon guard | unset | `1` allows a nested daemon despite `PAR_MUX_ENV` |
  | `XDG_RUNTIME_DIR` | par-mux default socket path (Unix) | unset: a per-UID directory under the temp directory | Directory for the default socket |
  | `TMPDIR` | System temp directory (Unix) | `/tmp` | Base for the per-UID socket fallback and debug logs |
  | `HOME` | par-mux | — | Fallback start directory when a pane's directory is gone |
  | `PATH` | par-mux client and Windows resume | — | Locating the `par-mux` binary; resolving `argv[0]` for Windows agent resume |

  Spawned child processes receive `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=kitty`, `KITTY_WINDOW_ID=1` and `KITTY_PID=<pid>`. Override any of them with `set_env()`. Inherited variables the library drops are listed in [SECURITY.md](SECURITY.md#inherited-environment).
  ```
- **Method**: Each row maps to a verified read site. `TERM_PROGRAM_VERSION` is removed because nothing reads or sets it.
- **Verify**:
  ```bash
  grep -c 'does not read environment variables' docs/CONFIG_REFERENCE.md   # 0
  for v in DEBUG_LEVEL PAR_TERM_REPLY_XTWINOPS PAR_MUX_SOCKET PAR_MUX_ENV PAR_MUX_ALLOW_NESTED XDG_RUNTIME_DIR SHELL HOME TMPDIR PATH COMSPEC; do grep -q "| \`$v\`" docs/CONFIG_REFERENCE.md || echo "missing $v"; done   # no output
  ```
  Then run parsight `find_broken_doc_links` to confirm that both anchors resolve.

### [DOC-116] `_native.pyi` has no docstrings and is `Any` everywhere (merges ARC-099, QA-204 typing half)
- **Card**: `01a0ef5bec0d7733aa744ade7a2e5797`
- **Files**:
  - `scripts/generate_stubs.py`:
    - :1-25 (module doc "Known limitations")
    - :67-78 (`HEADER`)
    - :124-170 (`render_params`)
    - :173-188 (`render_function`)
    - :191-269 (`render_class`)
    - :272-338 (`main`)
  - `python/par_term_emu_core_rust/_native.pyi` (generated; 0 docstrings, 1007 `-> Any`)
  - `CHANGELOG.md` `## [Unreleased]`
- **Measured at HEAD** against the built module (`.venv`):
  - All 622 class methods expose `__doc__`.
  - `__doc__` excludes the text signature (e.g. `Terminal.encode_key.__doc__` starts "Encode a key event…").
  - Getset descriptors carry `__doc__`.
  - A `Returns:` first line of the form `TYPE: description`, where `TYPE` parses as a Python type over builtins and native class names, exists for 82 methods. 524 have no Returns section, and 16 are unparseable or dotted (e.g. `tuple or None`, `asyncio.Queue`).
  - Typed Args (`name (TYPE): desc`) appear only 36 times. The convention is `name: description`, with no type.
- **Steps** (edit `scripts/generate_stubs.py` only; never hand-edit the `.pyi`):
  1. Add these imports and helpers:
     ```python
     import inspect
     import re

     BUILTIN_TYPE_NAMES = {
         "int",
         "float",
         "str",
         "bytes",
         "bool",
         "list",
         "dict",
         "tuple",
         "set",
         "object",
         "None",
     }
     RETURNS_RE = re.compile(r"^Returns:[ \t]*\n[ \t]+([^\n:]+?):", re.MULTILINE)
     ARGS_BLOCK_RE = re.compile(r"^Args:[ \t]*\n((?:[ \t]+.*\n?)+)", re.MULTILINE)
     TYPED_ARG_RE = re.compile(r"^[ \t]+([A-Za-z_]\w*) \(([^)]+)\):", re.MULTILINE)


     def doc_type(expr: str, known: set[str]) -> str | None:
         """Return ``expr`` when it is a type expression over builtins and native classes, else None."""
         try:
             tree = ast.parse(expr.strip(), mode="eval")
         except SyntaxError:
             return None
         for node in ast.walk(tree):
             if isinstance(node, ast.Attribute):
                 return None
             if isinstance(node, ast.Name) and node.id not in BUILTIN_TYPE_NAMES | known:
                 return None
             if isinstance(node, ast.Constant) and node.value is not None:
                 return None
         return expr.strip()


     def returns_type(doc: str | None, known: set[str]) -> str:
         m = RETURNS_RE.search(inspect.cleandoc(doc)) if doc else None
         return (doc_type(m.group(1), known) if m else None) or "Any"


     def arg_types(doc: str | None, known: set[str]) -> dict[str, str]:
         if not doc:
             return {}
         block = ARGS_BLOCK_RE.search(inspect.cleandoc(doc))
         if not block:
             return {}
         out: dict[str, str] = {}
         for name, expr in TYPED_ARG_RE.findall(block.group(1)):
             typed = doc_type(expr, known)
             if typed:
                 out[name] = typed
         return out


     def render_docstring(doc: str | None, indent: str) -> list[str]:
         if not doc or not doc.strip():
             return []
         text = inspect.cleandoc(doc).replace("\\", "\\\\").replace('"""', '\\"\\"\\"')
         lines = text.splitlines()
         if len(lines) == 1:
             return [f'{indent}"""{lines[0]}"""']
         return [
             f'{indent}"""{lines[0]}',
             *[f"{indent}{l}" if l else "" for l in lines[1:]],
             f'{indent}"""',
         ]
     ```
  2. `render_params(text_sig, *, implicit_self, types: dict[str, str] | None = None)`: render `f"{name}: {(types or {}).get(name, 'Any')}"` instead of `f"{name}: Any"`. Leave `*args`/`**kwargs` as `Any`.
  3. `render_function(name, text_sig, *, is_static=False, implicit_self=True, doc=None, known=frozenset())`:
     - Compute `ret = returns_type(doc, known)` and `types = arg_types(doc, known)`.
     - With a docstring, emit `def name(params) -> {ret}:` followed by `render_docstring(doc, indent + "    ")`.
     - Without one, keep the one-line `... -> {ret}: ...`.
  4. `render_class(cls, known)`:
     - Emit the class docstring (`cls.__doc__`) as the first body line.
     - Pass `getattr(obj, "__func__", obj).__doc__` into `render_function` for methods, staticmethods and classmethods.
     - For `getset_descriptor` properties, emit the getter as `def {name}(self) -> Any:` followed by `render_docstring(obj.__doc__, ...)`. Property types stay `Any`, because property docs carry no type.
     - Keep `__init__` and `__exit__` as they are, with no docstring.
  5. `main()`: build `known = set(classes)` before rendering, pass it through, and emit module-function docstrings the same way. After writing, print `typed returns: N` (count the `def` lines whose return is not `-> Any` or `-> None`).
  6. Update `HEADER` and the module docstring's first "Known limitations" bullet to: "Docstrings are copied from `__doc__`. Return types come from a Google-style `Returns:` line whose first token is a type over builtins and native classes, and parameter types from `name (TYPE):` Args entries. Everything else, including every property, is `Any`."
  7. Regenerate:
     ```bash
     make dev-streaming && make stubs
     ```
     Never regenerate from a `make dev` build. `generate_stubs.py:297-303` refuses one, and a `make dev` stub would silently drop the streaming classes.
  8. Add this bullet under `## [Unreleased]` → `### Changed` in `CHANGELOG.md` (create the heading if absent):
     ```markdown
     - **The Python stub carries docstrings and documented types** (DOC-116; `scripts/generate_stubs.py`, `_native.pyi`). Every class, method, function and property in `_native.pyi` now has its binding docstring for IDE hover help. Return and parameter types come from the Google-style `Returns:`/`Args:` sections where those name a concrete type; the rest stay `Any`.
     ```
- **Method**:
  - Deriving types from docstrings keeps the stub honest without adding a build dependency.
  - `pyo3-stub-gen` was rejected. It needs annotations on every binding and a second build path, which is out of proportion for this card.
  - Only syntactically valid types over known names are used, so a prose `Returns:` never becomes a bogus annotation.
  - Pitfall: a docstring whose type is wrong now becomes a typing error in consumers. `make checkall` runs `lint-python` → `uv run pyright .`, and that run covers `tests/`, which exercises many of these methods. Fix wrong docstrings in the Rust binding, never by special-casing the generator.
  - Pitfall: `check_api_reference.py` parses the stub with `ast` and reads only parameters and property names, so docstring bodies do not affect it. Confirm that with `make stub-check`.
  - Coordinate with QA-204. It owns the `stub-drift` Makefile target, and this card owns only the typing half. Regenerate after QA-190, DOC-100, DOC-117 and DOC-120, which change `__doc__`.
- **Verify**:
  ```bash
  make dev-streaming && make stubs            # prints "typed returns: N" (expect ~80)
  grep -c '"""' python/par_term_emu_core_rust/_native.pyi   # > 1000
  grep -A2 'def encode_key' python/par_term_emu_core_rust/_native.pyi   # -> bytes, docstring present
  uv run pyright python/
  make stub-check
  make checkall
  ```

### [DOC-117] Many binding docstrings lack Example sections
- **Card**: `01a0ef5bf0247d118876fca203e5318f`
- **Files** (Example count / pymethod count at HEAD):
  - `src/python_bindings/terminal/`: `search_api.rs` 0/11, `selection_api.rs` 0/6, `text_api.rs` 0/5, `bookmark_api.rs` 0/4, `scrollback_api.rs` 0/2, `image_api.rs` 0/7, `notification_api.rs` 0/14, `metrics_api.rs` 0/20
  - `src/python_bindings/pty.rs` 7/101, `src/python_bindings/streaming.rs` 4/77
  - New: `scripts/check_docstring_examples.py`
  - `Makefile` `stub-check` (shared with QA-204, ARC-115, DOC-128)
- **Steps**:
  1. Add an `Example:` section to every pymethod in `search_api.rs`, `selection_api.rs` and `text_api.rs` first. Use the shape of the existing convention (`src/python_bindings/terminal/input_api.rs:37-44`), placed after `Returns:`:
     ```rust
     /// Example:
     ///     ```python
     ///     from par_term_emu_core_rust import Terminal
     ///     term = Terminal(80, 24)
     ///     term.process_str("hello world\r\n")
     ///     <one call to this method, with a trailing comment showing the result>
     ///     ```
     ```
     Continue with bookmark, scrollback, image, notification and metrics, then `pty.rs` and `streaming.rs`. This card can close after the first three files plus the lint. File the remainder as a follow-up only if the orchestrator agrees.
  2. Run every new example against a `make dev` build, and write the comment from the observed output. Do not invent result values.
  3. Add `scripts/check_docstring_examples.py`:
     ```python
     #!/usr/bin/env python3
     """Warn (never fail) on native methods whose docstring lacks an Example section (DOC-117)."""

     from __future__ import annotations

     import par_term_emu_core_rust._native as native


     def main() -> None:
         missing: dict[str, int] = {}
         total = 0
         for name in sorted(dir(native)):
             cls = getattr(native, name)
             if not isinstance(cls, type):
                 continue
             for attr, obj in vars(cls).items():
                 if attr.startswith("_") or type(obj).__name__ not in (
                     "method_descriptor",
                     "builtin_function_or_method",
                     "staticmethod",
                     "classmethod",
                 ):
                     continue
                 total += 1
                 doc = getattr(getattr(obj, "__func__", obj), "__doc__", None) or ""
                 if "Example" not in doc:
                     missing[name] = missing.get(name, 0) + 1
         print(f"docstring examples: {total - sum(missing.values())}/{total} methods")
         for name, count in sorted(missing.items(), key=lambda kv: -kv[1]):
             print(f"  warn: {name}: {count} methods without an Example")


     if __name__ == "__main__":
         main()
     ```
  4. In the `Makefile` `stub-check` recipe, after `uv run python scripts/check_api_reference.py`, add a tab-indented `uv run python scripts/check_docstring_examples.py`.
- **Method**:
  - CLAUDE.md and CONTRIBUTING require Args, Returns and Example sections.
  - The lint is warn-level per AUDIT, so it never breaks `checkall` while coverage grows.
  - Pitfall: every docstring edit changes `__doc__`, so regenerate the stub (DOC-116) afterwards.
  - Pitfall: the `Makefile` is shared, so re-read it before editing.
- **Verify**:
  ```bash
  for f in search selection text; do printf "%s " $f; grep -c '///\s*Example' src/python_bindings/terminal/${f}_api.rs; done   # 11 6 5
  make dev && uv run python scripts/check_docstring_examples.py   # exit 0, prints counts
  make checkall
  ```

### [DOC-118] MATURIN_BEST_PRACTICES.md shows a stale configuration as current
- **Card**: `01a0ef5bf4067cd18f116e6d483fcf67`
- **Files**:
  - `docs/MATURIN_BEST_PRACTICES.md`:
    - `#### 2. **pyproject.toml Configuration**` (fenced `toml` at :109-125, `**Status**` bullets at :127-133)
    - `#### 3. **Cargo.toml Configuration**` (fenced `toml` at :136-170, bullets at :172-176)
    - :522 (scorecard "Maturin 1.13.3+, PyO3 0.29")
  - Truth, `pyproject.toml:1-4`: `maturin>=1.15.0,<2.0` (the doc says 1.13.3). The copied `[features]` lacks `pty_session`/`screenshot`/`subtle`/`zeroize`.
- **Steps**:
  1. Replace the pyproject fenced block with:
     ```markdown
     See [`pyproject.toml`](../pyproject.toml): `[build-system]`, `[project]` and `[tool.maturin]`.
     ```
     Replace the `Maturin version: \`>=1.13.3,<2.0\` (build), \`>=1.13.3\` (dev)` bullet with `- Maturin version bounds are set in \`pyproject.toml\` (build backend and dev group)`.
  2. Replace the Cargo fenced block with:
     ```markdown
     See [`Cargo.toml`](../Cargo.toml): `[package]`, `[lib]` (`crate-type = ["cdylib", "rlib"]`), the `[[bin]]` targets, `[features]` and `[profile.release]`.
     ```
     Replace `- PyO3 version: 0.29 (latest stable, made optional for flexibility)` with `- PyO3 is optional, behind the \`python\` feature (version in \`Cargo.toml\`)`. Replace `- Minimum Rust version: 1.98` with `- Minimum Rust version: \`rust-version\` in \`Cargo.toml\``.
  3. `:522`: replace `Maturin 1.13.3+, PyO3 0.29, optimal settings` with `maturin build backend, PyO3 behind a feature, release profile tuned (versions in the manifests)`.
  4. Keep the dated `**Last verified**: 2026-07-09 against 0.45.0` stamp (:517). It is an honest point-in-time marker. Keep the Python-version and manylinux compatibility tables, which the style guide allows.
- **Method**: This takes AUDIT's option (a), linking the real files. It avoids moving the file, and the MSRV entries in the CHANGELOG (:284, :330) keep pointing at an existing path.
- **Verify**:
  ```bash
  grep -c 'version = "0.45.0"\|1\.13\.3' docs/MATURIN_BEST_PRACTICES.md   # 0
  grep -c '^```toml' docs/MATURIN_BEST_PRACTICES.md                       # only blocks that are not config copies (verify: expect 0 or the CI yaml/toml samples)
  ```
  Then run parsight `find_broken_doc_links` to confirm that `../pyproject.toml` and `../Cargo.toml` resolve.

### [DOC-119] Mux design decisions are cited to an out-of-repo document
- **Card**: `01a0ef5bf7867dc1bade2df752aff380`
- **Files**:
  - New `docs/MUX_DECISIONS.md`
  - `docs/par-mux.md:1-19`, `docs/MUX.md:7`, `docs/MUX.md:375` (`the design's D3.3`), `docs/ARCHITECTURE.md:237`
  - Citations in code (grep at HEAD):
    - `D1`: `scrape.rs`, `Cargo.toml`
    - `D3`: `dispatch.rs`, `tree.rs`
    - `D3.1`: `Cargo.toml`
    - `D3.2`: `persist.rs`
    - `D3.3`: `dispatch.rs`, `command.rs`, `persist.rs`, `server.rs`, MUX.md
    - `D3.4`: `persist.rs`, `Cargo.toml`, `bin/par_mux/main.rs`
    - `D3.5`: `persist.rs`, `server.rs`, `ids.rs`, `pane.rs`
    - `D4`: `ipc.rs`, `Cargo.toml`
    - `D5`: `Cargo.toml`, `bin/par_mux/main.rs`
    - `D5.4`: `dispatch.rs`
    - `D6.2`, `D6.3`: `agent_resume.rs`, `persist.rs`
    - `T4.B`: `command.rs`
    - `T4.C`: `command.rs`, `dispatch.rs`
    - `T4.E`: `dispatch.rs`
  - Source: `~/Repos/par-agent-os/par-mux.md`:
    - `### D1.` :46, `### D3.` :64, `### D4.` :88, `### D5.` :104
    - D5.4 :2353
    - D3.1-D3.5 :2594-2602
    - T4.B/C/E :2670-2673
    - D6.2 :2867, D6.3 :2881
- **Steps**:
  1. Create `docs/MUX_DECISIONS.md`:
     ```markdown
     # par-mux Design Decisions

     One-line summaries of the design decisions that par-mux code comments cite by number (`D3.3`, `T4.C`, …). The full plan, with rationale and alternatives, lives in `par-mux.md` in the `par-agent-os` repository (see [par-mux.md](par-mux.md)). This file keeps the citations resolvable from this repository.

     | Id | Decision | Cited in |
     |----|----------|----------|
     | D1 | `mux` is a cargo feature of this crate, not a separate crate; it is absent from the `default` and `sim` builds | `src/mux/scrape.rs`, `Cargo.toml` |
     | D3 | Parity tier: implement what par-term's tmux client drives, not all of tmux | `src/mux/dispatch.rs`, `src/mux/tree.rs` |
     | D3.1 | Persistence serializes the replay-snapshot types through feature-gated serde derives (`serde` feature), not a DTO layer | `Cargo.toml` |
     | D3.2 | The state file is a versioned envelope; an unreadable or unknown-version file is quarantined and the daemon starts fresh | `src/mux/persist.rs` |
     | D3.3 | Save after every mutating command and on clean shutdown, atomically (temp file, fsync, rename); losing unsaved content on SIGKILL is accepted. The original synchronous save later moved to an off-lock coalescing worker (ARC-003) | `src/mux/dispatch.rs`, `src/mux/command.rs`, `src/mux/persist.rs`, `src/mux/server.rs`, [MUX.md](MUX.md#shutdown-semantics) |
     | D3.4 | The state file lives in the platform state directory, keyed by socket name, owner-only; the socket itself stays in the runtime/temp directory | `src/mux/persist.rs`, `Cargo.toml`, `src/bin/par_mux/main.rs` |
     | D3.5 | Restore rebuilds layout and content with fresh processes: spawn first, then restore the saved screen; id allocation resumes past saved ids | `src/mux/persist.rs`, `src/mux/server.rs`, `src/mux/ids.rs`, `src/mux/pane.rs` |
     | D4 | Transport is a cross-platform local socket (Unix socket, Windows named pipe); the WebSocket streaming path is untouched | `src/mux/ipc.rs`, `Cargo.toml` |
     | D5 | The server is a daemon that outlives its clients, and par-term auto-spawns it | `Cargo.toml`, `src/bin/par_mux/main.rs` |
     | D5.4 | Reattach resync: the daemon replays a pane's current screen on request (`refresh-client` without `-C`), reusing the terminal's snapshot export | `src/mux/dispatch.rs` |
     | D6.2 | An agent's resume invocation is never persisted; it is built at restore time from a table shipped in the binary | `src/mux/agent_resume.rs` |
     | D6.3 | Restore respawns agent panes through that invocation and fails safe: when none can be built, the pane spawns its original command | `src/mux/agent_resume.rs`, `src/mux/persist.rs` |
     | T4.B | `send-keys` contract: key names, `-l` literal, `-H` hex, and no implicit trailing newline | `src/mux/command.rs` |
     | T4.C | Sizing: pane terminals re-fit to layout geometry after every change; `resize-pane -x/-y`; `refresh-client -C` resizes with latest report wins | `src/mux/command.rs`, `src/mux/dispatch.rs` |
     | T4.E | Queries have fixed reply shapes and no `-F` format strings; push notifications cover what `-F` polling was for | `src/mux/dispatch.rs` |
     ```
     verify: check each row against the par-agent-os lines listed above before committing. D3.3's ARC-003 note rests on commit `6fa7c7e` ("move state persistence off the tree lock onto a coalescing worker (ARC-003)"). Re-grep the citations (`grep -rnoE '\b(D[0-9](\.[0-9])?|T4\.[A-Z])\b' src/ Cargo.toml tests/`) and add a row for any id not listed.
  2. `docs/par-mux.md`: after the first paragraph, add `One-line summaries of every cited decision are in [MUX_DECISIONS.md](MUX_DECISIONS.md).`
  3. `docs/MUX.md:7`: replace `from the design document [\`docs/par-mux.md\`](par-mux.md), which points at the authoritative plan in the \`par-agent-os\` repository.` with `summarized one line each in [MUX_DECISIONS.md](MUX_DECISIONS.md); the full plan lives in the \`par-agent-os\` repository (see [par-mux.md](par-mux.md)).`
  4. `docs/MUX.md:375`: replace `(the design's D3.3)` with `(the design's [D3.3](MUX_DECISIONS.md))`.
  5. `docs/ARCHITECTURE.md:237`: replace `the D-numbered design decisions cited in its code comments live in [par-mux.md](par-mux.md) (the \`par-agent-os\` repository's design document)` with `the D-numbered design decisions cited in its code comments are summarized in [MUX_DECISIONS.md](MUX_DECISIONS.md) (full plan: [par-mux.md](par-mux.md))`.
- **Method**:
  - Summaries are one line each, per AUDIT. The durable doc uses file names, not line numbers (style guide).
  - D1's original text names a stale feature list (`["pty_session", "tokio"]`), so the row states the decision, not the list.
  - D2 is not cited in code, so it is omitted.
  - DOC-129 adds the new file to the README docs list.
- **Verify**:
  ```bash
  test -f docs/MUX_DECISIONS.md
  for id in $(grep -rhoE '\b(D[0-9](\.[0-9])?|T4\.[A-Z])\b' src/mux src/bin/par_mux Cargo.toml | sort -u); do grep -q "^| $id |" docs/MUX_DECISIONS.md || echo "missing $id"; done   # no output (ignore false hits like D0 after inspection)
  ```
  Then run parsight `find_broken_doc_links` and confirm no new rows.

### [DOC-120] Remove the legacy stringly-typed event methods (D6: remove now)
- **Card**: `01a0ef5bfb137020942bfd26135ab3b4`
- **Decision**: D6 resolved by the user on 2026-09-29: remove now, with no deprecation period. This replaces the earlier doc-only entry.
- **Files**:
  - `src/python_bindings/terminal/mod.rs:1119-1132` (`poll_events_legacy` plus its doc block), `:1287-1299` (`poll_subscribed_events_legacy` plus its doc block)
  - `src/python_bindings/observer.rs:379-395` (`event_to_dict_legacy` plus its doc comment), `:540-575` (test `legacy_renderer_reproduces_pre_051_stringly_shape`)
  - `tests/test_observer.py:17-22` (class docstring), `:44-62` (`test_legacy_poll_returns_stringly_shape`, `test_legacy_poll_omits_unset_optional_fields`), and the legacy half of `test_poll_subscribed_events_native_and_legacy` (~`:62-75`)
  - `docs/API_REFERENCE.md:936`, `:941`, `:950`, `:1222`
  - `python/par_term_emu_core_rust/_native.pyi:2001`, `:2005` (regenerated, never hand-edited)
  - `CHANGELOG.md` `## [Unreleased]`
- **Steps**:
  1. Confirm there are no consumers (re-run before deleting):
     `grep -rn "poll_events_legacy\|poll_subscribed_events_legacy" ~/Repos/par-term-emu-tui-rust ~/Repos/par-term ~/Repos/pardeck --include=*.py --include=*.rs --include=*.swift | grep -v "/target/\|/.venv/"` must print nothing. If it prints anything, stop and report.
  2. In `mod.rs`, delete both `fn poll_*_legacy` methods and their `///` doc blocks. Keep the `HashMap` import, which is still used at `:729-756`.
  3. In `observer.rs`, delete `event_to_dict_legacy` and its doc comment.
     - In the test module, replace `legacy_renderer_reproduces_pre_051_stringly_shape` with `native_renderer_keeps_unset_optional_as_none`, which keeps only the final `event_fields(&TerminalEvent::HyperlinkAdded { ..., id: None })` → `EventField::None` assertion.
     - Remove `HashMap` from `use std::collections::{HashMap, HashSet};` if it becomes unused. Let clippy decide.
  4. In `tests/test_observer.py`:
     - Delete `test_legacy_poll_returns_stringly_shape` and `test_legacy_poll_omits_unset_optional_fields`.
     - Rename `test_poll_subscribed_events_native_and_legacy` to `test_poll_subscribed_events_native`, and drop its `legacy` assertions.
     - Remove the "still returns for one release" sentence from the class docstring.
  5. In `docs/API_REFERENCE.md`:
     - Delete the `poll_events_legacy()` (:936) and `poll_subscribed_events_legacy()` (:941) bullets.
     - At :950, change `(`None` when the variable is first set; absent from `poll_events_legacy()` output when unset)` to `(`None` when the variable is first set)`.
     - Remove both names from the :1222 method list.
  6. Add to `CHANGELOG.md` under `## [Unreleased]` (create `### Removed` if absent):
     `- **Legacy stringly-typed event methods removed — breaking (Python API)** (`src/python_bindings/terminal/mod.rs`, `src/python_bindings/observer.rs`; DOC-120). `poll_events_legacy()` and `poll_subscribed_events_legacy()`, the 0.50.0 migration bridge, are gone. Use `poll_events()` / `poll_subscribed_events()`, which return native Python types.`
  7. Regenerate the stubs: `make dev-streaming && make stubs`.
- **Method**:
  - The user chose removal because the project is iterating quickly, and no consumer exists.
  - Keeping the `EventField::None` assertion preserves the one behavior of the old test that still matters: the native renderer keeps unset optional keys as `None`.
  - Pitfall: the `check_api_reference.py` gate (`stub-check`) cross-checks API_REFERENCE against the stub. Remove the docs and regenerate the stub in the same commit, or the gate fails.
  - Pitfall: never regenerate stubs from a plain `make dev` build. It silently drops the streaming classes.
- **Verify**:
  ```bash
  grep -rn "poll_events_legacy\|poll_subscribed_events_legacy\|event_to_dict_legacy" src tests docs python   # no output
  cargo test --lib --no-default-features --features python-test observer
  make dev && uv run pytest tests/test_observer.py -v
  make checkall
  ```

### [DOC-121] The MUX.md fuzz commands exit with an error
- **Card**: `01a0ef5bfe9b72a08c31e51522c02688`
- **Blocked by (soft)**: ARC-121.
- **Files**:
  - `docs/MUX.md:428-433` (the `bash` block with `cd fuzz` and `cargo fuzz run … -max_total_time=60`)
  - Truth: `Makefile:916-931` (`cargo +nightly fuzz run <t> -- -max_total_time=$(FUZZ_SECONDS) -rss_limit_mb=512`, run from the repo root); `.github/workflows/fuzz.yml:31`
- **Detect ARC-121**: `grep -n '^fuzz-mux' Makefile`.
- **Steps**:
  1. Without ARC-121, replace the block with:
     ```bash
     cargo +nightly fuzz run mux_parse_command -- -max_total_time=60 -rss_limit_mb=512
     cargo +nightly fuzz run mux_hook_report -- -max_total_time=60 -rss_limit_mb=512
     ```
  2. With ARC-121, use the target names the grep returns, e.g.
     ```bash
     make fuzz-mux_parse_command
     make fuzz-mux_hook_report
     ```
     and add "(each runs for `FUZZ_SECONDS`, default 60)".
- **Method**:
  - libFuzzer flags must follow `--` (cargo-fuzz 0.13.2 rejects `-m…`).
  - `-rss_limit_mb=512` matches the Makefile and CI.
  - The block runs from the repo root, as the Makefile does, so `cd fuzz` is dropped.
- **Verify**:
  ```bash
  grep -c 'fuzz run mux_[a-z_]* -max_total_time' docs/MUX.md    # 0
  grep -c -- '-rss_limit_mb=512\|make fuzz-mux' docs/MUX.md     # >=2
  ```

### [DOC-122] Smaller MUX.md drifts
- **Card**: `01a0ef5c02137160b3e6a1070698f4cd`
- **Blocks**: DOC-129.
- **Files**:
  - `docs/MUX.md:364` and `:366` (identical "A structurally killed pane is reaped at kill time…" paragraphs), `:261` (the env-contract spawn paths), `:422` (link to `docs/fable/ENH-014-parser-fuzz-targets.md`)
  - Truth: `src/mux/dispatch.rs:870-876` (`respawn-pane` spawns through `factory.create_pane(..., &plan.context())`); `src/mux/tree.rs:1020-1062` (`begin_respawn` builds the context from the pane's current window and session); `break-pane`/`join-pane` move a live process without re-spawning it (MUX.md:205).
- **Steps**:
  1. Delete the paragraph at `:366` and its preceding blank line, keeping `:364`.
  2. Replace the first two sentences of `:261` (through `The ids stay valid; the name is advisory.`) with:
     `These are set for every spawn path: \`new-session\`, \`new-window\`, \`split-window\`, \`respawn-pane\`, and restore. They are **fixed at spawn**, as tmux's \`TMUX\`/\`TMUX_PANE\` are: a later \`rename-session\` leaves \`PAR_MUX_SESSION\` stale, and a \`swap-pane\` across windows or a \`break-pane\`/\`join-pane\` that moves the pane to another window leaves \`PAR_MUX_WINDOW_ID\` stale (and the session variables too when the move crosses sessions). \`respawn-pane\` re-seeds them with the pane's current ids. The ids stay valid; the name is advisory.`
  3. `:422`: replace `(see \`docs/fable/ENH-014-parser-fuzz-targets.md\` for the harness setup)` with `(see [Fuzzing](../CONTRIBUTING.md#fuzzing) for toolchain setup and the crash-to-regression policy)`.
- **Method**:
  - The respawn and re-seed facts follow from `begin_respawn` using the current window and session.
  - `join-pane` can cross sessions ("joining the last pane out of a session's only window closes that session").
  - CONTRIBUTING:77-91 holds the durable fuzz guidance that the plan file duplicated.
- **Verify**:
  ```bash
  grep -c 'A structurally killed pane is reaped at kill time' docs/MUX.md   # 1
  grep -c 'docs/fable' docs/MUX.md                                         # 0
  grep -n 'These are set for every spawn path' docs/MUX.md | grep -c respawn-pane   # 1
  ```
  Then run parsight `find_broken_doc_links` to confirm that `../CONTRIBUTING.md#fuzzing` resolves.

### [DOC-123] Replay pseudo-code uses nonexistent APIs
- **Card**: `01a0ef5c05ac7df291551632f608fd75`
- **Files**:
  - `docs/ADVANCED_FEATURES.md:2424-2442` (the `python` block whose comments show `term.begin_replay_session()` and `replay.current_state()`)
  - Truth:
    - `src/terminal/snapshot_manager.rs:63,75,100,117`: `SnapshotManager::new`, `with_defaults`, `take_snapshot(&mut self, &Terminal) -> usize`, `record_input(&mut self, &[u8])`
    - `src/terminal/replay.rs:61,94,156`: `ReplaySession::new(&SnapshotManager) -> Option<Self>`, `current_frame(&self) -> &Terminal`, `seek_to_timestamp(&mut self, u64) -> SeekResult`
    - Timestamps are Unix milliseconds (`snapshot_manager.rs:224`)
    - Modules: `terminal::replay`, `terminal::snapshot_manager` (`src/terminal/mod.rs:27,34`)
    - `docs/INSTANT_REPLAY.md` documents both
- **Steps**:
  1. In the Python block, delete the comment lines from `# 2. In Rust (feature available via FFI/Rust API):` through `# let restored_term = replay.current_state();`, and close the block after the `print(...)` line.
  2. After it, add:
     ````markdown
     Timeline navigation is a Rust API (`SnapshotManager` and `ReplaySession`); see [INSTANT_REPLAY.md](INSTANT_REPLAY.md) for the full reference:

     ```rust
     use par_term_emu_core_rust::terminal::replay::ReplaySession;
     use par_term_emu_core_rust::terminal::snapshot_manager::SnapshotManager;

     let mut manager = SnapshotManager::with_defaults();
     manager.take_snapshot(&term);
     // after each term.process(bytes):
     manager.record_input(bytes);

     if let Some(mut replay) = ReplaySession::new(&manager) {
         replay.seek_to_timestamp(target_unix_ms);
         let restored = replay.current_frame(); // &Terminal at that point in time
     }
     ```
     ````
- **Method**: Every call matches a `pub fn` read at HEAD. The snippet is a fragment (`term`, `bytes` and `target_unix_ms` come from the reader's code), as are the existing ones in INSTANT_REPLAY.md.
- **Verify**:
  ```bash
  grep -c 'begin_replay_session\|current_state()' docs/ADVANCED_FEATURES.md   # 0
  grep -n 'pub fn with_defaults\|pub fn take_snapshot\|pub fn record_input' src/terminal/snapshot_manager.rs   # 3 hits
  grep -n 'pub fn new\|pub fn current_frame\|pub fn seek_to_timestamp' src/terminal/replay.rs               # 3 hits
  ```

### [DOC-124] Broken intra-doc anchors
- **Card**: `01a0ef5c08917083ad4c52cf529de64f`
- **Files**:
  - `docs/API_REFERENCE.md:872`: link `CHANGELOG.md#0500---2026-09-21`. There are two defects. The path is relative to `docs/`, so it points at the nonexistent `docs/CHANGELOG.md`. The heading is also dated 2026-09-23 (`CHANGELOG.md:175`). parsight reports `anchor-not-found`.
  - `docs/SECURITY.md:40-41` (TOC) against headings `:141` `### ✅ DO: Use Command + Args Array Format` and `:164` `### ❌ DON'T: Concatenate User Input into Commands`
- **Steps**:
  1. `API_REFERENCE.md:872`: replace `(CHANGELOG.md#0500---2026-09-21)` with `(../CHANGELOG.md#0500---2026-09-23)`.
  2. `SECURITY.md:141`: `### DO: Use Command + Args Array Format`. `SECURITY.md:164`: `### DON'T: Concatenate User Input into Commands`. Keep the TOC links as they are.
- **Method**:
  - Removing the emoji makes GitHub's slugs (`do-use-command--args-array-format`, `dont-concatenate-user-input-into-commands`) match the existing TOC. With emoji, GitHub prefixes a hyphen.
  - The style guide also says not to rely on emoji and color for meaning.
- **Verify**:
  ```bash
  grep -c '0500---2026-09-21' docs/API_REFERENCE.md   # 0
  grep -c '^### [✅❌]' docs/SECURITY.md              # 0
  ```
  Then run parsight `find_broken_doc_links` (repository_id `par-term-emu-core-rust`): the `docs/API_REFERENCE.md:872` row is gone.

### [DOC-125] Code fences have no language tag
- **Card**: `01a0ef5c0b977ab1b0a3f9a3e1f8fd9f`
- **Files** (untagged opening fences at HEAD; all hold escape-sequence syntax, key lists, trees, ASCII diagrams or transcripts, so all get `text`):
  - `docs/ADVANCED_FEATURES.md` :424, :431, :595, :724, :2626, :2632
  - `docs/CONFIG_REFERENCE.md` :379
  - `docs/GRAPHICS_TESTING.md` :382
  - `docs/MACROS.md` :1007, :1013, :1018, :1029, :1047, :1141 (6, not 5)
  - `docs/MATURIN_BEST_PRACTICES.md` :22
  - `docs/STREAMING.md` :164, :617, :693
  - `docs/TESTING_KITTY_ANIMATIONS.md` :74, :98, :328
  - `docs/VT_TECHNICAL_REFERENCE.md` :248, :254, :260, :267, :392, :589, :616, :642, :772, :804, :841, :960, :1002, :1213, :1243
  - `CLAUDE.md` :153 (the Data Flow block; AUDIT says :156)
- **Steps**: Run last among 3d entries that touch these files. The script re-detects the fences at execution time, so line drift from earlier entries does not matter:
  ```bash
  python3 - <<'EOF'
  import pathlib, re
  files = ["docs/ADVANCED_FEATURES.md", "docs/CONFIG_REFERENCE.md", "docs/GRAPHICS_TESTING.md", "docs/MACROS.md",
           "docs/MATURIN_BEST_PRACTICES.md", "docs/STREAMING.md", "docs/TESTING_KITTY_ANIMATIONS.md",
           "docs/VT_TECHNICAL_REFERENCE.md", "CLAUDE.md"]
  for f in files:
      p = pathlib.Path(f); lines = p.read_text().splitlines(keepends=True); open_fence = None
      for i, l in enumerate(lines):
          m = re.match(r'^(\s*)(`{3,}|~{3,})(.*?)(\r?\n)?$', l)
          if not m: continue
          fence, info = m.group(2), m.group(3).strip()
          if open_fence is None:
              open_fence = fence
              if not info:
                  lines[i] = f"{m.group(1)}{fence}text{m.group(4) or ''}"
                  print(f"{f}:{i+1}")
          elif fence[0] == open_fence[0] and len(fence) >= len(open_fence) and not info:
              open_fence = None
      p.write_text("".join(lines))
  EOF
  ```
  Review each printed location. If a block is really a shell command or code, change `text` to `bash`/`python`/`rust`. None of the HEAD blocks listed above are.
- **Method**: This applies the style guide rule "Specify a language for every fenced code block", using `text` for diagrams and transcripts. The state machine only tags opening fences, and it handles four-backtick fences.
- **Verify**: re-run the same script with the write disabled (replace `p.write_text(...)` with `pass`). It must print nothing. Then `git diff --stat` must show only the nine files.

### [DOC-126] CHANGELOG compare links stop at 0.37.0
- **Card**: `01a0ef5c0e7a78508c982dcdccd6c6c4`
- **Files**: `CHANGELOG.md:1801` (the first link reference, `[0.37.0]: …`). Headings missing a reference: `Unreleased` and 0.38.0 through 0.57.0 (35 in all). Every matching `v*` tag exists on `origin`.
- **Steps**: Insert these lines immediately above `[0.37.0]: …`:
  ```markdown
  [Unreleased]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.57.0...HEAD
  [0.57.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.56.0...v0.57.0
  [0.56.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.55.0...v0.56.0
  [0.55.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.54.0...v0.55.0
  [0.54.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.53.0...v0.54.0
  [0.53.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.52.0...v0.53.0
  [0.52.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.51.0...v0.52.0
  [0.51.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.50.0...v0.51.0
  [0.50.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.49.0...v0.50.0
  [0.49.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.48.0...v0.49.0
  [0.48.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.47.0...v0.48.0
  [0.47.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.46.0...v0.47.0
  [0.46.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.45.0...v0.46.0
  [0.45.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.44.0...v0.45.0
  [0.44.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.43.1...v0.44.0
  [0.43.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.43.0...v0.43.1
  [0.43.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.42.4...v0.43.0
  [0.42.4]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.42.3...v0.42.4
  [0.42.3]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.42.2...v0.42.3
  [0.42.2]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.42.1...v0.42.2
  [0.42.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.42.0...v0.42.1
  [0.42.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.41.1...v0.42.0
  [0.41.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.41.0...v0.41.1
  [0.41.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.40.0...v0.41.0
  [0.40.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.39.8...v0.40.0
  [0.39.8]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.39.7...v0.39.8
  [0.39.7]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.39.6...v0.39.7
  [0.39.6]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.39.5...v0.39.6
  [0.39.5]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.39.4...v0.39.5
  [0.39.4]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.39.3...v0.39.4
  [0.39.3]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.39.2...v0.39.3
  [0.39.2]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.39.1...v0.39.2
  [0.39.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.39.0...v0.39.1
  [0.39.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.38.0...v0.39.0
  [0.38.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.37.0...v0.38.0
  ```
- **Method**: This is the Keep a Changelog convention, matching the existing reference format. When a release is cut, `[Unreleased]` moves to `compare/vX.Y.Z...HEAD` and one line is added. That is release-step work (enhancement E4), not this card.
- **Verify**:
  ```bash
  python3 - <<'EOF'
  import re
  t = open('CHANGELOG.md').read()
  heads = re.findall(r'^## \[([^\]]+)\]', t, re.M); refs = set(re.findall(r'^\[([^\]]+)\]:', t, re.M))
  missing = [h for h in heads if h not in refs]; print(missing); assert not missing
  EOF
  ```

### [DOC-127] Rustdoc gaps
- **Card**: `01a0ef5c11f67c30b651216100d1dae3`
- **Files**:
  - `src/screenshot/mod.rs:1` (no `//!`)
  - `src/grid/scroll.rs:272` (`pub fn resize`)
  - `src/keyboard.rs:43-48` (`modifiers::{SHIFT, ALT, CTRL, SUPER, HYPER, META}`)
  - `src/keyboard.rs:130-141` (`term_key_from_raw!`; the generated `from_raw` has no doc)
  - `src/python_bindings/observer.rs:437,477` (`new` on the `pub(crate)` observers)
  - `src/lib.rs` (lint)
  - **Not a gap**: `src/ffi.rs:242` `term_event_cb` is documented by the `///` block at `:233-238`, and cbindgen emits it into `include/terminal_core.h:284-290`. Leave it.
- **Steps**:
  1. Prepend to `src/screenshot/mod.rs`:
     ```rust
     //! Terminal-to-image rendering (`screenshot` feature).
     //!
     //! [`render_terminal`] and [`save_terminal`] render a [`Terminal`](crate::terminal::Terminal)'s
     //! grid, cursor and Sixel graphics to PNG, JPEG, SVG or BMP with embedded fonts.
     ```
  2. Above `pub fn resize` in `src/grid/scroll.rs`:
     ```rust
     /// Resize the visible grid to `cols` × `rows`. A width change reflows the
     /// screen and its scrollback; a height-only change resizes in place. Every
     /// row is marked damaged. A no-op when the size is unchanged or either
     /// dimension is zero.
     ```
  3. `src/keyboard.rs` modifier consts: add `/// Shift held.`, `/// Alt (Option) held.`, `/// Control held.`, `/// Super (Command) held.`, `/// Hyper held.` and `/// Meta held.` above the respective `pub const`.
  4. Inside `term_key_from_raw!`, directly above `pub fn from_raw(v: u16) -> TermKey {`, add `/// Map a raw \`TermKeyEvent.key\` value to a variant; any value that is not a defined discriminant becomes [\`TermKey::Unknown\`].`
  5. `observer.rs:437`: `/// Wrap \`callback\`; \`subscriptions\` limits delivery to those event kinds (\`None\` = all).` `observer.rs:477`: `/// Wrap an asyncio queue; \`subscriptions\` limits delivery to those event kinds (\`None\` = all).`
  6. Add `#![warn(missing_docs)]` at the top of `src/lib.rs`, before the first `pub mod`.
  7. Run `cargo clippy --all-targets --features python,streaming,mux,serde,streaming-bin -- -D warnings` (the `Makefile:298` flags). Every `missing_docs` hit is now an error. Document the real items. The generated protobuf module is the expected bulk: `src/streaming/proto.rs:31-32` `pub mod pb;` over `terminal.pb.rs` has 247 pub items and 112 doc lines. Add `#[allow(missing_docs)]` directly above `#[path = "terminal.pb.rs"]`, because generated code must not be hand-edited. If more than about 30 other items fail, stop and report the list instead of mass-allowing.
- **Method**:
  - The `ffi.rs:242` item was a false positive in AUDIT.
  - `missing_docs` only fires on reachable public items, so the `pub(crate)` observer docs are hygiene, not lint requirements.
  - Pitfall: `-D warnings` in `lint`/`clippy`/CI turns every warning into a hard failure.
  - Pitfall: `src/ffi.rs`, `src/keyboard.rs` and `src/lib.rs` are shared with ARC-093/ARC-098/ARC-101/ARC-112/QA-201. Re-read before editing.
- **Verify**:
  ```bash
  cargo clippy --all-targets --features python,streaming,mux,serde,streaming-bin -- -D warnings
  cargo test --lib --no-default-features --features rust-only,serde   # doc attrs compile in the sim-like build
  make checkall
  ```

### [DOC-128] `make help` omits real targets
- **Card**: `01a0ef5c15107b70a74f82a34eabfbbf`
- **Files**: `Makefile:12-104` (`help` recipe). At HEAD, `stubs` (:49) and `bench` (:55) **are already listed**. The targets actually missing are `xcframework` (:180), `caps-table`/`caps-table-check` (:325/:328), `fuzz-all` and the five `fuzz-*` (:916-931), `coverage`/`coverage-html`/`coverage-python`, `examples-basic` and `web-open`.
- **Detect new targets**: `grep -nE '^(stub-drift|lint-check|fuzz-mux[a-z_]*):' Makefile` (QA-204, ARC-115, ARC-121).
- **Steps** (recipe lines start with a tab):
  1. Under `Building:`, after `dev-streaming`: `@echo "  xcframework     - Build TerminalCore.xcframework (iOS device + simulator) from the C FFI (needs Xcode)"`.
  2. Under `Testing:`, after `test-web`:
     ```make
     	@echo "  coverage        - Rust coverage via cargo-llvm-cov (lib + integration tests, streaming feature)"
     	@echo "  coverage-html   - Generate and open an HTML Rust coverage report"
     	@echo "  coverage-python - Python coverage via pytest-cov"
     ```
  3. Under `Code Quality:`, after `ffi-surface-check`:
     ```make
     	@echo "  caps-table      - Regenerate the resource-caps table in docs/SECURITY.md from /// cap: annotations"
     	@echo "  caps-table-check - Fail when the docs/SECURITY.md caps table differs from the code"
     ```
     Add `stub-drift` / `lint-check` lines here if they exist, with their own recipe's description.
  4. New section before `Pre-commit Hooks:`:
     ```make
     	@echo "Fuzzing (nightly + cargo-fuzz; not part of checkall):"
     	@echo "  fuzz-all              - Run every fuzz target for FUZZ_SECONDS each (default 60)"
     	@echo "  fuzz-terminal_process - Fuzz the whole VTE pipeline"
     	@echo "  fuzz-sixel            - Fuzz the Sixel state machine"
     	@echo "  fuzz-kitty            - Fuzz the Kitty graphics APC parser"
     	@echo "  fuzz-apc_filter       - Fuzz the APC pre-filter"
     	@echo "  fuzz-tmux_control     - Fuzz the tmux control-mode parser"
     	@echo ""
     ```
     Add `fuzz-mux_*` lines if ARC-121 landed. Change the `fuzz-all` text to match its real target count.
  5. Under `Examples:`, after `examples`: `@echo "  examples-basic     - Run the basic terminal examples"`. Under `Web Frontend`, after `web-dev`: `@echo "  web-open        - Open http://localhost:3000 in a browser"`.
- **Method**: The descriptions come from each target's own echo or comment. The `grind-start-<backend>` variants are already covered by the "(backends: …)" help line.
- **Verify**:
  ```bash
  make -s help > /tmp/help.txt 2>/dev/null   # use the scratchpad in agent runs
  for t in $(grep -oE '^[a-z][a-z0-9_-]+:' Makefile | tr -d ':' | sort -u); do grep -qE "^\s+(make )?$t( |$)" /tmp/help.txt || echo "not in help: $t"; done
  # expected remaining: help, grind-start-anthropic/codex/grok/omp/zai (covered by the backends line)
  ```

### [DOC-129] Orphan and stale docs, and incomplete README indexes
- **Card**: `01a0ef5c184b7d3089c1d2d6cb7648b8`
- **Blocked by**: DOC-122.
- **Files**:
  - Delete: `docs/fable/ENH-001-diff-snapshots-api.md`, `ENH-002-osc-1337-currentdir.md`, `ENH-003-xtpushcolors-color-stack.md`, `ENH-004-legacy-alt-screen-modes.md`, `ENH-005-decsace-rectangular-extent.md`, `ENH-006-x10-mouse-mode.md`, `ENH-007-criterion-benchmarks.md`, `ENH-008-mux-persist-off-lock.md`, `ENH-009-mux-dispatch-decomposition.md`, `ENH-010-ascii-fast-lane.md`, `ENH-011-pty-wait-for-api.md`, `ENH-012-mux-client-backpressure.md`, `ENH-013-crate-package-hygiene.md`, `ENH-014-parser-fuzz-targets.md`, `ENH-015-dec-mode-table.md`. Also delete `docs/opus/ENH-025-grid-owned-damage.md`, `ENH-026-zero-alloc-dirty-ranges.md`, `ENH-027-cbindgen-ffi-drift-gate.md`, `ENH-028-shared-key-encoder.md`, `ENH-029-typed-telemetry-schema.md`, `ENH-030-api-checker-constructors.md`, `ENH-031-send-keys-hex-encoder.md`, `ENH-032-pin-actions-to-sha.md`.
  - **Keep** `docs/fable/BENCH-BASELINE-2026-08.md` and `docs/fable/BENCH-BASELINE-2026-09.md`. They are referenced by `CONTRIBUTING.md:67`, `docs/BENCHMARKING.md:28`, `scripts/bench_compare.py:11` and CHANGELOG.
  - Other references to the ENH-014 plan: `Makefile:906-907` and `fuzz/Cargo.toml:24`, besides MUX.md:422 (DOC-122).
  - `docs/research/OSC-9-4-PROGRESS-BAR-IMPLEMENTATION.md` (inbound links only from AUDIT files); `docs/VT_SEQUENCES.md:399` (`### Progress Bar (OSC 9;4)`)
  - `README.md:194-214` (docs list), `README.md:530-576` (examples list)
- **Steps**:
  1. Preconditions:
     - `grep -c docs/fable docs/MUX.md` returns 0 (DOC-122 landed).
     - The orchestrator confirms that every ENH-001…015 and ENH-025…032 card is `done` on the board. Sub-agents must not run `kanban`. verify: ENH-013 and ENH-015 have no `ENH-0NN` commit subject. Their work landed as `a615c48` (ARC-012/014/017) and `dbe7aee` (DEC mode table), so confirm those cards specifically.
  2. **Forbidden**: `rm -r docs/fable`, `git rm -r docs/opus`, any `docs/opus/*` glob, and any `docs/opus/ENH-0NN` with NN ≥ 033. The ENH-033+ plans are written this cycle, and `/enhancement-all` reads them. Delete by the exact 23 file names above only.
  3. Repoint the two remaining ENH-014 references:
     - `Makefile:906-907`: `# cargo-fuzz targets over the untrusted-byte parsers; see` / `# docs/fable/ENH-014-parser-fuzz-targets.md and CONTRIBUTING.md "Fuzzing".` → one line: `# cargo-fuzz targets over the untrusted-byte parsers; see CONTRIBUTING.md "Fuzzing".`
     - `fuzz/Cargo.toml:24`: `# rust-only: no Python toolchain in the fuzzer — see docs/fable/ENH-014-parser-fuzz-targets.md.` → `# rust-only: no Python toolchain in the fuzzer (see CONTRIBUTING.md "Fuzzing").`
  4. `git rm` the 23 files by exact path.
  5. Research doc: link it rather than remove it, because it is the parser implementation reference. Under `docs/VT_SEQUENCES.md` `### Progress Bar (OSC 9;4)`, after the sequence list, add `Implementation notes: [OSC 9;4 implementation guide](research/OSC-9-4-PROGRESS-BAR-IMPLEMENTATION.md) (dated research, 2026-02-09).`
  6. README docs list: append after the `Graphics Testing` bullet:
     ```markdown
     - **[Benchmarking](docs/BENCHMARKING.md)** - Criterion throughput benches and interleaved A/B comparison
     - **[Testing Kitty Animations](docs/TESTING_KITTY_ANIMATIONS.md)** - Testing Kitty graphics animation support
     - **[Regional Flag Limitation](docs/REGIONAL_FLAG_LIMITATION.md)** - Why regional-indicator flag emoji may not render as one glyph in some frontends
     - **[Maturin Best Practices](docs/MATURIN_BEST_PRACTICES.md)** - Wheel build and packaging compliance review
     - **[par-mux Design Pointer](docs/par-mux.md)** - Where the par-mux design plan lives
     - **[Documentation Style Guide](docs/DOCUMENTATION_STYLE_GUIDE.md)** - Standards for writing project documentation
     ```
     If DOC-119 landed, also add `- **[par-mux Design Decisions](docs/MUX_DECISIONS.md)** - One-line summaries of the D-numbered decisions cited in code`.
  7. README examples: add a section after `### Macros and Automation`:
     ```markdown
     ### Diagnostics and Test Scripts
     - `bce_scroll_test.py` - Background color erase (BCE) during scrolling
     - `char_test.py` - Character-specific rendering test
     - `gradient_test.py` - Minimal gradient test for terminal wrap behavior
     - `rich_mimic_test.py` - Rich-style output rendering test
     - `scroll_timing_test.py` - Scroll timing test
     - `streaming_debug.py` - Streaming server debug tool
     - `test_tui_clipboard.py` - TUI clipboard and selection features
     - `test_underline_styles.py` - Underline styles (SGR 4:x)
     - `render_utils.py` - Rendering helpers imported by the other examples (not run directly)
     ```
- **Method**:
  - This follows the `f14c76d` precedent ("remove docs/opus — all 9 plan files (ENH-016..024) shipped to done cards").
  - AUDIT's wording `docs/fable/*` (AUDIT.md:842) would also delete the referenced BENCH-BASELINE files, so delete by exact name.
  - The example descriptions come from each script's module docstring.
- **Verify**:
  ```bash
  ls docs/fable            # only BENCH-BASELINE-2026-08.md, BENCH-BASELINE-2026-09.md
  ls docs/opus             # no ENH-025..032; every ENH-033+ file still present
  grep -rn 'docs/fable/ENH\|docs/opus/ENH-0[23]' --include=*.md --include=Makefile --include=*.toml --include=*.py --include=*.rs . | grep -v '^./AUDIT'   # no output
  for f in $(ls examples/*.py | xargs -n1 basename); do grep -q "$f" README.md || echo "README missing $f"; done   # no output
  for d in BENCHMARKING MATURIN_BEST_PRACTICES REGIONAL_FLAG_LIMITATION TESTING_KITTY_ANIMATIONS par-mux; do grep -q "docs/$d.md" README.md || echo "missing $d"; done   # no output
  ```
  Then run parsight `find_broken_doc_links` (repository_id `par-term-emu-core-rust`): no new rows, and the ENH-028/029 stale-path rows are gone.

### [DOC-130] `CLAUDE.md:166` says "16 themed `*_api.rs` files", but there are 17
- **Card**: `01a0ef5c1b8a7443a6e9763a3e90e5aa`
- **Files**: `CLAUDE.md:166`. `ls src/python_bindings/terminal/*_api.rs` lists 17 files, including `input_api.rs`.
- **Steps**: Replace `(\`terminal/\` directory with \`mod.rs\` + 16 themed \`*_api.rs\` files,` with `(\`terminal/\` directory with \`mod.rs\` + themed \`*_api.rs\` files,`.
- **Method**: Drop the count rather than change it to 17. A count drifts with every new themed file, as CONTRIBUTING:108 already shows by omitting it.
- **Verify**:
  ```bash
  grep -c 'themed `\*_api.rs` files' CLAUDE.md          # 1
  grep -cE '[0-9]+ themed' CLAUDE.md                     # 0
  ```

### [DOC-131] Invalid Mermaid color
- **Card**: `01a0ef5c1f387ee38007d50791596091`
- **Files**: `docs/FFI_GUIDE.md:335` (`classDef free fill:#b71c1c,stroke:#f4436,stroke-width:2px,color:#ffffff`)
- **Steps**: Replace `stroke:#f4436` with `stroke:#f44336`.
- **Method**: `#f44336` is the style guide's Error/Failed stroke and pairs with the `#b71c1c` fill already there.
- **Verify**:
  ```bash
  grep -c '#f4436[^0-9a-f]' docs/FFI_GUIDE.md   # 0
  grep -c 'stroke:#f44336' docs/FFI_GUIDE.md    # 1
  ```

### [DOC-132] Placeholder paths flagged by parsight need waivers
- **Card**: `01a0ef5c234c7aa0971c9a99e4bc88af`
- **Files**:
  - `docs/DOCUMENTATION_STYLE_GUIDE.md:197` (table row `| File paths | Backticks | \`src/config.ts\` |`), `:210` (`src/client.ts`), `:212` (`.github/workflows/deploy.yml`)
  - `docs/MUX.md:340` (`…/claude-cli/cli.js`, inside the Claim liveness sweep paragraph)
  - All four are parsight `high` rows at HEAD.
- **Steps**:
  1. `:197`: put the waiver inside the last cell: `| File paths | Backticks | \`src/config.ts\` <!-- doc-path: example --> |`.
  2. `:210` and `:212`: append ` <!-- doc-path: example -->` to the end of each bullet line.
  3. `MUX.md:340`: append ` <!-- doc-path: example -->` to the end of the paragraph line. Re-locate it with `grep -n 'claude-cli/cli.js' docs/MUX.md` first, because DOC-122 shifts MUX.md lines.
- **Method**:
  - parsight drops any row whose citing line carries the marker and counts it in `_meta.suppressed.waived`.
  - An HTML comment inside the cell keeps the table's column count and does not render.
  - Leave the CHANGELOG, AUDIT* and README-0.44.0 hits alone, because they are historical. DOC-109 deletes the README one.
- **Verify**: run parsight `find_broken_doc_links` (repository_id `par-term-emu-core-rust`). The four rows are gone, and `_meta.suppressed.waived` is 4 or more.

### [DOC-133] ARCHITECTURE pins exact dependency versions
- **Card**: `01a0ef5c29327461b99da2812de7729d`
- **Files**:
  - `docs/ARCHITECTURE.md:875-935` (`## Dependencies`): Rust bullets :879-918, e.g. `` `vte` (0.15.0) ``, `` `tokio` (1.52.3) ``, `` `pyo3` (0.29, features: auto-initialize) ``
  - Python bullets :925-941 carry `(>=x.y.z)` bounds that have already drifted: maturin `>=1.13.3` (the real value is 1.15.0), pytest `>=9.0.3`, ruff `>=0.15.16`, pyright `>=1.1.410`.
  - The `**Python version requirements:** 3.12, 3.13, 3.14` line is a compatibility statement. Keep it.
- **Steps**: Run after DOC-107 and DOC-108, which also edit ARCHITECTURE.md:
  ```bash
  python3 - <<'EOF'
  import re, pathlib
  p = pathlib.Path('docs/ARCHITECTURE.md'); t = p.read_text()
  start = t.index('\n## Dependencies'); end = t.index('\n## ', start + 5)
  sec = t[start:end]
  sec = re.sub(r' \((?:>=)?\d+(?:\.\d+)*(?:,<\d+(?:\.\d+)*)?\)', '', sec)   # " (1.52.3)", " (>=12.2.0)", " (>=1.13.3,<2.0)"
  sec = re.sub(r'\((?:\d+(?:\.\d+)*), ', '(', sec)                          # "(0.29, features: …)" -> "(features: …)"
  p.write_text(t[:start] + sec + t[end:])
  EOF
  ```
  Keep the two existing `> **📝 Note:** See \`Cargo.toml\`/\`pyproject.toml\` for current version requirements` lines. They are now the only version pointers.
- **Method**: This applies the style guide's "Avoid duplicating dependency or package versions". The regexes touch only the `## Dependencies` section, and only parenthesized groups that start with a version number, so purpose text such as "(binary-only, via `streaming-bin`)" and "(path `derive/`)" survives.
- **Verify**:
  ```bash
  sed -n '/^## Dependencies/,/^## Build Process/p' docs/ARCHITECTURE.md | grep -nE '\((>=)?[0-9]+\.[0-9]'   # no output
  sed -n '/^## Dependencies/,/^## Build Process/p' docs/ARCHITECTURE.md | grep -c 'binary-only'          # unchanged (>=1)
  ```
  Then run parsight `find_broken_doc_links` and confirm no new ARCHITECTURE rows.

---

**Phase 3d exit gate:** `make checkall` (covers `stub-check`/`check_api_reference.py`, `ffi-surface-check`, `ffi-header-check`, `caps-table-check`, `lint-python` pyright), then `make dev-streaming && make stubs && uv run pyright python/` if any binding docstring changed after the last DOC-116 regeneration, then parsight `find_broken_doc_links` (repository_id `par-term-emu-core-rust`), which must show no rows for files 3d touched other than the historical CHANGELOG/AUDIT ones.
