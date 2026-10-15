# ENH-052: Split `render/renderer.rs` into paint, chrome, sink, and cursor modules

> Filed from the 2026-10-12 /opus-audit enhancement pass (AUDIT.md "Detailed Findings → Code Quality": "`renderer.rs` total complexity 356 (ENH-052)"). Board card: `[ENH-052]`.
> Sequencing:
> - Land this **after SEC-212**, which adds the `chrome_safe` sanitiser at the border-label paint (`renderer.rs:~1003` `border_label`) and the label painter.
> - Land it **after QA-243**, which collapses the seven `cursor_trace()` blocks in `focused_cursor` (`:765-832`) and `StdoutSink::place_cursor` (`:209-214`) into one helper. Both edit exactly the regions this card moves. Moving the code first would make each of those cards a cross-file conflict.
> - Land it **after ENH-050**, whose phase 1 replaces `renderer.rs:4`'s `use super::*` and `mod.rs:46`'s `use renderer::*` with explicit lists. This card then extends those lists rather than fighting globs.
> - Land it **before ENH-049**. ENH-049 does not touch `renderer.rs`, but it edits `render/mod.rs` and needs the render tree in its final shape.
> Line-number citations below are pinned to commit `f4161f6` (the commit that added this plan); re-locate them by symbol on later trees.

**Priority**: low · **Estimate**: M

## Goal

`crates/par-mux/src/mux/attach/render/renderer.rs` is 1,735 lines with **0 test lines**; its tests live in `render/tests/mod.rs`. parsight ranks it third among god objects, behind the core `terminal/mod.rs` and the generated stub: 77 functions, total complexity **356**, max 29, risk **Critical**, fan_in 13. It holds six unrelated responsibilities:
- the stdout sink and SGR/cursor writers;
- layout/divider geometry helpers;
- `RenderOptions`;
- `PaneRenderer`'s state API;
- pane-grid painting;
- divider/border/overlay chrome painting.

Split it along the seams parsight's `decomposition` section proposes (modularity **0.77**, confidence high) into four sibling files. `renderer.rs` keeps `RenderOptions`, `PaneRenderer`'s state API and the frame orchestrator `render_frame`. The move is mechanical and does not change behaviour.

**Deviation from the brief, on purpose.** The brief proposes a "sidebar-paint" module. Sidebar painting is **already** out of `renderer.rs`. `render/sidebar.rs` holds `impl PaneRenderer { sidebar_width, set_sidebar_width, set_geometry, set_sidebar_sections, sidebar_row_at, paint_sidebar }` (`sidebar.rs:6-128`), and `render_frame` only calls `self.paint_sidebar()` (`renderer.rs:1193`). So there is no sidebar module to create. The analogous seam still in `renderer.rs` is the chrome painters (dividers, pane boxes, labels, overlay), and that is what this card extracts.

## Current state (HEAD `aaf35ef`; re-locate by symbol — QA-243/SEC-212 will shift lines)

Regions of `renderer.rs`:

| Lines | Contents | parsight cluster |
|---|---|---|
| `:6-59` | `active_border_style`, `plain_border_style`, `divider_style` | structural 0 (with `display_rect`, `border_label`, `paint_pane_borders`, `label_cell_at`) |
| `:61-125` | `rows_overlap`, `PaneDir`, `pane_in_direction`, `cols_overlap`, `row_overlap`, `col_overlap` | structural 1 + 2 |
| `:127-183` | `map_color`, `map_flags` | structural 3 (paint) |
| `:185-334` | `StdoutSink` (`FlushSink` impl: `flush`, `repaint_all`, `place_cursor` with a trace block at `:209-214`), `push_cursor_shape`, `write_styled`, `write_color` | structural 4 + 5 |
| `:343-390` | `RenderOptions` + `Default` | co-change 0 |
| `:392-1148` | `impl PaneRenderer`, the state API: constructors, setters, layout, focus, overlay geometry, `divider_near`, `focused_cursor` (`:764-843`, 7 trace blocks), `display_rect`/`chrome`/`content_view`, emulator sizing, titles, `feed_output`, scroll API | co-change 1 + 2 |
| `:1149-1211` | `render_frame`, the orchestrator | — |
| `:1213-1338` | `paint_pane` (hotspot 1,425) | structural 3 |
| `:1339-1434` | `paint_dividers`, `paint_boundary_group`, `gutter_owns` | **agreement jaccard 0.857** with `Axis`, `cell`, `group_boundaries` |
| `:1435-1555` | `paint_overlay` | structural 3 |
| `:1556-1659` | `paint_pane_borders`, `label_cell_at` | structural 0 |
| `:1660-1735` | `Axis`, `BoundaryCell`, `BoundaryGroup`, `collect_boundaries`, `group_boundaries` | co-change 3 |

