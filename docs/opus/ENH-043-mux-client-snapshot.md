# ENH-043: par-mux `client-snapshot` command and payload-carrying `%window-*`/`%agent-*` notifications

> Filed from the 2026-10-08 /opus-audit enhancement pass (cycle `audit-2026-10-08`). Board card: `[ENH-043]`.
> Sequencing:
> - This is the long-term fix for **ARC-125**. ARC-125's short-term remedy (find the window's owner with one query, fetch titles once) is optional. If it lands first, this card replaces its query sequence on the snapshot path and keeps it as the fallback for older daemons.
> - Builds on **ENH-037** (done): `list-commands` capability discovery and the `COMMANDS` feature column.
> - Touches `render/mod.rs`, the hottest file in the audit (SEC-207, SEC-208, ARC-122…ARC-131). Read it before editing. If ARC-127 (the `Modal`/`PendingWork` regroup) lands first, the status-refresh call site moves, but the plan does not change.
> - Cross-repo: par-term does not need to change. It can adopt `client-snapshot` later through the same `list-commands` gate. File that on par-term's board after merge.

**Priority**: high · **Estimate**: L

## Goal

Each status refresh in the render attach client runs a sequence of blocking `send_checked` round trips on the single UI thread. That thread also pumps stdin and paints, and an agent-hook burst triggers a refresh on every `%agent-*` event. Each call can block for up to `REPLY_TIMEOUT` (10 s).

Two changes cut the steady-state cost of a refresh from **S+P+6 round trips to 1**, where S is the number of sessions and P the number of visible panes:

1. **A daemon-side `client-snapshot -t <window>` query.** One reply carries everything the status bar, tab strip, pane border labels and side panel read today:
   - the workspaces;
   - the sessions, with the owner of the shown window marked;
   - the shown session's windows, with the active window marked;
   - the effective title of every pane in the shown window;
   - the agent roster.

   The attach client uses it when the daemon advertises it, and falls back to the old queries when it does not.
2. **Patch state from notification payloads instead of re-querying,** where the payload already carries the fact. Stop marking the status stale for events that cannot change what the bar shows.

**On enriching notifications.** The brief asked to add payloads to `%window-*` and `%agent-*`. This plan deliberately does **not** append tokens to existing notification lines. Current core parsers would misread them (see Current state, "Why existing notification lines cannot grow"), and par-term pins the core crate. Instead:
- Notifications that already carry the needed fact are patched locally: `%agent-state-changed`, `%agent-released`, `%window-renamed` and `%pane-title-changed`.
- Notifications that do not carry it (`%window-add`, `%window-close`, `%sessions-changed`, `%workspaces-changed`) still mark the status stale. With this card, that costs one round trip.

## Current state

All anchors are at HEAD `2cf0957`.

- **The refresh sequence.** `StatusState::refresh` (`crates/par-mux/src/mux/attach/status.rs:80-202`) sends, in order:
  1. `list-sessions` (`:88`);
  2. `list-workspaces` (`:98`);
  3. `list-windows -t $N` once per session until it finds the owner of the shown window (`:124-140`). This is the S term;
  4. `list-windows -t <owner>` again (`:148`);
  5. `pane-title -t %<focused>` (`:178`);
  6. `list-agents` (`:186`).

  The pump's step 3b (`crates/par-mux/src/mux/attach/render/mod.rs:911-951`) then calls:
  - `refresh_pane_titles` (`render/navigate.rs:236-246`), which sends one `pane-title -t %N` per layout pane. This is the P term, and it re-sends the focused pane's title;
  - `refresh_sidebar` (`render/sidebar.rs:159-179`) while the panel is up, which sends a second `list-workspaces`.

  Worst case per refresh: S + P + 6 round trips. Two more refresh call sites run at seed time and on reseed: `render/session.rs:295` and `render/navigate.rs:603`.
- **What marks the status stale** (`render/mod.rs:1051-1073`):
  - `%agent-state-changed`, `%agent-released`, `%agent-telemetry-changed`, `%sessions-changed`, `%workspaces-changed`, `%window-renamed`, `%session-renamed`, `%window-add`, `%window-close`, `%client-attached` and `%client-left` all set `status_dirty`.
  - `%window-pane-changed`, `%session-window-changed` and `%client-session-changed` also set it (`:1019-1050`).
  - The status bar never renders telemetry: `StatusState.agents` is `(agent, state)` only (`status.rs:71`, filled at `:186-199`). Marking the status dirty on `%agent-telemetry-changed` is therefore a pure waste.
  - `%pane-title-changed` falls into the `_ =>` arm (`:1074`), so render mode ignores the title it carries.
