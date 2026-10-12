# ENH-049: Non-blocking command pipeline for the render pump

> Filed from the 2026-10-12 /opus-audit enhancement pass (AUDIT.md ARC-137 "Structural follow-up: ENH-049"; roadmap "Long-term 1"). Board card: `[ENH-049]`.
> Sequencing. Land this **last** of the render cards:
> - It needs **ARC-137** (the bounded `send_interactive` helper with the "daemon busy" flash). This card replaces that helper's blocking wait, and every sync path that remains uses it.
> - It needs **ARC-139/ARC-141**, which reshape `switch_*`/`focus_pane` and make `pane-info` typed and early-return.
> - It needs **ENH-050/051/052**, which reshape every call site this card converts.
> - It needs **DOC-144/DOC-145**, which rewrite the `docs/MUX.md` render-attach sections this card adds a paragraph to.
>
> Run it in four phases, each a separate commit with the gate green. Phases 2–4 can stop at any boundary, and the pump is still correct.

**Priority**: medium · **Estimate**: L

## Goal

The render pump (`WindowSession::pump`, `render/mod.rs:904-1012`) is single-threaded. It drains pushes with `conn.try_recv()`, routes stdin, and waits on `conn.recv_timeout(POLL)`, where `POLL` is 16 ms (`attach/mod.rs:243`). Every daemon command it issues is a **synchronous** `send_checked`: write one line, then block on the reply channel for up to `REPLY_TIMEOUT` = 10 s (`client.rs:18`). ARC-137 caps the user-initiated ones at about 1 s.

So pump responsiveness equals daemon latency. That includes the keystrokes themselves: `forward_chunked` (`attach/mod.rs:651-663`), which every typed byte, paste and mouse report goes through (`render/input_route.rs:153,191,321,855`), `send_checked`s each `send-keys -H` chunk and waits for its reply before reading more stdin. Chained commands serialise as well. `focus_pane` is `select-pane` → `pane-info` → `land_window` (`switch-client` → `select-window` fallback) → `reseed_window`, so it costs ≥ 4 round trips back to back.

**What this card can and cannot buy.** The daemon serves one connection's lines strictly in order (`Connection::serve`, `server/client.rs:144`). A `send-keys` written after a slow `select-pane` is therefore still *applied* after it, and no client change can reorder that. The gain is on the client:
- The pump thread stops waiting, so it keeps reading stdin, painting frames and draining notifications while replies are owed.
- Chains pipeline: N commands cost one write burst plus one round trip, instead of N serial round trips.
- A wedged daemon no longer freezes the UI. Bytes queue, and the "daemon busy" cue shows.

This card adds a reply-continuation queue. A command is written, its reply handler is parked, and the pump dispatches completed replies at the same 16 ms tick where it drains notifications.

**Done means:**
- the pump thread never blocks waiting for a reply on keystroke forwarding or on user-initiated chords and clicks. Each routing call returns in well under a frame regardless of daemon latency;
- replies are matched to the right handler even when sync and async commands interleave;
- a chain's later steps run when their predecessor's reply arrives.

**Explicitly out of scope:** `seed`, `reseed_window`'s size-report + `drain_around_layout` ordering, and `resize_to`. Their correctness depends on reading the reply *before* the notifications the reply queued (`session.rs:189-222`: the `%layout-change` that rides the size report must be the one consumed). They stay synchronous, bounded by ARC-137's helper. A chain that ends in a reseed runs that tail synchronously from inside its continuation.

## Current state (HEAD `aaf35ef`; re-locate by symbol)

- **Reply machinery.** `MuxClient` (`crates/par-mux/src/mux/client.rs`):
  - `reader_loop` splits replies onto `reply_rx: Receiver<Reply>` and notifications onto `notifications_rx`, both in FIFO order.
  - `send_checked_timeout` (`:229-255`) writes, then `recv_timeout`s. On timeout it bumps `abandoned_replies: usize` (`:36-41`) so the next wait discards that many blocks.
  - The bare counter assumes **every** outstanding reply belongs to a blocking caller. An async command in flight would have its reply stolen by the next sync wait. That is the central hazard this card fixes.
