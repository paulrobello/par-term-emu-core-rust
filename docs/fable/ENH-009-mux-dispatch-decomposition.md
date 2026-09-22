# ENH-009 — Decompose `dispatch_issued` and `parse_command` into per-command handlers

## Goal

Turn the two top churn × complexity hotspots in the repo (`server::dispatch_issued`, CC 86,
500 lines; `command::parse_command`, CC 63, 220 lines) into a table of small handler functions
with one shared post-dispatch tail, so adding a tmux command is one function plus one match
arm and the persistence rule ("structural commands save, content commands do not") is stated
on the command type rather than by which arms set `mutated = true`. This is the full version of
audit ARC-002 / QA-101 / QA-104, which land the minimum decomposition.

## Current State (verified 2026-09-22 at 9fa2237)

- `src/mux/server.rs:280-778` — one `match command` with ~20 arms; each arm takes `tree.lock()`,
  mutates, sets `mutated = true`, broadcasts `TmuxNotification`s, and returns `emit_block(...)`.
  Five `.expect("... verified the pane")` calls re-derive a window id after a mutation.
- `src/mux/command.rs:380-599` — `parse_command` holds four ad-hoc closures (`flag`, `has_flag`,
  `target_pane`, `target_window`, `trailing_after_target`) and duplicates `WxH` size parsing
  between `refresh-client -C` (inline) and `resize-pane -x/-y` (`parse_size_flag`).
- `handle_client` (`server.rs:250`) byte-sniffs `{` to route hook JSON before parsing.
- Tests are whole-dispatch string round trips at `server.rs:1040-1680` plus
  `tests/mux_*.rs`; they are the regression oracle for this refactor and must not change.

## Implementation Steps

1. `src/mux/command.rs`:
   - Add `struct Args<'a> { name: &'a str, args: &'a [&'a str], line: &'a str }` with methods
     `flag(&self, f) -> Option<String>`, `has_flag`, `pane(f) -> Result<PaneId,String>`,
     `window(f)`, `session(f)`, `trailing_after(f) -> String`,
     `size(f) -> Result<Option<(u16,u16)>,String>` (the existing `parse_size_flag` body).
   - Move each `match *name` arm body into `fn parse_<command>(a: &Args) -> Result<MuxCommand, String>`.
     `parse_command` becomes: split, build `Args`, `match name { "new-session" => parse_new_session(&a), ... }`.
   - Reuse `a.size("-C")` in `refresh-client` (deletes the 20-line inline copy).
   - Add `pub enum Line { Hook(String), Control(MuxCommand) }` and
     `pub fn parse_line(line: &str) -> Result<Line, String>` that does the `{` check; `handle_client`
     calls it instead of sniffing.
   - Add `impl MuxCommand { pub fn mutates(&self) -> bool }` returning `true` for every
     structural variant and `false` for `SendKeys`, `PasteBuffer`, `ListPanes`, `ListAgents`,
     `CapturePane`, `RefreshClient` (read-only) — copy the current `mutated = true` set exactly,
     then assert it in a unit test that walks every variant.
2. `src/mux/tree.rs`: make `split_pane`, `select_pane`, `resize_pane`, `resize_pane_absolute`,
   `swap_panes`, `kill_pane` return the owning `WindowId` in their `Ok` payload so the five
   `.expect` calls in the dispatcher disappear.
3. `src/mux/server.rs`:
   - Add
     ```rust
     struct Outcome {
         reply: String,
         notifications: Vec<TmuxNotification>,   // broadcast to all clients
         issuer_only: Vec<TmuxNotification>,     // sent only to the issuing client
         layout_changed: Option<WindowId>,
     }
     struct Ctx<'a> { tree: &'a Arc<Mutex<MuxTree>>, clients: &'a Clients, command_number: u32 }
     ```
   - One `fn cmd_<name>(ctx: &Ctx, ...fields) -> Outcome` per variant, in a new
     `src/mux/dispatch.rs` (keeps `server.rs` to the accept loop, client threads, and sinks).
   - `dispatch_issued` becomes: parse → `let outcome = match command { ... }` → shared tail:
     broadcast `notifications`, send `issuer_only` to `issuer`, `broadcast_layout_change` if
     `layout_changed`, persist if `command.mutates()` (the ENH-008 capture, or today's
     `save_to`), return `outcome.reply`.
   - Extract `fn push_to_clients(clients: &Clients, line: String)` and call it from
     `broadcast_notification`, `pane_output_sink`, `broadcast_layout_change` (QA-117).
4. Keep the `dispatch` test shim signature used by the unit tests at `server.rs:1040+`.
5. Update `src/mux/mod.rs` re-exports (`pub mod dispatch;` private is fine) and the module
   header comment that describes the dispatcher shape.

## Files to Touch

- `src/mux/command.rs`, `src/mux/server.rs`, `src/mux/tree.rs`, `src/mux/hooks.rs` (uses
  `parse_line`), new `src/mux/dispatch.rs`, `src/mux/mod.rs`
- `docs/ARCHITECTURE.md` mux section (added by audit DOC-002/DOC-004) once it exists

Sequence: land audit ARC-002 first if `/fix-audit` runs in the same cycle; this plan then
finishes the decomposition (the `Args` struct, `parse_line`, `mutates()`, `dispatch.rs`).

## Verify

- `cargo test --no-default-features --features rust-only,mux,serde -- --test-threads=1`
  green with zero test edits in `src/mux/server.rs` tests or `tests/mux_*.rs`.
- parsight after reindex: `find_most_complex_functions` shows `dispatch_issued` and
  `parse_command` below CC 15 each; no function in `src/mux/` above CC 30.
- `grep -c '\.expect(' src/mux/server.rs` decreases by 5.
- Unit test asserting `MuxCommand::mutates()` for every variant matches the pre-refactor set.
- `make checkall` green.

## Rollback

Single revert; wire protocol and persistence format are untouched.
