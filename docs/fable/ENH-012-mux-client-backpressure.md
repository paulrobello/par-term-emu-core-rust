# ENH-012 — Bounded per-client channels with eviction in the par-mux daemon

## Goal

Stop a stalled control-mode client from growing daemon memory without bound. Each client gets
an unbounded `std::sync::mpsc::channel::<String>()` (`src/mux/server.rs:215`) drained by a
writer thread into a blocking socket write; if the client stops reading, the socket buffer
fills, the writer blocks, and every `%output` line for every pane accumulates in that channel
for as long as the daemon lives (D5: panes outlive clients). Replace with a bounded channel and
evict on overflow, the same policy tmux applies to a control client that stops draining.
Companion to audit ARC-011, which lands the minimal `sync_channel` + `try_send` edit; this plan
adds a configurable depth, a warning notification, and a test.

## Current State (verified 2026-09-22 at 9fa2237)

- `server.rs:215` `let (tx, rx) = channel::<String>();`; writer thread at `:222-228`.
- Three send sites use `retain(|(_, tx)| tx.send(line.clone()).is_ok())`:
  `broadcast_notification` (`:801-803`), `pane_output_sink` (`:817`),
  `broadcast_layout_change` (`:860-862`). A send fails only when the receiver is dropped.
- `Clients = Arc<Mutex<Vec<(ClientId, Sender<String>)>>>` (check the alias near `:60`).
- Hook-report connections (`{`-prefixed lines) are answered in place and never registered
  for broadcasts, so they are unaffected.

## Implementation Steps

1. `src/mux/server.rs`:
   - `const CLIENT_QUEUE_DEPTH: usize = 4096;` (lines; a 4 KiB `%output` line is the upper
     bound per entry, so the worst case is ~16 MiB per client — document that).
   - `:215` → `let (tx, rx) = std::sync::mpsc::sync_channel::<String>(CLIENT_QUEUE_DEPTH);`
     and change `Clients` to hold `SyncSender<String>`.
   - Add `fn push_to_clients(clients: &Clients, line: &str)` (also QA-117):
     ```rust
     clients.lock().retain(|(id, tx)| match tx.try_send(line.to_string()) {
         Ok(()) => true,
         Err(TrySendError::Full(_)) => { log::warn!("par-mux: client {id} not draining; evicting"); false }
         Err(TrySendError::Disconnected(_)) => false,
     });
     ```
     Call it from the three send sites.
   - Eviction drops the `SyncSender`; the writer thread's `rx.recv()` then errors and the
     thread exits, which closes the socket — the client observes a clean disconnect.
2. `src/bin/par_mux/main.rs`: no CLI flag in this pass (ARC-017 adds clap; if it has landed,
   expose `--client-queue-depth` there and thread it through `MuxServer::bind`).
3. Test in `src/mux/server.rs` tests: register a client whose receiver is never drained, push
   `CLIENT_QUEUE_DEPTH + 1` lines through `push_to_clients`, assert the clients vec is empty
   afterwards and a drained sibling client still received every line.
4. `docs/MUX.md` (added by audit DOC-002) operational note: "a control client that stops
   reading for more than 4096 pushed lines is disconnected; reconnect and re-query."

## Files to Touch

- `src/mux/server.rs`, `CHANGELOG.md`, `docs/MUX.md` (once it exists)

## Verify

- New unit test passes; `cargo test --no-default-features --features rust-only,mux,serde -- --test-threads=1` green.
- Manual: attach a client with `nc -U <socket>` piped into `sleep 600` (never reads), run
  `yes` in a pane from a second client; daemon RSS (`ps -o rss`) stays flat and the first
  client's socket closes within a second. Record before/after RSS in the card notes.
- `make checkall` green.

## Rollback

Revert to `channel()` and `send`; no protocol change.
