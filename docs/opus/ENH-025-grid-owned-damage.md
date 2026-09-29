# ENH-025: Grid-owned damage tracking with per-consumer generations and a damage-completeness property test

> Filed from the 2026-09-28 /opus-audit enhancement pass (cycle `audit-2026-09-28`). Board card: `[ENH-025]`.
> Prerequisites: QA-150 (per-handler marking fixes) and ARC-058 (RIS marks every row) must land first. This enhancement makes their result structural.
> Also resolves AUDIT ARC-088 (a destructive shared `mark_clean`).

## Goal

Make the dirty-row contract impossible to forget, and let more than one renderer consume damage.

1. Damage is recorded where grid content changes, inside the `Grid` mutators, instead of in each of the roughly 45 escape-sequence handlers.
2. Damage is exposed as a per-row generation counter. Each consumer, whether the FFI renderer, the Python `get_dirty_rows`/`mark_clean` pair, or a future mirror, tracks its own "last seen" generation. They stop clearing one shared bitset for everyone.
3. A proptest invariant proves the contract: for any VT byte stream, every row whose visible content changed is reported dirty.

## Current state

- `Terminal.dirty_rows: Vec<u64>` is a bitset (`src/terminal/mod.rs`, field in the struct; helpers `mark_row_dirty`/`mark_rows_dirty`/`mark_clean`/`get_dirty_rows` around `:3153-3200`).
- Marking happens in the handlers. QA-150 is the third round of "this operation forgot to mark" (aa6edf1, 32b922f, QA-150).
- `mark_row_dirty` also feeds `triggers.pending_trigger_rows`. ARC-064 decouples that first, so it is not in scope here.
- `mark_clean` is called from the FFI (`src/ffi.rs` `terminal_mark_clean`) and Python (`src/python_bindings/terminal/mod.rs:1027-1028`). Two consumers would steal each other's damage.
- `Grid` mutators (`src/grid/{mod,edit,erase,rect,scroll}.rs`) that take `&mut self`: `insert_lines delete_lines insert_chars delete_chars insert_characters delete_characters clear_with_bg clear clear_row_with_bg clear_row clear_line_right clear_line_left clear_screen_below clear_screen_above erase_characters erase_chars clear_scrollback get_mut set row_mut set_line_wrapped restore_from_snapshot erase_rectangle scroll_up scroll_down scroll_region_up scroll_region_down resize_without_reflow resize` (plus zone helpers, which need no damage).
- The FFI round-trip test `ffi_round_trip_matches_core_state` (`src/ffi.rs`, test module) checks damage completeness over a handful of fixed frames. `proptest` is already a dev-dependency (`Cargo.toml:191`).

## Implementation

### Phase 1: property test first (proves the gap, then guards it)
1. Add `src/terminal/tests/damage_props.rs` (register it in `src/terminal/tests/mod.rs`) with a proptest strategy producing byte streams drawn from a weighted alphabet:
   - printable ASCII and wide chars (`"あ"`)
   - `\r\n`
   - CSI sequences with random small params from the set `@ P L M X K J S T r H d G`
   - `$x $v $z $r $t` rectangles, `ESC c`, `ESC 7/8`, `?1049h/l`, `ESC D/M/E`
2. The property: `let before = snapshot_rows(&term); term.mark_clean(); term.process(&bytes); let after = snapshot_rows(&term); for r in 0..rows { if before[r] != after[r] { assert!(term.get_dirty_rows().contains(&r)) } }`.
   - `snapshot_rows` compares cell chars **and** attributes of the active grid.
   - Handle the alt-screen switch by comparing the grid the renderer would show: `active_grid()` before and after.
3. Run it at `PROPTEST_CASES=2000` locally. It must pass once QA-150 has landed. If it fails, file each minimal failing case as a QA-150 follow-up and fix it before continuing.

