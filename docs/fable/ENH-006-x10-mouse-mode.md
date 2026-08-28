# ENH-006 — Wire X10 mouse mode (DECSET 9)

## Goal

Make `CSI ? 9 h` / `CSI ? 9 l` set and clear X10 mouse reporting. `MouseMode::X10`
exists (`src/mouse.rs:7`) but no DECSET arm ever sets it (audit findings DOC-003 /
QA-007), so legacy applications requesting X10 mouse get nothing. This is the oldest
mouse protocol: button-press only (no release, no motion), reported with the same
`CSI M CbCxCy` encoding as normal mode but only on press.

## Current State

- `src/mouse.rs` defines `MouseMode` including `X10`; the mouse-encoding logic already
  branches on mode — read it first: `grep -n "MouseMode" src/mouse.rs src/terminal/ -r`
  to find (a) where DECSET 1000/1002/1003 set modes (`src/terminal/sequences/csi/mode.rs`,
  the arms near the 1000-series), and (b) where events are encoded/filtered per mode.
- Check whether the encoder already handles `MouseMode::X10` (it may, since the variant
  exists) — if so, the entire change is the two DECSET/DECRST arms; if not, add the
  press-only filter: X10 reports button presses only, with no modifier encoding
  (xterm sends Cb without modifier bits in X10 mode).

## Implementation Steps

1. Add `9` arms to `handle_decset` (`MouseMode::X10`) and `handle_decrst` (clear to
   `MouseMode::None`/off — match how 1000's reset arm clears) in
   `src/terminal/sequences/csi/mode.rs`. Follow the 1000-series arm idiom exactly,
   including any mode-priority interaction (setting 9 after 1000: xterm lets the most
   recent set win — match the existing 1000/1002/1003 interplay pattern).
2. Audit the event-encoding path for X10 semantics: press events only (drop release and
   motion), coordinates 1-based with the 32-offset encoding, no modifier bits. Add the
   filter where release/motion events are gated by mode today.
3. Add 9 to DECRQM reporting if the 1000-series modes are reported there.
4. Tests: set mode 9 → press generates `CSI M` report with expected bytes; release
   generates nothing; motion generates nothing; DECRST 9 stops reporting; setting 1000
   after 9 switches semantics (release now reported). Mouse tests exist —
   `grep -rn "mouse" tests/ --include="*.py" -l` and the Rust mouse test module — extend
   them.
5. Docs: VT_TECHNICAL_REFERENCE.md mode-9 row to ✅ (after DOC-003's interim ❌);
   VT_SEQUENCES.md mode table; CHANGELOG.md.

## Files to Touch

- `src/terminal/sequences/csi/mode.rs` (two arms)
- `src/mouse.rs` / the mouse event encoder (press-only filter if absent)
- `docs/VT_SEQUENCES.md`, `docs/VT_TECHNICAL_REFERENCE.md`, `CHANGELOG.md`
- Mouse tests (Rust + Python)

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize mouse` — new + existing green
- `make dev && uv run pytest tests/ -k mouse -v`
- `make checkall`

## Rollback

Revert the commit; the variant returns to dead-but-declared, which is the current state.
Note: coordinate with audit fix QA-007, which explicitly leaves `MouseMode::X10` in
place because this plan wires it.
