# ENH-058: Bounded line reads in the par-mux client reader

> Filed from the 2026-10-12 /opus-audit enhancement pass (Security posture, "Hardening ideas"). Board card: `[ENH-058]`.
> Sequencing: independent of the open ARC/DOC remediation work. It touches only `crates/par-mux/src/mux/client.rs` and `server/protocol.rs` / `server/mod.rs` visibility, so it does not conflict with ARC-136/138/140 (attach `status.rs` / `render/`).
> Line-number citations below are pinned to commit `f4161f6` (the commit that added this plan); re-locate them by symbol on later trees.

**Priority**: low · **Estimate**: S

## Goal

`MuxClient`'s background reader (`reader_loop`) splits the daemon's byte stream with `BufReader::lines()`. `lines()` keeps growing one `String` until it sees `\n`, so a broken or wedged daemon that writes an unterminated line makes every client (`par-mux` CLI, `par-mux attach`, `AttachConn`) allocate without limit. The daemon already bounds the opposite direction with `fill_line_bounded` (SEC-104/SEC-127, 1 MiB). Reuse that same helper on the client side with a client budget, and close the connection when it trips. The caller's `send_checked` then reports `ConnectionAborted` instead of the process growing until OOM.

The threat model matches SEC-104: the daemon is the same user, so this guards against accidental runaway growth, not against an attacker.

## Current state

- **Client reader.** `crates/par-mux/src/mux/client.rs`:
  - `:12` imports `BufRead, BufReader`.
  - `from_stream()` `:197-209` spawns `reader_loop(stream, reply_tx, notification_tx)` on a plain thread.
  - `reader_loop()` `:299-361`: `let reader = BufReader::new(stream);` (`:304`), then `for line in reader.lines() { let Ok(line) = line else { break }; … }` (`:308-309`). The body handles block framing (`framing()` `:371`) and, outside a block, re-appends `\n` and feeds `TmuxControlParser::parse` (`:337-350`).
  - The client stream never gets a recv timeout. `grep -n set_recv_timeout client.rs` matches nothing, so `fill_buf` blocks and no poll wakes occur today.
  - `send_checked_timeout()` `:229-253` turns a dropped `reply_tx` (`RecvTimeoutError::Disconnected`) into `io::ErrorKind::ConnectionAborted, "server closed the connection"`. Ending `reader_loop` early is therefore already a clean, reported failure.
  - `:789` (`let mut lines = std::io::BufReader::new(stream).lines();`) belongs to the **test stub daemon** inside `replies_and_notifications_are_split_by_the_reader` (`:782`). It is test code, so leave it unchanged.
- **Daemon helper to mirror.** `crates/par-mux/src/mux/server/protocol.rs`:
  - `enum LineFill { Complete, Oversize, Eof }` at `:7-15` (`pub(super)`).
  - `fill_line_bounded<R: BufRead>(reader, buf: &mut Vec<u8>, max) -> io::Result<LineFill>` at `:21-48` (`pub(super)`). It retries `Interrupted`. Other errors propagate with `buf` intact, so a poll wake that splits a UTF-8 char loses nothing. This is the property `Lines` lacks, because `Lines` drops the partial on a timeout error. It copies at most `max + 1` bytes.
  - `read_control_line()` `:70-91` is the daemon's wrapper. It treats `Eof` with a non-empty buffer as a final line, the same as `Lines`, and decodes UTF-8 once at the end.
  - `is_poll_wake()` `:171-176` (`WouldBlock | TimedOut`).
  - `server/mod.rs:113-128` declares `mod protocol;` privately and does `use protocol::*;`. `MAX_CONTROL_LINE_BYTES = 1024 * 1024` with a `/// cap:` annotation is at `server/mod.rs:92-93`.
  - Unit tests: `server/tests.rs:224-262` (`ScriptedReader`), plus four `fill_line_bounded_*` tests (`:271`, `:281`, `:301`, and `fill_line_bounded_stops_at_the_first_newline`).
- **Legitimate client line sizes.**
  - A `%output` line carries one PTY read of at most 16 KiB (`par-term-emu-core/src/pty_session/reader.rs:78`). `escape_output` (`emit.rs:18-31`) widens it to at most 4 bytes per byte, about 64 KiB.
  - Reply bodies (`emit_block`, `emit.rs:222`) are written raw. The largest single line is `show-buffer` (`dispatch/buffers.rs:120-126`) of a buffer set by `set-buffer`. That buffer arrives on a daemon input line capped at 1 MiB, so it is at most just under 1 MiB.
  - A client budget of 4 MiB therefore never trips on a healthy daemon, and it still bounds a broken one.
