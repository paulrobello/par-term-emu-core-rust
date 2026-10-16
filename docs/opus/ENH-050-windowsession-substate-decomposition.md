# ENH-050: Decompose render `WindowSession` into owned sub-states with explicit imports

> Filed from the 2026-10-12 /opus-audit enhancement pass (AUDIT.md "Detailed Findings → Architecture & Design": the residual `WindowSession` decomposition after ARC-127). Board card: `[ENH-050]`.
> Prior work: **ARC-127 (done)** collapsed the modal booleans into the single `Modal` enum (`render/mod.rs:739-758`). This card covers what ARC-127 left behind.
> Sequencing. Every file this card touches is also edited by audit phases 1–3, so land it **after all of them**:
> - SEC-212 (`sidebar.rs`, `renderer.rs`)
> - ARC-136 + ARC-138 (`render/mod.rs` agent arms, `sidebar.rs`, `status.rs`)
> - ARC-140 (typed sidebar rows and `RosterEntry`)
> - ARC-139 → ARC-141 → ARC-137 (`navigate.rs`, `modal.rs`, `session.rs`, `input_route.rs`)
> - QA-243 (`session.rs` `frame()` heartbeat, `renderer.rs` trace helper)
> - QA-244 (comment fixes)
>
> Then land it **before ENH-051, ENH-052 and ENH-049**. Those three assume the explicit-import layout this card creates. Within this card, run the phases in order and commit each separately.
> Line-number citations below are pinned to commit `f4161f6` (the commit that added this plan); re-locate them by symbol on later trees.

**Priority**: medium · **Estimate**: L

## Goal

`WindowSession` (`crates/par-mux/src/mux/attach/render/mod.rs:569-720`) has 29 fields. It is implemented across seven files (`mod.rs`, `session.rs`, `navigate.rs`, `modal.rs`, `input_route.rs`, `sidebar.rs`, and `renderer.rs` for `PaneRenderer`), all of which import `use super::*`. Its two hottest methods are parsight hotspots:

| Method | Hotspot score | Complexity | Churn |
|---|---|---|---|
| `pump` (`:904-1012`) | 2,288 | 22 | 104 |
| `handle_event` (`:1111-1264`) | 2,236 | 26 | 86 |

Four changes, each behaviour-preserving:
1. **Explicit imports.** Replace the six production `use super::*` globs and every `super::super::` path with explicit `use` lists.
2. **Owned sub-states.** Extract three sub-states with narrow APIs: `CursorState`, `ChromeRows` and `SidebarState`.
3. **`status_patch.rs`.** Move `handle_event`'s payload-patch arms (pane title, agent state/release/telemetry, window rename) into a new `render/status_patch.rs`.
4. **Split `pump`.** Break `pump` into named per-step methods so its body reads as the numbered step list it already documents.

## Current state (HEAD `aaf35ef`; re-locate by symbol)

- **Globs.** `grep -n "use super::\*" crates/par-mux/src/mux/attach/render/*.rs` finds:
  - `modal.rs:5`, `session.rs:5`, `navigate.rs:4`, `input_route.rs:4`, `sidebar.rs:4` and `renderer.rs:4`. These are the six production globs.
  - `geometry.rs:138`, which is a `#[cfg(test)]` module and stays.
  - `render/tests/{mod,chords,session}.rs`, which also keep their globs: `tests/mod.rs` re-exports via `use super::{input_route,modal,session}::*`.

  `mod.rs:46` has `use renderer::*;`.
