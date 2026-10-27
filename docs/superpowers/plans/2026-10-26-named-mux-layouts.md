# Named Mux Layouts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `save-layout` / `load-layout` / `list-layouts` / `kill-layout` control verbs plus an attach-TUI menu entry whose dialog selects (load) or saves a named whole-daemon-tree layout, with confirmation before a load replaces live panes.

**Architecture:** A layout is the existing boot-restore snapshot (`persist::PersistState` — workspaces, sessions, windows, panes with spawn command, cwd, title, agent identity, terminal content) written to `<state base>/par-mux/layouts/<name>.json`. Saving reuses `MuxTree::to_persist_state()`; loading rebuilds a fresh tree via `MuxTree::from_persist_state_with_policy` (the boot-restore path, main.rs:1667) and swaps it into the daemon's shared `Arc<Mutex<MuxTree>>`. The TUI gets one command-menu entry opening a Save…/Load… dialog; load shows a confirm step and sends `-f` only after the user confirms.

**Tech Stack:** Rust (par-mux workspace member), serde (already behind the `mux` feature), the attach TUI's existing modal/picker/prompt machinery. No new features, no Python surface, no version bump.

**Spec:** Kanban card `01a17902cf0a779190e1f82209bcb6ae` (par-term-emu-core-rust) + Paul's approval 2026-10-26: whole-tree scope; TUI menu button list entry bringing up a select/save dialog with confirmation. Decisions D1 (whole tree) and D2 (replace-with-guard) are closed.

## Global Constraints

- Whole-tree scope only: no per-workspace extraction in this plan.
- Load replaces the live tree; a handler refuses when live panes exist and `force` is false. The TUI never sends `-f` before its own confirm step.
- Layout names validate to `[A-Za-z0-9._-]+` (non-empty; `.` and `..` rejected). A rejected name never touches the filesystem.
- Reuse the persist machinery; do not fork a second snapshot format. `format_version` mismatch surfaces as the existing `PersistError::UnsupportedVersion`.
- The save path captures under the tree lock and writes off-lock via the existing `write_state`; do not hold the tree lock across file I/O beyond what `to_persist_state()` already does.
- Rust style: type annotations on new code; comments state constraints, not narration; every changed line traces to this card. CI runs `-D warnings`.
- `list-commands` output and `docs/MUX.md` must include the four verbs (the docs gates in `make checkall` fail otherwise); check whether commit 4a0460f0's "caps table" generator consumes the verb list and regenerate it if so.

## Review Focus

1. A layout name carrying `/`, `..`, or unicode → refused before any filesystem access. (Task 1: `layout_name_validation`.)
2. `load-layout` onto a tree with live panes without `-f` → refused, tree unchanged, reply names the count. (Task 2: `load_layout_refuses_live_panes_without_force`.)
3. A truncated/corrupt layout file → clean error reply, daemon stays up, no partial swap. (Task 2: `load_layout_bad_file_errors_without_swap`.)
4. An attach client with a modal open when the tree swaps under it → no panic; the modal closes or re-seeds from the `%sessions-changed` re-fit. (Task 3: modal close on sessions-changed in the render tests; final live pass.)
5. `save-layout` of the same name twice → second write overwrites, `list-layouts` shows one entry. (Task 1: `layout_save_overwrites`.)

---

### Task 1: Layout store in `persist.rs`

**Files:**
- Modify: `crates/par-mux/src/mux/persist.rs` (near `state_file_in`, ~line 1108-1135, and the in-file `#[cfg(test)]` module)
- Test: same file's test module (the file keeps its tests inline)

