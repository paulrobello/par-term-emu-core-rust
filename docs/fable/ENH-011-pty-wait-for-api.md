# ENH-011 — `PtyTerminal.wait_for_update()` / `wait_for_text()` blocking wait API

## Goal

Give Python and Rust callers a real wait primitive on the PTY session instead of
`time.sleep(0.2)` polling. The test suite has 67 fixed sleeps (audit QA-102) because the API
offers only `update_generation()` / `has_updates_since()` (`src/pty_session.rs:1299-1311`,
`src/python_bindings/pty.rs:451-466`). A condition-variable wait that is signalled by the
reader thread's "content applied" generation bump gives sub-millisecond wakeups, removes the
race between spawn and first assert, and is the primitive the QA-102 `conftest.py` helper
should be built on so the helper is not itself a sleep loop.

## Current State (verified 2026-09-22 at 9fa2237)

- `update_generation: Arc<AtomicU64>` bumped at `src/pty_session.rs:679` (pre-processing) and
  `:826` (content applied, still inside the write guard).
- No `Condvar`/`Notify` anywhere in `pty_session.rs`; `wait()`/`try_wait()` (`:1131,1149`) are
  process-exit waits only.
- The Python binding has zero `py.allow_threads` calls (`src/python_bindings/pty.rs`), so any
  blocking wait added there must release the GIL explicitly or it deadlocks observers.
- `parking_lot` 0.12 (`Cargo.toml:140`) provides `Condvar` with `wait_for(&mut guard, Duration)`.

## Implementation Steps

1. `src/pty_session.rs`:
   - Add field `update_signal: Arc<(parking_lot::Mutex<()>, parking_lot::Condvar)>` next to
     `update_generation` (`:78`, init at `:121`).
   - In the reader thread, immediately after the `:826` `fetch_add` (content applied), and also
     on EOF/exit where `running` is cleared, call `update_signal.1.notify_all()`. Do this after
     the write guard is dropped (the `}; // write guard dropped here` at `:835`) so waiters that
     wake can take a read lock without contention.
   - Add:
     ```rust
     /// Block until the generation advances past `since` or `timeout` elapses.
     /// Returns the new generation, or `None` on timeout.
     pub fn wait_for_update(&self, since: u64, timeout: Duration) -> Option<u64> {
         let deadline = Instant::now() + timeout;
         let (lock, cv) = &*self.update_signal;
         let mut guard = lock.lock();
         loop {
             let now = self.update_generation();
             if now > since { return Some(now); }
             if !self.is_running() && now == since { return None; } // child gone, nothing coming
             let remaining = deadline.saturating_duration_since(Instant::now());
             if remaining.is_zero() { return None; }
             cv.wait_for(&mut guard, remaining);
         }
     }
     /// Block until `predicate(&Terminal)` holds, re-checking after every update.
     pub fn wait_until(&self, timeout: Duration, predicate: impl Fn(&Terminal) -> bool) -> bool {
         let mut gen = self.update_generation();
         loop {
             if predicate(&self.terminal().read()) { return true; }
             match self.wait_for_update(gen, /* remaining */) { Some(g) => gen = g, None => return false }
         }
     }
     ```
     (track the deadline once in `wait_until`; the sketch elides that arithmetic.)
2. `src/python_bindings/pty.rs`: add
   - `wait_for_update(self, since: int, timeout: float = 5.0) -> int | None` wrapping the call in
     `py.allow_threads(|| ...)`.
   - `wait_for_text(self, needle: str, timeout: float = 5.0, scrollback: bool = False) -> bool`
     implemented as `wait_until` with a predicate over `term.content()` (or the scrollback
     export when `scrollback=True`); also under `allow_threads`.
   - Google-style docstrings with Args/Returns/Example per CLAUDE.md.
3. `python/par_term_emu_core_rust/_native.pyi`: regenerate with `make stubs`, run `make stub-check`.
4. `docs/API_REFERENCE.md` (PtyTerminal section) and `README.md` PTY example: document both
   methods; show `term.spawn(...); assert term.wait_for_text("$ ")` replacing the sleep idiom.
5. Tests:
   - Rust: `wait_for_update` returns `Some` after `spawn("/bin/echo", ["x"])` and `None` after a
     1 s timeout on an idle pane; `wait_until` with a predicate on content.
   - Python `tests/test_pty.py`: one new test for each method; then migrate the 22 existing
     `time.sleep` sites in that file to `wait_for_text` (this is the QA-102 remedy for that
     file, done here with the real primitive).
   - Deadlock guard: a test that registers a `PyCallbackObserver` and calls `wait_for_text`
     from the main thread must complete (proves the GIL is released).

## Files to Touch

- `src/pty_session.rs`, `src/python_bindings/pty.rs`, `python/par_term_emu_core_rust/_native.pyi`
- `docs/API_REFERENCE.md`, `README.md`, `CHANGELOG.md` (Unreleased: Added)
- `tests/test_pty.py`, `src/pty_session.rs` tests module

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session` green
  including the two new tests.
- `uv run pytest tests/test_pty.py -v` green with `grep -c 'time.sleep' tests/test_pty.py`
  reduced from 22 to at most 1 (the negative-assertion case, commented).
- Observer deadlock test passes within its 5 s pytest timeout.
- `make stub-check` clean; `make checkall` green.

## Rollback

Remove the two methods and the condvar field; `update_generation()` semantics are unchanged.
