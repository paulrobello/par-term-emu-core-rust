# ENH-038: FFI scroll-aware damage, so an iOS renderer can blit on scroll and redraw only new rows (ABI v4, additive)

> Filed from the 2026-09-29 /opus-audit enhancement pass (cycle `audit-2026-09-29`). Board card: `[ENH-038]`.
> Sequencing:
> - After **ARC-100** (`HostConfig`), then **ARC-092** (one Terminal-owned damage clock). The new per-row content generation stamps from that clock, and the full-redraw sentinel below hooks ARC-092's single `set_visible_screen()`.
> - Independent of **D4**. If this card ships first it takes ABI **4** (additive), and D4's breaking batch (ARC-101/112/114) becomes **5**. Record the renumbering on the D4 cards.
> - Cross-repo follow-up: ParDeck adopts the call. It has no ABI guard today, so nothing breaks on its side.

**Priority**: medium · **Estimate**: L

## Goal

A one-line scroll today dirties every row. ParDeck (and any C/Swift embedder using `terminal_dirty_ranges_since`) therefore re-reads and redraws the whole screen on every line of output, even though all but one row only moved. Expose enough over the FFI for the renderer to blit the surviving rows by the scroll distance and redraw only the rows whose content actually changed. This saves a lot per frame on device, where readback plus glyph rasterization dominates.

## Current state (and why the brief's mechanism does not work)

- **Damage today.**
  - `Grid` keeps `row_gen: Vec<u64>` (position-based: row *i* was last damaged at generation *g*) and a monotonic `gen` (`src/grid/mod.rs:43-51`).
  - `scroll_up` calls `mark_rows_damage(0, rows-1)` (`src/grid/scroll.rs:69`). So do `scroll_down` (`:147`) and the region scrolls (`:164,210`).
  - So after one linefeed at the bottom, `terminal_dirty_ranges_since(gen)` returns every row, and no counter tells the renderer "these rows moved up by one".
- **`total_lines_scrolled` is the wrong counter.** Proposed as the backing value, it fails in four ways:
  1. It is incremented only when rows are pushed into scrollback (`scroll.rs:15,87`).
  2. `push_rows_to_scrollback` and `absorb_rows_into_scrollback` return early when `max_scrollback == 0` (`scroll.rs:8,80`). The alt grid is built that way (`src/terminal/mod.rs:1291`, "Alt screen has no scrollback"), so vim, less and htop never move it.
  3. A region scroll with `top > 0` (a status line under DECSTBM) never pushes to scrollback (`scroll.rs:170-172`), so it never counts either.
  4. `clear_scrollback` resets it to 0 (`src/grid/erase.rs:109`), so the delta can go backwards.

  A delta over it would be silently wrong in exactly the full-screen apps where scrolling matters.
- **The shape of scrolls.** All vertical scrolls are full-width row moves. `scroll_region_up/down(n, top, bottom)` rotate whole rows (`scroll.rs:151-215`). SU/SD (`src/terminal/sequences/csi/scroll.rs:25,40`), IND/NEL/RI (`src/terminal/sequences/esc.rs:47,71,99`) and linefeed (`src/terminal/write.rs:99,187,222,600`) all route there. DECLRMM does not narrow a scroll horizontally (`use_lr_margins` only gates *whether* linefeed scrolls, `write.rs:90-99`). So row-granular "moved" tracking is sound.
- **Existing FFI and ABI.**
  - `terminal_damage_generation` (`src/ffi.rs:562`), `terminal_dirty_ranges_since` (`:580`), `TermRowRange { start, end }` (`:929` pins its size at 8).
  - `TERM_CORE_ABI_VERSION = 3` in `src/ffi.rs:431` and `include/terminal_core_layout.h:25`.
  - The literal `3` is also hard-coded in `ffi::tests::abi_version_matches_header_macro` (`src/ffi.rs:1279,1286`).
  - The FFI_GUIDE rule (`docs/FFI_GUIDE.md:416`): "bumped together on any layout or contract change". There is no additive-minor scheme, so a new function is a version bump.
- **ParDeck** (`~/Repos/pardeck/ParDeck/CoreTerminal.swift:56`) uses `terminal_dirty_ranges` plus `mark_clean` and never checks `terminal_abi_version()`. An additive bump does not affect it.

## Design

Track content identity per row, alongside the existing positional damage:

- Add `row_content_gen: Vec<u64>` to `Grid`. It is stamped with the same clock value as `row_gen` whenever a row's *content* changes: writes, erases, `clear_row` on a scroll-exposed row, IL/DL-inserted blanks, resize and reflow. Unlike `row_gen`, it **moves with the row** on a scroll: `scroll_region_up` rotates `row_content_gen[top..=bottom]` exactly as it rotates the cells.
- Keep a small scroll log per grid: `VecDeque<ScrollOp { gen, top, bottom, delta: i32 }>`, capped at 64 entries and recorded by the scroll primitives. If the log overflows, or an op after `since` is not expressible (see the sentinel list), the query answers "full redraw".
- **Query semantics for a caller holding generation `since`.**
  - If every scroll op after `since` shares one `(top, bottom)` region, report the net `delta` and that region. Otherwise report the sentinel.
  - Report as dirty exactly the rows whose `row_content_gen > since`, positionally, on today's screen.
  - The renderer blits its previous frame's region `[top, bottom]` by `delta` rows (positive means content moved up), then redraws the dirty rows. Rows vacated by the blit were cleared by the scroll, so they carry a fresh content gen and are always in the dirty set.
