# ENH-041: Cut the `PyTmuxNotification::from` boilerplate with `empty(kind)` + struct-update syntax; one type-string table for observer events

> Filed from the 2026-09-29 /opus-audit enhancement pass (cycle `audit-2026-09-29`). Board card: `[ENH-041]`.
> Sequencing:
> - **Before QA-190** (the `exit_code` field). Landing this first turns QA-190 into a one-line field add plus one arm edit, instead of 35 more `exit_code: None,` lines.
> - If QA-190 lands first, this card absorbs its field with no plan change.
> - Also touches the same file as ARC-094, so read before editing.

**Priority**: low · **Estimate**: S

## Goal

`PyTmuxNotification::from(&TmuxNotification)` spells out every field in every arm. Most of the arms' fields are `None`, so each new notification variant costs about 20 lines of copy-paste, and a new *field* (QA-190's `exit_code`) costs one line in every arm. Replace that with an `empty(kind)` constructor plus struct-update syntax, so each arm lists only the fields it sets.

**Keep the exhaustive match.** Commit 4dde9e5 ("gated every python-test compile on the two new variants") shows the missing wildcard is what turns a new `TmuxNotification` variant into a compile error in the Python layer.

For the observer side, apply the equivalent cut that actually exists there: one source of truth for the 26 event-type strings.

## Current state

