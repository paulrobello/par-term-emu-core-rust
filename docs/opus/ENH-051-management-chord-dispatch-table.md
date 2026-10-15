# ENH-051: `management_chord` becomes a dispatch match over named handlers

> Filed from the 2026-10-12 /opus-audit enhancement pass (AUDIT.md "Detailed Findings → Code Quality": "`management_chord` (2,673; ENH-051)"). Board card: `[ENH-051]`.
> Sequencing. `crates/par-mux/src/mux/attach/render/navigate.rs` is touched by four audit items in the order ARC-140 → ARC-139 → ARC-141 → ARC-137 (AUDIT.md "3b"; the file-conflict table marks it "Same 3b agent, ordered"). Land this card **after all four**:
> - ARC-137 rewrites the `send_checked` sites inside these arms into a bounded `send_interactive`.
> - ARC-139 may lift `switch_workspace` landing into `attach/targets.rs`.
>
> Land it **after ENH-050** as well. ENH-050 replaces the `use super::*` glob and the `super::super::` paths in this file, and doing that first keeps the two diffs from colliding. Land it **before ENH-049**, which turns these handlers into queued continuations. Named handlers give that card one function per chord to convert.
> Line-number citations below are pinned to commit `f4161f6` (the commit that added this plan); re-locate them by symbol on later trees.

**Priority**: medium · **Estimate**: S

## Goal

`WindowSession::management_chord` (`render/navigate.rs:12-196`, ~185 lines) is the repo's #3 hotspot. parsight `hotspots` reports score **2,673**, cyclomatic complexity **27** and churn **99** over the last 14 days. It is one `match key` over the 16 `ManagementKey` variants. Each arm inlines a daemon command, its reply handling and its landing, and five arms are inline UI toggles.

Every new chord edits this function, which is why its churn is so high. Turn it into a thin dispatcher: an **exhaustive `match`** that maps each `ManagementKey` to one named `WindowSession` method. Each method keeps its arm's body unchanged.

The dispatcher must stay a `match`, not an array or a `HashMap<ManagementKey, fn>`. An exhaustive match is what makes adding a `ManagementKey` variant a compile error until it is handled, and the passthrough session relies on the same property (`attach/mod.rs:580-600`).

The byte → `ManagementKey` half of the table already exists: `WindowSession::management_key` (`render/input_route.rs:260-283`) and `Session::management_command` (`attach/actions.rs:159-166`). This card handles the key → handler half.

## Current state (HEAD `aaf35ef`; re-locate by symbol)

- **Call site.** `render/input_route.rs:116`: `PrefixChord::Management(key) => self.management_chord(key, conn)`. It is the only caller (`grep -rn "management_chord" crates/par-mux/src` shows the definition, this call, and a doc mention at `attach/mod.rs:700`).
- **`ManagementKey`** (`attach/mod.rs:702-741`) has 16 variants: `SplitRight`, `SplitDown`, `KillPane`, `NewWindow`, `SwapPrev`, `SwapNext`, `WorkspaceNext`, `WorkspacePrev`, `Zoom`, `RenameWindow`, `RenamePane`, `BorderCycle`, `Labels`, `WorkspacePicker`, `Sidebar` and `StatusBar`.
- **Arms of `management_chord`**, as lines in `navigate.rs`:
  - **Split** `:19-47`. `split-window -t {focused}[ -h]`. On `reply.ok` it parses the new `%N` from `reply.body[0]`, calls `reseed_window`, then `renderer.focus(n)` plus `select-pane -t %{n}`. The focus must come **after** the reseed, and `split_chord_reseeds_and_focuses_the_new_pane` pins that.
  - **KillPane** `:48-80`. `kill-pane`, then `list-panes -t {window}` and `marked_pane`. With a survivor it reseeds, focuses and selects. With none it sets `status_dirty`.
  - **Swap** `:81-106`. Takes the layout-order neighbour with wrap, then `swap-pane -s {focused} -t %{next}`. It is a no-op for fewer than two panes.
  - **NewWindow** `:107-120`. `new-window -t {session}`, then `land_window`.
  - **Workspace next and prev** `:121-129`. Calls `switch_workspace(conn, ±1)`, which already exists as a method (`:204`).
  - **Zoom** `:130-146`. `resize-pane -Z`. On ok it toggles `zoomed` and flashes.
  - **Rename** `:147-153`. Calls `enter_prompt(PromptTarget::Pane | Window(self.window))`.
  - **StatusBar** `:154-164`, **BorderCycle** `:165-177` and **Labels** `:184-193`. Toggles, each followed by a flash and `draw_status_row`.
  - **WorkspacePicker** `:178-180` and **Sidebar** `:181-183`. One-line delegations to `enter_ws_picker` and `toggle_sidebar`.