- **`super::super::` paths.** `grep -c 'super::super::'` reports 83 **lines** across render (`modal.rs` 46, `navigate.rs` 22, `sidebar.rs` 9, `input_route.rs` 7, `session.rs` 3, `mod.rs` 1, `renderer.rs` 1). Some lines hold two paths. The target is **0 lines** in production render files.
- **parsight decomposition of `render/mod.rs`**: modularity 0.63, confidence high. Structural cluster 3 is `{FlushSink, run, pump, apply_pending, refresh_status, tick_flash, handle_event, EventOutcome}`. Co-change cluster 2 is `{WindowSession, new, pump, apply_pending, tick_flash, pump_stdin, route_stdin_bytes}`. Co-change cluster 0 ties `handle_event` and `refresh_status` to `PaneEmulator`. That is the evidence that the event/status code churns with emulator changes and should move out of `mod.rs`.
- **Sub-state field usage** (counted with `grep -rn "self\.<f>\b\|session\.<f>\b"` under `render/`):

  | Sub-state | Field | Uses | Files |
  |---|---|---|---|
  | Cursor | `cursor_placed` | 6 | `session.rs`, `navigate.rs` |
  | Cursor | `last_cursor_emit` | 4 | `session.rs`, `tests/mod.rs` |
  | Cursor | `drag_cursor_shape` | 5 | `navigate.rs`, `session.rs`, tests |
  | Chrome | `status_row` | 8 | `navigate.rs`, `session.rs`, `mod.rs` |
  | Chrome | `tab_strip` | 14 | `input_route`, `session`, `mod`, `navigate`, tests |
  | Chrome | `status_bar_on` | 6 | — |
  | Chrome | `flash` | 30 | — |
  | Chrome | `flash_until` | 4 | — |
  | Sidebar | `sidebar_on` | 17 | `navigate`, `session`, `sidebar`, `mod`, tests |

  `status_dirty` (44 uses) and `status` (`StatusState`) stay on `WindowSession`. They are the event/refresh coupling, and moving them is churn without a seam.
- **`handle_event` arms to move** (`mod.rs`):
  - `PaneTitleChanged` (`:1197-1220`);
  - `AgentStateChanged` and `AgentReleased` (`:1221-1237`), which ARC-136 rewrites to also rebuild the panel's agent section;
  - `AgentTelemetryChanged` (`:1240`);
  - `WindowRenamed` (`:1241-1246`).

  The layout, focus and follow arms (`:1121-1196`) stay in `mod.rs`, because they feed `PendingWork`.
- **Cursor logic** lives in `session.rs::frame()` (`:705-768`): the heartbeat condition, the trace and the `cursor_placed`/`last_cursor_emit` writes. QA-243 makes it compute `belief_changed`/`heartbeat_due` once.
- **Tests that must stay green.** All of `render/tests/` (44 + 64 + 49 = **157** `#[test]` at `aaf35ef`), plus `tests/mux_attach.rs` `render_mode_*`. Pinned cursor behaviour includes the backdated-`last_cursor_emit` heartbeat test in `tests/mod.rs` (`grep -n last_cursor_emit render/tests/mod.rs`). Tests read fields directly (`session.sidebar_on`, `session.flash`, `session.tab_strip`), so every phase updates those accesses mechanically. Do not add `pub` accessors just for tests; the sub-state fields are `pub(super)` to the render module.

## Implementation

### Phase 1: explicit imports (no logic change)
1. For each of the six production files, replace `use super::*;` with an explicit list:
   - Delete the glob. Then `cargo check -p par-mux --features mux-bin,attach --all-targets 2>&1 | grep -E "^error\[E0(412|425|433|422|599)\]"` names every unresolved item.
   - Add `use super::{WindowSession, PaneRenderer, Modal, …};` and `use crate::mux::attach::{conn::AttachConn, ManagementKey, …};` until it compiles.
   - Prefer `crate::mux::attach::…` absolute paths over `super::super::`.
2. Replace every `super::super::X` in those files with the imported `X`. The common ones are `ManagementKey`, `SidebarSection`, `parse_workspace_line`, `marked_pane`, `config::Management`, `PickerEntry`/`PickerRef` and `forward_chunked`. Likewise replace `crate::mux::attach::conn::AttachConn` in signatures with the imported `AttachConn`.
3. `mod.rs:46` `use renderer::*;` becomes an explicit list of what `mod.rs` and the siblings use from `renderer`: `StdoutSink`, `RenderOptions`, `PaneDir`, `pane_in_direction`, the style fns and `map_color`. The compiler names the rest.
4. Commit: `refactor(mux): explicit imports across render/ (ENH-050 phase 1)`.

### Phase 2: `CursorState`
5. In `render/session.rs`, or a new `render/cursor.rs` if ENH-052 has already split the renderer's cursor code (only after QA-243):
   ```rust
   pub(super) struct CursorState {
       placed: Option<Option<(u16, u16, CursorStyle)>>, // was cursor_placed, initial Some(None)
       last_emit: std::time::Instant,                   // was last_cursor_emit
       pub(super) drag_shape: bool,                     // was drag_cursor_shape (config)
   }
   impl CursorState {
       pub(super) fn new(now: Instant) -> Self;
       /// The frame's emit decision: Some(cursor) when the sink must be written.
       pub(super) fn decide(&mut self, cursor: Option<(u16, u16, CursorStyle)>, flushed: bool, now: Instant) -> bool;
       pub(super) fn invalidate(&mut self); // whatever navigate.rs/session.rs do to cursor_placed today
   }
   ```
   `decide` contains QA-243's single `belief_changed` / `heartbeat_due` computation and the trace line, and it updates `placed`/`last_emit` when it returns true.
