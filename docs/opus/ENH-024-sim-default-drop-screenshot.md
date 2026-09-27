# ENH-024: Make `sim` slim by default (drop the implied `screenshot`) in the next minor release

> Filed from the 2026-09-26 run-2 /opus-audit enhancement pass (cycle `audit-2026-09-26-r2`). Board card: `[ENH-024]` (priority low, estimate S).
> Requires AUDIT ARC-043 (`swash` optional behind `screenshot`) first. Without it this change saves nothing.

## Goal

`sim` was introduced as the headless profile for pure-Rust embedders such as a server-side screen model (par-hack). It currently implies `screenshot` (`Cargo.toml:266`, `sim = ["screenshot"]`), and the `Cargo.toml` comment flags that removal as a future release-note item. Once ARC-043 makes `swash` optional, dropping `screenshot` from `sim` removes the font-shaping stack and the embedded fonts (JetBrains Mono and Noto Emoji, several MB) from every `sim` build. Embedders that render opt in with `features = ["sim", "screenshot"]`.

## Current state

- `Cargo.toml:266`: `sim = ["screenshot"]`, with the comment about ARC-021 phase 1 keeping today's default.
- The embedded fonts live in `src/screenshot/` (`include_bytes!`), so they are compiled only with `screenshot`.
- Known `sim` consumers: search `~/Repos` with `grep -rln 'par-term-emu-core-rust' ~/Repos/*/Cargo.toml | xargs grep -l '"sim"'`, and check each one's use of `screenshot::`/`Terminal::screenshot`.

## Implementation

1. Enumerate consumers (the command above). For each one that uses screenshot APIs, note that it must add `"screenshot"`. If par-hack or another first-party repo uses it, file a card on that project's board (upstream-filing protocol) with the one-line migration **before** releasing.
2. `Cargo.toml`: `sim = []`, and update the comment.
3. `src/lib.rs`: check that any `#[cfg(feature = "sim")]` code does not reference `screenshot` without its own cfg.
4. `ci.yml` sim guard: extend the guard (or ENH-019's assertions) to assert that `swash` is absent under `sim`.
5. CHANGELOG `[Unreleased]` → **Breaking (Rust embedders)**: "`sim` no longer implies `screenshot`; add `features = [\"sim\", \"screenshot\"]` to keep `crate::screenshot`." Add a matching README What's New line. Update the `sim` row in CLAUDE.md and `docs/RUST_USAGE.md` (including DOC-044's table).
6. Ship in the next minor release. The owner decides timing. This card implements the change on a branch and does not cut the release.

## Files to touch

- `Cargo.toml`
- `src/lib.rs` (only if needed)
- `.github/workflows/ci.yml` (the guard)
- `CHANGELOG.md`
- `README.md`
- `CLAUDE.md`
- `docs/RUST_USAGE.md`
- Downstream cards (board only)

## Verify

- `cargo check --no-default-features --features sim`
- `cargo tree --no-default-features --features sim -i swash` reports nothing (not in the tree).
- `cargo check --no-default-features --features sim,screenshot`
- `cargo test --lib --no-default-features --features sim` (if the sim test invocation exists; check the Makefile/CI for the exact command).
- Every first-party consumer found in step 1 either has no screenshot use or has a filed migration card.
- `make checkall`

## Rollback

Restore `sim = ["screenshot"]`. It is purely additive to revert.
