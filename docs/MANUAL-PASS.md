# par-mux attach — Manual Pass Checklist

Human pass over the attach client in real terminal emulators — the ship
gate's manual integration criterion. Build (from the repo root) and seed:

```sh
make mux-manual-seed
```

The target builds the attach daemon, wipes the socket's saved tree (stop +
state-file removal — no stale sessions from earlier runs), creates the
`demo` session, splits it, and prints the first pane's id.

Client control commands ride `--cmd` (the daemon's clap layer does not take
them bare). Prefer typing into the panes interactively once attached — that
is what this pass exercises. For scripted seeding, note `send-keys -l`
concatenates whitespace-separated tokens without the separator (`-l echo
LEFT` types `echoLEFT`); use the hex form for text with spaces (`-H` takes
space-separated hex byte pairs):

```sh
P=target/release/par-mux
PANE=$($P --socket /tmp/manual-mux --cmd list-panes | head -1)
$P --socket /tmp/manual-mux --cmd "send-keys -t $PANE -H 65 63 68 6f 20 4c 45 46 54"   # "echo LEFT"
$P --socket /tmp/manual-mux --cmd "send-keys -t $PANE Enter"
```

Then attach from the terminal under test:

```sh
$P attach --socket /tmp/manual-mux                        # render mode (default)
$P attach --mode passthrough --socket /tmp/manual-mux     # passthrough
```

Per terminal, in passthrough then render mode, check:

- **Byte fidelity**: `ls --color=always` shows colors; `htop` or `vim` fills,
  scrolls, and redraws cleanly on exit; a long CJK line does not split a glyph.
  CJK showing as boxed/missing glyphs in ONE host but not another is that host's
  font config, not the pipeline — the daemon grid and the paint's wide-char
  handling are pinned by tests (`wide_glyphs_readdress_the_next_cell`), so
  check the terminal's fallback first (iTerm2: Settings → Profiles → Text →
  "Use a different font for non-ASCII text"; verified 2026-10-13).
- **Resize**: drag the window edge larger and smaller; panes re-divide, no
  garbage row appears, the status bar stays painted (render mode).
- **Mouse**: a click moves the focus highlight (render); in vim with
  `:set mouse=a`, clicks and wheel land inside the pane, not the host.
  Hovering (no button) over an htop pane must do nothing — motion reaches a
  pane only when the pane requested it (1002 for drags, 1003 for hover);
  menus opening on hover is the failure this checks. Clicking the pane
  column RIGHT beside a divider (e.g. htop's last column) must reach the
  app exactly as it would in a plain terminal — no drag highlight, no
  layout resize; a drag engages only on the painted divider cell itself.
  A click on a `htop` column header must SORT exactly like the same click
  in a plain terminal — not open htop's sort-selection dialog (the
  `[Main]` tree panel). That dialog opening is the classic-dialect
  failure: htop under iTerm.app's bundled terminfo (its `TERMINFO_DIRS`
  is inherited by panes) enables only 1000, so the forwarded report must
  be spelled classic `ESC[M`, not SGR.
- **Scroll**: render mode — wheel scrolls the client's scrollback view when the
  pane does not own mouse mode; prefix `[` scrolls, `q` snaps back to live, and
  keys reach the pane afterwards. Passthrough — wheel reaches the pane.
- **Status bar** (render): `$0:demo`, the herdr-shaded tab strip — the active
  workspace/window a solid block, the rest dim with `|` separators — the
  focused pane's title. The agent roster no longer rides the bar: it lives
  in the side panel's agents section (next bullet).
- **Zoom** (render): prefix `z` zooms the focused pane to the full window —
  the pane's prompt re-lays-out at the full width (the child resized), the
  status row carries a bold ` Z ` cue; prefix `z` again restores the split,
  and selecting another pane (prefix `o`, or prefix+arrow) unzooms. The
  daemon-side `resize-pane -t %N -Z` from another terminal does the same.
- **Rename** (render): prefix `,` opens the window-rename prompt seeded with
  the current name; edit (Backspace works), Enter commits — the tab strip and
  status bar pick the new name up; Escape cancels. Prefix `$` is the same for
  the focused pane's title (check with `pane-title -t %N` from another
  terminal that Escape did NOT commit).
- **Border style** (render): prefix `B` cycles
  unicode → double (`║`) → heavy (`┃`) → ascii (`|`) → herdr — every pane
  drawing its own rounded box — repaint at once; set
  `[client] border-lines = "herdr"` (or `"double"`) and reload (`C-b C-r`)
  to check the config path.
- **Labels** (render): pane labels show in their borders by default in the
  per-pane-box modes (`pane-borders` or herdr); prefix `l` toggles them — the
  flash names off/on. The label is your prefix `$` title when set (it
  repaints within a beat of the rename), else the pane's own title.
- **Arrow navigation** (render): prefix+arrows move focus to the pane in
  that direction (side-by-side and stacked splits); at an edge the press
  is a silent no-op. Shift+arrows swap with the pane in that direction —
  the two panes exchange cells, focus following the content, a `swapped`
  flash confirming (an edge flashes `no pane in that direction` instead,
  never silent) — and resize mode (prefix `R`) reaches a divider on BOTH
  axes for panes nested under cross-orientation splits.
- **Workspace picker** (render): prefix `g` opens the ` workspaces ` modal —
  one row per workspace, the active one marked; `/` filters, j/k and arrows
  move, Enter lands on the workspace (its session's window re-seeds), a row
  click activates, esc/q dismisses.
- **Side panel** (render): the strip is up at attach by default
  (`[client] sidebar-on-launch = false` starts it hidden), so the first
  prefix `s` closes it. Prefix `s` toggles a left strip — one flat
  workspace row each (ONLY the active one a full-width inverted block),
  then the agents section: one `{workspace} · {agent} ({state})` row per
  rostered agent, grouped by workspace (id order), the focused pane's row
  inverted; clicking a workspace row lands on it, clicking an agent row
  focuses that pane (a same-window click just moves focus). Panes
  re-divide narrower around the strip with the vacated region fully
  erased (toggling off restores). While up, the top row shows the active
  workspace's label + the tabs. Rename a window, switch workspaces, or
  churn an agent from another terminal and the strip's rows follow on the
  next refresh.
- **Detach cleanliness**: prefix `d` in both modes; the prompt returns intact,
  colors and cursor normal, no leftover alt-screen or hidden cursor.
- **SSH**: from another machine, `ssh <host>` then run the same attach
  commands against the daemon's socket; everything above must hold identically.

## Results

| Terminal | Mode | Fidelity | Resize | Mouse | Scroll | Status bar | Zoom | Detach |
|----------|------|----------|--------|-------|--------|------------|------|--------|
| Ghostty | passthrough | pass | pass | pass | pass | n/a | n/a | pass |
| Ghostty | render | pass | pass | pass | pass | pass | pass | pass |
| iTerm2 | passthrough | pass | pass | pass | pass | n/a | n/a | pass |
| iTerm2 | render | pass | pass | pass | pass | pass | pass | pass |
| Terminal.app | passthrough | pass | pass | pass | pass | n/a | n/a | pass |
| Terminal.app | render | pass | pass | pass | pass | pass | pass | pass |
| SSH (note the host terminal) | render | pass | pass | pass | pass | pass | pass | pass |

Mark cells pass / fail. Under the table, note every failure: terminal, mode,
what broke, and the exact sequence.

When finished: `$P --socket /tmp/manual-mux --stop` shuts the seed daemon
down (or `kill` it — the socket lives under /tmp and dies with the process).
