# ENH-053: One `StatusFacts` builder behind `refresh_legacy` and `apply_snapshot`

> Filed from the 2026-10-12 /opus-audit enhancement pass (AUDIT.md "Detailed Findings → Code Quality": "`status.rs` `refresh_legacy` vs `apply_snapshot` 0.966 similar"). Board card: `[ENH-053]`.
> Sequencing: land this **after ARC-140** (typed `RosterEntry`, one `AgentRow::workspace()` accessor), and therefore after ARC-138 (`patch_agent` insert branch). Both of those edit `status.rs`, and ARC-140 already collapses the duplicated `ws` extraction closure that is part of this duplication. Run step 0 before writing anything.
> Line-number citations below are pinned to commit `f4161f6` (the commit that added this plan); re-locate them by symbol on later trees.

**Priority**: low · **Estimate**: S

## Goal

`StatusState` in `crates/par-mux/src/mux/attach/status.rs` fills the same nine fields from two wire shapes:

- **`apply_snapshot`** (`:193-290`) parses one `client-snapshot` body. It is the ENH-043 path.
- **`refresh_legacy`** (`:292-368`) parses five per-fact query replies: `list-sessions`, `list-workspaces`, `list-windows` through `find_owner`, `pane-title` and `list-agents`.

parsight's `duplicate_code` near-duplicate lane scores the pair at **0.966**. The duplication is in the *assignment* half:

- the workspace triple, which becomes `workspaces` plus `active_workspace`;
- the window rows, which become `(id, display name)` plus `active_window` with its first-window fallback;
- the agent rows, which become `(pane, agent, state, ws)`.

Because of it, each new fact (ARC-138's `ws`, ENH-061's notification `ws=`) has to be added twice.

Introduce one private builder, `StatusFacts`, that both paths fill, row by row, from their own parsing. Add one `StatusState::commit(facts)` that performs every field assignment. Leave the two transport paths and their error semantics unchanged:

- snapshot is all-or-nothing;
- legacy is per-fact keep-on-failure;
- `TimedOut` stops at the first slow query.

## Current state (HEAD `aaf35ef`; re-locate by symbol if lines drift)

- **Entry point.** `StatusState::refresh` (`:140-155`) dispatches on `conn.has_command_feature("client-snapshot", "v1")` to either `refresh_snapshot` (`:157-188`), which calls `apply_snapshot` and then derives `pane_title` from `pane_titles`, or `refresh_legacy`.
- **`apply_snapshot`** collects the locals `workspaces`, `sessions`, `owner`, `windows`, `active_window`, `pane_titles` and `agents` in one `match kind` loop. It returns `SessionGone` before touching `self` when no `*` session row exists, which gives the "nothing half-applied" contract pinned by `apply_snapshot_without_an_owner_is_session_gone` (`:837`). After that check it assigns all fields at `:263-289`.
- **`refresh_legacy`** assigns as it goes:
  - `self.workspaces` and `self.active_workspace` at `:312-319`, the same projection as `:263-270`;
  - `self.sessions` via `parse_session_line` (`targets.rs:28`);
  - `self.session_id`, `self.windows` and `self.active_window` from `find_owner` (`:373-414`) plus `WindowRow::parse` and `window_display_name`, the same projection as `:247-256` and `:273`;
  - `self.pane_title`;
  - `self.agents`, with a `ws` closure duplicated from `:229-236` at `:356-362`.

  The legacy path never fills `pane_titles` (field doc at `:101-103`).
