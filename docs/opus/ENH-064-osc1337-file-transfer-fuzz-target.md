# ENH-064 — cargo-fuzz target for the OSC 1337 multipart / file-transfer state machine

## Goal

A ninth… tenth cargo-fuzz target (`fuzz-iterm_transfer`) hammering the OSC 1337
`MultipartFile`/`FilePart`/inline-image state machine — the exact surface where SEC-001
lived (restart-orphaned transfers, unbounded active sets) — so the caps and eviction
invariants SEC-001 adds are proven stable under arbitrary interleavings, not just under
the hand-written regression tests.

## Current-state context

- Nine fuzz targets exist under `fuzz/` (`fuzz-terminal_process` already drives the VTE
  parser broadly; `fuzz-sixel`, `fuzz-kitty` cover the other graphics protocols' decoders).
  None targets the iTerm2 file-transfer manager's state machine specifically — its
  `active_transfers` map, the multipart buffer replacement, and the completed ring.
- Workspace detach gotcha is documented in project memory: `fuzz/` carries an empty
  `[workspace]` table; follow the existing target layout exactly.
- The caps: per-transfer 50 MiB, completed ring 32, active count cap (SEC-001). The
  invariant to enforce: no input sequence grows retained memory without bound — bounded
  by (active cap × per-transfer cap) + ring.

## Implementation steps

1. `cargo fuzz add iterm_transfer` in `fuzz/` (or copy the pattern of `fuzz-sixel`'s
   `Cargo.toml`/`build.rs` if `cargo fuzz add` fights the workspace layout).
2. Fuzz body: construct a `Terminal` (features as `fuzz-terminal_process` does — the
   core member builds without pyo3 under fuzz), then drive the OSC handlers directly:
   decode the fuzz input as a command stream — first byte selects
   `MultipartFile(start-params)` vs `FilePart(base64 chunk)` vs `File=inline` vs a
   terminal-reset/garbage passthrough — and call the corresponding
   `Terminal::process`-level bytes (`ESC ] 1337 ; … BEL`) rather than private APIs, so the
   parser path stays in scope. Seed a corpus of well-formed starts/chunks (see
   `osc1337-wire-ground-truth` vault note for the base64/part grammar).
3. **Oracle in the harness**: after each input, assert the cheap invariant —
   `terminal.file_transfer_manager_active_count() <= cap` (add a `#[cfg(fuzzing)]`-gated
   accessor if none exists — keep it `pub(crate)`) and cumulative retained bytes bounded;
   abort the fuzz run (return early / panic) when exceeded so the fuzzer finds violations.
4. Add the target to `make fuzz-all` (it currently runs nine) and to the fuzz seed corpus
   docs in `docs/SECURITY.md` if targets are listed there (check; if not, skip).
5. Run locally: `cargo fuzz run iterm_transfer -s none -- -max_total_time=120` clean.

## Files to touch

`fuzz/fuzz_targets/iterm_transfer.rs` (new), `fuzz/Cargo.toml` (target entry),
`Makefile` (`fuzz-all` list), possibly `crates/par-term-emu-core/src/terminal/file_transfer.rs`
(cfg-gated test accessor, no public API change).

## Verification

- `cargo fuzz run iterm_transfer -s none -- -max_total_time=120` exits clean
- `make fuzz-all` lists and runs ten targets without error
- `PATH="$PWD/.venv/bin:$PATH" make checkall` unaffected (fuzz targets don't build in the gate)

## Rollback

New target is additive; revert the commit removes it. If the cfg-gated accessor is unwanted,
it can land as `#[doc(hidden)] pub` instead — decide at review, both are contained.