- **`AttachConn`** (`attach/conn.rs:182-226`) wraps `send_checked`, `send_checked_timeout`, `try_recv`, `drain_pending_events` and `recv_timeout`.
- **`send_checked` call sites under `render/`**: `grep -c send_checked` gives **56**: navigate 21, modal 16, session 11, input_route 5, mod 2, sidebar 1. The audit's "53" omitted three. Add `forward_chunked`'s loop (4 render callers). They fall into four classes:

  | Class | Sites | This card |
  |---|---|---|
  | **Startup** (before the pump runs) | `mod.rs:474,480` (`set-client-colors`, `refresh-client -C`), `session.rs:11` `displayed_session`, `:27-101` `resolve_window_and_pane`/`marked_pane_of` | stay sync |
  | **Reseed / resize internals** | `session.rs:420` (seed size report), `:503` `replay_panes` `refresh-client -t`, `:533` `focused_pane_or_first`, `:663` `resize_to` report, `navigate.rs:547` `reseed_window` report | stay sync (ARC-137 bound) |
  | **Fire-and-forget**, i.e. `let _ = conn.send_checked(..)` | 24 sites (`grep -c "let _ = conn.send_checked"` over render), e.g. `select-pane` after split/kill, `swap-pane`, `select-workspace`, `kill-window`, `rename-*`, `reload-config`, divider `resize-pane` (`input_route.rs:820`), plus `forward_chunked` | **phase 2** |
  | **Reply-consuming / chains** | the rest: split, kill+survivor, new-window, zoom, `switch_workspace`, `switch_session`, `focus_pane`, `land_window`, picker/menu queries (`modal.rs:240-303,494-556,693-721`), `refresh_sidebar` (`sidebar.rs:181`) | **phases 3–4** |
- **Status path.** `status_query`/`refresh_snapshot` use `send_checked_timeout(STATUS_TIMEOUT = 500 ms)` (`status.rs:69-83,163`) and stay sync. With the fixed queue (phase 1) they wait correctly behind outstanding async commands.
- **Test harness.** `render/tests/mod.rs`:
  - `FakeScript { replies, failing, notify, delays }` (`:1380-1387`) with `delays` already supports a slow daemon per line;
  - `fake_daemon` (`:1389`), `scripted_conn` (`:1359`), `recording_conn` (`:1462`), `drained` (`:1609`) and `wait_recorded`.

  Today's chord tests call e.g. `session.route_plain(..)` and then assert immediately on recorded lines and session state, because the replies were synchronous.
- **Suites that must stay green**:
  - `render/tests/{mod,session,chords}.rs` (157 `#[test]`), whose assertions must be adapted only by inserting `settle()`;
  - `attach/tests.rs` (passthrough; it does not use the queue but shares `MuxClient`);
  - `crates/par-mux/tests/mux_attach.rs` `render_mode_*` (PTY end-to-end);
  - the `client.rs` unit tests for abandoned-reply accounting (`grep -n abandoned crates/par-mux/src/mux/client.rs`).

## Implementation

### Phase 1: owed-reply queue in `MuxClient` (no call-site change)
1. Replace `abandoned_replies: usize` with `owed: VecDeque<Owed>`:
   ```rust
   enum Owed { Abandoned, Forget, Async(u64) }  // Forget: fire-and-forget, dropped on arrival but counted as outstanding
   ```
   and add `completed: VecDeque<(u64, Reply)>`.
2. **`send_nowait(&mut self, command) -> io::Result<u64>`.** It writes the line, pushes `Owed::Async(next_token)` and returns the token. **`send_forget(&mut self, command) -> io::Result<()>`** writes the line and pushes `Owed::Forget`.
3. **`send_checked_timeout`.** It writes its command, then counts `ahead = self.owed.len()`. Its own reply is the `ahead`-th block to arrive. On each received reply it pops the front of `owed`:
   - `Abandoned` or `Forget` means drop the reply;
   - `Async(t)` means push `(t, reply)` to `completed`;
   - once the `ahead` earlier entries are consumed, the next reply is its own.

   On timeout it pushes `Owed::Abandoned` for its own slot, so the ordering is preserved.
