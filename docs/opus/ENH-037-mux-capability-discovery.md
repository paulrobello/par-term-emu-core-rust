# ENH-037: par-mux capability discovery (`list-commands`) and held-state replay on client registration

> Filed from the 2026-09-29 /opus-audit enhancement pass (cycle `audit-2026-09-29`). Board card: `[ENH-037]`.
> Sequencing:
> - **ARC-095** (a `pane-info` `exited=` token) is a prerequisite and stays separate. This card does not add that token. It generalizes ARC-095's optional "replay `%pane-exited` on registration" into a registration-time replay of all held state, and adds discovery.
> - The replay hooks the `ensure_registered` helper that **QA-199** extracts from `handle_client`, which is itself blocked by **SEC-127**. Order: SEC-127, QA-199, ARC-095, then this card.
> - ARC-090 (the zoom choke point) should land first, so the replayed zoom state is the enforced one.
> - Cross-repo follow-up: par-term consumes both halves. File that on par-term's board after merge.

**Priority**: medium · **Estimate**: M

## Goal

Two gaps let par-term misbehave against a daemon it did not build:

1. **No feature discovery.** par-term learns that a daemon lacks a command only by sending it and parsing `unknown command: <name>`. It does this for `version` (`par-term-mux/src/client.rs:340-345`) and for `pane-info` (`resync.rs:255`). Flag-level features cannot be probed that way at all: `resize-pane -Z`, `split-window -b` and `respawn-pane -k` either parse or fail on flags, depending on the daemon build.
2. **Held state is push-only.** `%pane-exited` is broadcast once, when the reaper observes a death. A client that registers later, such as a reattaching par-term or a second viewer, never learns that a pane is held. It shows a frozen screen with no exit chrome and no respawn affordance (ARC-095). Zoom state reaches a late client only if something happens to push a `%layout-change` for that window.

Add a `list-commands` query that advertises commands and feature tokens, and replay held state (dead panes and zoom) to each client as it registers.

## Current state

- **Commands.**
  - `COMMANDS` (`src/mux/command.rs:763-797`) has 33 `(name, parser)` pairs.
  - Unknown names fail in `parse_command` with `format!("unknown command: {name}")` (`command.rs:840`), which reaches the client as a numbered `%error` block.
  - `version` (`dispatch.rs:1276-1278`) returns `crate::mux::build_stamp()`, which is `<crate version>+<git sha[-dirty]>`.
- **A capability token on `version` is ruled out.** par-term's `check_daemon_version` (`par-term-mux/src/client.rs:256-285`) splits the first body line on the first `+` and compares everything after it as the sha. It returns `Mismatch` when the shas differ (`:276-281`). A trailing ` caps=…` would become part of the daemon "sha", so every up-to-date daemon would report `Mismatch` and prompt a restart toast. Discovery therefore needs its own command.
- **Registration.**
  - A connection joins the broadcast set on its first control command. `handle_client` (`src/mux/server.rs:450-708`, CC 41, hotspot 1271) repeats the `clients.lock().push(…)` block four times (`:529-536`, `:587-594`, `:632-639`, `:681-688`). QA-199 extracts those as `ensure_registered`.
  - Each client gets a bounded queue (`sync_channel(CLIENT_QUEUE_DEPTH = 4096)`, `server.rs:76,464`). Its writer thread drains that queue in order, so lines put on the client's own `tx` before the command's reply reach it before `%begin`.
- **Held state.**
  - `MuxPane::dead()` (`src/mux/pane.rs:348`) and `exit_code()` (`:363`) exist.
  - `reap_dead_panes` (`server.rs:960-1000`) marks a pane dead under the tree lock and broadcasts `%pane-exited` *after* releasing it.
- **Zoom.**
  - `MuxWindow::zoomed: Option<PaneId>` (`src/mux/tree/mod.rs:58`).
  - `broadcast_layout_change` (`server.rs:1057-1085`) renders zoom as a visible layout with the single pane plus raw flags `Z`. A per-client render is needed; today only the broadcast form exists.
- **Tests available.** `tests/mux_daemon.rs:391` already asserts `TmuxNotification::PaneRespawned` on the wire. The MEMORY note on broadcast registration applies: a client joins broadcasts on its first command, so a two-client test must register the observer first.

## Implementation

