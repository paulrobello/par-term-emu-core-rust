# ENH-003 — Implement XTPUSHCOLORS / XTPOPCOLORS / XTREPORTCOLORS (color palette stack)

## Goal

Implement xterm's color-stack sequences: `CSI # P` (XTPUSHCOLORS), `CSI # Q`
(XTPOPCOLORS), `CSI # R` (XTREPORTCOLORS). Currently unwired — `CSI # P` is misrouted to
DCH and `# Q` is ignored (audit findings DOC-003/QA context). Applications using
xterm's palette save/restore (e.g. some vim colorscheme plugins) currently corrupt
palette state or delete characters.

## Current State

- CSI dispatch: `src/terminal/sequences/csi/mod.rs:103` routes final byte `P` to DCH
  without checking the `#` intermediate; no `Q`/`R`-with-`#` arms.
- Palette state: the terminal holds a 256-color palette with OSC 4 set-color support —
  locate it: `grep -rn "palette" src/terminal/ src/color_utils.rs` (the OSC 4 handler
  shows the authoritative palette storage and the dynamic colors fg/bg/cursor).
- xterm semantics: the stack holds up to 10 entries; PUSH with no params pushes the full
  palette (dynamic colors + 256 palette); `CSI 1 # P` etc. select subsets — implement
  the no-param full-palette form; parameterized subsets may be a follow-up. POP on an
  empty stack is a no-op. XTREPORTCOLORS replies with the current stack depth via
  `CSI ? Pi ; Ps # R` — check invisible-island ctlseqs for the exact reply form and
  match byte-for-byte.

## Implementation Steps

1. In `csi/mod.rs`, add intermediate-aware dispatch for `#` + `P`/`Q`/`R` **before** the
   bare `P` (DCH) arm — follow the existing intermediate-dispatch idiom used for `$`/`*`
   families (and coordinate with audit fix QA-010, which adds the same idiom for `* x`).
2. Add `ColorStackState` (Vec of saved palette snapshots, capped at 10) to the Terminal's
   feature-area sub-structs (follow the ARC-001-era pattern of `pub(crate)` sub-structs
   in `src/terminal/mod.rs`).
3. Handlers in a new `src/terminal/sequences/csi/color_stack.rs` (matching the per-topic
   file layout): push clones the current palette + dynamic colors; pop restores and
   truncates; report queues the reply bytes through the same response channel other
   reports use (see `csi/report.rs` for the queuing idiom).
4. Reset behavior: RIS/DECSTR clears the stack — find the reset handlers
   (`grep -rn "fn reset" src/terminal/`) and clear there.
5. Tests: push → change palette via OSC 4 → pop → assert original color restored;
   pop-on-empty is a no-op; stack cap at 10 (11th push drops oldest or is ignored —
   match xterm: xterm ignores pushes beyond the cap... verify against xterm docs and
   state the choice in the test); DCH regression: bare `CSI P` still deletes a char.
6. Docs: docs/VT_SEQUENCES.md + docs/VT_TECHNICAL_REFERENCE.md rows flip to ✅ (after
   DOC-003's interim removal); CHANGELOG.md (Added); README What's New if user-facing
   notable.

## Files to Touch

- `src/terminal/sequences/csi/mod.rs` (dispatch)
- `src/terminal/sequences/csi/color_stack.rs` (new)
- `src/terminal/mod.rs` (state sub-struct + reset wiring)
- `docs/VT_SEQUENCES.md`, `docs/VT_TECHNICAL_REFERENCE.md`, `CHANGELOG.md`
- Tests inline + `tests/` Python-side palette check if palette is Python-queryable

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize color_stack` — new tests
- `cargo test --lib --no-default-features --features pyo3/auto-initialize dch` — DCH regression suite still green
- `make checkall`

## Rollback

Revert the commit(s). The dispatch change is additive (new intermediate arms); bare `P`
DCH behavior is untouched, so rollback restores exactly the prior misroute-with-`#`
behavior.
