# ENH-021: Expose `--kitty-file-media` and a safe default `--input-rate-limit` on `par-term-streamer`

> Filed from the 2026-09-26 run-2 /opus-audit enhancement pass (cycle `audit-2026-09-26-r2`). Board card: `[ENH-021]` (priority medium, estimate S).
> Related: AUDIT SEC-112 (config not applied to single-session), QA-131 (unbounded queue), DOC-041.

## Goal

The standalone streamer hardcodes `kitty_file_media: FileMediaMode::default()` (`src/bin/streaming_server/main.rs:283`), so operators cannot turn file media fully `off` (or `all` for trusted local use). `--input-rate-limit` defaults to `0` (unlimited, `cli.rs:280-282`), which makes QA-131's memory-growth scenario reachable by default. This card exposes the file-media mode as a flag and gives the rate limit a conservative non-zero default.

## Current state

- `src/bin/streaming_server/cli.rs`: clap `Args`, and every flag has `env = "PAR_TERM_*"`. `input_rate_limit: usize` has default `"0"`.
- `main.rs:283` builds `StreamingConfig { ..., kitty_file_media: FileMediaMode::default(), ... }`.
- `graphics::kitty::FileMediaMode` has variants `Off`, `TempOnly` (default) and `All`. Find its `FromStr` or string parsing used by the Python binding with `find_symbol FileMediaMode`.
- `StreamingConfig.input_rate_limit_bytes_per_sec` (`src/streaming/config.rs:422`).

## Implementation

1. `cli.rs`: add
   ```rust
   /// Kitty graphics file media (t=f / t=t): off | temp_only | all
   #[arg(long, default_value = "temp_only", env = "PAR_TERM_KITTY_FILE_MEDIA", value_parser = parse_file_media)]
   pub kitty_file_media: FileMediaMode,
   ```
   `parse_file_media` should reuse the existing string mapping. If none is public, add `impl FromStr for FileMediaMode` in `kitty.rs` and use it from the Python binding too, so there is one vocabulary.
2. `main.rs:283`: `kitty_file_media: args.kitty_file_media`.
3. Rate limit default: change `default_value = "0"` to `"1048576"` (1 MiB/s per client, well above human typing and normal pastes; the 256 KiB paste cap means a max paste still clears in under a second). Keep `0` meaning unlimited. Mention the new default in the flag docstring.
   - **Behavior change**: record it in the CHANGELOG `[Unreleased]` under Changed with the opt-out (`--input-rate-limit 0`).
   - Confirm the limiter is per client, not per session, by reading `src/streaming/server.rs` near the `rate limiter` construction (`:1827`/`:2138`), and state the correct scope in the docs.
4. `docs/STREAMING.md`: add both flags and their env vars to the CLI table, and link the SECURITY.md kitty section.
5. Tests: clap parse tests in `cli.rs` (if a test module exists; otherwise add one):
   - `--kitty-file-media off` gives `Off`.
   - An invalid value fails.
   - The default rate limit equals 1048576.

## Files to touch

- `src/bin/streaming_server/cli.rs`
- `src/bin/streaming_server/main.rs`
- `src/graphics/kitty.rs` (a `FromStr`, only if absent)
- `src/python_bindings/streaming.rs` (reuse `FromStr`, only if refactored)
- `docs/STREAMING.md`
- `CHANGELOG.md`

## Verify

- `cargo test --no-default-features --features streaming-bin --bin par-term-streamer` (the clap tests pass).
- `cargo run --no-default-features --features streaming-bin --bin par-term-streamer -- --help | grep -E "kitty-file-media|input-rate-limit"` shows both flags with defaults.
- `make checkall`.

## Rollback

Revert the default to `"0"` and remove the flag. `StreamingConfig` is unchanged, so embedders are not affected.
