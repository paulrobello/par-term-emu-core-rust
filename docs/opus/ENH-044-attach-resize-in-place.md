# ENH-044: Resize the attach render client in place instead of rebuilding every pane emulator

> Filed from the 2026-10-08 /opus-audit enhancement pass (cycle `audit-2026-10-08`). Board card: `[ENH-044]`.
> Sequencing:
> - **Hard prerequisite: ARC-122** (`RenderOptions` owned by `WindowSession` and passed to `PaneRenderer::new(cols, rows, &RenderOptions)`). At HEAD `2cf0957` `RenderOptions` does not exist. This plan assumes it does and names the pre-ARC-122 fallback only where it matters.
> - Before ARC-127 if possible. ARC-127 regroups `WindowSession` fields, and this card changes three of its methods, so whichever lands second rebases.
> - `render/mod.rs` and `render/session.rs` are audit hot files (SEC-208, ARC-122…ARC-132). Read them before editing.

**Priority**: medium · **Estimate**: M

## Goal

Today every host resize, sidebar-toggle refit and window reseed:
- throws the whole `PaneRenderer` away (`PaneRenderer::new`), which drops every pane emulator, the user-title store, the sidebar sections and any open overlay;
- re-applies display options by hand, which caused the ARC-122 regression;
- sends one blocking `refresh-client -t %N` replay per pane.

On a 6-pane window, a terminal drag-resize therefore costs 6 or more replay round trips per resize step, each a full screen-restore byte stream, all on the UI thread.

Change this so that:
1. `PaneRenderer` gets `resize(cols, rows)`. It keeps emulators, options, titles, focus, sidebar sections and overlay, and rebuilds only the frame buffers.
2. `resize_to` and `reseed_window` resize in place.
3. Only panes that are **new to the renderer** are replayed. Surviving panes are re-fit by `apply_layout`, which already resizes existing emulators. The rest of their content is reconciled by the `%output` stream, with the exception in Implementation step 4.

## Current state

All anchors are at HEAD `2cf0957`.

- **The rebuild sites:**
  - `WindowSession::resize_to` (`crates/par-mux/src/mux/attach/render/session.rs:473-555`):
    - `PaneRenderer::new(cols, rows, self.border_glyphs)` at `:487`;
    - hand re-applies `bg`, `pane_borders`, `show_label_in_border`, `pane_gaps`, `scrollbar_gutter` and `sidebar_width` at `:490-500`, **but not the border colours** (ARC-122);
    - sends the size report at `:501-503`;
    - drains pending events and keeps only the window's `%layout-change` (`:506-519`), **discarding every other drained event, including `%output`**;
    - `apply_layout` at `:524`;
    - sends a `refresh-client -t %N` replay for **every** layout pane at `:525-537`;
    - repaints everything at `:539-553`.
  - `WindowSession::reseed_window` (`render/navigate.rs:525-621`):
    - `PaneRenderer::new` at `:540` with the same hand-maintained re-apply list (`:545-559`);
    - size report, then a drain for the `%layout-change` (`:563-591`);
    - `self.replay_all_panes(conn)` at `:592`.
  - The pump's parked-layout step (`render/mod.rs:836-840`), reached from another client's split, resize or zoom through `handle_event`'s `LayoutChange` arm (`:983-1013`), runs `apply_layout` and then `replay_all_panes`. It does not rebuild the renderer, but it replays every pane.
  - Callers of `resize_to`: the host-resize step (`render/mod.rs:895`) and the parked sidebar refit (`:871-881`, which re-queries the sidebar because "the refit reconstructed the renderer, dropping the strip's sections").
- **`apply_layout` already preserves surviving emulators** (`render/renderer.rs:434-458`):
  - it `retain`s emulators whose pane is still in the layout;
  - it creates new ones with `or_insert_with`;
  - it calls `emulator.resize(cols, rows)` only when the size differs.

  So the emulator-preservation machinery exists. What defeats it is the `PaneRenderer::new` that precedes it.