4. **`poll_replies(&mut self) -> Vec<(u64, Reply)>`.** It non-blockingly `try_recv`s `reply_rx`, resolves each against the front of `owed` with the same rule, and returns `completed.drain(..)` plus the newly resolved async replies, in order.
5. **`outstanding(&self) -> usize`.** The count of `Forget` + `Async` entries in `owed`, used for backpressure and by `settle`.
6. **Expose them on `AttachConn`** (`conn.rs`): `send_nowait`, `send_forget`, `poll_replies`, `outstanding`.
7. **Unit tests in `client.rs`**, over a local socket pair in the style of the existing `client.rs` tests:
   - (a) async A, then sync B: B gets B's reply, and A's reply appears in `poll_replies`;
   - (b) sync timeout, then async C, then sync D: the abandoned block is dropped, C is delivered and D gets D's reply;
   - (c) three async in a row are delivered in order; (d) forget replies are dropped, never returned by `poll_replies`, and `outstanding()` falls to 0 once they arrive.
8. Commit: `feat(mux): owed-reply queue lets async and sync commands share one connection (ENH-049 phase 1)`.

### Phase 2: fire-and-forget sites and key forwarding
9. In `render/` (not passthrough), replace each `let _ = conn.send_checked(X)` with `let _ = conn.send_forget(X)`. Keep a site sync only where the *next statement* needs the daemon to have applied the command before a sync read that follows. At `aaf35ef` the candidates are:
   - `select-workspace` followed by `land_on_workspace`'s `list-sessions` (`navigate.rs:218,337-338`);
   - `select-pane` followed by `pane-info` in `focus_pane`.

   The daemon processes one connection's lines in order, so a sync read issued after an async write still observes it. Mark these async too, and note the ordering argument in one line in each.
10. Add `forward_keys(conn, pane, bytes)` in `attach/mod.rs` next to `forward_chunked`, using `send_forget` per chunk. It stops sending when `conn.outstanding() > 64` and returns the unsent tail, which is backpressure against a wedged daemon. The caller keeps the tail in a `WindowSession` field `stdin_backlog: Vec<(String, Vec<u8>)>`, and `pump` flushes it at the start of step 3. Render callers (`input_route.rs:153,191,321,855`) switch to it. Passthrough keeps `forward_chunked`.
10a. **Drain replies every tick from phase 2 onward.** In `pump`, call `conn.poll_replies()` right after the step 1 notification drain and again after step 9's `recv_timeout`. In phase 2 no `Async` tokens exist yet, so the returned vec is empty and is dropped. What matters is that the call resolves `Forget` entries. Without it `outstanding()` never falls, and typing stops after 64 chunks. Phase 3 replaces this call with `dispatch_replies`.
11. Commit (phase 2).

### Phase 3: reply continuations
12. Add `render/pending_cmd.rs`:
    ```rust
    pub(super) type Continuation = Box<dyn FnOnce(&mut WindowSession, &mut AttachConn, Reply)>;
    #[derive(Default)]
    pub(super) struct PendingCommands { by_token: HashMap<u64, Continuation> }
    impl WindowSession {
        /// Write `command` now; run `then` on the pump tick its reply arrives.
        pub(super) fn send_then(&mut self, conn: &mut AttachConn, command: &str,
                                then: impl FnOnce(&mut Self, &mut AttachConn, Reply) + 'static);
        /// Pump step 1b: dispatch every completed reply's continuation, in arrival order.
        pub(super) fn dispatch_replies(&mut self, conn: &mut AttachConn);
    }
    ```
    - On a `send_nowait` error, the continuation is dropped and the `ARC-137` "daemon busy" flash is shown.
    - A transport error from `send_then` must not panic.
    - A continuation receives `Reply { ok: false, .. }` for `%error` and handles it exactly as the old `if !reply.ok { return; }` did.
    - Add `pending_cmds: PendingCommands` to `WindowSession`.
