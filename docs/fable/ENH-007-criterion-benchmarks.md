# ENH-007 — Criterion benchmark suite for the VTE processing hot path

## Goal

Add a criterion benchmark harness measuring the throughput-critical paths: plain-text
`process()`, SGR-heavy streams, wide-char/emoji streams, scrolling, and Sixel/Kitty
graphics ingestion. The repo has no `benches/` and no criterion dependency (verified
2026-08-27), so performance work on the #1 hotspot (`write_char`, complexity 90 — audit
QA-006 refactors it) currently has no regression guard: a refactor can silently cost
20% throughput and nothing notices.

## Current State

- No `[[bench]]` targets, no `benches/` directory, no criterion in Cargo.toml.
- The library builds in several profiles; benches must avoid the `python`
  extension-module linking problem — same constraint as tests: use
  `--no-default-features` (benches of the core need no PyO3 at all; prefer the pure-Rust
  surface via the `rust-only`/`sim` profile so no Python toolchain is involved).
- Bench-relevant entry points: `Terminal::process(&[u8])` (`src/terminal/mod.rs:2578`),
  grid scrolling (`src/grid/scroll.rs`), graphics parsers (`src/sixel.rs`,
  `src/graphics/kitty.rs`).

## Implementation Steps

1. Cargo.toml: add `criterion = { version = "0.7", features = ["html_reports"] }` under
   `[dev-dependencies]` (check the current criterion major at implementation time), and:

   ```toml
   [[bench]]
   name = "terminal_throughput"
   harness = false
   required-features = ["rust-only"]
   ```

   Verify the `required-features` interaction: `cargo bench --no-default-features
   --features rust-only` must build without a Python interpreter. If dev-deps +
   feature-gating fight (bench targets and `required-features` have sharp edges),
   fall back to gating the bench body with `#[cfg(feature = "rust-only")]` and an empty
   main otherwise.
2. `benches/terminal_throughput.rs` with benchmark groups:
   - `plain_ascii`: 1 MiB of lorem-ipsum lines through `Terminal::process` (80×24)
   - `sgr_heavy`: alternating truecolor SGR + short text (ls --color-style output shape)
   - `unicode_wide`: CJK + emoji + ZWJ sequences (exercises `write_char`'s hard paths)
   - `scroll`: many newlines on a small grid with scrollback enabled
   - `cursor_addressing`: full-screen repaint pattern (vim-redraw shape: CUP + line writes)
   - `sixel_decode` and `kitty_decode`: a representative graphic payload each (generate
     deterministically in the bench, no fixture files needed; a few KB is enough)
   Use `Throughput::Bytes(input.len())` so results read as MB/s.
3. Makefile: add `bench:` target (`cargo bench --no-default-features --features rust-only`)
   — do NOT add to `checkall` (benches are on-demand, not a gate).
4. Record a baseline: run once, commit `docs/fable/BENCH-BASELINE-2026-08.md` with the
   summary table (criterion's `target/criterion` output is gitignored via `target/`).
5. Docs: CONTRIBUTING.md gains a short "Benchmarks" section (how to run, how to compare
   with `--save-baseline`/`--baseline`).

## Files to Touch

- `Cargo.toml` (dev-deps + bench target) — coordinate with audit ARC-006/ARC-007 edits
  to the same file (sequence after them if run in the same cycle)
- `benches/terminal_throughput.rs` (new)
- `Makefile` (`bench` target)
- `CONTRIBUTING.md`, `docs/fable/BENCH-BASELINE-2026-08.md` (generated baseline)

## Verify

- `cargo bench --no-default-features --features rust-only -- --test` (criterion's smoke
  mode: runs each bench once, no timing) — exits 0
- Full run produces per-group MB/s numbers; baseline file committed
- `make checkall` still green (benches don't affect the gate)

## Rollback

Delete `benches/`, the Cargo.toml block, and the Make target. No runtime code is touched.