- **`PaneEmulator` already has `resize`** (`render/mod.rs:258-261`): `self.term.resize(cols, rows)`. That is `Terminal::resize` (`crates/par-term-emu-core/src/terminal/mod.rs:1553`), the same reflowing resize the daemon's pane runs (`crates/par-mux/src/mux/pane.rs:566` through the PTY session). No new emulator method is needed.
- **The renderer's state that `new` resets** (`render/renderer.rs:338-364`):
  - pane state: `emulators`, `layout`, `focused`, `user_titles`;
  - frame size and sidebar: `width`, `height`, `sidebar_w`, `sidebar_sections`;
  - options: `glyphs`, `bg`, `pane_borders`, `show_label_in_border`, `pane_gaps`, `scrollbar_gutter`, `reserved_chrome`, `border_active`, `border_plain`;
  - transient UI: `drag_divider`, `overlay`;
  - frame buffers: `buffer`, `prev_buffer` (`Buffer::empty(area)`), and `dirty`.

  Only `width`, `height`, `buffer`, `prev_buffer` and `dirty` depend on the host size.
- **Ordering on the daemon.** The PTY reader applies bytes to the pane terminal and **then** fires the output callback that pushes `%output` (`crates/par-term-emu-core/src/pty_session/reader.rs:279-291`). The callback runs after the terminal write lock is released. A dispatch resize can therefore run between "bytes applied at the old size" and "`%output` pushed", so the client may receive those bytes **after** the `%layout-change`. Today's unconditional replay papers over this. In-place resize needs the rule in Implementation step 4.
- **Tests to extend:**
  - `render/tests/chords.rs:290-341` drives `resize_to` against `two_pane_session` (`render/tests/mod.rs:1399-1416`) with a `FakeScript` whose `notify` map pushes the `%layout-change` before a size report's reply.
  - `render/tests/chords.rs:263` drives `reseed_window(&mut conn, "@0")`.
  - `render/tests/session.rs:1074` (`pane_border_ring_and_label_honor_border_colors`) checks ring colours but never across a rebuild.
  - `wait_recorded`, `drained` and `RecordingSink` (`render/tests/mod.rs:1270`) are the helpers.

## Implementation

1. **`PaneRenderer::resize(&mut self, cols: u16, rows: u16)`** (`render/renderer.rs`, next to `frame_size`):
   - If `(cols, rows) == (self.width, self.height)`, return without changes.
   - Otherwise:
     - set `width` and `height`;
     - set `buffer` and `prev_buffer` to `Buffer::empty(RtRect::new(0, 0, cols, rows))`;
     - set `drag_divider = None`, because a divider hit is geometry from the old frame;
     - set `dirty = true`.
   - Keep `emulators`, `layout`, `focused`, `user_titles`, `sidebar_w`, `sidebar_sections`, `overlay` and every option field. The overlay is modal state that `WindowSession` still believes is open, and `new` dropping it was a latent bug.
   - Do **not** re-fit emulators here. Their size follows the daemon's layout, which `apply_layout` installs next.
   - Doc comment: the frame changes size, the panes keep their state, and the caller installs the daemon's new layout with `apply_layout`.
2. **`resize_to`** (`render/session.rs:473`):
   - Replace `:487-500` (the `PaneRenderer::new` and the re-apply list) with:
     ```rust
     self.renderer.resize(cols, rows);
     self.renderer.set_sidebar_width(if self.sidebar_on { self.sidebar_width } else { 0 });
     ```
     The sidebar width stays because `pending_grid_refit` parks the sidebar toggle's refit through here. Under ARC-122 the width may live in `RenderOptions`, so keep whichever setter ARC-122 left.
   - Replace the drain-and-discard (`:506-519`) with `let (layout_event, after) = self.drain_around_layout(conn);` (step 4).
   - Replace the per-pane replay loop (`:525-537`) with `self.replay_panes(conn, &replay)`, where `replay` comes from step 4.
3. **`reseed_window`** (`render/navigate.rs:525`):
   - Replace `:539-559` with `self.renderer.resize(host_cols, host_rows)`. The frame size is unchanged on a reseed, so this is a no-op kept for symmetry. Keep `set_sidebar_width` and `refresh_sidebar` as they are.
   - Use `drain_around_layout` in place of `:572-591`.
   - Replace `self.replay_all_panes(conn)` (`:592`) with `self.replay_panes(conn, &replay)`.
   - A reseed to a **different** window has no surviving panes. `apply_layout` retains none, so `replay` is every pane and behaviour is unchanged. A reseed of the **same** window replays only panes it did not have.