- **Caps table.** `scripts/gen_caps_table.py` scans `crates/*/src/**/*.rs` for `/// cap:` (`:95-99`). `make caps-table-check` is part of `checkall` (`Makefile:387-388`, `:525`). Adding a cap constant therefore requires `make caps-table`, which regenerates the table in `docs/SECURITY.md` (the existing row is at `:1351`). The prose about the daemon's per-line budget is at `docs/SECURITY.md:1089-1095`.
- **Out of scope (note only).** `src/bin/par_mux/main.rs:413-421` (`pane_endpoint_refusal`) and `:443-447` (`stop_daemon`) also use unbounded `read_line` on a daemon socket. Both are short-lived CLI paths in the binary crate. Mention them in the card's notes as a possible follow-up, and do not change them here.

## Implementation

1. **Share the helper (visibility only, no move).** In `server/protocol.rs`, change `pub(super) enum LineFill` and `pub(super) fn fill_line_bounded` and `pub(super) fn is_poll_wake` to `pub(crate)`. In `server/mod.rs`, after `use protocol::*;` (`:127`), add:
   ```rust
   pub(crate) use protocol::{fill_line_bounded, is_poll_wake, LineFill};
   ```
   The daemon and `server/tests.rs` keep compiling unchanged, because `use protocol::*` still brings the items into scope. Check that nothing else is named `LineFill` (`grep -rn LineFill crates/par-mux/src` should match only protocol.rs and tests.rs today).
2. **Client cap constant.** In `client.rs`, next to `REPLY_TIMEOUT` (`:18`), add:
   ```rust
   /// Per-line byte budget for the client reader. A healthy daemon's longest
   /// line is a `show-buffer` body under the daemon's own 1 MiB input budget
   /// (a `%output` line is at most ~64 KiB), so 4x that never trips; a broken
   /// daemon streaming an unterminated line gets the connection closed
   /// instead of growing this process without bound.
   /// cap: Bytes accumulated from one daemon reply or notification line before the client closes the connection.
   const MAX_REPLY_LINE_BYTES: usize = 4 * 1024 * 1024;
   ```
   The `/// cap:` line must be the last doc line before the `const`. Follow the regex at `gen_caps_table.py:31-35`, which allows further `///` lines after `cap:` but not before. If the regex needs `cap:` to come first, put the `cap:` line first and the explanation after it, as `server/mod.rs:85-93` does.
3. **Testable line reader.** Add to `client.rs` (below `reader_loop`):
   ```rust
   /// One line from the daemon, as `reader_loop` consumes it.
   #[derive(Debug, PartialEq, Eq)]
   enum DaemonLine {
       /// A line with its `\n` (and a preceding `\r`) stripped, like `Lines`;
       /// also an unterminated final line read before EOF.
       Line(String),
       /// EOF with nothing pending, a read fault, a non-UTF-8 line, or a
       /// line past `max`: stop reading.
       Closed,
       /// The line passed `max` bytes.
       Oversize,
   }

   fn read_daemon_line<R: BufRead>(reader: &mut R, max: usize) -> DaemonLine {
       let mut buf = Vec::new();
       loop {
           match crate::mux::server::fill_line_bounded(reader, &mut buf, max) {
               Ok(crate::mux::server::LineFill::Complete) => break,
               Ok(crate::mux::server::LineFill::Eof) if buf.is_empty() => return DaemonLine::Closed,
               Ok(crate::mux::server::LineFill::Eof) => break,
               Ok(crate::mux::server::LineFill::Oversize) => return DaemonLine::Oversize,
               // No recv timeout is set on the client stream today; if one is
               // ever added, a wake must keep `buf` (Lines would drop it).
               Err(err) if crate::mux::server::is_poll_wake(&err) => continue,
               Err(_) => return DaemonLine::Closed,
           }
       }
       if buf.last() == Some(&b'\n') {
           buf.pop();
           if buf.last() == Some(&b'\r') {
               buf.pop();
           }
       }
       String::from_utf8(buf).map_or(DaemonLine::Closed, DaemonLine::Line)
   }
   ```
   - Mapping non-UTF-8 to `Closed` keeps today's behavior. `lines()` yields `Err(InvalidData)`, and `let Ok(line) = line else { break }` ends the loop.
   - The CR-then-LF strip matches `Lines`, which strips `\n` and then one `\r`.
   - Import with `use crate::mux::server::{fill_line_bounded, is_poll_wake, LineFill};` at the top rather than the full paths. The paths above only show where the items come from.
