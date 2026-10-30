# par-mux `select-layout` + M-1..M-5 Alt Chords — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to execute this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** tmux-style arrangement presets for par-mux: a daemon `select-layout` command (even-horizontal, even-vertical, main-horizontal, main-vertical, tiled) plus attach-client chords `prefix M-1..M-5`, with full Alt-chord support in the chord system.

**Architecture:** Daemon side, `MuxTree::apply_layout_preset` rebuilds a window's `LayoutTree` from its pane list through the existing `mutate_layout` choke point (unzoom + PTY re-fit + `%layout-change` + state save). Client side, five new `ManagementKey` variants dispatch the command; the chord system gains an Alt-aware `ChordKey` type threaded through config, both routers (render + passthrough), the settings dialog, help, and palette.

**Tech Stack:** Rust (par-mux member), tmux 3.8 (golden geometry reference, probed live), make gates.

**Spec:** The design was approved in chat (Paul, 2026-10-30): tmux verb and names, all 5 layouts, `M-1..M-5` defaults with full Alt support. The pinned decisions live in Global Constraints below.

## Global Constraints

- Verb: `select-layout [-t <target>] <even-horizontal|even-vertical|main-horizontal|main-vertical|tiled>`; unknown name → error naming the five valid names.
- `-t` required, resolved like `swap-window`'s window target (window directly; session → its active window; pane → its owner).
- Preset shapes (probed from live tmux 3.8 at 80×24; par-mux geometry is full-coverage — no tmux divider cells):
  - **even-horizontal**: single row, equal widths.
  - **even-vertical**: single column, equal heights.
  - **tiled**: row-major grid top-left fill; columns = `round(sqrt(n))`; a lone pane in the last row takes the full row width (tmux n=3: row of 2 + full-width pane; n=5: 2,2,1).
  - **main-horizontal**: main pane (first pane in layout order) on top, the rest in one row below.
  - **main-vertical**: main pane left, the rest in one column to the right.
  - **main share = 2/3** of the split extent (par-mux call; tmux's optionless default degenerates to a 1-cell minor strip).
- `select-layout` on a one-pane window: success, no-op (no error).
- `select-layout` on a zoomed window: unzooms (rides `mutate_layout`, same as every mutation).
- Chord defaults: `M-1` even-horizontal, `M-2` even-vertical, `M-3` main-horizontal, `M-4` main-vertical, `M-5` tiled (tmux convention).
- Unbound Alt chord after prefix: consumed silently (tmux parity, matches the existing unbound-chord rule). Without the prefix armed, Alt keys still forward to the pane unchanged.
- Config keys (kebab): `layout-even-horizontal`, `layout-even-vertical`, `layout-main-horizontal`, `layout-main-vertical`, `layout-tiled` under `[client]`, spelling `M-1` style; generated config treats them like `split-right`.
- Every TUI-affecting setting ships in the settings dialog (standing CLAUDE.md rule): the five chords appear in the `keys` tab, help modal, and palette.
- Docs: docs/MUX.md (attach chords section, command reference row, config keys), CHANGELOG.md `[Unreleased]` Added entry.
- Never push (standing rule); commits are local, one per task.
- Gates: `make checkall` per task; `make checkall-ci` before done.

## Review Focus

- **Zoomed window + M-5**: the user expects the zoom cleared and the whole window tiled, not a tiled-behind-zoom ghost. Test in Task 2 (`select_layout_unzooms_and_reshapes`).
- **One-pane window + any preset**: expects success, layout unchanged, no error flash. Test in Task 2 (`select_layout_one_pane_is_noop`).
- **Alt+key forwarding when prefix not armed**: M-1 typed into a pane program (e.g. a readline `M-1` binding) must reach the pane as before. Test in Task 4 (existing Alt-forwarding tests stay green; add one if none covers ESC+digit explicitly).
- **Unbound Alt chord with prefix armed**: consumed silently, not forwarded (a pane must not see prefix+ESC+7 as Alt+7). Test in Task 4.
- **Nine panes at 80×24 even-horizontal**: every pane keeps ≥1 column; no zero-width rects. Test in Task 1 (`preset_preserves_minimum_pane_size`).

---

### Task 1: `LayoutPreset` + `LayoutTree` preset builder

**Files:**
- Modify: `crates/par-mux/src/mux/layout.rs`
- Test: same file's `#[cfg(test)]` module (uses the existing `TmuxLayout::parse` helper for string round-trips)

**Interfaces:**
- Consumes: existing `LayoutTree` (`leaf`, `Split { direction, ratio, first, second }`, `pane_ids()`, `geometry(x, y, w, h)`), `SplitDirection::{Horizontal, Vertical}`.
- Produces:
  - `pub enum LayoutPreset { EvenHorizontal, EvenVertical, MainHorizontal, MainVertical, Tiled }` with `pub fn name(self) -> &'static str` and `pub fn parse(s: &str) -> Option<Self>` (kebab names, tmux spellings).
  - `pub fn for_preset(panes: &[PaneId], preset: LayoutPreset) -> LayoutTree` (leaf order in, tree out; panes non-empty).

- [ ] **Step 1: Write failing tests** in `layout.rs`'s test module: `preset_even_horizontal_is_one_row` (n=4 at 80×24: all `y==0 && h==24`, widths 20±1), `preset_even_vertical_is_one_column`, `preset_tiled_grid_shapes` (n=2 → 2 rows; n=3 → row of 2 + full-width pane; n=5 → rows of 2,2,1; n=9 → 3×3), `preset_main_horizontal_top_main_bottom_row` (main h≈16 = 2/3 of 24; minors row below, equal widths), `preset_main_vertical_left_main_right_column`, `preset_preserves_minimum_pane_size` (n=9 even-horizontal, all widths ≥1), `preset_covers_full_extent` (n=1..9 all presets: union of rects == 0,0,80,24, no overlaps — sum of areas == 1920), `preset_name_parse_round_trip`.
- [ ] **Step 2: Run** `cargo test -p par-mux --lib mux::layout` — expect FAIL (no `LayoutPreset`).
- [ ] **Step 3: Implement** `LayoutPreset` + `for_preset` in `layout.rs`. Builders:
  - Chain builder for even-*/main strips: `Split { direction, ratio: 1.0/(remaining as f32), first: pane, second: rest }`.
  - Tiled: split panes into rows per the global rule, build each row as a right-split chain (each row's lone pane stays a bare leaf so it takes the full row width), then chain rows top→bottom with `Horizontal` splits.
  - Main-*: one `Split` at 2/3; minor strip = chain (even-horizontal's minor row uses `Vertical` splits, main-vertical's column uses `Horizontal`).
- [ ] **Step 4: Run** `cargo test -p par-mux --lib mux::layout` — PASS.
- [ ] **Step 5: Commit** `feat(mux): layout preset trees for select-layout`

### Task 2: Daemon `select-layout` command

**Files:**
- Modify: `crates/par-mux/src/mux/command/mod.rs` (enum variant + verb table + mutating list), `crates/par-mux/src/mux/command/panes.rs` (parser — pane group, layout is a pane-tree concern like swap-pane)
- Modify: `crates/par-mux/src/mux/tree/layout_ops.rs` (tree op), `crates/par-mux/src/mux/dispatch/panes.rs` + `crates/par-mux/src/mux/dispatch/mod.rs` (routing + save-classification list)
- Test: `crates/par-mux/src/mux/dispatch/tests.rs`

**Interfaces:**
- Consumes: Task 1's `LayoutPreset`/`for_preset`; existing `mutate_layout` choke point; window-target resolution (`a.window("-t")`).
- Produces: `MuxCommand::SelectLayout { target: AnyTarget, preset: LayoutPreset }`; `MuxTree::apply_layout_preset(&mut self, window: WindowId, preset: LayoutPreset) -> Result<WindowId, MuxError>` (returns the window for the `%layout-change` broadcast).

- [ ] **Step 1: Write failing tests** in `dispatch/tests.rs`: `select_layout_reshapes_the_window` (window with 3 right-split panes → `select-layout -t @N tiled` → `%layout-change` names the window; geometry has a tiled shape; terminals re-fit), `select_layout_unzooms_and_reshapes` (zoom first, then select → unzoomed), `select_layout_one_pane_is_noop`, `select_layout_rejects_unknown_preset` (error names the five names), `select_layout_requires_target` (missing `-t` → parse error), `select_layout_saves_state` (appears in the structural save set).
- [ ] **Step 2: Run** `cargo test -p par-mux --lib mux::dispatch::tests::select_layout` — FAIL.
- [ ] **Step 3: Implement**: enum variant + `parse_select_layout` (target via the window resolver; preset via `LayoutPreset::parse`, error text per Global Constraints) + verb table row `("select-layout", parse_select_layout, &[])` + `apply_layout_preset` in `layout_ops.rs`:

```rust
pub fn apply_layout_preset(&mut self, window: WindowId, preset: LayoutPreset) -> Result<WindowId, MuxError> {
    self.mutate_layout(window, |w| {
        let panes = w.layout.pane_ids();
        w.layout = LayoutTree::for_preset(&panes, preset);
        Ok(())
    })?;
    Ok(window)
}
```

plus the dispatch route arm and both save/mutation-classification lists (mirror `SwapPanes`' entries).
- [ ] **Step 4: Run** the Task 2 tests — PASS. Also `cargo test -p par-mux --lib` whole-lib sanity.
- [ ] **Step 5: Commit** `feat(mux): select-layout command applies arrangement presets`

### Task 3: `ChordKey` — Alt-aware chord representation

**Files:**
- Modify: `crates/par-mux/src/mux/config.rs` (type + parser + speller + `Management` fields + `ClientSection` fields + `reload_client_chords` + gen-config `render()`)
- Test: `config.rs` tests module

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct ChordKey { pub key: u8, pub alt: bool }`
  - `pub fn parse_chord(spec: &str) -> Option<ChordKey>` — `M-x` (upper or lower x, `1`..`9`, letters, punctuation), `C-x` (existing `parse_prefix` behavior), single chars (plain). Also `M-C-x`? No — out of scope.
  - `pub fn spell_chord(c: &ChordKey) -> String` — the inverse spelling (`M-1`, `%`, `C-a`).
  - `Management` fields become `ChordKey` (existing defaults `plain()`); five new fields `layout_even_horizontal..layout_tiled` defaulting to `alt(b'1')..alt(b'5')`.
  - `ClientSection`: `layout_even_horizontal..layout_tiled: Option<String>` (config keys per Global Constraints).

- [ ] **Step 1: Write failing tests**: `parse_chord_understands_m_keys` (`M-1` → {b'1', alt}, `m-a` case-insensitive like `C-`), `parse_chord_keeps_plain_and_c_keys` (regression: `"%"`, `"C-a"`, `M-` prefix on a multi-char token → None), `spell_chord_round_trips_parse_chord`, `management_defaults_bind_m1_through_m5` (the five layout fields default to alt 1..5), `config_file_layout_keys_round_trip` (TOML `layout-even-horizontal = "M-1"` → resolved ChordKey; gen-config output spells them).
- [ ] **Step 2: Run** `cargo test -p par-mux --lib mux::config` — FAIL.
- [ ] **Step 3: Implement.** Type + parser + speller first; then change the `Management` fields to `ChordKey`. The ripple beyond `config.rs` is compile-level only and stays minimal: router comparison sites (`input_route.rs`, `attach/mod.rs`) compare `ChordKey { key: byte, alt: false }` against the fields so behavior is unchanged; Task 4 rewires them for real Alt matching. No other behavior change in this task.
- [ ] **Step 4: Run** config tests, then `cargo test -p par-mux --lib` (the attach-mod ripples must compile and all existing tests stay green) — PASS.
- [ ] **Step 5: Commit** `feat(mux): Alt-aware chord keys (ChordKey) with M-1..M-5 layout defaults`

### Task 4: Client chords — routers, both modes

**Files:**
- Modify: `crates/par-mux/src/mux/attach/mod.rs` (`ManagementKey` variants, `passthrough_management` arms, passthrough `prefix_chord` ESC arm)
- Modify: `crates/par-mux/src/mux/attach/render/navigate.rs` (`management_chord` arms + `chord_select_layout`), `crates/par-mux/src/mux/attach/render/input_route.rs` (`management_key(ChordKey)` table lookup + Alt arm in the key router), `crates/par-mux/src/mux/attach/render/mod.rs` if chord tables live there (`management.help` chip etc.)
- Test: `crates/par-mux/src/mux/attach/render/tests/session.rs` (+ tests.rs for passthrough)

**Interfaces:**
- Consumes: Task 2's `select-layout` (client sends `select-layout <name> -t <window>`), Task 3's `ChordKey`/`Management` fields.
- Produces: `ManagementKey::{LayoutEvenHorizontal, LayoutEvenVertical, LayoutMainHorizontal, LayoutMainVertical, LayoutTiled}`; render `chord_select_layout(&mut self, conn, preset)`; passthrough arms send the same command.

- [ ] **Step 1: Write failing tests** (render tests/session.rs): `m1_chord_sends_select_layout_even_horizontal` (feed prefix byte + ESC b'1' → the conn's last sent command is `select-layout even-horizontal -t @N`), one test per preset key (M-2..M-5), `unbound_alt_chord_with_prefix_armed_is_consumed` (prefix + ESC b'7' → nothing sent, nothing forwarded to the pane emulator), `alt_key_without_prefix_still_forwards` (ESC b'1' alone → pane emulator receives Alt+1, no command sent), and `m5_on_zoomed_window_unzooms_then_tiles` (zoom first, assert the `%layout-change` shows the unzoomed tiled shape).
- [ ] Step 1b: passthrough test (tests.rs): `passthrough_m5_sends_select_layout` (prefix + ESC + b'5' → `select-layout tiled -t @N` on the wire).
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement:**
  - Render router: the prefix-armed key route — a `TermKeyEvent` with ALT modifier and one ASCII byte matches `management_key(ChordKey { key, alt: true })` before the fixed table arms; unbound Alt = `Unbound` (consumed). The `management_key` table becomes `fn management_key(&self, chord: ChordKey) -> Option<ManagementKey>`.
  - `chord_select_layout` in navigate.rs mirrors `chord_swap`: `send_then(conn, &format!("select-layout {} -t {}", preset.name(), self.window), ...)` — flash on `!reply.ok`.
  - Passthrough `prefix_chord` ESC arm: when the byte after ESC is a single printable that is not `[`/`O` (not a CSI/SS3 opener) and not `<` (SGR mouse), look up `management_command(ChordKey{key, alt: true})`; bound → `passthrough_management(key)`; unbound → consumed (do not forward ESC+byte). Incomplete (ESC at end of burst): current arrow-arm behavior (tolerated race, unchanged).
  - `passthrough_management` arms: the five layout keys send `select-layout <name> -t <focused window>` (passthrough knows its window id).
- [ ] **Step 4: Run** — PASS, plus `cargo test -p par-mux --features attach --lib mux::` whole-suite.
- [ ] **Step 5: Commit** `feat(mux): prefix M-1..M-5 chords apply window layouts in both attach modes`

### Task 5: Settings dialog, help, palette

**Files:**
- Modify: `crates/par-mux/src/mux/attach/render/modal_settings.rs` (keys tab rows + chord prompt validation via `parse_chord`), `modal_help.rs`, `modal_menu.rs` (palette rows)
- Test: `crates/par-mux/src/mux/attach/render/tests/` (settings/help/palette pins, following the existing named pins like `keys_section_scrolls_and_spells_chords`)

**Interfaces:**
- Consumes: Task 3's `spell_chord`, `parse_chord`; Task 4's variants.
- Produces: UI rows for the five chords everywhere management chords are listed.

- [ ] **Step 1: Write failing tests**: `settings_keys_tab_lists_the_five_layout_chords` (the keys section's rows name all five settings keys, values spelled `M-1`..`M-5`), `settings_chord_prompt_accepts_m_keys` (committing `M-1` on a layout row parses and writes `layout-even-horizontal = "M-1"` via `set_client_key`; committing `bogus` flashes without writing), `help_and_palette_list_the_layout_chords`.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement**: keys tab rows (one per config key, label + live value spelled via `spell_chord`), chord prompt validation switched to `parse_chord` where it currently validates bytes only, help modal + palette rows.
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(mux): layout chords surface in settings, help, and palette`

### Task 6: Docs + CHANGELOG

**Files:**
- Modify: `docs/MUX.md` (attach chords paragraph near the `%`/`"` split chords, ~line 346; command reference table row near `swap-pane` ~line 983; config keys section), CHANGELOG.md `[Unreleased]` → `### Added`
- Modify: `README.md` only if its mux section enumerates chords (check; keep surgical)

- [ ] **Step 1: Implement** the doc edits (state the 5 chords, the presets, the 2/3 main-share rule vs tmux's degenerate default, rebindable via `[client] layout-*` keys and the settings dialog).
- [ ] **Step 2: Verify** per repo rule: grep the diff for every named feature (select-layout, M-1..M-5, layout-even-horizontal) present in MUX.md + CHANGELOG.
- [ ] **Step 3: Commit** `docs(mux): select-layout command and M-1..M-5 chords`

### Task 7: Full gate + e2e

**Files:**
- Modify: `crates/par-mux/tests/mux_daemon.rs` (one e2e test)

- [ ] **Step 1: Write failing e2e** `select_layout_e2e_tiled_via_control_socket` (spawn daemon harness — follow the file's existing harness; new-session, 3 splits, `--cmd select-layout tiled -t @1`, assert list-panes leaf order unchanged and geometry tiled (e.g. first row's two panes share y=0 with h < 24; third pane at y>0), then kill daemon).
- [ ] **Step 2: Run** `cargo test -p par-mux --test mux_daemon select_layout` — FAIL then PASS.
- [ ] **Step 3: Run `make checkall`** — fix everything. **Then `make checkall-ci`** (this change touches no labeled tests, but the release bar for done is the CI-parity gate).
- [ ] **Step 4: Commit** `test(mux): select-layout e2e through the control socket`

## tmux probe script (reference for Task 1 implementers)

```bash
#!/bin/bash
# Live tmux 3.8 arrangement shapes at 80x24 (run anywhere; kills only its own throwaway server).
S=/tmp/pt-golden.sock; rm -f $S
tmux -S $S -f /dev/null new-session -d -x 80 -y 24 -s g 'sleep 300'
T="tmux -S $S"
for preset in even-horizontal even-vertical main-horizontal main-vertical tiled; do
  for n in 1 2 3 4 5 6 7 8 9; do
    $T new-window -d -t g: -n w 'sleep 300'   # create BEFORE killing the prior window (exit-empty)
    W=$($T list-windows -t g: -F '#{window_name}' | tail -1)
    for ((i=1;i<n;i++)); do $T split-window -h -t g:$W.0 -p 50 -d 'sleep 300'; done
    $T select-layout -t g:$W $preset
    echo "=== $preset n=$n: $($T list-panes -t g:$W -F '#{pane_id}:#{pane_left},#{pane_top},#{pane_width}x#{pane_height}' | tr '\n' ' ')"
    $T kill-window -t g:$W
  done
done
$T kill-server
```