- **Related types.** `RefreshRows { workspaces: Option<Vec<(String, String, bool)>> }` (`:62-65`) carries the roster to the side panel. The agents field is `Vec<(u32, String, String, String)>` (`:104-108`), and ARC-140 replaces it with `RosterEntry`.
- **Tests that must stay green**, unchanged in assertion content:
  - `status.rs` unit tests (`mod tests` `:745+`): `apply_snapshot_fills_the_same_state_as_the_legacy_queries` (`:805`), `apply_snapshot_ignores_an_unknown_row_kind` (`:823`), `apply_snapshot_without_an_owner_is_session_gone` (`:837`), `apply_snapshot_with_a_bad_header_is_query` (`:854`), `payload_patches_update_agents_and_window_names` (`:868`), plus the `compose_*` suite.
  - `attach/tests.rs`: `passthrough_status_refresh_uses_client_snapshot_when_advertised`, `passthrough_status_refresh_keeps_the_legacy_round_without_the_feature`, `passthrough_status_snapshot_refusal_falls_back_to_legacy` (`~:2741-2860`).
  - `render/tests/session.rs`: `snapshot_refresh_is_one_round_trip` (`:2338`), `legacy_refresh_still_runs_without_the_feature` (`:2393`), `sidebar_agents_section_groups_by_workspace` (`:2277`).
  - `tests/mux_agents.rs`: `client_snapshot_matches_the_individual_queries` (`:436`), which compares the live daemon's two shapes.

## Implementation

0. **Re-read after ARC-140.** `git log --oneline -- crates/par-mux/src/mux/attach/status.rs` should show ARC-138 and ARC-140. Re-read `:129-420`. Wherever ARC-140 already merged a projection (the `ws` closure, an `AgentRow::workspace()` accessor), skip that part of step 1 and use what it built. If `agents` is now `Vec<RosterEntry>`, use `RosterEntry` everywhere below instead of the tuple.
1. **Add the builder** next to `RefreshRows`:
   ```rust
   /// One refresh's facts, filled row by row from either wire shape and
   /// committed whole by [`StatusState::commit`] (ENH-053). `None` /
   /// empty-and-unset fields keep the state's previous value — the legacy
   /// path's per-fact keep-on-failure rule.
   #[derive(Debug, Default)]
   struct StatusFacts {
       workspaces: Option<Vec<(String, String, bool)>>,
       sessions: Option<Vec<(String, String)>>,
       session_id: Option<String>,
       windows: Option<Vec<(String, String)>>,
       active_window: Option<String>,
       pane_title: Option<String>,
       pane_titles: Option<Vec<(u32, String)>>,
       agents: Option<Vec<RosterEntry>>, // or the tuple, pre-ARC-140
   }
   impl StatusFacts {
       fn push_window_row(windows: &mut Vec<(String, String)>, active: &mut Option<String>, row: WindowRow);
       fn agent_entry(row: AgentRow) -> Option<RosterEntry>; // pane_number + ws extraction, once
   }
   ```
   - `push_window_row` holds the `if row.active { … }` and `window_display_name` logic, which both paths currently write separately.
   - `agent_entry` is the single agent-row projection.
2. **Add `StatusState::commit(&mut self, facts: StatusFacts) -> Option<Vec<(String, String, bool)>>`.** For each `Some` field it assigns, in the current order:
   - `workspaces` sets both `self.workspaces` (the id/name projection) and `self.active_workspace`;
   - `sessions`, `session_id` and `windows`;
   - `active_window`, using `facts.active_window.or_else(|| first window id)` whenever `windows` is `Some`;
   - `pane_titles`, `pane_title` and `agents`.

   It returns the workspace triple for `RefreshRows`. `None` fields leave the state alone.
3. **Rewrite `apply_snapshot`** as a parse into a fully populated `StatusFacts`: every field `Some`, including empty vectors, because the snapshot is authoritative.
   - Keep the header check and the `SessionGone`-before-commit ordering. `commit` runs only after `owner` is known, which preserves "nothing half-applied".
   - It returns `Ok(self.commit(facts).unwrap_or_default())`.
   - `refresh_snapshot` keeps deriving `pane_title` from `pane_titles` after the commit, unchanged.
4. **Rewrite `refresh_legacy`** to fill a `StatusFacts` query by query and call `commit` once at the end.
   - Each `status_query(...)?` that returns `Ok(None)` leaves its field `None`, which keeps the previous value. That is today's behaviour.
   - Queries are still issued in the same order: `list-sessions`, `list-workspaces`, `find_owner`, `pane-title`, `list-agents`. A `TimedOut` (`?`) still returns **before** `commit`.
   - **Behaviour delta, which must be decided and stated in the commit.** Today a legacy refresh that times out at `pane-title` has *already* written `workspaces`, `sessions` and `windows`. Committing once at the end discards them, so the stale bar stands whole. That is consistent with the snapshot path and with the `StatusError::TimedOut` doc ("the stale state stands"). Keep the end-commit. If a test pins the partial write, update its assertion and cite this card. No such test exists at `aaf35ef`: the grep for `TimedOut` in `attach/tests.rs` and `render/tests/` finds only full-timeout cases. Confirm with `grep -rn "TimedOut" crates/par-mux/src/mux/attach/`.
   - `find_owner` is unchanged. It is the legacy-only owner search.