- **Full-redraw sentinel** (`TERM_SCROLL_FULL_REDRAW` flag). Returned when `since` predates any of the following:
  - a screen switch;
  - a resize or reflow;
  - RIS;
  - a snapshot restore;
  - `clear_scrollback`;
  - scroll-log overflow;
  - mixed scroll regions.

  Implement the first four through ARC-092's single `set_visible_screen()` / clock-raise path, so no per-site obligation is added. The renderer then treats every row as dirty, which is exactly today's behavior.
- `row_gen` and every existing function are unchanged, so v2/v3 consumers see identical results.

## Implementation

1. **`src/grid/mod.rs`.**
   - Add the `row_content_gen` field and `ScrollOp` log.
   - Split `mark_row_damage` into `mark_row_damage` (positional, today's behavior) and a `mark_row_content` that stamps both vectors. Every content mutator uses the latter. The ENH-025 property test (`src/terminal/tests/damage_props.rs`) already enumerates the mutators, so extend it rather than hunting call sites by hand.
2. **`src/grid/scroll.rs` and `src/grid/edit.rs`.**
   - Rotate `row_content_gen` alongside `cells`/`wrapped` in `scroll_up/down` and `scroll_region_up/down`. Record the `ScrollOp`.
   - IL/DL (`edit.rs:7,37`) are region scrolls anchored at the cursor row, so they record an op with that region.
   - Newly cleared rows get `mark_row_content`.
3. **`src/terminal/mod.rs`.**
   - Add `pub fn scroll_damage_since(&self, since: u64) -> ScrollDamage { full_redraw: bool, top: u32, bottom: u32, delta: i32 }`.
   - Add `pub fn for_each_content_dirty_range_since(&self, since: u64, f)`, reusing the `for_each_damage_range_since` coalescing shape over `row_content_gen`.
4. **`src/ffi.rs`** (additive only).
   - Add `#[repr(C)] pub struct TermScrollDelta { pub delta: i32, pub top: u32, pub bottom: u32, pub flags: u32 }`, with bit 0 = `TERM_SCROLL_FULL_REDRAW`.
   - Add `terminal_scroll_delta_since(term, gen, out: *mut TermScrollDelta) -> bool`.
   - Add `terminal_content_dirty_ranges_since(term, gen, out, cap) -> u32`, with the same NULL/0 sizing contract as `terminal_dirty_ranges_since`.
   - Add layout asserts (`size_of::<TermScrollDelta>() == 16`) next to `:913-929`.
5. **Bump the ABI to 4.**
   - `TERM_CORE_ABI_VERSION` in `src/ffi.rs:431` and `include/terminal_core_layout.h:25`, plus its comment.
   - `#define TERM_SCROLL_FULL_REDRAW 1u` in the layout header, with a `_Static_assert` on `TermScrollDelta`'s size.
   - Both literals in `abi_version_matches_header_macro` (`src/ffi.rs:1279,1286`).
   - Extend `layout_header_defines_match_rust` for the new define.
   - Regenerate the header with `make ffi-header`.
6. **Docs.**
   - `docs/FFI_GUIDE.md`: a "Scroll-Aware Damage" subsection under "Per-Consumer Damage" with a full C render-loop example (blit, then redraw the content-dirty rows, with the sentinel fallback).
   - A v4 row in the ABI table (DOC-103's table).
   - README C-surface list (DOC-107's list).
   - CHANGELOG `[Unreleased]` "Added".
   - Say explicitly that the brief's `total_lines_scrolled` backing was rejected and why, so a later reader does not "simplify" to it.
7. **Python.** Not in scope. The Python consumer (`dirty_rows_since`) is unaffected. Expose it later only on request.

## Files to touch

- `src/grid/mod.rs`, `src/grid/scroll.rs`, `src/grid/edit.rs`, `src/grid/erase.rs` (content stamps)
- `src/terminal/mod.rs` (query methods, sentinel hook via ARC-092's path)
- `src/terminal/tests/damage_props.rs` (content-gen and scroll-log properties)
- `src/ffi.rs` (struct, two functions, ABI constant, both test literals, layout asserts)
- `include/terminal_core_layout.h` (ABI 4, `TERM_SCROLL_FULL_REDRAW`, static assert)
- `include/terminal_core.h` (regenerated by `make ffi-header`, never hand-edited)
- `docs/FFI_GUIDE.md`, `README.md`, `CHANGELOG.md`

## Verify

- A new FFI test passes under `cargo test --lib --no-default-features --features pyo3/auto-initialize ffi::`. It fills a 24-row terminal, takes `gen`, feeds one `\n` at the bottom row, and then:
  - `terminal_scroll_delta_since` reports `delta == 1`, `top == 0`, `bottom == 23` and no full-redraw flag;
  - `terminal_content_dirty_ranges_since` returns only row 23;
  - `terminal_dirty_ranges_since` still returns all 24 rows, so v3 behavior is unchanged.
- The same test passes on the alternate screen (`\x1b[?1049h`) and under `DECSTBM 2;23`. In the alt-screen case the delta is non-zero although `total_lines_scrolled` stays 0.
- A property test in `damage_props.rs` passes. For random VT input it compares two frames: one rebuilt by blitting the previous frame by the reported delta and re-reading only the content-dirty rows, and one read fresh. They are cell-for-cell identical, and any sentinel case falls back to a full read.
- `make ffi-header && make ffi-header-check ffi-surface-check` pass. The regenerated header contains `TermScrollDelta`, `terminal_scroll_delta_since` and `terminal_content_dirty_ranges_since`, and `terminal_abi_version()` returns 4.
- `make xcframework` builds, and `make checkall` is green.

## Rollback

Revert the commit. The change is additive, so v3 consumers never called the new symbols. Reverting the ABI to 3 is safe only if no embedder shipped against 4, so check ParDeck's pinned core version before reverting after a release.
