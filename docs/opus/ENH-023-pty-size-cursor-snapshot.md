# ENH-023: Lock-free size/cursor snapshot for `PtySession` polling consumers

> Filed from the 2026-09-26 run-2 /opus-audit enhancement pass (cycle `audit-2026-09-26-r2`). Board card: `[ENH-023]` (priority low, estimate M).
> Builds on AUDIT QA-130 (switch read-only getters to read locks). Do QA-130 first.

## Goal

Polling UIs (the Python TUI's refresh loop, par-term status bars, and the mux `list-panes` formatting) call `size()` and `cursor_position()` many times per second. Even with read locks (QA-130), each call contends with the PTY reader thread's write lock during output bursts. Publishing `(cols, rows, cursor_col, cursor_row, generation)` atomically from the reader thread makes these queries wait-free.

## Current state

- `src/pty_session.rs:1442` (`cursor_position`) and `:1449` (`size`) take the terminal lock (write today, read after QA-130).
- The reader thread (`start_reader_thread`, `:763-1006`) calls `terminal.write().process(..)` per chunk. `resize` (find with `find_symbol resize scope_path PtySession`) changes the size.
- `parking_lot` is already a dependency. `arc-swap` is not.

## Implementation

1. Add `struct Geometry { cols: AtomicU32, rows: AtomicU32, cursor: AtomicU64 /* col<<32|row */, generation: AtomicU64 }` in `pty_session.rs` (no new dependency; `AtomicU64` packing is enough). Store an `Arc<Geometry>` on `PtySession`.
2. Update it:
   - In the reader thread after each `process()`, while still holding the write guard, read `term.size()`/`term.cursor()` and store them with `Ordering::Release`.
   - In `resize()` after the terminal resize.
   - At construction.
3. `size()` and `cursor_position()` read the atomics (`Ordering::Acquire`). Cols, rows and cursor are published separately, so a reader could see a new size with an old cursor during a resize. Document that the values are each individually consistent and eventually consistent as a pair. Callers that need a consistent pair can use a new `snapshot_geometry()` that takes the read lock.
4. The Python `PtyTerminal` binding: `src/python_bindings/pty.rs:211,722` call `self.inner.size()` and benefit automatically. Check whether the macro-generated `size()`/`cursor_position()` getters on `PyPtyTerminal` go through `term_ref()` instead. If so, override them for `PyPtyTerminal` to use the atomics, which requires excluding them from `impl_terminal_query_getters!` for PTY or adding PTY-specific methods. Keep the Python surface identical.
5. Bench: add a criterion bench in `benches/` that calls `size()` from 4 threads while a 5th thread feeds `process()` with 1 MiB chunks. Compare against QA-130's read-lock version and record the numbers in the card notes. Use the interleaved A/B methodology from project memory (`bench-interleaved-ab-methodology`).

## Files to touch

- `src/pty_session.rs`
- `src/python_bindings/pty.rs` (only if the macro path bypasses `inner.size()`)
- `benches/pty_geometry.rs` (new) plus a `[[bench]]` entry in `Cargo.toml`

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize pty_session`: the existing size and cursor tests pass.
- A new test that after `resize(100, 30)`, `size() == (100, 30)` without any process call.
- `make test-pty`
- The bench shows p99 `size()` latency under contention at least 10 times lower than the read-lock baseline. If it does not, report and close as not worth it (a valid outcome).
- `make checkall`

## Rollback

Revert to the lock-based getters. The atomics are private and add no API.