13. **Pump wiring.** Replace phase 2's two `conn.poll_replies()` calls in `pump` (`mod.rs:904`) with `self.dispatch_replies(conn)`. One sits right after the step 1 notification drain (before `apply_pending`), the other right after step 9's `recv_timeout`. Replies are then serviced at least every 16 ms tick. Continuations may set `pending.*`, `status_dirty` or `flash`, and the same tick's later steps pick those up.
14. Convert the **single-reply** sites to `send_then`. In each case the body after the reply becomes the closure:
    - split (parse `%N`, reseed, focus);
    - new-window → `land_window`;
    - zoom (toggle the cue on ok);
    - kill-pane → `list-panes` (step 15 chains it);
    - `refresh_sidebar`'s `list-workspaces`;
    - the picker and menu queries in `modal.rs` (open the picker in a "loading" state; the continuation fills `entries` and calls `refresh_picker`).
15. Commit (phase 3).

### Phase 4: chains
16. Rewrite each chain as nested `send_then`. The final reseed runs synchronously inside the last continuation (out-of-scope note above):
    - **`focus_pane`**: `select-pane` (nowait), then `send_then("pane-info -t %N", |s, c, r| if window differs { s.land_window(c, &w) } else { s.status_dirty = true })`.
    - **`land_window`**: `send_then("switch-client -t W", |s, c, r| if r.ok { s.reseed_window(c, &w) } else { s.send_then("select-window -t W", |s, c, r| if r.ok { s.reseed_window(c, &w) }) })`.
    - **`switch_workspace`** / **`switch_session`** / **`switch_window`**: their `list-*` queries become `send_then`, and the landing becomes the continuation.
    - **kill-pane**: `send_then("kill-pane", |…| s.send_then("list-panes -t W", |…| survivor → reseed + focus + select-pane nowait, else status_dirty))`.
17. **Staleness guard.** Each chain's closure captures `self.window` at issue time as `expected`. When the continuation runs and `s.window != expected` (another landing completed in between), it drops its work, so a slow reply cannot pull the view back. Pin this with a test.
18. Commit (phase 4).

### Tests (added across phases 2–4, in `render/tests/`)
19. Add `fn settle(session: &mut WindowSession, conn: &mut AttachConn)` to `tests/mod.rs`. In phase 2 it loops `conn.poll_replies()`, and from phase 3 on it loops `session.dispatch_replies(conn)`, in both cases with a 5 ms sleep until `conn.outstanding() == 0`. From phase 3 it also waits until `session.pending_cmds` is empty. It has a 5 s deadline and panics with the outstanding count when that passes. Insert `settle(&mut session, &mut conn)` after the routing call in every chord, menu, picker and click test whose assertions now depend on a continuation. Do not change assertion contents.
20. **New test `slow_daemon_does_not_block_key_forwarding`** (`tests/session.rs`). The fake daemon, like the real one, handles lines in order and sleeps before each delayed reply, so the assertion is about the *client* returning, not about daemon arrival order.
    - Script `delays` with `"select-pane -t %2"` → 2 s. Route a sidebar-row click to pane 2 (the `sidebar_agent_row_click_focuses_the_pane` setup) and time it. Then route the plain bytes `b"abc"` through `route_stdin_bytes` and time that too, with `Instant`.
    - Assert that each routing call returns in < 200 ms.
    - Then `settle`. Assert that the recorded lines contain `select-pane -t %2`, then the click's `pane-info`, then `send-keys -t … -H 61 62 63`, in that order.
    - **Proving the test bites:** temporarily point the render callers back at `forward_chunked` and `focus_pane` back at blocking sends. The routing-call assertion must then fail at ~2 s. Restore afterwards. Do not prove it with `git stash`, because that removes the test too.
21. **New test `focus_pane_chain_pipelines_and_ignores_a_stale_landing`.** Click pane A, which belongs to window @2, then before settling press the window-switch chord to @3. `settle`. The view must end on @3 and must never reseed back to @2. Assert on `session.window` and on recorded `refresh-client -C` count/order.
22. **New test `async_reply_never_answers_a_later_sync_query`.** Issue a `send_then` on a delayed line, then run `refresh_status` (sync `client-snapshot`). The status state must parse the snapshot reply, and the continuation must receive its own reply.