- **`SessionGone` contract.** When no session owns the window, `refresh` returns `Err(StatusError::SessionGone)` (`status.rs:141-143`) and **leaves `session_id` at its previous value**. The landing logic at `render/mod.rs:919-932` reads that stale `session_id` to find the session's active window. A failed `list-sessions` is `Err(StatusError::Query)` (`:88-93`): stale state survives, and the next mark retries.
- **The existing reply shapes the snapshot mirrors:**
  - `list-workspaces` (`dispatch/sessions.rs:220-240`): `+N: name[ active]`.
  - `list-sessions` (`dispatch/sessions.rs:134-173`): `+W: wname: $N: name`.
  - `list-windows -t $N` (`dispatch/windows.rs:205-251`): `@N <*|-> name`.
  - `pane-title` (`dispatch/panes.rs:294-307`): one line, `Pane::effective_title()` (`crates/par-mux/src/mux/pane.rs:320-324`).
  - `list-agents` (`dispatch/client.rs:211-270`): `%N <agent> <state> <source> [key=value…]`, built per pane with `roster_row_entry` (`dispatch/client.rs:189`) and sorted by pane id.
- **Owner lookup.** `MuxTree::session_of_window` (`crates/par-mux/src/mux/tree/mod.rs:748`) resolves a window's session in one call under the tree lock, which replaces the client-side S-term scan.
- **Command plumbing:**
  - `MuxCommand` enum: `crates/par-mux/src/mux/command/mod.rs:100+`. `ListAgents`/`ListCommands` are at `:124-128`.
  - `mutates()`: `:485-537`. Its query arms, which return `false`, are at `:514-535`.
  - `COMMANDS` table: `:1080-1128`. Each row is `(name, parser, feature tokens)`.
  - `list_commands_body()`: `:1135-1151`.
  - Parsers live in per-group files. `parse_list_agents` is at `command/panes.rs:25`, and `Args::window` (`command/mod.rs:690-695`) parses a `-t <window>` target.
  - `route_command` (`dispatch/mod.rs:198-253`) is wildcard-free, so a new variant fails to compile until it has a group.
  - The client group router is `dispatch/client.rs:8-18`.
- **A capability-gate trap.** `parse_command_features` (`attach/conn.rs:253-264`) records only `(command, token)` pairs, so a `COMMANDS` row with `&[]` is invisible to `has_command_feature` (`conn.rs:159-163`). The new row therefore carries a feature token (`v1`), and the client gates on `has_command_feature("client-snapshot", "v1")`.
- **Why existing notification lines cannot grow.** Every parser in `crates/par-term-emu-core/src/tmux_control.rs` folds a trailing token into a field:
  - `parse_window_close` (`:744-748`) takes the whole trimmed args as `window_id`.
  - `parse_window_add` (`:756-768`) uses `splitn(4, ' ')`, so anything after the triple lands in `window_raw_flags`. The client's zoom check reads that field (`render/mod.rs:994`).
  - `parse_agent_state_changed` (`:996-1023`) joins every word before a trailing `source=` into `state`.
  - par-term (`../par-term/par-term-tmux/src/parser_bridge.rs:97`) consumes these through the core crate. An appended token would corrupt window ids, zoom flags and agent state in every client built against today's core.
- **Docs gates.**
  - `scripts/check_mux_docs.py` (run by `make mux-docs-check`, part of `checkall`) diffs `COMMANDS` against the `docs/MUX.md` "Command Reference" rows (`docs/MUX.md:400-470`).
  - The status-bar paragraph at `docs/MUX.md:208` documents the old query round ("a burst of agent churn costs one `list-workspaces`/`list-sessions`/`list-windows`/`pane-title`/`list-agents` round").
- **Test harnesses.**
  - `StatusState` and `refresh` are `pub(crate)`, so round trips can only be counted inside the crate. Use the render tests' fake daemon: `scripted_conn` and `fake_daemon` (`attach/render/tests/mod.rs:1144-1230`) record every received line, answer from a `replies` map keyed by the exact line, and let a script override the `list-commands` reply. Existing refresh tests call `session.status.refresh(&mut conn, "@0", 1)` (for example `render/tests/session.rs:109`).
  - The real-daemon tests in `crates/par-mux/tests/mux_agents.rs` (`#![cfg(all(feature = "mux", unix))]`) have a `Control` helper (`:31-80`, with `body_lines`) and `hook_report` (`:317`) for claiming an agent.
