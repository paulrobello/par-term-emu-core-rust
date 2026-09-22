# ENH-013 — Crate package hygiene: exclude `web_term/`, gate streaming-only deps, workspace for `derive/`

## Goal

Shrink what `cargo package` ships and what every build profile compiles. Three independent
manifest fixes, each verified by a command: (a) 24 committed `web_term/` build artifacts
(13 minified JS chunks) are packaged into the crates.io crate although the streamer reads
`web_root` from disk or downloads it (`src/bin/streaming_server/cli.rs:169`,
`frontend_download.rs`); (b) `subtle` and `zeroize` are unconditional deps used only under
`#[cfg(feature = "streaming")]`, and `src/streaming/{error,protocol}.rs` (1,934 lines) compile
into the `sim` profile that promises "grid + terminal + screenshot only"; (c) `derive/` is
not a workspace member, so it has its own `derive/target/` and lockfile. Also (d) drop `tokio`
from the `mux` feature, which no mux code uses. Audit ARC-004/005/012/014 land these as
defects; this plan is the combined, ordered execution with the CI guard extended.

## Current State (verified 2026-09-22 at 9fa2237)

- `Cargo.toml:15-38` `exclude` list lacks `web_term/`; `git ls-files web_term | wc -l` = 32.
- `Cargo.toml:141-142` `subtle = "2.6.1"`, `zeroize = "1.8.2"` unconditional.
- `src/lib.rs:77` `pub mod streaming;` unconditional; `src/streaming/mod.rs:44-45` compiles
  `error` and `protocol` unconditionally, `proto`/`client`/... gated on `streaming`.
- `src/python_bindings/streaming.rs:1064-1107` has non-streaming stubs of `encode_/decode_*`,
  meaning the `python` feature already tolerates streaming being absent.
- `Cargo.toml:219` `mux = ["pty_session", "tokio", ...]`; `grep -rl tokio src/mux src/bin/par_mux`
  = 0 files. `tests/mux_feature_isolation.rs:4` repeats the tokio claim.
- No `[workspace]` in `Cargo.toml`; `derive/target/` exists on disk.
- `.github/workflows/ci.yml:140` D1 guard: `cargo tree --no-default-features --features sim
  -e normal | grep -qE 'tokio|portable-pty'` must be empty.
- No `.parsightignore`; parsight's top god objects / most complex functions are `web_term` chunks.

## Implementation Steps

1. Packaging: add `"web_term/"` to the `exclude` array at `Cargo.toml:15`. Verify with
   `cargo package --list --allow-dirty | grep -c web_term` = 0.
2. Analytics: create `.parsightignore` containing `web_term/` (one line). Reindex
   (`index_directory` incremental) and confirm `find_god_objects` no longer lists chunks.
3. Deps: `subtle = { version = "2.6.1", optional = true }`, `zeroize = { version = "1.8.2",
   optional = true }`; append `"subtle", "zeroize"` to the `streaming` feature list (`:206`).
4. Module gating: `src/lib.rs:77` → `#[cfg(any(feature = "streaming", feature = "python",
   feature = "python-test"))] pub mod streaming;`. Build every profile:
   - `cargo check --no-default-features --features sim`
   - `cargo check --no-default-features --features rust-only`
   - `cargo check --no-default-features --features rust-only,mux,serde`
   - `cargo check --no-default-features --features python-test`
   - `cargo check --all-targets --features python,streaming,mux,serde`
   Fix any `use crate::streaming::...` that now fails outside those features by gating the
   importer the same way (expected: `src/python_bindings/mod.rs` re-exports and
   `src/terminal/event.rs` if it names `ServerMessage`).
5. `mux` feature: remove `"tokio"` from `Cargo.toml:219`; rewrite the comment at `:213-214`
   to "Needs real PTYs and a local socket"; fix `tests/mux_feature_isolation.rs:4`.
6. Workspace: add to the root manifest
   ```toml
   [workspace]
   members = [".", "derive"]
   ```
   then `rm -rf derive/target` (build artifact, already covered by `.gitignore:3`). The
   derive crate's independent version policy (CLAUDE.md) is unaffected. Confirm
   `cargo metadata --format-version 1 | jq '.workspace_members | length'` = 2 and that
   `make dev` (maturin) still builds; maturin reads `[package]`, not the workspace.
7. CI guard: extend `.github/workflows/ci.yml:140` regex to
   `'tokio|portable-pty|subtle|zeroize|prost|axum'`.
8. `CHANGELOG.md` Unreleased: packaging + feature notes (no API change; `sim` consumers that
   accidentally used `streaming::protocol` types would break — call that out).

## Files to Touch

- `Cargo.toml`, `.parsightignore` (new), `src/lib.rs`, possibly `src/streaming/mod.rs`,
  `tests/mux_feature_isolation.rs`, `.github/workflows/ci.yml`, `CHANGELOG.md`

Sequence before audit ARC-008 (gate feature-string alignment) so those strings are written once.

## Verify

- `cargo package --list --allow-dirty | grep -c web_term` prints 0.
- `cargo tree --no-default-features --features sim -e normal | grep -E 'tokio|portable-pty|subtle|zeroize|prost|axum'` prints nothing.
- `cargo tree --no-default-features --features rust-only,mux -e normal | grep -c tokio` prints 0.
- All five `cargo check` profiles in step 4 succeed; `make test-rust` and `make dev` succeed.
- `git status` shows `derive/target` absent and no lockfile churn beyond the workspace merge.
- `make checkall` green.

## Rollback

Each step is an independent manifest edit; revert individually.
