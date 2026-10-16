# ENH-061: `ws=+N` tail token on `%agent-state-changed`

> Filed from the 2026-10-12 /opus-audit enhancement pass. Board card: `[ENH-061]`.
> Sequencing: land this **after ARC-138 and ARC-140**. ARC-138 adds the local `status_dirty = true` fallback in `patch_agent`'s insert branch. ARC-140 retypes the same `patch_agent` / `set_sidebar_roster` lines (`AUDIT.md:377-378`). This card builds on both and keeps ARC-138's fallback for a `ws`-less line.
> Line-number citations below are pinned to commit `f4161f6` (the commit that added this plan); re-locate them by symbol on later trees. The `tmux_control.rs` citations were symbolized after the ARC-004 split of that file into the `tmux_control/` module (`notification.rs`, `parser.rs`, `tests.rs`).

**Priority**: low · **Estimate**: M

## Goal

The audit's ARC-138 local fix does not cover one case. When `%agent-state-changed` arrives for a pane the attach client has not seen, the client inserts the row with an empty workspace. It then marks the status dirty, and a full `client-snapshot`/`list-agents` refresh fills `ws`. Until that refresh lands, the row renders ungrouped.

This card closes that gap. The daemon will emit the owning workspace on the push line as a trailing `ws=+N` token:

```
%agent-state-changed %N <agent> <state> source=<hook|scrape> ws=+N
```

The attach client's roster then stays fully incremental, with correct grouping and no re-query.

`list-agents` already carries the same `ws=` key from the same map (`roster_rows`, `crates/par-mux/src/mux/dispatch/client.rs:253-264`), so the push and the roster agree by construction.

**The brief's backward-compatibility premise is wrong.** The brief says older parsers ignore extras. The core parser shipped through 0.60.0 does not ignore them, as shown below. This plan therefore makes the parser tolerant, fixes the token order so old parsers fail closed, and flags the Rust API break.

## Current state (HEAD aaf35ef)

### Emitter

`crates/par-mux/src/mux/emit.rs:118-132` builds `" source={source}"` only when `source` is non-empty, then formats `%agent-state-changed {pane_id} {agent} {state}{tail}\n`.

The round-trip tests are at `emit.rs:632-645` (`agent_state_changed_round_trips`) and `:647-690` (the source-less shape).

### Variant

`TmuxNotification::AgentStateChanged { pane_id, agent, state, source }` is defined in `crates/par-term-emu-core/src/tmux_control/notification.rs`. The enum has **no `#[non_exhaustive]`** (grep finds none). Adding a field is a breaking change for any downstream crate that destructures the variant without `..` or builds it with a struct literal.

### Core parser

`TmuxControlParser::parse_agent_state_changed` (`tmux_control/parser.rs`) splits on whitespace. It peels a final token only when `parts.len() >= 4` and that token starts with `source=`. Everything from index 2 up to that point is joined into `state`, because multi-word states round-trip.

The tests are the `AgentStateChanged` cases at the end of `par_mux_extension_optional_fields_decode` (`tmux_control/tests.rs`):
- multi-word state with source;
- no source;
- the three-token edge case: `%1 claude source=hook` gives state `source=hook` and an empty source.

`get_impact(parse_agent_state_changed, upstream)` returns `Low` risk, and its only caller chain is `parse_line` → `parse_control_data` → `parse`. Every consumer goes through `TmuxControlParser::parse`.

### Constructors (daemon)

All three constructors build the notification while a `pane_mut` borrow on the tree guard is live:
- `crates/par-mux/src/mux/hooks/report.rs:116-121` (state report, `source: "hook"`);
- `hooks/report.rs:256-265` (session-report rebroadcast);
- `crates/par-mux/src/mux/scrape.rs:599-604` (scrape tick).

### Workspace lookup

`MuxTree` has these lookups:
- `window_of_pane` (`crates/par-mux/src/mux/tree/mod.rs:754`);
- `session_of_window` (`:748`);
- `workspace_of_session` (`:775`), which reads `session_workspace`, the same map `roster_rows` reads at `dispatch/client.rs:259-264`.

`WorkspaceId`'s `Display` is the `+N` form, because `roster_rows` uses `.to_string()` for `ws=`.

### Fan-out

`push_to_clients_skipping` (`crates/par-mux/src/mux/server/broadcast.rs:31-50`) clones **one** line to every `ClientEntry` `(id, tx, evicted, abort)`. Those entries carry no capability field, and no client-to-daemon capability verb exists: `list-commands` feature tokens are daemon-to-client only (`crates/par-mux/src/mux/command/mod.rs:1130-1147`).