- **Struct and field privacy.** `PaneRenderer` is defined in `render/mod.rs:297-368` with private fields (`emulators`, `layout`, `buffer`, …). Sibling modules of `render` (`sidebar.rs` today) already write `impl PaneRenderer` blocks that read those fields directly, because child modules can see their ancestor's private items. Every new file therefore uses the same pattern: an `impl PaneRenderer` block in a sibling module of `render`, with no field-visibility change.
- **Imports.** `render/mod.rs:46` has `use renderer::*;`, which re-exports the free fns to siblings and tests. Pick **re-export through `renderer.rs`**: `renderer.rs` adds `pub(super) use super::{paint::*, chrome::*, sink::*};`. That way `mod.rs`'s import (glob, or ENH-050's explicit list) keeps resolving and the `render/tests/` globs keep compiling unchanged. The alternative, editing every importer, touches more files for no gain.
- **Tests that pin this code**, all in `render/tests/mod.rs` (44 `#[test]`):
  - `two_pane_split_renders_at_daemon_rects`, `styled_cells_map_fg_bg_and_attrs` and `ascii_glyphs_fallback`;
  - `horizontal_dividers_paint_on_stacked_splits`, `focus_highlights_adjacent_dividers` and `border_label_strips_control_characters`;
  - `wide_chars_mark_their_spacer`, `combining_marks_join_the_symbol` and `scroll_viewport_paints_history_rows`;
  - `scrolled_gutter_stays_outside_the_content_with_the_side_panel_up` and `divider_yields_the_gutter_column_with_the_side_panel_up`;
  - `focused_cursor_maps_cell_plus_origin_and_hides_when_scrolled` and `content_view_area_equals_the_pty_grid_in_every_combination`;
  - plus the integration `tests/mux_attach.rs` `render_mode_layout_and_pane_replay_match_daemon_ground_truth` (`:1424`).

## Implementation

1. **Re-read `renderer.rs` whole** after QA-243 and SEC-212 land. Confirm that `focused_cursor` now uses the trace helper and that `border_label`/the label painter call `chrome_safe`. If either card has not landed, **stop** and leave the card blocked. Moving that code first is exactly the conflict the sequencing note forbids.
2. **Create `render/sink.rs`.** Move `StdoutSink` and its `FlushSink` impl, `push_cursor_shape`, `write_styled` and `write_color`. Module doc: `//! The stdout flush sink: per-cell CUP + SGR writes and the host cursor's DECSCUSR placement.` Move QA-243's trace helper here if it lives at sink level, or leave it in `render/mod.rs` next to `cursor_trace()`. Do not duplicate it.
3. **Create `render/paint.rs`.** Move `map_color`, `map_flags`, and an `impl PaneRenderer` block holding `paint_pane` and `paint_overlay`. Module doc: `//! Cell painting: a pane's live/scrollback grid into its content view, and the modal overlay.`
4. **Create `render/chrome.rs`.** Move:
   - the three style fns (`active_border_style`, `plain_border_style`, `divider_style`);
   - `Axis`, `BoundaryCell`, `BoundaryGroup`, `collect_boundaries`, `group_boundaries`, and the overlap helpers (`rows_overlap`, `cols_overlap`, `row_overlap`, `col_overlap`);
   - an `impl PaneRenderer` block with `paint_dividers`, `paint_boundary_group`, `gutter_owns`, `paint_pane_borders` and `label_cell_at`.

   Module doc: `//! Pane chrome: shared dividers (tmux half rule), per-pane border boxes, and their embedded labels.`
