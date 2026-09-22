# ENH-015 — Single DEC private-mode table with a mode-symmetry test

## Goal

Replace the four parallel `match param` blocks in `handle_decset` / `handle_decrst`
(`src/terminal/sequences/csi/mode.rs:103-205` and `:207-308`, CC 71 and 66, the byte-identical
`old_mode` snapshot at `:104-123` and `:208-227`) with one table-driven implementation, and add
a test that every DEC private mode the terminal can set it can also reset and report
(DECRQM). ENH-004 and ENH-006 each needed four coordinated edits here; the next mode should
need one. Audit ARC-009/QA-105 land the two-function extraction; this plan finishes it with the
table and the symmetry test.

## Current State (verified 2026-09-22 at 9fa2237)

- `handle_decset(param: u16)` computes `old_mode: Option<String>` via a 14-arm match, then a
  second match setting the mode `true`; `handle_decrst` mirrors it with `false`.
- Asymmetric modes: 47/1047/1049 (alt screen save/restore semantics differ on set vs reset),
  9/1000/1002/1003 and 1005/1006/1015 (mouse mode / encoding enums, not booleans), 6 (origin
  mode also homes the cursor).
- DECRQM reporting lives in `src/terminal/sequences/csi/report.rs` (`handle_csi_report`, CC 48)
  with its own list of known modes.
- Tests: `src/terminal/tests/modes.rs` covers individual modes; no test enumerates the set.

## Implementation Steps

1. `src/terminal/sequences/csi/mode.rs`:
   - `pub(crate) fn dec_mode_label(&self, param: u16) -> Option<String>` — the single copy of
     the `old_mode` table.
   - `pub(crate) fn set_dec_private_mode(&mut self, param: u16, enabled: bool)` — one match;
     boolean modes become `N => self.modes.x = enabled`; asymmetric ones keep an explicit
     `if enabled { ... } else { ... }` inside their arm (copy both existing bodies verbatim).
   - `handle_decset(p)` → `let old = self.dec_mode_label(p); self.set_dec_private_mode(p, true);
     self.emit_mode_changed(p, old, true)` (whatever the current event emission is; keep it
     identical), same for `handle_decrst`.
2. Introduce `pub(crate) const DEC_PRIVATE_MODES: &[u16] = &[1, 6, 7, 9, 25, 47, 69, 80, 1000,
   1002, 1003, 1004, 1005, 1006, 1015, 1047, 1048, 1049, 2004, 2026]` (derive the exact list
   from the arms present at implementation time) and use it from DECRQM in `report.rs` so the
   "known mode" list has one source.
3. Symmetry test in `src/terminal/tests/modes.rs`: for each `p` in `DEC_PRIVATE_MODES`, process
   `CSI ? p h`, assert `dec_mode_label(p)` reflects set (or the DECRQM reply is "set"), process
   `CSI ? p l`, assert reset; assert `dec_mode_label` returns `Some` for every entry and `None`
   for a sentinel like 9999.
4. `docs/VT_SEQUENCES.md`: no content change; confirm the DECSET table there matches
   `DEC_PRIVATE_MODES` (add a test-time doc check only if cheap: grep the doc for each number).

## Files to Touch

- `src/terminal/sequences/csi/mode.rs`, `src/terminal/sequences/csi/report.rs`,
  `src/terminal/tests/modes.rs`

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize modes` green
  including the new symmetry test.
- parsight after reindex: `calculate_cyclomatic_complexity handle_decset` and `handle_decrst`
  each below 5 (they become two-line wrappers); `set_dec_private_mode` below 30.
- `uv run pytest tests/test_terminal.py -k "mode or decset or alt_screen or mouse" -q` green.
- `make checkall` green.

## Rollback

Single-file revert for `mode.rs`; `report.rs` change is additive.
