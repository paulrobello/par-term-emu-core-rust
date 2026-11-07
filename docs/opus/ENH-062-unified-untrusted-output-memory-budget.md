# ENH-062 — Unified untrusted-output memory budget across graphics and file-transfer stores

## Goal

One documented, caps-table-enforced aggregate memory budget for everything untrusted PTY
output can allocate: the graphics store (Sixel/iTerm2 inline/Kitty), the iTerm2 multipart
buffer, and the file-transfer manager. Today only the kitty path has an evicting aggregate
budget (256 MiB graphics store); SEC-001's fix adds per-manager caps, but each store still
polices itself with its own constants and its own event vocabulary. This enhancement makes
"how much can one pane's output cost me" a single documented number with one telemetry event.

## Current-state context (post SEC-001 fix)

- `crates/par-term-emu-core/src/graphics/` — graphics store with aggregate byte budget and
  eviction (kitty/inline path); `/// cap:` constants feed `docs/SECURITY.md`'s generated
  caps table (`python3 scripts/gen_caps_table.py`).
- `crates/par-term-emu-core/src/terminal/file_transfer.rs` — `FileTransferManager`:
  per-transfer 50 MiB cap, 32-entry completed ring, active-transfer count cap (SEC-001).
- `crates/par-term-emu-core/src/terminal/graphics.rs` — `iterm_multipart_buffer` (one
  in-flight inline buffer) + the OSC 1337 handlers.
- Events: `TerminalEvent::InlineImageDropped { reason }` (graphics) and
  `FileTransferFailed` (transfers) — two vocabularies for "budget said no".

## Implementation steps

1. **Define the budget type** in `crates/par-term-emu-core/src/graphics/mod.rs` (or a new
   `graphics/budget.rs`): `pub struct OutputMemoryBudget { total: usize, used: AtomicUsize }`
   with `reserve(bytes) -> Option<Guard>` / `release`. Wire one instance onto `Terminal`
   state (a `pub(crate)` sub-struct in `terminal/state.rs`, per the frozen-struct rule —
   `make terminal-fields-check` baseline +1 field).
2. **Account the three stores** against it: graphics-store insertions (existing byte
   accounting, switch its internal counter to the shared budget), `active_transfers`
   accumulated `data` growth (extend `append_data`'s size check to consult the shared
   budget), and the multipart buffer's chunk vec. On `Terminal` drop / grid reset, guards
   release naturally (RAII).
3. **One event vocabulary**: add `TerminalEvent::OutputBudgetExceeded { store, bytes }`;
   keep existing per-store events for compat, emit the new one alongside. Surface through
   `poll_events()` (Python) automatically via the existing event plumbing.
4. **Document + gate**: make the total a `/// cap:` constant (`DEFAULT_OUTPUT_MEMORY_BUDGET`,
   recommend 320 MiB = today's 256 graphics + headroom) so it lands in the generated caps
   table; regenerate with `python3 scripts/gen_caps_table.py`.
5. **Tests**: unit tests per store (reservation, release on drop, eviction pressure order);
   one integration test driving kitty + iTerm2 transfers together over the total and
   asserting eviction/`OutputBudgetExceeded` ordering is deterministic (oldest-first).

## Files to touch

`crates/par-term-emu-core/src/graphics/mod.rs` (+ new `budget.rs`), `terminal/graphics.rs`,
`terminal/file_transfer.rs`, `terminal/state.rs` (one field), `terminal/mod.rs` (event enum),
`docs/SECURITY.md` (regenerated), `docs/ARCHITECTURE.md` (one paragraph in the graphics
section). Python bindings unchanged (events already flow); no API surface growth
(`api-surface-check` unaffected).

## Verification

- `cargo test -p par-term-emu-core graphics` and `file_transfer` filters
- `PATH="$PWD/.venv/bin:$PATH" make checkall` (caps-table leg green with new row;
  terminal-fields baseline updated in the same commit)
- Memory smoke: a Python script pushing 300 MiB of kitty graphics through one Terminal
  asserts resident memory stays bounded and `OutputBudgetExceeded` events arrive.

## Rollback

Single-workspace commit; the budget struct is additive. Rolling back = revert the commit —
no wire format, config, or persisted state changes. Keep SEC-001's per-manager caps
independent so they remain meaningful without the shared budget.