1. **`list-commands` command** (`src/mux/command.rs`, `src/mux/dispatch.rs`).
   - Add `MuxCommand::ListCommands`, a `parse_list_commands` that takes no args, a `COMMANDS` row `("list-commands", parse_list_commands)`, and `mutates() => false`.
   - Reply body: one line per command, `name [feature …]`, sorted by name. Generate the names from `COMMANDS` itself, not a second list.
   - Feature tokens come from a `const COMMAND_FEATURES: &[(&str, &[&str])]` next to `COMMANDS`:
     - `resize-pane`: `zoom`, `absolute`
     - `split-window`: `before`, `start-dir`
     - `respawn-pane`: `kill`, `start-dir`
     - `pane-info`: `cmd`, plus `exited` once ARC-095 lands
     - `refresh-client`: `cell-pixels`
     - `capture-pane`: `escape`
   - Add one daemon-level line, `features replay-held-state`, so a client can tell whether registration replay (step 2) happens. It has no `%` prefix: `MuxClient` treats a `%` line inside an open block as body (`src/mux/client.rs:250-300`), but a tmux-mode reader that classifies by `%` would not. `features` is not a command name, so the line cannot collide with a command row.
   - **Grammar rule** (documented in MUX.md): tokens are `[a-z-]+`; a client must ignore unknown tokens and lines; a daemon never removes a token without a CHANGELOG "Removed" entry.
   - Unit test: every `COMMANDS` name appears exactly once in the reply; every `COMMAND_FEATURES` key is a `COMMANDS` name, so a typo fails.
2. **Registration-time replay** (`src/mux/server.rs`), inside QA-199's `ensure_registered`.
   - **Close the reaper race.** Take the tree lock, snapshot the held state, then push the client entry while still holding it. That nests `clients` inside `tree`. Today no site nests the two in either order:
     - `run_with_state_path` takes them "one at a time — never nested" (`server.rs:302-315`);
     - `reap_dead_panes` releases the tree before broadcasting (`:966-993`);
     - `push_to_clients` takes only `clients` (`:886-902`).

     So tree-then-clients introduces no inversion. Document it as the lock order in a comment on `ensure_registered`, and never take `tree` while holding `clients`.
     - A death marked before the snapshot is in the snapshot.
     - A death marked after it is broadcast to a client set that already includes this client.
     - The one overlap is a pane that is in the snapshot and whose broadcast has not yet gone out. That client receives `%pane-exited` twice. It is idempotent (state is "held with code N"), so document "may repeat" rather than adding de-duplication.
   - **Snapshot contents**, both lists sorted for deterministic tests:
     - `%pane-exited %N [code]` for every pane where `dead()`, rendered by `emit()`.
     - `%layout-change @W …` for every window with `zoomed.is_some()`. Extract the line-rendering half of `broadcast_layout_change` into `render_layout_change(&MuxTree, WindowId) -> Option<String>` and reuse it, so the broadcast and replay forms cannot drift.
   - Send the replay lines on the registering client's own `tx`, before the reply to the command that registered it. Queue the lines only after the registration succeeded.
   - Hook connections never register, so they never get a replay.
3. **Docs.**
   - MUX.md: a `list-commands` row in the Command Reference, a "Capability discovery" paragraph (grammar, feature tokens, fallback), and a "Late clients" paragraph under Notifications ("on registration a client receives `%pane-exited` for each held pane and `%layout-change` for each zoomed window; a held pane's `%pane-exited` may repeat").
   - CHANGELOG `[Unreleased]` "Added" bullet.
   - If ENH-033 has landed, its gate forces the new row automatically.
4. **Client guidance** (MUX.md, for par-term). Send `list-commands` once after `version`. An `unknown command: list-commands` error means a pre-0.58 daemon: assume the 0.57.0 feature set, known from its `version` stamp, or none.

## Files to touch

- `src/mux/command.rs` (variant, parser, `COMMANDS` row, `COMMAND_FEATURES`, `mutates`, tests)
- `src/mux/dispatch.rs` (`cmd_list_commands` and its match arm)
- `src/mux/server.rs` (`ensure_registered` replay, `render_layout_change` extraction)
- `tests/mux_daemon.rs` (late-client replay integration tests)
- `docs/MUX.md`, `CHANGELOG.md`

## Verify

- `cargo test --lib --no-default-features --features rust-only,mux,serde mux::command` passes, including a new test asserting that the `list-commands` reply contains every `COMMANDS` name exactly once and that every `COMMAND_FEATURES` key is a `COMMANDS` name.
- A new `tests/mux_daemon.rs` test passes under `cargo test --no-default-features --features rust-only,mux,serde --test mux_daemon -- --test-threads=1`. It creates a pane, types `exit 3` into its shell (`send-keys -t %N -l 'exit 3'`, then `send-keys -t %N Enter`, which is sent as CR; `new-session`/`new-window` cannot take a command, per QA-219), waits until `pane-info` or a `%pane-exited` shows the pane held, then connects a new client whose first command is `list-panes`. That client receives `%pane-exited %N 3` before the `%begin` of its reply.
- A second new test passes: zoom a pane (`resize-pane -Z`), then connect a fresh client whose first command is `list-panes`. It receives a `%layout-change @W …` line whose flags field is `Z` before its reply block. A client that registered before the zoom receives no replay line.
- `par-mux --cmd version` still prints exactly one `<version>+<sha>` line with no extra tokens. `par-mux --cmd list-commands` lists 34 command lines plus one `features replay-held-state` line.
- `make checkall` is green, and `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` passes.

## Rollback

Remove the variant, the `COMMANDS` row and the replay call in `ensure_registered`. Clients that never sent `list-commands` are unaffected. Clients that did fall back to their `unknown command` path.