5. **Add one unit test** to `status.rs` `mod tests`: `legacy_and_snapshot_commit_identical_state`.
   - Build a `StatusFacts` from the legacy row shapes that correspond to `snapshot_body()`: the `list-workspaces` lines `+0: alpha *` / `+1: beta` (check against `parse_workspace_line`'s accepted shape in `targets.rs:34`), the `list-sessions` lines, the window rows and the agent rows.
   - Push those rows through the same helpers `refresh_legacy` uses. Factor the per-query parse into `fn legacy_facts_from(sessions: &[String], workspaces: Option<&[String]>, owner: (String, Vec<String>), title: Option<&[String]>, agents: Option<&[String]>) -> StatusFacts` so it is testable without a connection.
   - Commit it and assert equality with `apply_snapshot(&snapshot_body())`'s state, with `pane_titles` excluded because the legacy path does not have it. That makes the 0.966 duplication a single tested equivalence.

## Files to touch

- `crates/par-mux/src/mux/attach/status.rs` (builder, `commit`, both paths, one test)

## Verify

Prefix `make` with `PATH="$PWD/.venv/bin:$PATH"`. Attach and render tests need `--features mux-bin,attach`, because `mux-bin` alone compiles **0** attach tests.

- **Baseline first** (before editing): `cargo test -p par-mux --features mux-bin,attach --lib mux::attach > /tmp/enh053-base.log 2>&1; echo EXIT=$?`. Record the `test result: ok. N passed` count. After the change, the same command prints `EXIT=0` with `passed` ≥ N+1 and `0 failed`. N must be nonzero; a `0 passed` result means the wrong feature set.
- `cargo test -p par-mux --features mux-bin,attach --lib status::tests > /tmp/enh053-st.log 2>&1; echo EXIT=$?` prints `EXIT=0`, and the log lists `legacy_and_snapshot_commit_identical_state ... ok` and `apply_snapshot_without_an_owner_is_session_gone ... ok`.
- `cargo test -p par-mux --features mux-bin,attach --test mux_agents client_snapshot_matches_the_individual_queries -- --test-threads=1 > /tmp/enh053-int.log 2>&1; echo EXIT=$?` prints `EXIT=0` with `1 passed`.
- **Duplication gone.** After reindexing, parsight `audit_repository` `duplicate_code` (`repository_id` `repo-db09184b0394f6287256c56299eaeb0f`) no longer lists the `refresh_legacy` / `apply_snapshot` pair at ≥ 0.90. `grep -c 'find(|(k, _)| k == "ws")' crates/par-mux/src/mux/attach/status.rs` prints `0` or `1`, never `2`.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh053-ci.log 2>&1; echo EXIT=$?` prints `EXIT=0`.

Acceptance criteria:
- `apply_snapshot` and `refresh_legacy` both end in a single `StatusState::commit`, with no direct field assignments left in either body (`grep -n "self\.\(workspaces\|windows\|agents\|sessions\) =" status.rs` shows only `commit` and the `patch_*` methods).
- New `legacy_and_snapshot_commit_identical_state` passes, and the existing `status::tests`, the passthrough/render snapshot-vs-legacy tests and `mux_agents::client_snapshot_matches_the_individual_queries` stay green (nonzero counts, same or higher than the baseline).
- parsight no longer reports the `refresh_legacy`/`apply_snapshot` near-duplicate at ≥ 0.90.
- `make checkall-ci` exits 0.

## Rollback

The change is contained in one file and has no wire impact. Revert `status.rs`. The only behaviour change is the legacy path's end-of-refresh commit on a mid-round `TimedOut`, and the next tick retries in either shape.