4. **Replay selection and the output-race rule** (`render/session.rs`, new helpers):
   - `fn drain_around_layout(&mut self, conn) -> (Option<(String, String, String)>, Vec<u32>)`. Drain `conn.drain_pending_events()` **in order** and do not discard anything:
     - **Before** the shown window's `LayoutChange`: feed every `Output`/`ExtendedOutput` through `self.feed_pane(…)`. Those bytes were produced at the old size, and the emulator is still at the old size. Pass every other event to `self.handle_event(event)`, keeping its `EventOutcome`, so `%agent-*`, `%window-*` and similar events still mark state. An `EventOutcome::End` is stored in a field (`self.pending_end = true`) that the pump checks right after `resize_to` returns.
     - The `LayoutChange` itself is captured and returned. Its zoom flag sets `self.zoomed` as today.
     - **After** the `LayoutChange`: an `Output` for a pane that survives the layout is **not** fed. Its pane id goes into the returned "race" list, because those bytes may have been applied at the old size on the daemon (see Current state, ordering). Every other event goes to `handle_event`.
   - Then the caller runs:
     ```rust
     let before: HashSet<u32> = self.renderer.layout().iter().map(|r| r.pane).collect();
     self.renderer.apply_layout(layout);
     let replay: Vec<u32> = self.renderer.layout().iter().map(|r| r.pane)
         .filter(|p| !before.contains(p) || race.contains(p)).collect();
     ```
   - `fn replay_panes(&mut self, conn, panes: &[u32])` is today's `replay_all_panes` body restricted to `panes`. `replay_all_panes` becomes `let all = …layout ids…; self.replay_panes(conn, &all)` and keeps its seed caller (`render/session.rs:291`).
   - With no layout event (the daemon refused the report, or the geometry did not change), `replay` is empty and `apply_layout` is not called. That matches today's "no event, no replay" path.
