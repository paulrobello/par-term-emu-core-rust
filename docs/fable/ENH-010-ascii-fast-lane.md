# ENH-010 — ASCII fast lane in the print path (plain-text throughput)

## Goal

Raise plain-ASCII ingestion throughput. The committed baseline
(`docs/fable/BENCH-BASELINE-2026-08.md`) shows `plain_ascii` at 3.44 MiB/s and `scroll` at
3.40 MiB/s versus 17 MiB/s for `cursor_addressing` and 11.7 MiB/s for `sgr_heavy` — the
lowest numbers are the most common workload (`cat` a log, `ls -R`, compiler output). Every
printable byte currently walks: `debug::log_print` format guard, Kitty placeholder check,
normalization-form check, ACS charset translate, regional-indicator check,
`try_apply_combining_mark`, a 5-arm control-char match, `unicode_width_config::char_width`,
and finally `write_normal_cell`. Add a single early branch for the overwhelmingly common
case: `c` is printable ASCII (0x20..=0x7E), G0 charset is `Ascii`, no combining state is
pending, and the pending-wrap / insert-mode / auto-wrap paths are handled by the same
`write_normal_cell` tail.

## Current State (verified 2026-09-22 at 9fa2237)

- `src/terminal/perform.rs:12-37` — `print()` calls `debug::log_print` unconditionally
  (`src/debug.rs:321` checks `is_enabled(Trace)` inside, but the call still constructs
  nothing until enabled — cheap, keep), then normalization dispatch, then `write_char`.
- `src/terminal/write.rs:20-141` — `write_char`: ACS translate via `active_charset()`
  (`src/terminal/mod.rs:1233`), `grapheme::is_regional_indicator`, `try_apply_combining_mark`
  (`:296`, checks previous cell), control-char match, then `write_normal_cell(c, cols)` (`:147`).
- `write_normal_cell` computes `char_width` through `unicode_width_config::char_width` on
  every call; for ASCII this is always 1.
- `Cell` construction (`:224-232`) is already allocation-free (`SmallVec::new()`).
- Bench harness exists: `benches/terminal_throughput.rs`, `make bench`, baseline file.

## Implementation Steps

1. `src/terminal/write.rs`: add at the top of `write_char`, before the ACS translate:
   ```rust
   // ASCII fast lane: printable ASCII with the plain charset active needs none of
   // the Unicode machinery below. Combining marks and regional indicators are
   // never ASCII, so skipping those checks is exact, not approximate.
   if (' '..='~').contains(&c) && self.active_charset() == Charset::Ascii {
       self.write_normal_cell_known_width(c, 1, cols);
       return;
   }
   ```
   `Charset` needs `PartialEq` (add to its derive at `src/terminal/mod.rs:96` if missing).
   `cols` must be computed before this branch (`let (cols, _) = self.size();` already
   precedes the regional-indicator check; move the fast lane after that line).
2. Refactor `write_normal_cell(c, cols)` into
   `write_normal_cell_known_width(&mut self, c: char, char_width: usize, cols: usize)` holding
   the existing body minus the `char_width` computation, and keep `write_normal_cell` as a
   two-line wrapper that computes the width and delegates. Zero behavior change for the
   non-ASCII path.
3. `src/terminal/perform.rs::print`: nothing to change — normalization of an ASCII char is
   identity and the Kitty placeholder is non-ASCII. Confirm `normalize_char` is not called for
   ASCII when a form is configured; if it is, add the same `(' '..='~')` guard ahead of it.
4. Optional second lever (measure first): in `Terminal::process_internal`
   (`src/terminal/mod.rs:2860+`), the vte `Parser::advance` loop hands bytes one at a time to
   `print`. vte 0.15 (`Cargo.toml:61`) supports batching via `advance(&mut performer, &[u8])`;
   confirm the current call already passes the slice rather than iterating bytes. If it
   iterates, switch to the slice form — vte's internal UTF-8 fast path only engages on slices.
5. Re-run `make bench` and update `docs/fable/BENCH-BASELINE-2026-08.md` (or add a dated
   `BENCH-BASELINE-2026-09.md` per the CONTRIBUTING "Benchmarks" section) with before/after for
   all seven groups.

## Files to Touch

- `src/terminal/write.rs`, `src/terminal/mod.rs` (Charset derive, possibly the advance call)
- `docs/fable/BENCH-BASELINE-2026-09.md` (new) and `CONTRIBUTING.md` pointer

## Verify

- `cargo bench --no-default-features --features rust-only -- plain_ascii scroll` shows
  `plain_ascii` at or above 6 MiB/s (target ≥ 1.75× baseline 3.44) and `scroll` improved;
  `unicode_wide`, `sgr_heavy`, `cursor_addressing`, `sixel_decode`, `kitty_decode` within
  ±5% of baseline (no regression).
- `cargo test --lib --no-default-features --features pyo3/auto-initialize` green — the 30+
  `write_char` unit tests in `src/terminal/write.rs:649+` and the DEC Special Graphics tests
  (charset translate path) are the behavior oracle.
- `uv run pytest tests/test_terminal.py tests/test_terminal_bindings.py -q` green.
- `make checkall` green.

## Rollback

Delete the fast-lane branch and the `_known_width` split; the wrapper form is
behavior-identical so a partial rollback (keep the split, drop the branch) is also safe.
