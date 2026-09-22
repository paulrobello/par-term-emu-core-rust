# ENH-014 — cargo-fuzz targets for the four untrusted-byte parsers

## Goal

Add libFuzzer targets for the code that consumes bytes from untrusted programs:
`Terminal::process` (the whole VTE pipeline), `SixelParser`, `KittyParser::parse_chunk`, and
`TmuxControlParser::parse`. The kitty parser shipped three bugs in v0.43.1 and the repo has
`proptest` as a dev-dep but no coverage-guided fuzzing and no `fuzz/` directory (verified).
A nightly CI job runs each target for a bounded time; a crash artifact becomes a regression
test.

## Current State (verified 2026-09-22 at 9fa2237)

- Entry points: `Terminal::process(&mut self, &[u8])` (`src/terminal/mod.rs:2821`);
  `SixelParser::new_with_limits(SixelLimits)` (`src/sixel.rs:270`) and its feed method;
  `KittyParser::parse_chunk(&mut self, &str) -> Result<bool, GraphicsError>` (`src/graphics/kitty.rs:232`);
  `TmuxControlParser::parse(&mut self, &[u8]) -> Vec<TmuxNotification>` (`src/tmux_control.rs:270`).
- `proptest = "1.11.0"` in dev-deps; no `fuzz/`, no `cargo-fuzz` mention in `Makefile`.
- Decoders already enforce limits (`SixelLimits`, image dimension caps per project memory
  "Image Decoding Bounds"); the fuzzer is the proof those caps hold.
- CI is `workflow_dispatch`-only (`ci.yml:3-4`); the fuzz job follows the same trigger plus a
  `schedule:` cron.

## Implementation Steps

1. `cargo fuzz init` at the repo root (needs nightly for the runner; targets compile on stable
   with `--sanitizer none` fallback). The generated `fuzz/Cargo.toml` depends on the parent
   crate with `default-features = false, features = ["rust-only"]` so no Python toolchain.
   Add `fuzz/target/` and `fuzz/artifacts/` to `.gitignore`; commit `fuzz/corpus/*/` seeds.
2. Targets in `fuzz/fuzz_targets/`:
   - `terminal_process.rs`: `Terminal::with_scrollback(80, 24, 1000)`, `process(data)`, then
     call `content()`, `capture_snapshot()`, and `export_scrollback()` (exercise read paths on
     the mutated state).
   - `sixel.rs`: `SixelParser::new_with_limits(SixelLimits::new(4096, 4096, 65535))` fed the
     input; assert any produced image's `width * height` ≤ the limit product.
   - `kitty.rs`: `KittyParser::new()`, split input on `\x1b\\` into chunks, `parse_chunk` each
     (`from_utf8_lossy`), then `build_graphic` if the parser reports completion.
   - `tmux_control.rs`: `TmuxControlParser::new().parse(data)` twice with the input split at
     every byte offset in a small window (exercises partial-line buffering).
   Each target is `fuzz_target!(|data: &[u8]| { ... })` with no panics allowed; wrap nothing in
   `catch_unwind` — a panic is a finding.
3. Seeds: write 3–5 corpus files per target from existing test fixtures
   (`src/terminal/tests/`, `tests/assets/`, the deterministic generators in
   `benches/terminal_throughput.rs`).
4. `Makefile`: `fuzz-<target>` and `fuzz-all` targets running
   `cargo +nightly fuzz run <target> -- -max_total_time=60`; not part of `checkall`.
5. `.github/workflows/fuzz.yml`: `workflow_dispatch` + `schedule: cron '17 6 * * *'`, Linux
   only, matrix over the four targets, `-max_total_time=600`, upload `fuzz/artifacts/` on
   failure. Document the two triggers in `.github/workflows/README.md`.
6. Regression policy in `CONTRIBUTING.md` "Fuzzing": a crash artifact is copied to
   `src/<module>/tests/fuzz_regressions/` and asserted in a normal `#[test]`.

## Files to Touch

- `fuzz/` (new: Cargo.toml, 4 targets, corpus), `.gitignore`, `Makefile`,
  `.github/workflows/fuzz.yml` (new), `.github/workflows/README.md`, `CONTRIBUTING.md`

## Verify

- `cargo +nightly fuzz build` succeeds for all four targets.
- `cargo +nightly fuzz run terminal_process -- -max_total_time=60 -runs=0` (corpus only) exits 0
  for each target — no crash on seeds.
- `make fuzz-all` runs each target 60 s locally without a crash artifact (any artifact found is
  filed as a defect card, not a blocker for this card).
- `make checkall` still green (fuzz crate is outside the workspace member list, so it does not
  affect the gate; confirm `cargo check --all-targets ...` from the Makefile ignores `fuzz/`).

## Rollback

Delete `fuzz/`, the Make targets, and the workflow. No library code changes.
