# ENH-005 — Implement DECSACE (Select Attribute Change Extent)

## Goal

Implement `CSI Ps * x` (DECSACE), which controls whether DECCARA/DECRARA
(change/reverse attributes in rectangular area) operate on the full stream between two
points or strictly the rectangle. VT_TECHNICAL_REFERENCE previously (falsely) claimed
full support; audit fix QA-010 makes the sequence a parsed no-op. This enhancement makes
it real. Supersedes QA-010's no-op once landed.

## Current State

- Rectangular ops live in `src/grid/rect.rs` (per CLAUDE.md's grid layout); DECCARA /
  DECRARA handlers: `grep -rn "DECCARA\|DECRARA\|change_attributes_in_rect" src/terminal/sequences/csi/ src/grid/`.
- After QA-010, `csi/mod.rs` recognizes the `*` intermediate for final `x` and ignores it.
- DECSACE semantics (VT420): `Ps = 0 | 1` → attribute changes affect the *stream*
  (wrapping from start position to end position, like text selection); `Ps = 2` →
  strictly the rectangle. Default (0/1) is stream mode. **Check what DECCARA/DECRARA do
  today**: if they currently operate rectangle-only, the missing piece is stream mode +
  the mode switch; the audit's dispatch trace suggests rectangle behavior exists.

## Implementation Steps

1. Add an `attribute_change_extent` field (enum `Extent { Stream, Rect }`, default
   Stream per spec — but verify against xterm's actual default behavior and the
   existing DECCARA implementation's assumption; if the current implementation is
   rect-only and tests pin that, defaulting to Rect preserves behavior — decide from
   the code, state the choice in the changelog) on the terminal's rectangular-ops
   state area.
2. Replace QA-010's no-op arm with a real handler: parse `Ps` (0/1 → Stream, 2 → Rect,
   others ignored), set the field.
3. Thread the extent through DECCARA/DECRARA: in stream mode, apply attributes from
   (top,left) through (bottom,right) in reading order (full rows between the endpoints),
   matching how DECSEL-style stream ops address cells; in rect mode, current behavior.
4. Reset: RIS/DECSTR restores the default extent.
5. Tests: DECCARA with extent 2 changes only the rectangle (existing behavior pinned);
   with extent 1, a two-row span changes trailing cells of row 1 and leading cells of
   row 2 outside the rectangle columns; DECSACE parse test (replaces QA-010's no-op
   test — keep the "no spurious reply" assertion); DECRQM/state query if applicable.
6. Docs: VT_SEQUENCES.md add row; VT_TECHNICAL_REFERENCE.md flip DECSACE to ✅ with the
   extent semantics; CHANGELOG.md.

## Files to Touch

- `src/terminal/sequences/csi/mod.rs` (dispatch — builds on QA-010's arm)
- The DECCARA/DECRARA handler file (locate in step 0) + `src/grid/rect.rs`
- `src/terminal/mod.rs` (state field + reset)
- `docs/VT_SEQUENCES.md`, `docs/VT_TECHNICAL_REFERENCE.md`, `CHANGELOG.md`

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize decsace` and
  the existing `rect`/`DECCARA` tests — all green
- `make checkall`

## Rollback

Revert to QA-010's no-op arm (sequence parsed, ignored, no reply) — one-commit revert;
grid rect ops fall back to unconditional rectangle behavior.
