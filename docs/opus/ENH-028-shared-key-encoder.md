# ENH-028: Grow `keyboard::encode_key` into the single key encoder for par-term, ParDeck and the Python API

> Filed from the 2026-09-28 /opus-audit enhancement pass (cycle `audit-2026-09-28`). Board card: `[ENH-028]`.
> Sequencing: after AUDIT QA-151 (`TermKeyEvent.key: u16`) and QA-172 (astral codepoints). The par-term migration is a separate upstream card, filed once this ships.

## Goal

`src/keyboard.rs` exists so "key translation is implemented once, in the emulator". Today it covers legacy xterm sequences and kitty level-1 disambiguation. par-term still ships its own encoder, `~/Repos/par-term/par-term-input/src/key_encoding.rs` (465 lines), which covers xterm `modifyOtherKeys` modes 1/2 and the option-as-meta/ESC modes. Desktop and iOS frontends therefore encode keys differently.

Extend the core encoder to cover those modes, reading the terminal's own negotiated state, expose it to Python, and give par-term a drop-in path.

## Current state

- `src/keyboard.rs`:
  - `encode_key(ev, term)` (`:303`) selects `encode_legacy` (`:157`, honoring `term.application_cursor()`) or `encode_kitty` (`:265`, when `keyboard_flags & 1`).
  - Modifier bits follow kitty order.
  - It has no modifyOtherKeys handling and no option-key mode.
- Terminal state already tracks modifyOtherKeys: `Terminal::modify_other_keys_mode()` / `set_modify_other_keys_mode` (`src/terminal/mod.rs:2288-2293`), set by `CSI > 4 ; Pv m`.
- par-term's encoder (`par-term-input/src/key_encoding.rs`):
  - `handle_key_input_with_mode(input, modify_other_keys_mode, …)`
  - `try_modify_other_keys_encoding` (`:339`)
  - `apply_option_key_mode` (`:21`), with a per-side option mode (Normal / Meta / Esc)
  - It is called from `par-term/src/app/input_events/key_handler/*`.
- Kitty levels above 1 (report event types, alternate keys, all keys as escapes, associated text) are not implemented in core.

## Implementation

1. Read `par-term-input/src/key_encoding.rs` in full and list every behavior branch as a table (input → bytes), with its tests if any. That table is the conformance spec. Put it in `src/keyboard.rs` tests as data-driven cases.
2. Add an options struct so frontend-owned settings (not terminal state) are explicit. Mirror it `#[repr(C)]` for the FFI as `TermKeyOptions`, and add `terminal_encode_key_ex(term, ev, opts, out, cap)`. Keep `terminal_encode_key` as `_ex` with default options.
   ```rust
   #[repr(C)] #[derive(Clone, Copy, Default)]
   pub struct KeyEncodeOptions { pub left_option: u8 /* 0 normal, 1 meta(8th bit), 2 esc-prefix */, pub right_option: u8 }
   pub fn encode_key_with(ev: &TermKeyEvent, term: &Terminal, opts: &KeyEncodeOptions) -> Vec<u8>
   ```
3. Legacy path: when `term.modify_other_keys_mode() >= 1`, encode modified keys as `CSI 27 ; mods ; code ~`, following par-term's exact rules. Its Shift-only exception is noted at `:111-113`. Mode 2 extends this to keys that normally produce control characters.
4. Option handling applies only to Alt-modified text keys on macOS-style input. Encode per `opts` for the relevant side. The event must carry side information, so add a `TERM_MOD_ALT_RIGHT`-style bit. Check par-term's `KeyInput` for how side is represented, and choose a bit in the currently unused modifier range (above META=32).
5. Kitty levels 2 to 4 are out of scope for this card. Note that in the module doc, and file a follow-up if par-term needs them.
6. Python binding: `Terminal.encode_key(key: int, modifiers: int, codepoint: int = 0, left_option: int = 0, right_option: int = 0) -> bytes`, with a Google-style docstring, an API_REFERENCE entry and a stub regen.
7. Header: add `TermKeyOptions`, the `terminal_encode_key_ex` prototype and the new modifier bit, and bump `TERM_CORE_ABI_VERSION` (if ARC-063 has landed). Add FFI_GUIDE key-encoding section updates.
8. After shipping, file a par-term upstream card to replace `key_encoding.rs` with calls into `encode_key_with`. The card should include the conformance table as its acceptance test. Do not edit par-term from this repo.

## Files to touch

- `src/keyboard.rs` (encoder, options, tests), `src/ffi.rs` (`terminal_encode_key_ex`), `include/terminal_core.h`
- `src/python_bindings/terminal/` (the new binding), `python/par_term_emu_core_rust/_native.pyi` (regenerated)
- `docs/API_REFERENCE.md`, `docs/FFI_GUIDE.md`, `CHANGELOG.md` (Unreleased → Added)

## Verify

- The data-driven conformance tests derived from par-term's encoder pass. Every row in the step-1 table is a test case, and the case count is reported.
- Tests for modifyOtherKeys mode 1 and mode 2 (e.g. Ctrl+Shift+A under mode 2 gives `CSI 27;6;65~`), option-as-ESC (`ESC a`), option-as-meta (`0xE1`), and kitty level 1 unchanged.
- `uv run pytest tests -k encode_key -v` for the Python binding.
- `make xcframework` (header smoke-compile).
- `make checkall` is green.

## Rollback

All additions: `_ex` function, options struct, new binding. Reverting removes them, and `terminal_encode_key`'s behavior is unchanged for callers that pass no options.
