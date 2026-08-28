# ENH-002 — Wire `OSC 1337;CurrentDir=` as a cwd-update alias

## Goal

Support iTerm2's `OSC 1337;CurrentDir=<path>` as an alias for the OSC 7 working-directory
update. Today the payload falls through to the iTerm2 inline-image handler and is
rejected (audit finding DOC-010); docs/VT_SEQUENCES.md:384 documents it as if supported.

## Current State

- OSC 7 (`OSC 7;file://host/path`) working-directory tracking is implemented — locate the
  handler: `grep -rn "handle_osc" src/terminal/sequences/osc/` and find the OSC 7 arm
  (likely `osc/shell.rs`, which owns shell-integration OSCs).
- OSC 1337 dispatch: `src/terminal/sequences/osc/` routes `1337;` payloads to the iTerm2
  graphics/file handler; `CurrentDir=` is not distinguished, so it errors out of the
  image path.
- iTerm2 semantics: `CurrentDir=` carries a plain absolute path (no `file://` URL
  wrapping, unlike OSC 7).

## Implementation Steps

1. In the OSC 1337 dispatch (find the entry point in `src/terminal/sequences/osc/`),
   before the image/file handling, check whether the payload starts with `CurrentDir=`.
2. If so, extract the path (everything after `=`, no URL decoding — iTerm2 sends it raw),
   and call the same state update the OSC 7 handler performs (set the tracked cwd and
   emit the same `TerminalEvent` the OSC 7 path emits, e.g. cwd-changed — read the OSC 7
   arm and reuse its tail; extract a shared `fn set_current_dir(&mut self, path)` helper
   if the logic is more than two lines).
3. Empty path after `=`: ignore (no state change) — match OSC 7's handling of malformed
   input.
4. Tests (Rust): feed `\x1b]1337;CurrentDir=/tmp/somewhere\x07`, assert the cwd getter
   reports it and the cwd-changed event fires; assert a `1337;File=...` payload still
   reaches the image path (regression); Python test via the cwd-query API if exposed.
5. Docs: docs/VT_SEQUENCES.md — change the `OSC 1337;CurrentDir=` row from unsupported
   (DOC-010's interim state) to supported; note it in CHANGELOG.md (Added).

## Files to Touch

- `src/terminal/sequences/osc/` (1337 dispatch + shared cwd helper; likely `iterm.rs`/`shell.rs`)
- `docs/VT_SEQUENCES.md`
- `tests/` (Rust inline test beside the handler; Python test if cwd is Python-visible)
- `CHANGELOG.md`

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize current_dir` — new tests pass
- Existing iTerm2 graphics tests still pass: `cargo test --lib --no-default-features --features pyo3/auto-initialize iterm`
- `make checkall`

## Rollback

Revert the commit. The change is a new early-return branch in OSC 1337 dispatch; removing
it restores the (harmless) rejection path.