5. **Parked-layout step** (`render/mod.rs:836-840`): apply the same rule. Compute `before`, call `apply_layout(layout)`, then `self.replay_panes(conn, &new_ids)`. This path comes from a notification `handle_event` already consumed in order, so it has no race list: the `%output` lines that follow the `%layout-change` in the stream are fed normally by the next drain. Keep `self.daemon_layout = layout.clone()`.
6. **Sidebar re-query after refit** (`render/mod.rs:876-880`). With the sections preserved, the comment "the refit reconstructed the renderer, dropping the strip's sections" is false. Update the comment to say that the re-query refreshes the active mark. Leave the call itself, since ENH-043 owns sidebar query cost.
7. **Tests** (`render/tests/chords.rs` or `render/tests/session.rs`, using `two_pane_session` plus a `FakeScript` whose `notify` maps each size report to a `%layout-change @0 …` line for the new geometry):
   - `resize_in_place_replays_no_unchanged_panes`:
     - Feed panes 1 and 2 distinct text.
     - `resize_to(&mut conn, 100, 23, &mut sink)` with the notify layout keeping panes `%1` and `%2` at new widths.
     - Assert the recorded lines after `drained(&rx)` contain exactly one `refresh-client -t %1 -C …` size report and **zero** `refresh-client -t %N` lines without `-C`.
     - Assert pane 1's emulator still holds its text, now at its new size.
   - `resize_in_place_replays_a_pane_new_to_the_layout`: the notify layout adds pane `%3`. Assert exactly one replay line, `refresh-client -t %3`, and that pane 3's emulator holds the scripted replay body.
   - `resize_keeps_border_colors`: `set_border_colors(Some(Rgb(1,2,3)), Some(Rgb(4,5,6)))`, enable pane borders, then `resize_to`. Render a frame and assert the focused ring cell's `fg` is `Rgb(1,2,3)` and the unfocused one's is `Rgb(4,5,6)`, at coordinates computed from the new layout as `pane_border_ring_and_label_honor_border_colors` does. Repeat after `reseed_window(&mut conn, "@0")`.
   - `resize_output_after_layout_change_forces_replay`: script the size report's `notify` as two lines, `%layout-change …` then `%output %1 late`. Assert one replay line, `refresh-client -t %1`, and that the `late` bytes were not fed directly (pane 1 shows the replay body, not `late`).
   - `resize_output_before_layout_change_is_fed_at_old_size`: `notify` is `%output %1 early` then `%layout-change …`. Assert zero replays and that pane 1's grid contains `early`.
   - `in_place_resize_matches_fresh_replay`. This is the reflow-equivalence check and a pure `PaneEmulator` test in `render/tests/mod.rs`:
     - Build emulator A at 40x10 and feed it a fixed byte stream: 30 numbered lines, some longer than 40 columns, with SGR colours.
     - Resize A to 60x8.
     - Build a core `par_term_emu_core::terminal::Terminal` at 40x10, feed it the same bytes, resize it to 60x8, and take its `export_screen_restore_sequence()` (the bytes the daemon's `refresh-client -t` replay sends, `crates/par-mux/src/mux/dispatch/client.rs:172`).
     - Feed that stream into a fresh emulator B at 60x8.
     - Assert A's and B's visible grids are equal cell for cell (character and colours) and their cursors are equal.
   - `reseed_to_another_window_replays_every_pane`: reseed to `@1`, whose scripted layout has panes `%5` and `%6`. Assert exactly two replay lines, for `%5` and `%6`.
   - `resize_preserves_open_overlay`: open the menu overlay (the existing modal entry in `modal.rs:384`), `resize_to`, and assert `renderer` still has the overlay.
8. **Docs.** No MUX.md wire change. Add one `CHANGELOG.md` `[Unreleased]` Changed bullet: render attach resizes and reseeds in place, replaying only panes new to the window.

## Files to touch

- `crates/par-mux/src/mux/attach/render/renderer.rs` (`PaneRenderer::resize`)
- `crates/par-mux/src/mux/attach/render/session.rs` (`resize_to`, `drain_around_layout`, `replay_panes`, `replay_all_panes`)
- `crates/par-mux/src/mux/attach/render/navigate.rs` (`reseed_window`)
- `crates/par-mux/src/mux/attach/render/mod.rs` (parked-layout step `:836-840`, the `pending_end` check after `resize_to`, the sidebar-refit comment `:876-880`)
- `crates/par-mux/src/mux/attach/render/tests/chords.rs` and/or `tests/session.rs`, `tests/mod.rs`
- `CHANGELOG.md`

## Verify

- `grep -n "PaneRenderer::new" crates/par-mux/src/mux/attach/render/session.rs crates/par-mux/src/mux/attach/render/navigate.rs` returns no match: neither `resize_to` nor `reseed_window` rebuilds the renderer.
- `cargo test -p par-mux --features attach --lib resize_in_place_replays_no_unchanged_panes` passes: a host resize sends one size report and zero `refresh-client -t %N` replays when every pane survives.
- `cargo test -p par-mux --features attach --lib resize_in_place_replays_a_pane_new_to_the_layout` passes: exactly one replay, for the new pane.
- `cargo test -p par-mux --features attach --lib resize_keeps_border_colors` passes: the configured `border-active-color`/`border-color` survive both `resize_to` and `reseed_window`.
- `cargo test -p par-mux --features attach --lib resize_output_after_layout_change_forces_replay` passes: `%output` arriving after the `%layout-change` in the drain triggers that pane's replay instead of a direct feed.
- `cargo test -p par-mux --features attach --lib resize_output_before_layout_change_is_fed_at_old_size` passes: pre-layout output is fed and no replay is sent.
- `cargo test -p par-mux --features attach --lib in_place_resize_matches_fresh_replay` passes: an emulator resized in place equals, cell for cell and cursor included, a fresh emulator fed the post-resize replay.
- `cargo test -p par-mux --features attach --lib reseed_to_another_window_replays_every_pane` passes: replays equal the panes not already in the renderer.
- `cargo test -p par-mux --features attach --lib resize_preserves_open_overlay` passes.
- `cargo test -p par-mux --features attach --lib attach::render` passes, including the existing `chrome-redeclare` resize tests and the herdr reseed test at `chords.rs:263`.
- `make checkall` exits 0.

## Rollback

Revert the commit. The change is client-only with no wire or persistence change, and reverting restores the rebuild-and-replay-everything path. The race rule only adds replays, so a partial revert of steps 4 and 5 (falling back to `replay_all_panes`) is also safe.