- **Path noise.** 22 of `navigate.rs`'s lines carry `super::super::` (`grep -c 'super::super::' render/navigate.rs` → 22), mostly `super::super::ManagementKey::…`. ENH-050 removes them. If ENH-050 has not landed, add `use crate::mux::attach::ManagementKey;` at the top of `navigate.rs` and spell the arms `ManagementKey::X`.
- **Prior art for the shape.** The passthrough `Session` already dispatches the same enum to named methods (`split_pane`, `kill_focused_pane`, `new_window_in_session`, `swap_pane`, `switch_workspace`) at `attach/mod.rs:580-600`. ARC-139 records that the two sides have drifted. This card does not unify them, because ARC-139 owns that work. It only gives the render side the same shape.
- **Tests that pin each arm** (`render/tests/chords.rs`, 49 `#[test]`):
  - `split_chord_reseeds_and_focuses_the_new_pane` (`:39`) and `split_down_chord_refused_does_nothing_more` (`:77`);
  - `kill_chord_lands_on_the_survivor_or_marks_the_view_stale` (`:97`) and `swap_chords_pick_the_layout_neighbor_and_wrap` (`:142`);
  - `new_window_chord_creates_selects_and_reseeds` (`:173`) and `zoom_chord_toggles_the_cue_only_on_success` (`:206`);
  - `border_cycle_reaches_herdr_boxes_that_survive_a_reseed` (`:244`) and `border_toggle_redeclares_chrome_on_the_next_size_report` (`:287`);
  - `toggle_chords_flip_their_state_and_park_refits` (`:357`) and `rename_chords_open_seeded_prompts` (`:409`);
  - `workspace_chords_select_the_neighbor_and_land_on_it` (`:443`), `workspace_landing_takes_the_resumed_session_not_the_first_listed` (`:511`) and `landing_on_an_empty_workspace_only_marks_the_status_stale` (`:547`);
  - `workspace_picker_opens_on_the_active_row_and_lands_on_enter` (`:567`) and `prefix_chord_classifies_bytes_in_precedence_order` (`:1940`).

  The integration tests are `tests/mux_attach.rs` `render_mode_zoom_rename_border_chords` (`:734`), `render_mode_split_chord_reseeds_the_window_with_the_new_pane` (`:2658`), `render_mode_workspace_chord_selects_and_lands` (`:2967`), `render_mode_sidebar_toggle_minimal` (`:3200`), `render_mode_status_bar_toggle_hides_and_restores_the_row` (`:3705`) and `render_mode_border_style_survives_split` (`:4128`).

## Implementation

1. **Re-read `navigate.rs` whole.** ARC-137, ARC-139, ARC-141 and ENH-050 will have changed the arm bodies, their send helper and the imports. Carry the bodies as they are at that point; never paste them from this plan.
2. **Extract one method per arm** on `impl WindowSession` in `navigate.rs`, each `pub(super)` unless it is used only by the dispatcher, in which case make it private (`fn`). Move each arm's body in verbatim, changing only `return;` into the method's own `return;`.

   | Variant(s) | Method |
   |---|---|
   | `SplitRight`, `SplitDown` | `fn chord_split(&mut self, conn, horizontal: bool)` |
   | `KillPane` | `fn chord_kill_pane(&mut self, conn)` |
   | `SwapPrev`, `SwapNext` | `fn chord_swap(&mut self, conn, dir: i32)` |
   | `NewWindow` | `fn chord_new_window(&mut self, conn)` |
   | `Zoom` | `fn chord_zoom(&mut self, conn)` |
   | `StatusBar` | `fn toggle_status_bar(&mut self)`. It needs no `conn`, because the refit is parked through `pending.grid_refit`. |
   | `BorderCycle` | `fn cycle_border_style(&mut self)` |
   | `Labels` | `fn toggle_border_labels(&mut self)` |

   `focused` is computed at the top of today's function. Compute it inside each method that uses it (split, kill, zoom; swap already recomputes). This drops the unused computation for the toggle arms.
