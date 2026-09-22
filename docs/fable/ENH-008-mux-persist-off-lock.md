# ENH-008 — Move par-mux state persistence off the tree lock onto a coalescing writer thread

## Goal

Stop every structural control command (`new-window`, `split-window`, `kill-pane`, `rename-window`,
`select-*`, `resize-*`, `swap-pane`) from serializing every pane's full scrollback to JSON and
fsyncing it while `tree.lock()` is held. Capture the `PersistState` under the lock (cheap: clones
of already-materialized snapshots), then hand it to one persist thread that coalesces bursts and
writes the newest state. Companion to audit ARC-003, which lands the minimal version; this plan is
the full design including lazy per-pane snapshot reuse.

## Current State (verified 2026-09-22 at 9fa2237)

- `src/mux/server.rs:772` — `crate::mux::persist::save_to(&tree.lock(), path)` runs inside
  `dispatch_issued` after every mutating command, and `:116` runs the final save on shutdown.
- `src/mux/persist.rs:419-443` — `save_to(tree, target)` calls `tree.to_persist_state()` then
  `File::create` + `serde_json::to_writer` + `set_permissions(0600)` + `sync_all` + `rename`.
- `src/mux/persist.rs:211-260` — `to_persist_state` calls `pane.terminal().read().capture_snapshot()`
  for every pane; `TerminalSnapshot` (`src/terminal/replay_snapshot.rs:14-60`) carries
  `scrollback_cells: Vec<Cell>` for primary and alt grids.
- `src/mux/scrape.rs:466` (`scrape_tick`) and `src/mux/hooks.rs` take `tree.lock()` on the
  accept-loop heartbeat, so they stall for the whole save.
- `pane_output_sink` (`server.rs:812`) does not take the tree lock; output keeps flowing.
- D3.3 in `par-mux.md` accepts losing the last window on `kill -9`, so per-command durability
  is not a requirement; a clean SIGTERM still gets a synchronous final save.

## Implementation Steps

1. `src/mux/persist.rs`: split `save_to` into two functions.
   - `pub fn write_state(state: &PersistState, target: &Path) -> Result<(), PersistError>` — the
     body of today's `save_to` from `create_dir_all` onward (tmp file, to_writer, 0600, sync_all,
     rename). No tree access.
   - Keep `pub fn save_to(tree: &MuxTree, target: &Path)` as
     `write_state(&tree.to_persist_state(), target)` so the shutdown path and existing tests are
     untouched.
2. `src/mux/server.rs`: add a persist worker.
   - New field on `MuxServer`: `persist_tx: Option<std::sync::mpsc::Sender<PersistState>>` and
     `persist_join: Option<JoinHandle<()>>`.
   - In `run_with_state_path`, when `state_path` is `Some`, spawn a thread:
     ```rust
     let (tx, rx) = std::sync::mpsc::channel::<PersistState>();
     let path = state_path.clone();
     let join = std::thread::spawn(move || {
         while let Ok(mut newest) = rx.recv() {
             // Coalesce: drain everything queued behind and keep the last.
             while let Ok(later) = rx.try_recv() { newest = later; }
             if let Err(err) = crate::mux::persist::write_state(&newest, &path) {
                 log::error!("par-mux: state save failed: {err}");
             }
         }
     });
     ```
   - Pass a clone of `tx` into `handle_client` alongside `state_path` (the signature already
     threads `state_path: Option<PathBuf>`; replace it with `persist: Option<Sender<PersistState>>`).
   - In `dispatch_issued` (`:770-774`), replace the `save_to(&tree.lock(), path)` block with
     `if mutated { if let Some(tx) = persist { let state = tree.lock().to_persist_state();
     let _ = tx.send(state); } }`. The lock is held only for the capture.
   - On shutdown (`run_persisting`, `:113-119`): drop `persist_tx` first (closes the channel), join
     the worker (drains any queued state), then run the existing synchronous `save_to` final save.
     This preserves "a clean SIGTERM never loses the last window".
3. Lazy snapshot reuse (second step, same card):
   - `src/mux/pane.rs`: add `snapshot_cache: Mutex<Option<(u64, TerminalSnapshot)>>` keyed by
     `PtySession::update_generation()` (`src/pty_session.rs:1299`). Add
     `pub fn persisted_snapshot(&self) -> TerminalSnapshot` that returns the cached clone when the
     generation matches, else captures, stores, and returns.
   - `src/mux/persist.rs:248`: call `pane.persisted_snapshot()` instead of
     `pane.terminal().read().capture_snapshot()`.
   - The generation bumps on every PTY read (`pty_session.rs:679,826`), so an idle pane costs one
     `Vec<Cell>` clone instead of a full grid walk.
4. Tests:
   - `src/mux/server.rs` test near `:1658` asserts "save after every mutating command". Add a
     `pub(crate) fn flush_persist(&self)` on `MuxServer` for tests (send a sentinel, or join and
     respawn) or, simpler, make that test poll the file mtime with a 2 s deadline (the
     `wait_until` shape from `tests/mux_restart.rs:107`).
   - New unit test: enqueue 50 states in a burst, assert `write_state` ran fewer than 50 times
     (count via an `AtomicUsize` injected through a test-only hook) and the file holds the last.
   - `tests/mux_restart.rs`, `tests/mux_reattach.rs` must stay green unchanged.

## Files to Touch

- `src/mux/persist.rs` (split `save_to`, snapshot reuse call)
- `src/mux/server.rs` (worker thread, `dispatch_issued` capture-only save, shutdown ordering)
- `src/mux/pane.rs` (snapshot cache)
- `CHANGELOG.md` (Unreleased: "par-mux persistence no longer blocks control traffic")

Sequence after audit ARC-002 (dispatch decomposition) and ARC-016/QA-103 (shutdown handle) if
they run in the same cycle; all three edit `dispatch_issued`/`run_with_state_path`.

## Verify

- `cargo test --no-default-features --features rust-only,mux,serde -- --test-threads=1`
  (the Makefile `test-rust` mux line) passes, including `tests/mux_restart.rs` and
  `tests/mux_reattach.rs`.
- New burst-coalescing unit test passes and asserts fewer writes than commands.
- Manual latency check: start `par-mux`, create 4 panes, `yes | head -c 5M` in each, then time
  `split-window` from a second client. Before: hundreds of ms with 4 full-scrollback panes.
  After: under 10 ms (only the capture runs under the lock). Record both numbers in the
  card's notes.
- `make checkall` green.

## Rollback

Revert the three source files; the on-disk format is unchanged (same `PersistState`,
same `FORMAT_VERSION`), so a daemon built before or after reads the same file.