Per-client gating of the token is therefore not available without new protocol, and this plan rejects it.

### Attach client

- `crates/par-mux/src/mux/attach/render/mod.rs:1223-1232` destructures `AgentStateChanged { pane_id, agent, state, .. }`. It is already `..`-tolerant.
- It calls `status.patch_agent(&pane_id, &agent, Some(&state))`.
- `patch_agent` (`crates/par-mux/src/mux/attach/status.rs:456-475`):
  - the update branch keeps the stored `ws` (`self.agents[index].3.clone()`);
  - the insert branch stores `String::new()`.
- `set_sidebar_roster` (`render/sidebar.rs:205-240`) labels `{ws} · {agent} ({state})` only when the `ws` id is found in `ws_names`.
- `crates/par-mux/src/mux/attach/mod.rs:509`, the passthrough-mode match, uses `{ .. }`.
- The unit test is `render/tests/session.rs:2426-2470` (`agent_event_patches_without_a_query`).

### Streamer

The streamer is unaffected on the wire:
- `src/streaming/roster.rs:53-72` (`parse_state_changed`) takes state from token 3 and scans the remaining tokens for the `source` key, ignoring any other key.
- `parse_line` (`:76-99`) ignores `ws=` on snapshots.
- `roster_watcher.rs:169-176` re-emits the core notification through `emit()` and then `translate_notification`, so it picks up the new token transparently and drops it.
- `proto/terminal.proto:505-511` `AgentEntry` has no `ws` field, so deltas stay consistent with snapshots.
- `src/streaming/proto.rs`, `py_convert.rs:291-310` and `terminal.pb.rs` need **no change**.

### Python bindings

The `tmux_to_py!` macro (`src/python_bindings/types/notification.rs:164-183`) destructures every listed field with no `..`. The row at `:219-220` maps `pane_id`, `agent → name`, `state → value` and `source`. The exhaustive fixture list is at `:525-530`, and `:628` is a `{ .. }` match.

`PyTmuxNotification` (`:20-75`) has no workspace field.

### Integration tests with exact-line asserts

- `crates/par-mux/tests/mux_hooks.rs:547`, `:766`, `:935`;
- `crates/par-mux/tests/mux_agents.rs:170`.

All four assert `…{action} source=hook\n` byte-exact.

### Docs

- The notification table row in `docs/MUX.md:711` reads ``%agent-state-changed %N <agent> <state> [source=hook\|scrape]``.
- The provenance bullet at `docs/MUX.md:814` names the `source=` token.
- `scripts/check_mux_docs.py` has no `%agent-state-changed` grammar contract today (grep finds no `agent-state-changed` in it). ENH-056 is the roster-grammar contract card.

### Sister repo `../par-term` (depends on `par-term-emu-core-rust = "0.60"`, caret, `Cargo.toml:12`)

These sites break when the field is added:
- `src/app/tmux_handler/notifications/mux_drain.rs:61-66` is an exhaustive `if let … AgentStateChanged { pane_id, agent, state, source }` with no `..`, so it fails to compile.
- `src/app/tmux_handler/notifications/mux.rs:5443-5448` is a struct-literal test helper, so it fails to compile.

These sites are unaffected:
- `mux_drain.rs:208` uses `{ .. }`.
- `par-term-tmux/src/parser_bridge.rs:194-200` uses `#[allow(unreachable_patterns)]` with a wildcard.

### Backward compatibility, concretely (why the token order matters)

These cases show how each parser handles the new daemon line `%agent-state-changed %3 kimi working source=hook ws=+1`:

| Parser | Result |
|---|---|
| core ≤0.60.0 (`parse_agent_state_changed`) | Last token is `ws=+1`, not `source=`. State becomes `working source=hook ws=+1` and source is empty. |
| par-term on core ≤0.60.0 | `AgentEntry::from_push` calls `AgentSource::parse("")`, which returns `None`. The push is dropped ("dropped unattributed agent push", `mux_drain.rs:94-97`). The roster stays at its last `list-agents` value. This is **fail-closed**: stale, never corrupt. |
| old `par-mux attach` (≤0.60.0) | Patches state `working source=hook ws=+1` into the panel label until the next refresh. This is cosmetic, and a same-version daemon and client never hit it (`attach/conn.rs:88-97` warns on stamp mismatch). |
| streamer, any version (`roster.rs:53-72`) | Correct: state `working`, source `hook`, `ws` ignored. |
| new parser vs old daemon (no `ws=`) | `ws` is empty, so the client falls back to the existing ARC-138 `status_dirty` path. |