**Interfaces:**
- Consumes: `MuxTree::to_persist_state()` (persist.rs:573), `write_state(state, target)` (persist.rs:1277), `PersistState` serde derives, `PersistError`.
- Produces (all `pub`, all `#[must_use]` where returning values):
  - `pub fn layouts_dir_in(base: &Path) -> PathBuf` — `base/par-mux/layouts` (mirrors `state_file_in`'s ARC-017 test-seam pattern).
  - `pub fn layouts_dir() -> PathBuf` — `layouts_dir_in(&platform_state_dir())`.
  - `pub fn valid_layout_name(name: &str) -> bool` — true iff non-empty, all chars in `[A-Za-z0-9._-]`, and not `"."`/`".."`.
  - `pub fn layout_file_in(base: &Path, name: &str) -> Result<PathBuf, PersistError>` — validated name → `layouts_dir_in(base)/<name>.json`; invalid name → a new `PersistError::InvalidLayoutName { name: String }` variant (follow the existing variant style; it needs no source).
  - `pub fn save_layout_in(base: &Path, tree: &MuxTree, name: &str) -> Result<PathBuf, PersistError>` — capture via `tree.to_persist_state()`, write via `write_state`, return the file path. Overwrites silently.
  - `pub fn load_layout_in(base: &Path, name: &str) -> Result<PersistState, PersistError>` — read the file, serde-parse. Missing file → the existing missing-file error shape; parse failure → the existing read/parse error shape (mirror `load_or_quarantine`'s error mapping, WITHOUT quarantine — a bad layout file is not the daemon's boot state).
  - `pub fn list_layouts_in(base: &Path) -> Vec<String>` — sorted file stems of `*.json`; missing dir → empty vec.
  - `pub fn kill_layout_in(base: &Path, name: &str) -> Result<(), PersistError>` — delete; missing file → the missing-file error shape.

- [ ] **Step 1: Write the failing tests** (in persist.rs's test module; create temp dirs under `std::env::temp_dir()` with a process-unique suffix, as existing persist tests do):
  - `layout_name_validation`: `valid_layout_name` accepts `dev`, `work-1`, `a.b_c`; rejects `""`, `"."`, `".."`, `"a/b"`, `"../x"`, `"a b"`, `"üñî"`.
  - `layout_save_load_round_trip`: build a minimal `MuxTree` the way existing persist round-trip tests in this file do (one session, one window, one pane running a stored command), `save_layout_in`, `load_layout_in` → `PersistState` equal to the captured state; returned path is `layouts_dir_in(base)/<name>.json`.
  - `layout_save_overwrites`: save twice under one name; `list_layouts_in` returns exactly one entry.
  - `layout_list_and_kill`: save two names; list is sorted; `kill_layout_in` removes one; list reflects it; killing a missing name errors.
  - `layout_rejects_bad_name_without_io`: `save_layout_in(base, tree, "../x")` errors and created no directory under base.
- [ ] **Step 2: Run them to verify they fail** — `cargo test -p par-mux --features mux-bin,attach layout_` (expected: compile failure, functions undefined).
- [ ] **Step 3: Implement** the eight items above. Body choice that matters: `load_layout_in` uses `std::fs::read_to_string` + `serde_json::from_str`, mapping errors into `PersistError` the way `load_or_quarantine` does.
- [ ] **Step 4: Run the tests to verify they pass** — same command, expected PASS; then `cargo test -p par-mux --features mux-bin,attach persist::` to show no persist regressions.
- [ ] **Step 5: Commit** — `git add crates/par-mux/src/mux/persist.rs && git commit -m "feat(mux): named layout store in persist (save/load/list/kill)"`.

### Task 2: Verbs, dispatch handlers, live tree swap

**Files:**
- Modify: `crates/par-mux/src/mux/command/mod.rs` (`MuxCommand` enum at :111, `parse_command` at :1662, `mutates()`)
- Create: `crates/par-mux/src/mux/dispatch/layouts.rs` (add `mod layouts;` to `dispatch/mod.rs`)
- Modify: `crates/par-mux/src/mux/dispatch/mod.rs` (`route_command` group arm — it is exhaustive, so the compiler guides the arm)
- Modify: `docs/MUX.md` is NOT here (Task 4 owns docs); but `list-commands`' output source — grep `list-commands` in `crates/par-mux/src/mux/dispatch/` and add the four verbs where the others are listed.
- Test: `crates/par-mux/tests/` (follow the existing daemon integration suite's harness — the `MuxClient`-based suite that boots a daemon; see memory: replies fuse with `join("")`, so join `"\n"` and assert effects; never index `sessions()` positionally)

**Interfaces:**
- Consumes: Task 1's store functions; `Ctx` (`tree`, `clients`, `config: Option<&Mutex<EffectiveConfig>>`, `output_epoch` — dispatch/mod.rs:81); `EffectiveConfig` fields `remain_on_exit: bool` and `agent_append: AgentAppend` (config.rs:694/736); `MuxTree::from_persist_state_with_policy(&PersistState, Box<dyn PaneFactory>, bool, &AgentAppend)` (persist.rs:685); `MuxTree::factory()` (the `cmd_respawn_pane` seam, dispatch/panes.rs:634); `MuxTree::all_panes_dead()` (used by reap).
- Produces:
  - `MuxCommand::SaveLayout { name: String }`, `MuxCommand::LoadLayout { name: String, force: bool }`, `MuxCommand::ListLayouts`, `MuxCommand::KillLayout { name: String }`; verbs `save-layout`, `load-layout` (`-f`), `list-layouts`, `kill-layout`. `mutates()`: true for all but `ListLayouts`.
  - Replies: `list-layouts` body = one name per line (match `list-sessions`' body shape); the others = a one-line confirmation (`layout saved: <name>` etc.).
  - Handler group `layouts` in `route_command`.

- [ ] **Step 1: Write the failing integration tests** (in the daemon suite; each boots its own daemon on a temp socket/state dir, as the suite's existing tests do):
  - `save_load_layout_round_trip`: `new-session -n s1` (one pane running `$SHELL -c 'sleep 30'`), `save-layout t1`, then `new-session -n s2` so the tree differs, `load-layout t1 -f`, then `list-sessions` shows `s1` and not `s2`; `list-panes` of the restored pane shows the stored command.
  - `load_layout_refuses_live_panes_without_force`: with a live pane up, `load-layout t1` (no `-f`) → error reply; tree unchanged (`list-sessions` still shows the pre-load sessions).
  - `load_layout_bad_file_errors_without_swap`: write a corrupt `.json` into the layouts dir via `save-layout` of a good name then filesystem-replace the file with garbage; `load-layout` → error reply; daemon answers a follow-up `list-layouts` (alive).
  - `list_layouts_lists_and_kill_layout_removes`: `save-layout a`, `save-layout b`, `list-layouts` → `a\nb`; `kill-layout a`; `list-layouts` → `b`.
- [ ] **Step 2: Run to verify they fail** — `cargo test -p par-mux --features mux-bin,attach --test <suite> layout` (expected: compile failure on the `MuxCommand` variants).
- [ ] **Step 3: Implement** the variants + parsing + handlers. The swap is the one non-obvious body:

```rust
// cmd_load_layout, after the name/file checks:
let (remain, append, factory) = {
    let cfg = ctx.config.map(|c| c.lock());
    let guard = ctx.tree.lock();
    (cfg.as_deref().map(|c| c.remain_on_exit).unwrap_or(false),
     cfg.as_deref().map(|c| c.agent_append.clone()),
     guard.factory())
};
let append = append.unwrap_or_default();
let fresh = MuxTree::from_persist_state_with_policy(&state, factory, remain, &append)?;
*ctx.tree.lock() = fresh; // old tree's Pane Drops kill their PTY children
```

  Lock-order note: the tree lock and config lock must not nest the other way anywhere else in this handler; take config first or document the order chosen. After a successful swap, queue the same sessions-changed lifecycle notification the kill/remove cascade uses (grep the existing `%sessions-changed` emit) so attached clients re-seed; `dispatch_command`'s tail then persists the NEW tree because `succeeded` is set.
- [ ] **Step 4: Run the new tests to verify they pass**, then the whole daemon suite for regressions; then `make lint` (clippy + fmt auto-fix) scoped to the member.
- [ ] **Step 5: Commit** — `git commit -m "feat(mux): save/load/list/kill-layout control verbs with live tree swap"`.

### Task 3: Attach TUI — menu entry, layouts dialog, confirmations

**Files:**
- Modify: `crates/par-mux/src/mux/attach/render/mod.rs` (`MenuAction` at :1097 — add `Layouts`; the `Modal` enum — add a `Layouts` variant carrying the dialog state)
- Modify: `crates/par-mux/src/mux/attach/render/modal_menu.rs` (the command menu's item list + `menu_enter`/`menu_click` dispatch — wherever `MenuAction::Settings` is handled, add the `Layouts` neighbor)
- Create: `crates/par-mux/src/mux/attach/render/modal_layouts.rs` (the dialog; add `mod modal_layouts;` next to `modal_settings`)
- Modify: the help panel's command listing (grep the file listing `Settings`/keybinds help rows) — one row for the layouts dialog.
- Test: `crates/par-mux/src/mux/attach/render/tests/` (the `Harness` pattern used by `session.rs`; memory: attach tests need features `mux-bin,attach` and PTY-spawning fixtures go through `spawn_attach_with_config`)

**Interfaces:**
- Consumes: Task 2's verbs over `AttachConn` (`send_checked` / `send_forget`, input_route.rs's usage); the existing picker (`attach/picker.rs`), text prompt (`attach/text_input.rs`), and `modal_settings.rs` as the dialog precedent (panel composition, key routing, pointer routing through `input_route.rs`'s modal precedence).
- Produces:
  - `MenuAction::Layouts` in the command menu (` menu ` chip) — opens `Modal::Layouts`.
  - `Modal::Layouts` dialog states: `Choose` (rows `Save…` / `Load…`), `SavePrompt` (text input), `SaveConfirmOverwrite { name }`, `LoadPick { names, selected }`, `LoadConfirm { name }`.
  - Save flow: `SavePrompt` → (name exists? `SaveConfirmOverwrite` : send) → `save-layout <name>` → status flash + close.
  - Load flow: `LoadPick` populated from a `list-layouts` control query (empty list renders a `(no saved layouts)` row and only closes) → `LoadConfirm { name }` ("Replace current panes with layout `<name>`?") → on confirm send `load-layout <name> -f`, close, status flash. Esc backs out one level at every state.
- [ ] **Step 1: Write the failing render tests** (Harness-based, mirroring `render/tests/session.rs`'s modal tests):
  - `command_menu_offers_layouts_entry`: open the command menu → the action row list contains the layouts entry; activating it opens `Modal::Layouts` in `Choose`.
  - `layouts_save_flow_prompts_then_sends`: Choose → Save… → type `dev` → Enter → the recorded outgoing commands contain `save-layout dev`; dialog closed.
  - `layouts_save_existing_name_confirms_overwrite`: with the fake conn primed so `list-layouts`/existence check reports `dev` present (prime the dialog state directly if the Harness has no daemon), Save… → `dev` → Enter shows the overwrite confirm; confirming sends; Esc cancels without sending.
  - `layouts_load_flow_picks_confirms_sends_force`: Choose → Load… → pick `dev` → confirm shows `Replace current panes` → Enter sends `load-layout dev -f`; Esc at confirm sends nothing.
  - `layouts_modal_closes_on_sessions_changed`: with the dialog up, deliver a sessions-changed re-seed → no panic, dialog closed (Review Focus 4).
- [ ] **Step 2: Run to verify they fail** — `cargo test -p par-mux --features mux-bin,attach layouts_` (compile failure: no `MenuAction::Layouts`).
- [ ] **Step 3: Implement**, following `modal_settings.rs` for panel/key/pointer routing and `text_input.rs`/`picker.rs` for the sub-widgets. Keep every new dialog row reachable by arrows + Enter + Esc only (pointer support comes via the existing modal precedence — do not add wheel handling here; that is the separate wheel card).
- [ ] **Step 4: Run the new tests to verify they pass**, then the attach render test module; `make lint`.
- [ ] **Step 5: Commit** — `git commit -m "feat(mux): attach TUI layouts dialog with save/load confirmations"`.

### Task 4: Docs, command listing, gates

**Files:**
- Modify: `docs/MUX.md` (verbs + a short "Named layouts" section: storage path, replace semantics, the `-f` guard)
- Modify: `CHANGELOG.md` (`[Unreleased]` → `Added`)
- Modify: `list-commands` output if Task 2 did not already cover it
- Verify: `Makefile` — whether the "caps table" generator (commit 4a0460f0) consumes the verb list; regenerate if so.

- [ ] **Step 1: Docs + changelog edits** (present tense, repo style guide).
- [ ] **Step 2: `PATH="$PWD/.venv/bin:$PATH" make checkall`** — fix everything it raises (docs checks, api surface, lint) until green.
- [ ] **Step 3: `PATH="$PWD/.venv/bin:$PATH" make checkall-ci`** — the CI-parity gate (Windows cross-target check included). Expected: green.
- [ ] **Step 4: Live smoke pass** (main checkout's built binary is fine for this): boot a daemon, create two sessions, save a layout, add a third session, load the layout from the attach TUI dialog, confirm the confirm prompt, verify the tree matches the saved one and the attach client re-fit. Record what you saw.
- [ ] **Step 5: Commit** — `git commit -m "docs(mux): named layouts verbs and guide"`.

## Out of scope (recorded, not built here)

- Per-workspace layout extraction/merge (D1 closed whole-tree).
- Mouse-wheel scrolling in dialogs (board card `01a1789e1bd3…`).
- On-exit drop-to-shell (board card `01a1789e2075…`).