- **Out of scope.** Passthrough mode's `refresh_status` (`attach/pump.rs:73-150`) has the same pattern on a smaller scale (no per-pane titles). Its adoption is optional and not part of this card's criteria.

## Implementation

1. **Wire shape** (document it in MUX.md exactly as written here). `client-snapshot -t <window>` replies with one row per line. The first word of each row is its kind. Rows appear in this order:
   ```
   snapshot 1
   workspace +W <*|-> <name>
   session $N <*|-> +W <name>
   window @N <*|-> <name>
   pane %N [<title>]
   agent %N <agent> <state> <source> [key=value…]
   ```
   - `snapshot 1` is the format version. Additive rule: a client ignores unknown row kinds and unknown trailing `key=value` tokens. A daemon only appends new row kinds and never changes an existing row's positional fields without bumping the version.
   - `workspace`: every workspace in id order. `*` marks the daemon's active workspace. The name is the rest of the line.
   - `session`: every session, in the same order `list-sessions` lists them (workspace order, then session order). `*` marks the session that owns `<window>`, `+W` is its workspace id, and the name is the rest of the line.
   - `window`: the owning session's windows in window order. `*` marks the session's active window. The name is the rest of the line.
   - `pane`: one row per pane of `<window>`, in `Window::panes()` order (`tree/mod.rs:102`). The title is the rest of the line, from `Pane::effective_title()`, with every `\r` and `\n` replaced by a space so that one title stays one row. An empty title is the bare `pane %N`.
   - `agent`: exactly the `list-agents` row, prefixed with `agent `. Rows are sorted by pane id. Build each with `roster_row_entry`, factored out of `cmd_list_agents` as described in step 3.
   - Errors:
     - A window target that does not resolve gets the resolver's normal error reply (`no such window …`).
     - A window that resolves but has no owning session gets `Outcome::err(ctx, "no session owns <window>")`.
     - The client maps both to `SessionGone` (step 7).
2. **Command parse** (`crates/par-mux/src/mux/command/`).
   - Add the variant `MuxCommand::ClientSnapshot { window: Target<WindowId> }` beside `ListAgents` (`command/mod.rs:~124`), with a doc comment that names ENH-043.
   - Add `parse_client_snapshot` in `command/panes.rs` next to `parse_list_agents`: `Ok(MuxCommand::ClientSnapshot { window: a.window("-t")? })`. Reject stray positionals the way `parse_list_windows` does (`command/windows.rs:41-47`, `reject_positionals`), with value flags `&["-t"]`.
   - Add the row `("client-snapshot", parse_client_snapshot, &["v1"])` to `COMMANDS`. The `v1` token is the client's gate (see the capability-gate trap).
   - Add `| MuxCommand::ClientSnapshot { .. }` to the `false` arm of `mutates()` (`:514-535`), so a query never triggers a state save.
   - Add parser tests in `command/tests.rs`:
     - `client-snapshot -t @3` parses to `ClientSnapshot { window: Target::Id(WindowId(3)) }`;
     - `client-snapshot` with no `-t` is an error containing `requires -t`;
     - `client-snapshot -t @3 stray` is an error;
     - `cmd.mutates()` is false.
3. **Dispatch** (`crates/par-mux/src/mux/dispatch/`).
   - In `route_command` (`dispatch/mod.rs:245`), extend the client group: `command @ (MuxCommand::RefreshClient { .. } | MuxCommand::ListAgents | MuxCommand::ClientSnapshot { .. })`.
   - In `route_client_command` (`dispatch/client.rs:8-18`), add `MuxCommand::ClientSnapshot { window } => cmd_client_snapshot(ctx, window)`.
   - Refactor `cmd_list_agents` (`dispatch/client.rs:211-270`): move the roster-building body into `fn roster_rows(guard: &MuxTree) -> Vec<String>`, which returns the sorted `"%N <entry>"` strings. `cmd_list_agents` then becomes `Outcome::ok(ctx, &roster_rows(&guard).join("\n"))` with a byte-identical reply. The existing list-agents tests must stay green.
   - Write `cmd_client_snapshot(ctx, window)`. Take **one** `ctx.tree.lock()` for the whole reply, so the snapshot is consistent. Then:
     - resolve with `guard.resolve_window_target(window)`, mapping an error to `Outcome::err`;
     - get the owner with `guard.session_of_window(id)`. `None` gives the error from step 1;
     - emit the rows in step 1's order, reusing the same sort and iteration rules as `cmd_list_workspaces` (sort `guard.workspaces()`), `cmd_list_sessions` (workspaces sorted, then `ws.sessions`), `cmd_list_windows` (`session.windows`, `session.active`), `cmd_pane_title` (`effective_title()`) and `roster_rows`.
     - Do not hold any other lock while reading pane titles. `effective_title` takes the pane terminal's read lock, exactly as `cmd_pane_title` does under the tree lock today.
   - Add dispatch unit tests in `dispatch/tests.rs` using the existing tree-harness pattern:
     - two sessions, the second with two windows, one of them split, and a pane given a spaced title with `select-pane -T`;
     - assert the exact reply lines, including `*` on the owning session and the active window and the spaced title as the rest of the line;
     - a title containing `\n` comes back on one `pane` row;
     - an unknown window is an error reply;
     - a claimed agent pane appears as an `agent` row equal to `"agent " +` its `list-agents` row.