If `ws=` were placed **before** `source=`, an old core parser would still find `source=hook` last and yield state `working ws=+1` with source `hook`. par-term would accept that silently as a corrupt state. So `ws=` is always emitted **after** `source=`.

## Implementation

1. **Variant** (`TmuxNotification::AgentStateChanged`, `tmux_control/notification.rs`). Add a field after `source`:
   ```rust
   /// Owning workspace id (`+N`), the side panel's grouping key. Empty
   /// when the line carried no `ws=` token (a pre-ENH-061 daemon).
   ws: String,
   ```
   Extend the variant doc's `Arguments:` line to `…, source, ws`.

2. **Tolerant parser** (`parse_agent_state_changed`, `tmux_control/parser.rs`). Replace the single-`source=` peel with a tail loop.
   ```rust
   fn parse_agent_state_changed(args: &str) -> Option<TmuxNotification> {
       let parts: Vec<&str> = args.split_whitespace().collect();
       if parts.len() < 3 {
           return None;
       }
       // Trailing `key=value` tokens are metadata, never state. Index 2 is
       // always state (a three-token line keeps `source=hook` as its state),
       // and unknown keys are ignored so a future token never folds into it.
       let mut end = parts.len();
       let (mut source, mut ws) = (String::new(), String::new());
       while end > 3 {
           let Some((key, value)) = parts[end - 1].split_once('=') else { break };
           if key.is_empty() || !key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
               break;
           }
           match key {
               "source" => source = value.to_string(),
               "ws" => ws = value.to_string(),
               _ => {}
           }
           end -= 1;
       }
       Some(TmuxNotification::AgentStateChanged {
           pane_id: parts[0].to_string(),
           agent: parts[1].to_string(),
           state: parts[2..end].join(" "),
           source,
           ws,
       })
   }
   ```
   States are drawn from the fixed set `working|blocked|idle` (`hooks/report.rs:32`, the scrape patterns' `state =`), so no legitimate state word contains `=`.

   Extend `par_mux_extension_optional_fields_decode` (`tmux_control/tests.rs`) with these cases:
   - `%1 claude working source=hook ws=+2` gives state `working`, source `hook`, ws `+2`;
   - `%1 claude waiting for input source=scrape ws=+0` gives a multi-word state;
   - `%1 claude working source=hook future=x` gives state `working`, source `hook`, ws empty (an unknown key is ignored);
   - `%1 claude working ws=+2` gives source empty, ws `+2`.

   Keep the existing three-token assertion: state `source=hook`, source empty, ws empty.

3. **Emitter** (`emit.rs:118-132`). Destructure `ws`, then append `format!(" ws={ws}")` after the source tail when `ws` is non-empty. The order is `source=` first, then `ws=`. Add a comment pinning the order: "ws after source: a pre-ENH-061 parser then drops the line (empty source) rather than accepting `ws=` as part of a hook-attributed state."

   Add `agent_state_changed_with_ws_round_trips` and an exact-line assert: `"%agent-state-changed %3 kimi working source=hook ws=+1\n"`.

4. **Tree helper** (`tree/mod.rs`, next to `workspace_of_session` at `:775`):
   ```rust
   /// The workspace owning `pane` (pane → window → session → workspace).
   pub fn workspace_of_pane(&self, pane: PaneId) -> Option<WorkspaceId> {
       let window = self.window_of_pane(pane)?;
       let session = self.session_of_window(window)?;
       self.workspace_of_session(session)
   }
   ```
   It reads the same `session_workspace` map that `roster_rows` uses, so the push and `list-agents` cannot disagree. Add a unit test in the tree tests: a pane in a session of workspace `+1` yields `Some(+1)`, and an unknown pane yields `None`.

5. **Constructors.** Compute `ws` **before** the `pane_mut` borrow, while holding the same `guard`. Use `guard.workspace_of_pane(id).map(|w| w.to_string()).unwrap_or_default()`.
   - `hooks/report.rs`: in both `handle_state_report` (`:59` guard, `:116`) and `handle_session_report` (`:256`), compute `ws` right after `tree.lock()` and before `guard.pane_mut(header.pane_id)`.
   - `scrape.rs:524-604`: in the `for pane_id in panes` loop, compute `ws` before `guard.pane_mut(pane_id)` (`:541`).

6. **Every other constructor or destructure in this repo.** Add `ws` to each:
   - `hooks/tests.rs:111`, `:725`, `:999`. Derive the expected value from the test tree's workspace, for example `tree.lock().workspace_of_pane(p).unwrap().to_string()`. Do not hardcode it.
   - `scrape.rs:1038`.
   - `emit.rs:633`, `:652`, `:684`.
   - the `AgentStateChanged` expectations in `par_mux_extension_optional_fields_decode` (`tmux_control/tests.rs`).
   - `render/tests/session.rs:2438`.
   - `src/python_bindings/types/notification.rs:525`.
   - Run `cargo check -p par-mux --all-targets --features mux-bin,attach` and the root crate's checks to surface any site this list misses.

7. **Attach client.**
   - Change `patch_agent` (`status.rs:456`) to `patch_agent(&mut self, pane: &str, agent: &str, state: Option<&str>, ws: &str) -> bool`:
     - Update branch: keep the stored `ws` when the payload `ws` is empty, otherwise take the payload's.
     - Insert branch: store the payload `ws`.
     - Return `true` only when a row was inserted with an empty `ws` (the caller's "needs a refresh" signal).
   - In `render/mod.rs:1223`, destructure `ws` and pass `&ws`. Keep ARC-138's `status_dirty = true` only when `patch_agent` returns `true`. If ARC-140 has changed the row type to a struct, apply the same rule to its field.
   - `agent_event_patches_without_a_query` (`render/tests/session.rs:2428`): send `ws: "+1".into()` for a new pane. Assert `!session.status_dirty`, that the row's ws is `+1`, and that `received_within_300ms(&rx).is_empty()`.
   - Add `agent_event_without_ws_marks_status_dirty`: a new pane with `ws: String::new()` sets `status_dirty`.
   - Passthrough mode (`attach/mod.rs:509`) already uses `{ .. }`, so it needs no change.

8. **Python bindings.**
   - Add `pub workspace_id: Option<String>` to `PyTmuxNotification` (`notification.rs:20-75`) with a doc line: "Owning workspace id `+N` (for `agent-state-changed`; None when the daemon sent no `ws=`)". Include it in the getter list and in `empty()` as `None`.
   - Change the macro row at `:219` to `… source => source: own, ws => workspace_id: nonempty };`. The `nonempty` mode already exists at `:169`.
   - Extend the `:525` fixture with `ws: "+1".to_string()` and assert the projected `workspace_id`.
   - Run `make dev && make stubs` to regenerate `python/par_term_emu_core_rust/_native.pyi`.
   - Add a `workspace_id` row to the `TmuxNotification` table in `docs/API_REFERENCE.md`.

9. **Integration tests.** Change the exact-line asserts at `mux_hooks.rs:547`, `:766`, `:935` and `mux_agents.rs:170` to `…source=hook ws={ws}\n`. In each test, read `ws` from the same daemon with `list-agents`: parse the row's `ws=` extra for that pane. The assert then proves the push and the roster agree.

10. **Docs.**
    - `docs/MUX.md:711`: change the row to ``%agent-state-changed %N <agent> <state> [source=hook\|scrape] [ws=+N]`` and say that trailing tokens are `key=value`, `ws=` (the owning workspace, same value as `list-agents`' `ws=`) follows `source=`, and unknown keys must be ignored.
    - `docs/MUX.md:814`: mention the `ws=` token next to `source=`.
    - Update the render-mode status paragraph at `:274` if ARC-136/DOC-144 have not already described the side-panel patching.
    - Do **not** add a `check_mux_docs.py` contract here. ENH-056 owns roster-grammar contracts, so note in its card that `%agent-state-changed`'s tail keys could join it.
    - CHANGELOG `[Unreleased]`: add `### Added` for the token. Add a **breaking** `### Changed` note that `TmuxNotification::AgentStateChanged` gains a `ws: String` field, so Rust embedders that destructure without `..` or use struct literals must add it. Name par-term's two sites. The next release must be a minor bump. Also name the fail-closed behaviour for ≤0.60.0 parsers that receive the new line.

11. **par-term follow-up (not edited by this card).** In the report, ask the orchestrator to file a par-term card. It should cover:
    - add `ws` (or `..`) at `src/app/tmux_handler/notifications/mux_drain.rs:61-66` and `mux.rs:5443-5448` when par-term bumps to the release carrying this card;
    - optionally carry `ws` into its `AgentEntry`.

    No `../par-term` file is touched here.

## Files to touch

- `crates/par-term-emu-core/src/tmux_control/notification.rs` (variant), `crates/par-term-emu-core/src/tmux_control/parser.rs` (parser), `crates/par-term-emu-core/src/tmux_control/tests.rs` (tests)
- `crates/par-mux/src/mux/emit.rs` (emitter, tests)
- `crates/par-mux/src/mux/tree/mod.rs` (`workspace_of_pane`, test)
- `crates/par-mux/src/mux/hooks/report.rs`, `crates/par-mux/src/mux/hooks/tests.rs`
- `crates/par-mux/src/mux/scrape.rs`
- `crates/par-mux/src/mux/attach/status.rs`, `crates/par-mux/src/mux/attach/render/mod.rs`, `crates/par-mux/src/mux/attach/render/tests/session.rs`
- `crates/par-mux/tests/mux_hooks.rs`, `crates/par-mux/tests/mux_agents.rs`
- `src/python_bindings/types/notification.rs`, `python/par_term_emu_core_rust/_native.pyi` (generated), `docs/API_REFERENCE.md`
- `docs/MUX.md`, `CHANGELOG.md`
- **Not touched:** `proto/terminal.proto`, `src/streaming/*` (`roster.rs` already ignores unknown keys), `scripts/check_mux_docs.py`, `../par-term`

## Verify

Acceptance criteria:
- **Parser:** `cargo test -p par-term-emu-core par_mux_extension_optional_fields_decode` passes with a nonzero count. It covers `ws=` decode, an ignored unknown key, a multi-word state, and the three-token `source=hook`-as-state edge.
- **Daemon:** `cargo test -p par-mux --features mux-bin --test mux_hooks --test mux_agents` passes with nonzero counts. Each broadcast line ends `source=hook ws=+N`, with `+N` equal to the `ws=` that `list-agents` reports for the same pane.
- **Attach:** `cargo test -p par-mux --features mux-bin,attach agent_event` runs at least 2 tests and passes. A `ws`-carrying insert leaves `status_dirty` false with no round trip. An empty-`ws` insert sets `status_dirty`.
- **Gate:** `PATH="$PWD/.venv/bin:$PATH" make checkall-ci` exits 0 after `make dev && make stubs`, and `git diff --exit-code python/par_term_emu_core_rust/_native.pyi` is clean after the regen.

Commands:
```bash
cargo test -p par-term-emu-core par_mux_extension_optional_fields_decode > /tmp/enh061-core.log 2>&1; echo EXIT=$?
grep -E 'test result: ok\. [1-9]' /tmp/enh061-core.log
cargo test -p par-mux --features mux-bin emit:: > /tmp/enh061-emit.log 2>&1; echo EXIT=$?
grep -c 'agent_state_changed_with_ws_round_trips ... ok' /tmp/enh061-emit.log    # 1
cargo test -p par-mux --features mux-bin --test mux_hooks --test mux_agents > /tmp/enh061-int.log 2>&1; echo EXIT=$?
grep -E 'test result: ok\. [1-9]' /tmp/enh061-int.log
cargo test -p par-mux --features mux-bin,attach agent_event > /tmp/enh061-att.log 2>&1; echo EXIT=$?
grep -E 'test result: ok\. ([2-9]|[1-9][0-9])' /tmp/enh061-att.log   # nonzero; mux-bin alone false-greens 0
cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming roster > /tmp/enh061-stream.log 2>&1; echo EXIT=$?   # streamer still ignores ws=
PATH="$PWD/.venv/bin:$PATH" make dev && PATH="$PWD/.venv/bin:$PATH" make stubs
git diff --stat python/par_term_emu_core_rust/_native.pyi   # shows only the workspace_id addition, then commit it
PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh061-ci.log 2>&1; echo EXIT=$?   # EXIT=0
```

Backward-compat spot check (documentation evidence, not a gate): check out `v0.60.0` in a scratch worktree and add a throwaway test that parses `%agent-state-changed %3 kimi working source=hook ws=+1`. It yields an empty `source`, so par-term's `from_push` drops the line, which is fail-closed. Record the result in the CHANGELOG breaking note.

## Rollback

Revert the commit. The token is additive on the wire. A rolled-back daemon stops emitting `ws=`, and the new parser and attach client treat a missing `ws` as empty, which falls back to ARC-138's refresh path. Rolling back only the daemon side (emitter plus constructors) is safe on its own.

The Rust API field is the one non-additive piece. If a release has shipped, do not remove the field; leave it empty instead.

## Dependencies / sequencing

- **After ARC-138** (adds the empty-`ws` `status_dirty` fallback this card keeps) and **ARC-140** (retypes `patch_agent` rows). Rebase onto both, and apply step 7 to whatever row type ARC-140 leaves.
- **Independent of ENH-056.** If ENH-056 lands first, consider extending its contract to the `%agent-state-changed` tail keys as a follow-up, not in this card.
- **Release coupling.** This card requires a minor version bump at the next release cut, because of the new field on a non-`#[non_exhaustive]` public enum variant. par-term needs the follow-up card from step 11 before it bumps its core dependency.
