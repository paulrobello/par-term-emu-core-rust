# ENH-004 — Implement legacy alternate-screen modes 47, 1047, 1048

## Goal

Support the three legacy DEC private modes older full-screen applications still use:
mode 47 (plain alt screen), 1047 (alt screen, clear on exit), 1048 (save/restore cursor
only). Only 1049 is currently wired (audit finding DOC-003); apps hardcoding 47/1047
today get no screen switch at all.

## Current State

- DECSET/DECRST dispatch: `src/terminal/sequences/csi/mode.rs` — `handle_decset`
  (:103, arm list :123-140) and `handle_decrst` (:196, arms :216-229) handle 1049 only.
- 1049's implementation shows the building blocks: alt-screen grid switch, cursor
  save/restore, clear-on-enter. Read the 1049 arms first — the three legacy modes are
  recombinations of exactly those pieces:
  - **47**: switch to alt screen / back. No cursor save, no clear.
  - **1047**: switch to alt screen; on DECRST, clear the alt screen before switching back.
  - **1048**: DECSC/DECRC equivalent (save cursor on set, restore on reset) with NO
    screen switch — likely can call the existing DECSC/DECRC handlers
    (`grep -rn "save_cursor\|restore_cursor" src/terminal/`).
- Mode query (DECRQM) should report the new modes as set/reset accordingly — find the
  DECRQM handler (`grep -rn "DECRQM\|request_mode" src/terminal/sequences/`) and add
  the three modes to its reporting table.

## Implementation Steps

1. Factor the 1049 arm's internals (if not already helpers) into: `enter_alt_screen(clear: bool)`,
   `exit_alt_screen(clear_alt: bool)`, reusing existing cursor save/restore fns.
2. Add arms 47, 1047, 1048 to both `handle_decset` and `handle_decrst` per the semantics
   above. Mind interaction rules: setting 47 while already in 1049's alt screen is a
   no-op (already alt); track which mode entered the alt screen only if the existing
   state distinguishes it — xterm does not nest alt screens, and neither should this.
3. Add the three modes to DECRQM reporting.
4. Tests: for each mode — set, write text, verify alt grid content; reset, verify
   primary content intact; 1047 reset clears the alt screen (re-set 1047 shows blank);
   1048 set/reset round-trips cursor position without touching the grid; 1049 regression
   suite untouched; interaction test: 1049 set → 47 set (no-op) → 1049 reset restores.
5. Docs: VT_SEQUENCES.md mode table + VT_TECHNICAL_REFERENCE.md rows to ✅ (post
   DOC-003 cleanup); CHANGELOG.md.

## Files to Touch

- `src/terminal/sequences/csi/mode.rs` (both handlers)
- `src/terminal/mod.rs` or the screen-switch owner (helper extraction; locate the 1049
  implementation's home first)
- DECRQM handler file
- `docs/VT_SEQUENCES.md`, `docs/VT_TECHNICAL_REFERENCE.md`, `CHANGELOG.md`
- Tests: Rust inline + `tests/test_*alt*` Python coverage if an alt-screen query is exposed

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize alt_screen` — new + existing 1049 tests green
- `make dev && uv run pytest tests/ -k "alt" -v`
- Manual: `make dev`, run a PTY session, `vim` then `:q` under `TERM=xterm` variants that
  emit 1047/1048 — screen restores cleanly
- `make checkall`

## Rollback

Revert the commit. New arms only; 1049 path untouched except helper extraction (which the
1049 regression tests pin).