6. `frame()` calls `if self.cursor.decide(cursor, flushed, Instant::now()) { sink.place_cursor(cursor); }`. Replace the `navigate.rs` and `session.rs` writes to `cursor_placed` with the matching `CursorState` method; read each site to name it. The heartbeat test backdates `last_emit` through a `#[cfg(test)] pub(super) fn backdate(&mut self, by: Duration)`.
7. Commit (phase 2).

### Phase 3: `ChromeRows`
8. Add the struct:
   ```rust
   pub(super) struct ChromeRows {
       pub(super) status_row: status::StatusRow,
       pub(super) tab_strip: TabStrip,
       pub(super) status_bar_on: bool,
       flash: Option<String>,
       flash_until: Option<Instant>,
   }
   impl ChromeRows {
       pub(super) fn flash(&mut self, text: impl Into<String>); // replaces every `self.flash = Some(..)`
       pub(super) fn flash_text(&self) -> Option<&str>;
       /// tick_flash's body: true when the flash cleared (caller repaints the status row).
       pub(super) fn tick(&mut self, now: Instant) -> bool;
   }
   ```
   `WindowSession::tick_flash` (`mod.rs:1096-1107`) becomes `if self.chrome.tick(now) { self.draw_status_row(); }`. `draw_status_row`, `flush_status_row`, `draw_tab_strip` and `flush_tab_strip` stay on `WindowSession`, because they read `status`, `geometry` and the renderer, but they access the rows through `self.chrome`.
9. Rewrite all 30 `flash` sites as `self.chrome.flash(..)`. Change test reads of `session.flash` to `session.chrome.flash_text()`. Keep `flash_survives_a_fast_pump_and_clears_after_its_deadline` (`tests/chords.rs:1980`) green.
10. Commit (phase 3).

### Phase 4: `SidebarState`
11. Add `pub(super) struct SidebarState { on: bool }` with `is_on()`, `toggle() -> bool`, `set(bool)`. It owns the *session-level* visibility only. The renderer still owns the sections, and `render_opts.sidebar_width` stays the configured record. Then:
    - Move `toggle_sidebar`, `refresh_sidebar`, `set_sidebar_roster`, `ws_rows_only` and ARC-136's `rebuild_agent_section` so they take `&mut self` but branch on `self.sidebar.is_on()`.
    - Leave them in `sidebar.rs`; the methods do not move files.
    - Replace all 17 `sidebar_on` sites.
12. Commit (phase 4). This phase is the smallest. If the field turns out to be the only member after ARC-136/140, keep it anyway, because the narrow API is what the card is asking for.

### Phase 5: `status_patch.rs` and the `handle_event` split
13. Create `render/status_patch.rs` (`impl WindowSession`), declared `mod status_patch;` in `mod.rs`, with:
    - `pub(super) fn on_pane_title_changed(&mut self, pane_id: &str, title: &str)`
    - `pub(super) fn on_agent_changed(&mut self, pane_id: &str, agent: &str, state: Option<&str>)`, whose body is whatever ARC-136 put in the two agent arms
    - `pub(super) fn on_window_renamed(&mut self, window_id: &str, name: &str)`

    Move each arm body verbatim. In `handle_event` each arm becomes one call plus `EventOutcome::Continue`. `AgentTelemetryChanged` stays a one-line `Continue` arm.
14. Extract the `LayoutChange` arm body into `fn on_layout_change(&mut self, layout: &str, visible: &str, flags: &str)` in `mod.rs`.
15. Commit (phase 5).