## Files to touch

- `crates/par-mux/src/mux/client.rs` (owed queue, `send_nowait`, `send_forget`, `poll_replies`, `outstanding`, unit tests)
- `crates/par-mux/src/mux/attach/conn.rs` (wrappers)
- `crates/par-mux/src/mux/attach/mod.rs` (`forward_keys`)
- `crates/par-mux/src/mux/attach/render/pending_cmd.rs` (new), `mod.rs` (field, pump wiring, `stdin_backlog`), `navigate.rs`, `modal.rs`, `input_route.rs`, `sidebar.rs`
- `crates/par-mux/src/mux/attach/render/tests/{mod,session,chords}.rs` (`settle`, three new tests)
- `docs/MUX.md` attach section: one paragraph stating that render-mode commands are pipelined and replies are dispatched at frame cadence

## Verify

Attach and render tests need `--features mux-bin,attach`, because `mux-bin` alone compiles **0** of them.

- **Baseline before phase 1.** `cargo test -p par-mux --features mux-bin,attach --lib mux:: > /tmp/enh049-base.log 2>&1; echo EXIT=$?` gives `EXIT=0`. Record N passed. After each phase the same command prints `EXIT=0`, `0 failed`, and passed ≥ N. After phase 4 it is ≥ N+7: 4 client tests and 3 render tests.
- `cargo test -p par-mux --features mux-bin,attach --lib client::tests > /tmp/enh049-cl.log 2>&1; echo EXIT=$?` prints `EXIT=0`, with the four owed-queue tests listed as `ok`.
- `cargo test -p par-mux --features mux-bin,attach --lib render::tests::session::slow_daemon_does_not_block_key_forwarding > /tmp/enh049-lat.log 2>&1; echo EXIT=$?` prints `EXIT=0` with `1 passed`. With the render callers temporarily switched back to `forward_chunked`/blocking `focus_pane`, the same command prints `EXIT=101`. Restore them afterwards.
- **Sync sites confined.** `grep -n "send_checked(" crates/par-mux/src/mux/attach/render/{navigate,modal,input_route,sidebar}.rs` lists only sites whose enclosing fn is `reseed_window` or a documented sync exception. Each remaining line has a `// sync:` constraint comment naming why.
- `cargo test -p par-mux --features mux-bin,attach --test mux_attach render_mode_ -- --test-threads=1 > /tmp/enh049-int.log 2>&1; echo EXIT=$?` prints `EXIT=0` with nonzero `passed`.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh049-ci.log 2>&1; echo EXIT=$?` prints `EXIT=0`.

Acceptance criteria:
- `slow_daemon_does_not_block_key_forwarding` passes: with a 2 s daemon delay on the click's command, the click and the following keystroke routing calls each return in < 200 ms, and after settling the daemon received `select-pane`, `pane-info` and `send-keys` in order. It fails (~2 s) when the callers are switched back to blocking sends.
- The owed-reply queue tests (async-then-sync, timeout-then-async-then-sync, ordered async, forget-drained) and `async_reply_never_answers_a_later_sync_query` pass, so interleaved sync/async replies are never mismatched.
- No `send_checked` remains in `render/{navigate,modal,input_route,sidebar}.rs` outside `reseed_window` and the `// sync:`-annotated exceptions, and the full `mux::` lib suite passes at ≥ baseline+7 with 0 failed.
- `mux_attach render_mode_` passes with a nonzero count, and `make checkall-ci` exits 0.

## Rollback

- **Phase 1** alone is behaviour-neutral: with no `send_nowait`/`send_forget` callers the queue only ever holds `Abandoned`, which is the old counter.
- **Phases 2–4** are reversible per commit. Reverting phase 4 leaves chains blocking but bounded (ARC-137), and reverting phase 2 restores synchronous key forwarding.
- **If continuation dispatch misbehaves in the field,** a one-line escape hatch is to make `send_then` call `send_checked_timeout` (the ARC-137 bound) and run `then` inline. That restores synchronous semantics without touching any call site. Keep that synchronous variant as a private fn beside `send_then`, so switching back is a one-line edit.