5. **Placement of `divider_near`, `PaneDir` and `pane_in_direction`.** These are *hit-test and navigation* geometry over rects, not painting. Move `PaneDir` and `pane_in_direction` to `chrome.rs` beside the overlap helpers they call. Keep `divider_near` in `renderer.rs`'s state API, because it reads `self.layout` for the input router. `border_label` and `display_rect` also stay in `renderer.rs`: they are state queries the painters call.
6. **Declare the modules** in `render/mod.rs` next to `mod renderer;` (`:42`): `mod chrome; mod paint; mod sink;`. Add the re-export line from "Imports" above to `renderer.rs`. Visibility stays as it is today: `pub(super)` items stay `pub(super)`, which now means visible to `render`.
7. **`renderer.rs` after the move** holds the module doc (rewritten to "the pane renderer's state: layout, focus, emulators, titles, scroll, display options, and the frame orchestrator"), `RenderOptions`, the state-API `impl PaneRenderer`, and `render_frame`. Target: ≤ 950 lines. The moved regions total about 850 of the current 1,735.
8. **No logic edits.** Use `git diff --stat` and `git diff -M --color-moved=dimmed-zebra` to confirm the moved blocks are byte-identical apart from `use` lines and the doc comments in step 2–4.

## Files to touch

- `crates/par-mux/src/mux/attach/render/renderer.rs` (shrinks)
- `crates/par-mux/src/mux/attach/render/{sink,paint,chrome}.rs` (new)
- `crates/par-mux/src/mux/attach/render/mod.rs` (three `mod` lines; import list if ENH-050 made it explicit)

## Verify

Attach and render tests need `--features mux-bin,attach`, because `mux-bin` alone compiles **0** of them.

- **Baseline before moving.** `cargo test -p par-mux --features mux-bin,attach --lib mux::attach::render > /tmp/enh052-base.log 2>&1; echo EXIT=$?` gives `EXIT=0`. Record N passed (≥ 157 at `aaf35ef`). After the move the same command prints `EXIT=0`, the same N passed and `0 failed`.
- `wc -l crates/par-mux/src/mux/attach/render/renderer.rs` prints ≤ 950. `ls crates/par-mux/src/mux/attach/render/{sink,paint,chrome}.rs` lists all three. `grep -c "fn paint_pane\b\|fn paint_dividers\|fn paint_pane_borders\|fn paint_overlay\|impl FlushSink for StdoutSink" crates/par-mux/src/mux/attach/render/renderer.rs` prints `0`.
- **Pure move.** `git diff -M HEAD~1 --stat` shows insertions ≈ deletions, within 60 lines for module docs, `use` lists and `mod` lines.
- After reindexing, parsight `audit_repository` `god_objects` no longer lists `render/renderer.rs` with total complexity ≥ 200, and no new render file exceeds 200.
- `cargo test -p par-mux --features mux-bin,attach --test mux_attach render_mode_layout_and_pane_replay_match_daemon_ground_truth -- --test-threads=1 > /tmp/enh052-int.log 2>&1; echo EXIT=$?` prints `EXIT=0` with `1 passed`.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh052-ci.log 2>&1; echo EXIT=$?` prints `EXIT=0`.

Acceptance criteria:
- `render/sink.rs`, `render/paint.rs` and `render/chrome.rs` exist. `renderer.rs` is ≤ 950 lines and no longer defines `paint_pane`, `paint_dividers`, `paint_pane_borders`, `paint_overlay` or `StdoutSink`'s `FlushSink` impl.
- parsight `god_objects` reports no render file with total complexity ≥ 200 after reindexing.
- The render lib suite passes with the same nonzero count as the baseline (≥ 157, 0 failed), and `make checkall-ci` exits 0.

## Rollback

This is a pure file move with no behaviour change. Revert the commit; `git revert` restores the single file. Because `renderer.rs` re-exports the moved items, no importer outside `render/mod.rs` changed, so a partial revert (one module back) is also safe.