4. **Rewire `reader_loop`** (`:304-309`). Replace `let reader = BufReader::new(stream);` / `for line in reader.lines() { let Ok(line) = line else { break };` with:
   ```rust
   let mut reader = BufReader::new(stream);
   loop {
       let line = match read_daemon_line(&mut reader, MAX_REPLY_LINE_BYTES) {
           DaemonLine::Line(line) => line,
           DaemonLine::Oversize => {
               log::warn!(
                   "par-mux client: daemon line exceeded {MAX_REPLY_LINE_BYTES} bytes; closing the connection"
               );
               break;
           }
           DaemonLine::Closed => break,
       };
   ```
   The `match framing(&line)` body (`:310-358`) stays unchanged. `log = "0.4"` is already a par-mux dependency (`crates/par-mux/Cargo.toml:34`). Breaking out drops `reply_tx`, so a pending `send_checked` returns `ConnectionAborted` (`:245-250`). Remove the `BufRead` import only if it becomes unused (`read_daemon_line`'s bound still needs it).
5. **Unit tests** in `client.rs` `mod tests` (`:497`), next to `framing_parses_only_well_formed_numbered_lines`:
   - `read_daemon_line_strips_lf_and_crlf`: `Cursor::new(b"a\nb\r\nc".to_vec())` yields `Line("a")`, then `Line("b")`, then `Line("c")` (the EOF partial), then `Closed`.
   - `read_daemon_line_trips_the_budget_without_a_newline`: `Cursor` over `vec![b'x'; 300]` with `max = 100` yields `Oversize`.
   - `read_daemon_line_closes_on_non_utf8`: `b"\xff\n"` yields `Closed`.
   - `#[cfg(unix)] an_unterminated_oversize_daemon_line_aborts_the_pending_command`. Reuse `temp_socket` / `bind_local_listener` as in `:782-800`. The stub daemon reads the command line, then writes `%begin 1 5 1\n` followed by `MAX_REPLY_LINE_BYTES + 1` bytes of `x` in 64 KiB chunks with **no newline**. It then holds the socket open: it sleeps after writing, or blocks on a `read` until the client drops. Assert that `client.send_checked("show-buffer")` returns `Err` with `kind() == io::ErrorKind::ConnectionAborted` within the 10 s `REPLY_TIMEOUT`. The connection must stay open, so that only the budget, not EOF, can end the reader. Join the stub thread after dropping the client.
6. **Docs.**
   - `docs/SECURITY.md`: after the `MAX_CONTROL_LINE_BYTES` bullet (`:1089-1095`), add a bullet on the client side's `MAX_REPLY_LINE_BYTES` (4 MiB). A daemon line over the budget closes the client's connection and the pending command reports "server closed the connection".
   - Then run `PATH="$PWD/.venv/bin:$PATH" make caps-table` to regenerate the caps table row.
   - `CHANGELOG.md` `[Unreleased]` → `### Security`: one bullet citing ENH-058 and `crates/par-mux/src/mux/client.rs`.

## Files to touch

- `crates/par-mux/src/mux/server/protocol.rs` (three visibility changes)
- `crates/par-mux/src/mux/server/mod.rs` (one `pub(crate) use` re-export)
- `crates/par-mux/src/mux/client.rs` (cap const, `DaemonLine`, `read_daemon_line`, `reader_loop`, four tests)
- `docs/SECURITY.md` (prose bullet and the regenerated caps table)
- `CHANGELOG.md` (`[Unreleased]` Security bullet)

## Verify

Acceptance criteria:
- `reader_loop` in `crates/par-mux/src/mux/client.rs` no longer calls `.lines()`. `grep -n 'reader.lines()' crates/par-mux/src/mux/client.rs` matches nothing, and the only remaining `.lines()` in that file is the test stub at the old `:789`. Daemon lines go through `fill_line_bounded` with `MAX_REPLY_LINE_BYTES`.
- `cargo test -p par-mux --features mux-bin,attach --lib mux::client:: > /tmp/enh058-t.log 2>&1; echo EXIT=$?` prints `EXIT=0`. The log's `test result:` line shows a **nonzero** passed count that includes the four new tests: `grep -cE 'read_daemon_line_|an_unterminated_oversize_daemon_line' /tmp/enh058-t.log` prints `4`. The oversize-socket test passes, which shows a pending `send_checked` gets `ConnectionAborted` without waiting for EOF.
- The daemon side is unchanged and green: `cargo test -p par-mux --features mux-bin,attach --lib fill_line_bounded > /tmp/enh058-d.log 2>&1; echo EXIT=$?` prints `EXIT=0`, with 4 tests passed.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh058-ci.log 2>&1; echo EXIT=$?` prints `EXIT=0`. That run includes `caps-table-check`, proving that the `MAX_REPLY_LINE_BYTES` row is in `docs/SECURITY.md` (`grep -c MAX_REPLY_LINE_BYTES docs/SECURITY.md` prints at least `2`).

Optional Windows check (the reader is cross-platform): run the CLAUDE.md "Local Windows VM" step 4 command `cargo check --locked -p par-mux --lib --tests --features mux-bin` from the main checkout.

## Rollback

Revert `client.rs` to restore `reader.lines()`. The protocol.rs/mod.rs visibility widening is harmless on its own, so it can stay or go. Reverting `client.rs` also removes the cap const, so rerun `make caps-table` (or revert the `docs/SECURITY.md` table hunk) or `caps-table-check` fails. The wire is unchanged, so no client or daemon compatibility concern exists in either direction.

## Dependencies / sequencing

- None blocking. Land it any time. If ARC-137 / ENH-049 (bounded interactive commands) lands first, nothing conflicts: both only rely on `send_checked_timeout`'s existing `Disconnected` → `ConnectionAborted` mapping.
- Run `make caps-table` after the const lands, in the same commit.