3. **Rewrite `management_chord`** as the dispatcher. Keep its doc comment, shortened to the landing contract:
   ```rust
   pub(super) fn management_chord(&mut self, key: ManagementKey, conn: &mut AttachConn) {
       match key {
           ManagementKey::SplitRight => self.chord_split(conn, true),
           ManagementKey::SplitDown => self.chord_split(conn, false),
           ManagementKey::KillPane => self.chord_kill_pane(conn),
           ManagementKey::SwapPrev => self.chord_swap(conn, -1),
           ManagementKey::SwapNext => self.chord_swap(conn, 1),
           ManagementKey::NewWindow => self.chord_new_window(conn),
           ManagementKey::WorkspaceNext => self.switch_workspace(conn, 1),
           ManagementKey::WorkspacePrev => self.switch_workspace(conn, -1),
           ManagementKey::Zoom => self.chord_zoom(conn),
           ManagementKey::RenameWindow => self.enter_prompt(PromptTarget::Window(self.window.clone())),
           ManagementKey::RenamePane => self.enter_prompt(PromptTarget::Pane),
           ManagementKey::StatusBar => self.toggle_status_bar(),
           ManagementKey::BorderCycle => self.cycle_border_style(),
           ManagementKey::Labels => self.toggle_border_labels(),
           ManagementKey::WorkspacePicker => self.enter_ws_picker(conn),
           ManagementKey::Sidebar => self.toggle_sidebar(conn),
       }
   }
   ```
   There is no `_ =>` arm. Exhaustiveness is the point.
4. **Comments.** Each extracted method carries the arm's existing explanatory comment as its doc comment (`///`). Do not add new narration (house rule: comments state constraints).
5. **No new tests are required.** Every arm is already pinned (list above). If any arm turns out to have no unit test at execution time, re-check `chords.rs`. `Labels` is covered by `toggle_chords_flip_their_state_and_park_refits`. Add a minimal `recording_conn` test in that file's style for any uncovered arm before refactoring it.

## Files to touch

- `crates/par-mux/src/mux/attach/render/navigate.rs` (only this file)

## Verify

Attach and render tests need `--features mux-bin,attach`, because `mux-bin` alone compiles **0** of them.

- **Baseline before editing.** `cargo test -p par-mux --features mux-bin,attach --lib mux::attach::render > /tmp/enh051-base.log 2>&1; echo EXIT=$?` gives `EXIT=0`. Record N from the `test result: ok. N passed` line. At `aaf35ef` there are 157 `#[test]` under `render/tests/`, so N ≥ 157. Afterwards the same command prints `EXIT=0`, `N passed` (same N) and `0 failed`.
- `cargo test -p par-mux --features mux-bin,attach --lib render::tests::chords > /tmp/enh051-ch.log 2>&1; echo EXIT=$?` prints `EXIT=0`, and `grep -c ' ok$' /tmp/enh051-ch.log` is ≥ 49.
- `cargo test -p par-mux --features mux-bin,attach --test mux_attach render_mode_ -- --test-threads=1 > /tmp/enh051-int.log 2>&1; echo EXIT=$?` prints `EXIT=0` with a nonzero `passed` count.
- **Complexity.** After reindexing, parsight `audit_repository` `cyclomatic_complexity` with `target: "management_chord"` and `file_path: "render/navigate.rs"` gives complexity ≤ 17. That bound is one per arm with no nesting. Record each extracted `chord_*` method's complexity in the commit message. None exceeds 12; `chord_kill_pane` is the largest at about 8 before ARC-137's additions. `sed -n '/fn management_chord/,/^    }$/p' crates/par-mux/src/mux/attach/render/navigate.rs | wc -l` prints ≤ 30, and the dispatcher has no `_ =>` arm.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh051-ci.log 2>&1; echo EXIT=$?` prints `EXIT=0`.

Acceptance criteria:
- `management_chord` is an exhaustive, wildcard-free `match` of at most 30 lines, and parsight reports its complexity at 17 or less (one per arm), with no extracted handler above 12.
- The full render lib suite passes with the same nonzero count as the pre-change baseline (`cargo test -p par-mux --features mux-bin,attach --lib mux::attach::render`), including all 49 `render::tests::chords` tests.
- The `mux_attach` `render_mode_` integration tests pass with a nonzero count.
- `make checkall-ci` exits 0.

## Rollback

This is a pure intra-file move with no wire or behaviour change. Revert `navigate.rs`.