4. **Docs** (`docs/MUX.md`).
   - **Command Reference** (`:400-470`): add after the `list-commands` row (`:434`): `| \`client-snapshot\` | \`-t <window>\` | One reply carrying the attach client's whole status read: \`snapshot 1\`, then \`workspace\`, \`session\`, \`window\`, \`pane\` and \`agent\` rows (see the client-snapshot bullet below) | — |`.
   - Add a bullet after the `list-commands` bullet (`:472`) that gives step 1's grammar, the additive and version rule, the `\r`/`\n`-to-space rule for titles, the `v1` feature token, and the **client contract**: use it only when `list-commands` advertises `client-snapshot v1`, otherwise fall back to the individual queries.
   - Rewrite the status-bar sentence at `:208` ("a burst of agent churn costs one … round") to: "one re-query per burst repaints the row. Against a daemon advertising `client-snapshot v1` that re-query is a single `client-snapshot -t <window>` round trip, and older daemons get the per-fact `list-workspaces`/`list-sessions`/`list-windows`/`pane-title`/`list-agents` round. `%agent-state-changed`, `%agent-released`, `%window-renamed` and `%pane-title-changed` patch the row from their own payload without a query."
   - `make mux-docs-check` must pass.
5. **Snapshot parse on the client** (`crates/par-mux/src/mux/attach/status.rs`).
   - Add a `pane_titles: Vec<(u32, String)>` field to `StatusState`, with an accessor `pub(crate) fn pane_titles(&self) -> &[(u32, String)]`.
   - Add `fn apply_snapshot(&mut self, body: &[String], window: &str) -> Result<(), StatusError>`, a pure function so it can be unit-tested without a connection. Parse each row by its first word:
     - `workspace` fills `workspaces` and `active_workspace`;
     - `session` fills `sessions` (id and name) and sets `session_id` to the `*` row;
     - `window` fills `windows` and `active_window`, falling back to the first window as today (`:170-174`);
     - `pane` fills `pane_titles`. `pane_title` (the focused pane's title) is set later by the caller from `pane_titles`;
     - `agent` fills `agents` with fields 2 and 3 as `(agent, state)`.
     - Unknown kinds are ignored. A missing or non-`1` `snapshot` header returns `Err(StatusError::Query)`.
     - If there is no `*` session row, return `Err(SessionGone)` and do not touch `session_id`.
   - Split `refresh` into:
     - `refresh_snapshot(&mut self, conn, window, focused)`. It sends `client-snapshot -t {window}`. A transport error is `Err(Query)`. A `reply.ok == false` is `Err(SessionGone)`, and `session_id` keeps its previous value, which preserves the landing contract at `render/mod.rs:919-932`. On success it calls `apply_snapshot`, then sets `pane_title` from `pane_titles` for `focused`.
     - `refresh_legacy`, today's body moved verbatim.
     - `refresh` dispatches: `if conn.has_command_feature("client-snapshot", "v1") { refresh_snapshot } else { refresh_legacy }`. The existing `refresh` callers (`render/session.rs:295`, `render/navigate.rs:603`, `render/mod.rs:914`) keep their signatures.
   - Add unit tests in `status.rs`'s test module, each named with the prefix `apply_snapshot_`:
     - a full body gives the same `StatusState` as the legacy fixtures (`:519` style);
     - an unknown row kind `future x y` is ignored;
     - no `*` session gives `SessionGone` with `session_id` unchanged;
     - a bad header gives `Query`.
6. **One refresh entry point** (`render/mod.rs`, `render/navigate.rs`, `render/sidebar.rs`).
   - Extract step 3b's body (`render/mod.rs:911-951`, the `match self.status.refresh(…)` plus titles plus sidebar) into `WindowSession::refresh_status_facts(&mut self, conn) -> Result<(), status::StatusError>` in `render/navigate.rs`, so tests can drive it. The pump keeps its `SessionGone` landing and `Query` arms around the call.
   - Make `refresh_pane_titles` snapshot-aware. When `conn.has_command_feature("client-snapshot", "v1")`, call `self.renderer.set_user_title(pane, title.trim())` for each layout pane from `self.status.pane_titles()`, with no queries. A layout pane with no snapshot row gets `""`. Otherwise run today's per-pane loop.
   - Make `refresh_sidebar` snapshot-aware. When the feature is present, build the rows from `self.status.workspaces()` plus `active_workspace`, as `(format!("ws:{id}"), name, id == active)`, instead of sending `list-workspaces`. Add a `pub(crate) fn active_workspace(&self) -> Option<&str>` accessor on `StatusState`.
7. **Patch from payloads** (`render/mod.rs` `handle_event`, `:973`).
   - Move `AgentStateChanged` and `AgentReleased` out of the dirty-only arm into a `StatusState::patch_agent(&mut self, pane: &str, agent: &str, state: Option<&str>)` call, then `self.draw_status_row()` with **no** `status_dirty`.
     - `Some(state)` upserts the `(agent, state)` chip for that pane. `None` (released) removes the pane's chip.
     - To keep roster order (pane-id order, as `list-agents` sorts it), change `agents` to `Vec<(u32, String, String)>` keyed by the numeric pane id, and keep `compose` reading `(agent, state)` (`status.rs:312-320`).
   - `WindowRenamed { window_id, name }`: `StatusState::patch_window_name`, then redraw the status row and the tab strip, with no dirty mark.
   - `PaneTitleChanged { pane_id, title }`: when the pane is in the layout, `self.renderer.set_user_title(pane, title.trim())`. When it is the focused pane, also update `StatusState`'s `pane_title` through a `set_pane_title` setter, then redraw the status row. No dirty mark.
   - `AgentTelemetryChanged`: `EventOutcome::Continue`, with **no** dirty mark. The bar does not render telemetry, so add a comment saying so.
   - Keep `status_dirty` for `%window-add`, `%window-close`, `%sessions-changed`, `%workspaces-changed`, `%session-renamed`, `%client-attached`, `%client-left` and the selection-follow events. They do not carry ownership or order. With the snapshot, each still costs one round trip.
   - Payload patching does **not** depend on the daemon feature: these payloads exist on every daemon this client supports.
8. **Round-trip test** (in-crate, `attach/render/tests/session.rs`, next to `strip_click_switches_windows_without_forwarding`):
   - `snapshot_refresh_is_one_round_trip`:
     - Script `list-commands` → `"client-snapshot v1\nlist-commands\nfeatures replay-held-state\n"`, plus one `client-snapshot -t @0` reply with 2 workspaces, 3 sessions, 2 windows, 3 panes and 2 agents.
     - Apply a 3-pane layout, drain the handshake lines from `rx`, then call `session.refresh_status_facts(&mut conn)` with the side panel on.
     - Collect the lines received within 300 ms. Assert the list is exactly `["client-snapshot -t @0"]`, and that `status.windows()`, the renderer's user titles for all 3 panes and the sidebar sections match the scripted reply.
   - `legacy_refresh_still_runs_without_the_feature`: the same scene with the default `list-commands` reply. Assert that the recorded lines contain `list-sessions`, `list-agents` and one `pane-title -t %N` per layout pane, and do not contain `client-snapshot`.
   - `agent_event_patches_without_a_query`: feed `handle_event(TmuxNotification::AgentStateChanged { pane_id: "%2", agent: "claude", state: "blocked", source: "hook" })`. Assert that `status_dirty` stays false, that no line reaches `rx` within 300 ms, and that the status row's composed text contains `claude:blocked`. Then send `AgentReleased` for `%2` and assert the chip is gone.
   - `telemetry_event_does_not_mark_status_dirty`.
9. **Integration test** (`crates/par-mux/tests/mux_agents.rs`, real daemon): `client_snapshot_matches_the_individual_queries`.
   - Create session `a`, then session `b` with `new-window` and a `split-window` in its second window.
   - Title one pane `select-pane -T 'my build'`, and claim a pane with `pane.report_agent` through `hook_report`.
   - Send `client-snapshot -t <b's second window>`.
   - Assert:
     - the first line is `snapshot 1`;
     - the `workspace` rows correspond 1:1, in order, to `list-workspaces` (id, name, active marker);
     - the `session` rows correspond to `list-sessions`, and exactly one row is `*`, the one for `b`;
     - the `window` rows equal `list-windows -t <b>` with `window ` prefixed;
     - the `pane` rows' titles equal `pane-title -t %N` for each pane in `list-panes -t <window>`;
     - the `agent` rows equal `list-agents` with `agent ` prefixed.
   - Also assert that `client-snapshot -t @999` is an error reply, and that `list-commands` contains the line `client-snapshot v1`.

## Files to touch

- `crates/par-mux/src/mux/command/mod.rs` (variant, `COMMANDS` row, `mutates`)
- `crates/par-mux/src/mux/command/panes.rs` (`parse_client_snapshot`)
- `crates/par-mux/src/mux/command/tests.rs`
- `crates/par-mux/src/mux/dispatch/mod.rs` (`route_command` arm)
- `crates/par-mux/src/mux/dispatch/client.rs` (`roster_rows`, `cmd_client_snapshot`)
- `crates/par-mux/src/mux/dispatch/tests.rs`
- `crates/par-mux/src/mux/attach/status.rs` (`apply_snapshot`, the `refresh` split, patch helpers, the agents key)
- `crates/par-mux/src/mux/attach/render/mod.rs` (step 3b call, `handle_event` arms)
- `crates/par-mux/src/mux/attach/render/navigate.rs` (`refresh_status_facts`, `refresh_pane_titles`)
- `crates/par-mux/src/mux/attach/render/sidebar.rs` (`refresh_sidebar`)
- `crates/par-mux/src/mux/attach/render/tests/session.rs`
- `crates/par-mux/tests/mux_agents.rs`
- `docs/MUX.md` (Command Reference row, client-snapshot bullet, status-bar paragraph `:208`)
- `CHANGELOG.md` (`[Unreleased]`: Added `client-snapshot`; Changed: render attach patches agent/title/rename events locally)

## Verify

- `cargo test -p par-mux --features mux-bin --lib command::tests` passes, including the new `client-snapshot` parse tests and `list_commands_reply_lists_every_command_exactly_once`.
- `cargo test -p par-mux --features mux-bin --lib dispatch::tests` passes, including the new `client-snapshot` reply-shape tests, and the existing `list-agents` tests pass unchanged after the `roster_rows` extraction.
- `par-mux --cmd list-commands` against a fresh daemon prints a line that is exactly `client-snapshot v1`.
- `cargo test -p par-mux --features attach --lib snapshot_refresh_is_one_round_trip` passes: with the feature advertised, one status refresh (titles and side panel included) sends exactly one line, `client-snapshot -t @0`.
- `cargo test -p par-mux --features attach --lib legacy_refresh_still_runs_without_the_feature` passes: without the feature, the refresh still sends `list-sessions`, `list-agents` and one `pane-title` per pane, and never sends `client-snapshot`.
- `cargo test -p par-mux --features attach --lib agent_event_patches_without_a_query` passes: `%agent-state-changed` and `%agent-released` update the chips with zero round trips and leave `status_dirty` false.
- `cargo test -p par-mux --features attach --lib telemetry_event_does_not_mark_status_dirty` passes.
- `cargo test -p par-mux --features attach --lib apply_snapshot` passes: an unknown row kind is ignored, a missing `*` session returns `SessionGone` with `session_id` unchanged, and a bad header returns `Query`.
- `cargo test -p par-mux --features mux-bin --test mux_agents client_snapshot_matches_the_individual_queries -- --test-threads=1` passes against a real daemon.
- `grep -n "client-snapshot" docs/MUX.md` shows a Command Reference row and a client-contract bullet, and `make mux-docs-check` exits 0.
- `grep -n "costs one \`list-workspaces\`" docs/MUX.md` returns nothing: the status-bar paragraph describes the single-round-trip path.
- `git diff 2cf0957 -- crates/par-term-emu-core/src/tmux_control.rs crates/par-mux/src/mux/emit.rs` is empty: no existing notification wire line changed.
- `make checkall` exits 0.

## Rollback

Revert the commit. The command is additive and feature-gated on the client side, so a reverted daemon simply stops advertising `client-snapshot v1` and clients fall back to the per-fact queries. No persisted state or existing wire line changes.
