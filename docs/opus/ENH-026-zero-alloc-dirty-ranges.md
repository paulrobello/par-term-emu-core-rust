# ENH-026: Allocation-free `terminal_dirty_ranges` for the per-frame FFI render loop

> Filed from the 2026-09-28 /opus-audit enhancement pass (cycle `audit-2026-09-28`). Board card: `[ENH-026]`.
> Resolves AUDIT ARC-085. Independent of ENH-025. If ENH-025 Phase 3 lands first, apply the same technique to `terminal_dirty_ranges_since`.

## Goal

ParDeck's documented render loop calls `terminal_dirty_ranges` twice per frame: once with `out = NULL` to size, then to fill. Each call allocates a `Vec<usize>` (`get_dirty_rows()`) and a `Vec<TermRowRange>`. The goal is to remove both allocations by coalescing ranges straight from the `u64` bitset words into the caller's buffer while counting the total.

## Current state

- `src/ffi.rs:471-500` `terminal_dirty_ranges`: calls `term_ref.get_dirty_rows()` (allocating a `Vec<usize>`), builds `Vec<TermRowRange>`, copies `min(len, cap)` into `out`, and returns `len`.
- `src/terminal/mod.rs:3176-3195`: `dirty_row_indices()` already iterates the bitset without allocating. It is private (`fn`).
- `benches/ffi_readback.rs` measures this exact path: the status and agent frames, dirty ranges then `terminal_read_row` per range, against the `terminal_get_state` snapshot.

## Implementation

1. Make `dirty_row_indices` `pub(crate)`. Better: add `pub(crate) fn for_each_dirty_range(&self, mut f: impl FnMut(u32, u32))`, which walks the bitset words. For each non-zero word it uses `trailing_zeros` to jump to the next set bit and extends a run while bits are consecutive, including across word boundaries. It calls `f(start, end)` per maximal run, with no allocation.
2. Rewrite `terminal_dirty_ranges`:
   ```rust
   let mut total: u32 = 0;
   term_ref.for_each_dirty_range(|start, end| {
       if total < cap && !out.is_null() {
           unsafe { out.add(total as usize).write(TermRowRange { start, end }) };
       }
       total += 1;
   });
   total
   ```
   The behavior is identical, including the NULL/cap-0 sizing call.
3. Keep `get_dirty_rows()` as is for Python.
4. Unit tests for `for_each_dirty_range`:
   - empty
   - a single row
   - a run crossing the 63/64 word boundary (rows 62..=66)
   - two separate runs
   - all rows of a 200-row terminal
   - Compare each against the old coalescing algorithm, kept as a test-only reference function.

## Files to touch

- `src/terminal/mod.rs` (the new helper)
- `src/ffi.rs` (`terminal_dirty_ranges`, tests)
- `benches/ffi_readback.rs` (no change needed; used for measurement)

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize dirty_range`: the new tests pass and match the reference algorithm.
- `cargo test --lib --no-default-features --features pyo3/auto-initialize ffi`: `ffi_round_trip_matches_core_state` passes unchanged.
- `cargo bench --no-default-features --features rust-only --bench ffi_readback` with the interleaved A/B method (project memory `bench-interleaved-ab-methodology`, `--save-baseline` needs the `--bench` target): the status-frame dirty-path time does not regress, and allocation count per frame drops to zero. Confirm with a counting global allocator in a one-off test, or with `dhat` if available. If neither is practical, report the timing only and say so.
- `make checkall` is green.

## Rollback

Revert the two-function change. The API and ABI are unchanged.