- **Motivation (parsight, 14-day window).**
  - `notification::PyTmuxNotification::from` (`src/python_bindings/types/notification.rs:129-851`): CC 35, churn 9, **hotspot 315** (#7 in the repository).
  - `observer::event_fields` (`src/python_bindings/observer.rs:34-359`): CC 35, **hotspot 140**.
  - The related `TmuxNotification::notification_type` (`src/tmux_control.rs:209-246`, CC 35, hotspot 245) and `TmuxControlParser::parse_line` (CC 37, hotspot 259) are the parser side and stay as they are.
- **The actual shape of `from`.**
  - `PyTmuxNotification` has **18 fields** (`notification.rs:11-67`), 19 once QA-190 adds `exit_code`.
  - The match has **34 arms**, one per `TmuxNotification` variant.
  - Every arm is a full struct literal. The file contains **519 `: None,` lines**, and the function spans 723 lines.
  - Nothing implements `Default` for it.
- **Duplicate type strings.** Each arm hard-codes `notification_type: "<kind>".to_string()`. Diffing those 34 strings against `TmuxNotification::notification_type()` shows them **identical**, arm for arm. So `empty(notif.notification_type())` removes a second hand-kept string table as well as the `None` lines.
- **No tests.**
  - Nothing exercises the converter today. There is no `mod tests` in `notification.rs`, and no Python test calls `get_tmux_notifications`/`drain_tmux_notifications` (`src/python_bindings/terminal/mod.rs:978-1000`).
  - A refactor of 700 lines therefore needs a golden test captured *before* the change.
  - `make test-rust` runs the bindings' unit tests with `--features python-test` (`Makefile:223`).
- **`event_fields` does not have this boilerplate.**
  - It builds a `Vec<(String, EventField)>` through a local `put!` macro, and each arm pushes only its own fields. There are no struct literals to shorten, so `empty(kind)` does not apply.
  - Its real duplication is elsewhere:
    - The 26 `put!("type", EventField::Str("<kind>".to_string()))` strings are a second copy of the table in `PyTerminal::parse_event_kind` (`src/python_bindings/terminal/mod.rs:1574-1603`, string to `TerminalEventKind`). Nothing ties the two together, so a new event kind can get a type string that its subscription filter never matches.
    - Diffed at HEAD, the two 26-string sets are **identical**, and `TerminalEventKind` has exactly 26 variants. So a single `as_str()` changes no observable string on either side.
    - It has 22 `map_or(EventField::None, …)` expressions.
  - `TerminalEventKind` (`src/terminal/event.rs:263`) has no `as_str()`.
- **Cyclomatic complexity will not drop much.** An exhaustive 34-arm match keeps CC around 35 by construction. That is the intended safety property, not debt. The measurable win is lines, the `None` count, and one string table instead of two.

## Implementation

1. **Golden test first** (`src/python_bindings/types/notification.rs`, new `#[cfg(test)] mod tests`).
   - Build one instance of every `TmuxNotification` variant with distinct non-empty values in every field it carries, for example `PaneExited { exit_code: Some(3) }` and `PaneExited { exit_code: None }`.
   - Convert each and render all 18 fields with `format!("{:?}", …)` of a local tuple. `PyTmuxNotification` does not derive `Debug`, so add `#[derive(Debug)]` if pyo3 allows, or render the fields explicitly.
   - Commit the rendered lines as a `const GOLDEN: &str` captured from the **unchanged** converter, and assert equality.
   - Add a completeness guard: a `match` over a `TmuxNotification` value with no wildcard, inside the test, listing every variant the golden set covers. A new variant then fails the *test* build too, not only the converter.
2. **`empty(kind)`.**
   ```rust
   impl PyTmuxNotification {
       /// Every optional field unset; arms fill in only what their variant carries.
       fn empty(kind: &str) -> Self {
           Self { notification_type: kind.to_string(), source: None, pane_id: None, /* … all 18 … */ }
       }
   }
   ```
   It is private to the module and not a `#[pymethods]` item, so no Python API changes.
3. **Rewrite each arm** as, for example:
   ```rust
   TmuxNotification::PaneExited { pane_id, exit_code } => Self {
       pane_id: Some(pane_id.clone()),
       name: exit_code.map(|code| code.to_string()),
       ..Self::empty(kind)
   },
   ```
   with `let kind = notif.notification_type();` bound once above the `match`. Keep the match exhaustive and add no `_ =>` arm.
4. **Observer type strings.**
   - Add `impl TerminalEventKind { pub fn as_str(self) -> &'static str }` in `src/terminal/event.rs`, an exhaustive match that returns the exact strings used today (`"bell"`, `"title_changed"`, …, `"screen_cleared"`).
   - Add `pub const ALL: [TerminalEventKind; 26]`.
   - `event_fields` then emits `put!("type", EventField::Str(event.kind().as_str().to_string()))` once before its match, and each arm drops its own `type` push. Keep each arm's field order after `type` unchanged, because dict insertion order is observable to Python callers.
   - `parse_event_kind` becomes `TerminalEventKind::ALL.into_iter().find(|k| k.as_str() == kind)`.
   - Unit test: `ALL` round-trips through `as_str()` and `parse_event_kind` for all 26, and the strings match the pre-change table (copied into the test as a literal list).
5. **Do not change** the 22 `map_or(EventField::None, …)` calls beyond an optional private `opt_str`/`opt_int` helper. That is cosmetic and not required by this card.
6. Run `make stubs` from a `make dev-streaming` build and confirm `_native.pyi` has no diff (see the stub-regen note in project memory). This change must not alter the Python surface.

## Files to touch

- `src/python_bindings/types/notification.rs` (`empty`, the arms, the new tests module)
- `src/python_bindings/observer.rs` (`event_fields` type push)
- `src/python_bindings/terminal/mod.rs` (`parse_event_kind`)
- `src/terminal/event.rs` (`TerminalEventKind::as_str`, `ALL`)

## Verify

- The golden test captured from the pre-change converter passes unchanged after the rewrite: `cargo test --lib --no-default-features --features python-test notification` covers all 34 variants and all 18 fields.
- `grep -c ': None,' src/python_bindings/types/notification.rs` drops from 519 (all inside `from` today) to at most 25 (the `empty()` body plus any arm that sets a field to `None` explicitly), and `grep -c '_ =>' src/python_bindings/types/notification.rs` stays 0.
- Adding a scratch variant `Scratch` to `TmuxNotification` (local only, reverted afterwards) makes `cargo check --lib --no-default-features --features python-test` fail with a non-exhaustive-match error in `notification.rs`.
- `cargo test --lib --no-default-features --features python-test observer` passes, including the new test that all 26 `TerminalEventKind::ALL` values round-trip through `as_str()` and `parse_event_kind`. `grep -c '"type"' src/python_bindings/observer.rs` drops from 26 to 1 (the single push before the match; today's count includes 3 pushes rustfmt split across lines).
- `uv run pytest tests/test_observer.py tests/test_contextual_events.py -v` passes, and `make checkall` is green with no diff in `python/par_term_emu_core_rust/_native.pyi` after `make dev-streaming && make stubs`.

## Rollback

Revert the commit. The Python surface and output are unchanged, so no consumer is affected either way.