### Phase 2: move marking into Grid
1. Add `damage: Vec<u64>` (bitset) to `Grid`, sized to `rows` and resized in `resize`/`resize_without_reflow`.
2. Every `&mut self` content mutator marks the rows it touches: `set`/`get_mut`/`row_mut` mark their row, scroll ops mark their region, `clear*` mark their range, rect ops mark `top..=bottom`, `restore_from_snapshot` marks all.
   - `get_mut`/`row_mut` hand out `&mut` and are marked conservatively (mark on access).
3. `Terminal::mark_row_dirty` becomes a thin forward to `active_grid_mut().mark_row(row)` for the handful of non-grid visual changes (cursor-shape, selection) that still need it. Remove the per-handler calls only after the property test passes with them deleted. Remove handler calls in batches of about 10 and re-run the property test after each batch.
4. Alt-screen switch: marks all rows of the newly active grid (already done in aa6edf1; keep that).

### Phase 3: per-consumer generations
1. Replace the bitset with `row_gen: Vec<u64>` plus a `gen: u64` counter on `Grid`. Each mark does `gen += 1; row_gen[row] = gen`.
2. New API:
   - `Terminal::damage_generation() -> u64`
   - `Terminal::dirty_rows_since(gen: u64) -> impl Iterator<Item = usize>` (rows with `row_gen[r] > gen`)
3. Compatibility:
   - Keep `get_dirty_rows()`/`mark_clean()` as a built-in "default consumer" that stores its own last-seen generation, so existing Python and FFI behavior is unchanged.
   - Add FFI `terminal_damage_generation(term) -> uint64_t` and `terminal_dirty_ranges_since(term, gen, out, cap) -> uint32_t` in the header. This is an ABI addition, so bump `TERM_CORE_ABI_VERSION` if ARC-063 has landed.
   - Add Python `damage_generation()` and `dirty_rows_since(gen)` bindings with docstrings, API_REFERENCE entries and a stub regen.
4. Alt grid: generations are per grid. A screen switch bumps the global `gen` and marks every row of the new grid.

## Files to touch

- `src/grid/mod.rs`, `src/grid/edit.rs`, `src/grid/erase.rs`, `src/grid/rect.rs`, `src/grid/scroll.rs`
- `src/terminal/mod.rs` (helpers and the switch), `src/terminal/sequences/**` (removing the handler marks)
- `src/terminal/tests/damage_props.rs` (new), `src/terminal/tests/mod.rs`
- `src/ffi.rs`, `include/terminal_core.h` (Phase 3)
- `src/python_bindings/terminal/*_api.rs` (the damage API file; find it with `grep -rn "fn get_dirty_rows" src/python_bindings`)
- `docs/API_REFERENCE.md`, `docs/FFI_GUIDE.md`, `python/par_term_emu_core_rust/_native.pyi` (regenerated)

## Verify

- `PROPTEST_CASES=2000 cargo test --lib --no-default-features --features pyo3/auto-initialize damage_props` passes.
- The property test also passes with **all** handler-level `mark_row_dirty`/`mark_rows_dirty` calls in `src/terminal/sequences/` removed. This proves marking is structural. Check with `grep -rn "mark_rows\?_dirty" src/terminal/sequences | wc -l` → 0, excluding the non-grid visual cases, which are listed in a comment.
- A new test: two consumers (FFI generation A, Python default consumer) each observe the same edit, and one consumer's `mark_clean` does not hide it from the other.
- `cargo bench --no-default-features --features rust-only --bench terminal_throughput` shows no more than 3% regression versus a baseline taken before Phase 2. Use the interleaved A/B method (project memory `bench-interleaved-ab-methodology`).
- `make xcframework` succeeds (header smoke-compile) after Phase 3.
- `make checkall` is green.

## Rollback

Each phase is a separate commit. Phase 1 (the test) is always kept. Phase 2 can be reverted to per-handler marking, and the property test keeps guarding it. Phase 3's new APIs are additive, so reverting removes them without breaking `get_dirty_rows`/`mark_clean` users.