### Phase 6: split `pump`
16. `pump` keeps its loop. Each numbered step comment becomes a method returning `ControlFlow<()>` (`std::ops::ControlFlow`), or `Result<ControlFlow<()>, String>` where it calls `apply_pending`/`resize_to`:
    - `fn drain_pushes(&mut self, conn) -> ControlFlow<()>` is step 1. It breaks on `End` or a disconnect.
    - `fn pump_pending(&mut self, conn, sink) -> Result<ControlFlow<()>, String>` covers steps 2 and 4: `apply_pending` plus the `pending_end` take.
    - `fn pump_resize(&mut self, conn, sink, current: &mut (u16, u16)) -> Result<ControlFlow<()>, String>` is step 5.
    - `fn wait_next_push(&mut self, conn) -> ControlFlow<()>` is step 9's `recv_timeout(POLL)`.

    The loop body becomes about 15 lines of `?`/`if let Break` calls in the same order. Do not reorder steps. The order (drain → pending → stdin → pending → resize → status → flash → drag → frame → wait) is the pump's contract.
17. Commit (phase 6).

## Files to touch

- `crates/par-mux/src/mux/attach/render/mod.rs`, `session.rs`, `navigate.rs`, `modal.rs`, `input_route.rs`, `sidebar.rs`, `renderer.rs` (imports; field moves)
- `crates/par-mux/src/mux/attach/render/status_patch.rs` (new)
- `crates/par-mux/src/mux/attach/render/tests/{mod,chords,session}.rs` (mechanical field-path updates only)

## Verify

Attach and render tests need `--features mux-bin,attach`, because `mux-bin` alone compiles **0** of them. Run the per-phase checks after every phase commit.

- **Baseline before phase 1.** `cargo test -p par-mux --features mux-bin,attach --lib mux::attach > /tmp/enh050-base.log 2>&1; echo EXIT=$?` gives `EXIT=0`. Record N passed; N must be ≥ 157. After each phase the same command prints `EXIT=0` with the same N passed and `0 failed`.
- **Imports gone.**
  - `grep -l "^use super::\*;" crates/par-mux/src/mux/attach/render/*.rs` prints nothing.
  - `grep -c "super::super::" crates/par-mux/src/mux/attach/render/{mod,session,navigate,modal,input_route,sidebar,renderer,status_patch}.rs` prints `0` for every file. Use `grep -h ... | wc -l` to get one total of `0`.
- **Sub-states exist.** `grep -n "struct CursorState\|struct ChromeRows\|struct SidebarState" crates/par-mux/src/mux/attach/render/*.rs` prints 3 lines. `sed -n '/^struct WindowSession {/,/^}/p' crates/par-mux/src/mux/attach/render/mod.rs | grep -cE "^\s+(cursor_placed|last_cursor_emit|drag_cursor_shape|flash|flash_until|status_row|tab_strip|status_bar_on|sidebar_on):"` prints `0`. The grep is scoped to the `WindowSession` block because the new sub-state structs legitimately have fields with these names.
- **Hotspots reduced.** Record both complexities in step 0 with parsight `audit_repository` `cyclomatic_complexity` (`target: "pump"` / `"handle_event"`, `file_path: "render/mod.rs"`). At `aaf35ef` they are 22 and 26. After reindexing, `pump` is ≤ half its baseline, because its body becomes about 6 `ControlFlow` checks. `handle_event` is strictly below its baseline, because the moved arms drop their nested `let-else`/`if` branches while the ~14 match arms stay. `status_patch.rs` exists and holds the three `on_*` methods.
- `cargo test -p par-mux --features mux-bin,attach --test mux_attach render_mode_ -- --test-threads=1 > /tmp/enh050-int.log 2>&1; echo EXIT=$?` prints `EXIT=0` with nonzero `passed`.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh050-ci.log 2>&1; echo EXIT=$?` prints `EXIT=0`.

Acceptance criteria:
- Production render files contain no `use super::*;` and zero `super::super::` lines (both greps above are empty or zero).
- `CursorState`, `ChromeRows` and `SidebarState` exist, the moved fields are gone from `WindowSession`, and the payload-patch arms live in `render/status_patch.rs`.
- After reindexing, parsight reports `pump` complexity at no more than half its step-0 baseline (22 at `aaf35ef`), and `handle_event` strictly below its baseline (26).
- The render lib suite keeps its baseline count (≥ 157, 0 failed), `mux_attach render_mode_` passes with a nonzero count, and `make checkall-ci` exits 0.

## Rollback

Each phase is its own commit and is behaviour-neutral, so revert the phases in reverse order. Phase 1 (imports) is the most valuable alone and the least risky. If a later phase conflicts with an in-flight card, keep phase 1 and drop the rest.
