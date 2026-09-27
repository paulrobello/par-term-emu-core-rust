# ENH-019: Per-feature build matrix with dependency-tree assertions (`cargo hack`)

> Filed from the 2026-09-26 run-2 /opus-audit enhancement pass (cycle `audit-2026-09-26-r2`). Board card: `[ENH-019]` (priority medium, estimate S).
> Consumer: `/enhancement-all` / `/enhancement-next`. Depends on AUDIT ARC-040 (a push-triggered CI job to host it). The plan also works as a dispatch-only step if ARC-040 has not landed.

## Goal

Two feature-hygiene defects shipped unnoticed in 0.53.0:
- `screenshot = []` left `swash` unconditional (ARC-043).
- `mux` pulls in the binary-only dependency `clap` (ARC-044).

A third, the `streaming-bin` breakage, went unnoticed for about 6 hours (card 01a0df71). Each of these would have been caught mechanically:
- by building every feature on its own, and
- by asserting that heavy dependencies stay out of the slim profiles.

## Current state

- `Cargo.toml [features]` has about 16 features, including `python`, `python-test`, `screenshot`, `pty_session`, `streaming`, `streaming-bin`, `mux`, `serde`, `rust-only`, `sim`, `full`, `regenerate-proto` and `jemalloc`.
- `python` and `sim` are mutually exclusive at compile time (`src/lib.rs:51`, a `compile_error!`). `python` needs a Python interpreter to link tests, and `extension-module` fails to link outside maturin.
- `ci.yml` has a `sim` dependency-tree guard step, which is the model for the assertions below. It is dispatch-only.
- `make checkall` compiles only one wide feature set for clippy (`Makefile:268`).

## Implementation

1. Add `scripts/check_features.sh` (bash, `set -euo pipefail`):
   - Run `cargo hack check --each-feature --no-default-features --exclude-features python,python-test,full,regenerate-proto,jemalloc`, plus explicit checks for the combinations that matter: `rust-only`, `sim`, `rust-only,mux`, `rust-only,streaming`, `streaming-bin`, and `mux-bin` (if ARC-044 has landed; otherwise `mux`).
   - Run `cargo check --no-default-features --features python-test --lib` separately, because it needs a Python interpreter.
   - Add dependency assertions with a helper `assert_absent <features> <crate>`. The helper runs `cargo tree --no-default-features --features "$1" -e normal -i "$2"` and fails if the exit code is 0 (the crate is present). Assertions:
     - `rust-only`: no `swash` (after ARC-043), no `clap`, no `pyo3`, no `tokio`.
     - `sim`: no `portable-pty`, no `pyo3`, no `tokio`.
     - `rust-only,mux`: no `clap` (after ARC-044), no `tokio-tungstenite`.
     - `rust-only,streaming`: no `clap`, no `tracing-subscriber` (use whatever `streaming-bin` adds; read `Cargo.toml`).
   - Guard the assertions for ARC-043 and ARC-044 behind a check that those ARC fixes are present, for example `grep -q 'screenshot = \["dep:swash"\]' Cargo.toml`, so the script is useful before and after them.
2. Add a `make check-features` target that runs the script (not part of `checkall`, because it takes minutes).
3. CI: add a `features` job running `cargo install cargo-hack --locked` then `make check-features`, to `ci-fast.yml` (from ARC-040) or to `ci.yml` if ARC-040 has not landed.
4. Document it in CONTRIBUTING.md under the testing and CI section: when to run it (any `Cargo.toml` `[features]` or dependency edit).

## Files to touch

- `scripts/check_features.sh` (new)
- `Makefile` (a `check-features` target plus a `##` help line)
- `.github/workflows/ci-fast.yml` or `ci.yml`
- `CONTRIBUTING.md`

## Verify

- `make check-features` exits 0 on the current tree.
- Negative control: temporarily add `clap` to `rust-only` in a scratch branch and run the script. It must exit non-zero with a clear message naming `clap`. Revert the change.
- The CI job appears in the workflow and passes (dispatch it once).

## Rollback

Delete the script, the Makefile target and the CI job. Nothing else depends on them.
