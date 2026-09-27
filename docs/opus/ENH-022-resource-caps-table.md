# ENH-022: Generated resource-caps table in SECURITY.md, with a drift check

> Filed from the 2026-09-26 run-2 /opus-audit enhancement pass (cycle `audit-2026-09-26-r2`). Board card: `[ENH-022]` (priority low, estimate S).

## Goal

SEC-109 showed that `docs/SECURITY.md:895` claimed "zlib decompression is capped at 1 MiB". That cap exists only in the streaming wire protocol, and the claim went unnoticed. The code has about 37 `MAX_*`/`DEFAULT_MAX_*` size constants across the parsers, the mux and streaming. A generated table keeps the security doc's resource-limit claims tied to the code.

## Current state

The constants are scattered. A sample:
- `src/graphics/mod.rs:37,43` (`MAX_IMAGE_DIMENSION`, `MAX_IMAGE_PIXELS`)
- `src/graphics/iterm.rs:17`
- `src/sixel.rs:25-41`
- `src/terminal/mod.rs:144-154,936`
- `src/terminal/sequences/dcs/mod.rs:47`
- `src/mux/server.rs:85`, `src/mux/hooks.rs:76`, `src/mux/persist.rs:38`
- `src/streaming/server.rs:45-54`, `src/streaming/proto.rs:46`
- `src/terminal/file_transfer.rs:87`, `src/bin/streaming_server/frontend_download.rs:34`

`grep -rn "MAX_[A-Z_]*: *usize *=" src` lists them. SECURITY.md describes some of them in prose.

## Implementation

1. Add a `/// cap: <one-line description>` doc-comment convention to each **security-relevant** constant: those that bound untrusted input size. Skip cosmetic limits such as `MAX_PALETTE_STACK`. Pick the set with the grep above, which gives about 25 entries. The SEC-109 kitty caps join the set when that fix lands.
2. `scripts/gen_caps_table.py` (stdlib only):
   - Regex-scan `src/**/*.rs` for `/// cap: (.+)\n\s*(pub(\(crate\))? )?const (\w+): \w+ = (.+);`.
   - Evaluate simple arithmetic in the value (`64 * 1024 * 1024`) with a restricted `ast` literal evaluator, and render human units (MiB/KiB).
   - Emit a Markdown table (Constant, Value, Location `path:line`, Description) between the markers `<!-- caps-table:start -->` and `<!-- caps-table:end -->` in `docs/SECURITY.md`.
   - With `--check`, exit non-zero if the file content would change.
3. Insert the markers in SECURITY.md under a new `## Resource Limits` section, and replace the prose numbers that duplicate the table with a reference to it. Keep the prose for semantics (what happens when a cap is hit).
4. Makefile: add a `caps-table` target (regenerate) and put `caps-table-check` into `checkall` (it is fast).

## Files to touch

- `scripts/gen_caps_table.py` (new)
- `docs/SECURITY.md`
- About 25 `.rs` files (doc-comment lines only, no code changes)
- `Makefile`

## Verify

- `make caps-table && git diff docs/SECURITY.md` shows the table.
- `make caps-table-check` exits 0 on the clean tree.
- Negative control: change `MAX_CONTROL_LINE_BYTES` to `2 * 1024 * 1024` locally. `make caps-table-check` must fail. Revert.
- `make checkall`.

## Rollback

Remove the markers and the table, the script and the Makefile hooks. The doc comments are harmless and can stay.
