# ENH-018 — Fuzz targets for the mux control-protocol parser

> Filed from the 2026-09-26 /opus-audit enhancement pass. Board card: `[ENH-018]`
> (priority medium, estimate M). Consumer: `/enhancement-all` / `/enhancement-next`.

## Goal

The mux daemon executes client commands: it spawns/kills panes, changes layouts, persists state,
and runs hooks. Its control-line parser (`parse_command`, cyclomatic complexity 63 in
`src/mux/command.rs`) and the hook-report JSON grammar (byte-sniffed `{` in
`src/mux/server.rs` `handle_client`) are exactly the kind of adversarial-input surface that
fuzzing covers — and they have no fuzz targets. `docs/fable/ENH-014-parser-fuzz-targets.md`
covered the VT parser; this card covers the mux control protocol.

## Current state

- `fuzz/` exists with VT-parser targets (from ENH-014); workspace-detach gotcha already handled
  (fuzz/ carries its own empty `[workspace]` table — keep it that way; project memory
  `cargo-fuzz-workspace-detach-gotcha`).
- `src/mux/command.rs`: `parse_command` (`:380-599`, CC 63) parses the tmux-style command
  language; `parse_line`/`Args` structures from the ARC-002 decomposition.
- `src/mux/server.rs:250,477-505`: hook-JSON routing byte-sniffs `{` and parses
  `serde_json::Value` from client lines.
- `src/mux/dispatch.rs`: `dispatch_command` mutates `MuxTree` per command — the invariant to
  fuzz: parsing junk never panics and never corrupts the tree.

## Implementation

1. **Target 1 — command parsing**: `fuzz/fuzz_targets/mux_parse_command.rs`:
   `parse_command(&bytes)` must never panic; assert it returns `Ok(cmd)` or a structured error
   for any input. Include multi-line inputs (the wire protocol is line-based; also fuzz
   `parse_line` with embedded `\n`, `%`-prefixed notification-looking lines, and the hook-JSON
   shapes).
2. **Target 2 — hook JSON**: `fuzz/fuzz_targets/mux_hook_report.rs`: feed arbitrary bytes
   through the hook-report branch's parse+validate path (`src/mux/hooks.rs` validation included).
   Invariants: no panic; rejected inputs produce the existing error, never a partial pane-metadata
   write.
3. **Target 3 — dispatch round-trip property**: a Rust-side property test (proptest or a seeded
   loop, no libFuzzer needed) that applies N random-but-typed commands to an in-memory `MuxTree`
   (no real PTY spawns — the `PaneFactory` test double from the mux test suite) and asserts
   invariants after each: every window has ≥1 pane, every pane's parent/child links are
   symmetric, `to_persist_state()` → `MuxTree::from_persist_state()` round-trips. This catches
   state-machine corruption the parser fuzzer can't (it doesn't need fuzzed bytes, just fuzzed
   *sequences*).
4. **CI hook**: one `cargo fuzz build` + short `cargo fuzz run -max_total_time=60` per target in
   the nightly bench workflow (ENH-016's `bench.yml`) or a small dedicated scheduled workflow;
   do NOT put fuzzing in per-push CI (cost). Seed corpus: the existing mux tests' valid command
   strings (`tests/mux_daemon.rs` exercises most commands — lift a few into
   `fuzz/corpus/mux_parse_command/`).
5. **Docs**: extend the fuzzing section created by ENH-014 (if it documented one) or add a short
   subsection to `docs/MUX.md` ("Development — fuzzing the control protocol").

## Files to touch

- `fuzz/fuzz_targets/mux_parse_command.rs` (new)
- `fuzz/fuzz_targets/mux_hook_report.rs` (new)
- `fuzz/Cargo.toml` (list new targets)
- `src/mux/` — test-visible seams only (e.g. `pub(crate)` on a parse entry point if needed; no
  behavior change)
- `tests/mux_property.rs` or in-crate `#[cfg(test)]` property module (target 3)
- `.github/workflows/` scheduled fuzz job (or a job in ENH-016's bench.yml)
- `fuzz/corpus/mux_parse_command/*` (seed files)

## Verify (acceptance criteria on the card)

1. Both cargo-fuzz targets build (`cargo fuzz build`) and run 60 s without crashes on the seeded
   corpus; the fuzz/ workspace remains detached from the root workspace (empty `[workspace]`
   table intact).
2. The dispatch round-trip property test runs in `cargo test --features rust-only,mux,serde mux::`
   and survives 1,000+ random command sequences.
3. Scheduled CI job exists and has completed one green run (or is verified via
   `workflow_dispatch`); docs mention how to run the targets locally.

## Rollback

Delete the targets/workflow; the property test can stay regardless.
