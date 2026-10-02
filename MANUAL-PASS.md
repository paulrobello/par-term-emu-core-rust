# par-mux attach — Manual Pass Checklist

Human pass over the attach client in real terminal emulators — the ship
gate's manual integration criterion. Build (from the repo root) and seed:

```sh
cargo build --release --bin par-mux --no-default-features --features mux-bin,attach
P=target/release/par-mux
$P --socket /tmp/manual-mux &                  # start a daemon
$P --socket /tmp/manual-mux new-session -s demo
$P --socket /tmp/manual-mux split-window -t %0 -h
$P --socket /tmp/manual-mux send-keys -t %0 -l 'echo LEFT'; $P --socket /tmp/manual-mux send-keys -t %0 Enter
$P --socket /tmp/manual-mux send-keys -t %1 -l 'echo RIGHT'; $P --socket /tmp/manual-mux send-keys -t %1 Enter
```

Then attach from the terminal under test:

```sh
$P attach --socket /tmp/manual-mux                        # passthrough (default)
$P attach --mode render --socket /tmp/manual-mux          # render mode
```

Per terminal, in passthrough then render mode, check:

- **Byte fidelity**: `ls --color=always` shows colors; `htop` or `vim` fills,
  scrolls, and redraws cleanly on exit; a long CJK line does not split a glyph.
- **Resize**: drag the window edge larger and smaller; panes re-divide, no
  garbage row appears, the status bar stays painted (render mode).
- **Mouse**: a click moves the focus highlight (render); in vim with
  `:set mouse=a`, clicks and wheel land inside the pane, not the host.
- **Scroll**: render mode — wheel scrolls the client's scrollback view when the
  pane does not own mouse mode; prefix `[` scrolls, `q` snaps back to live, and
  keys reach the pane afterwards. Passthrough — wheel reaches the pane.
- **Status bar** (render): `$0:demo`, the window list with `*` on the active
  window, the focused pane's title, agent chips when an agent is rostered.
- **Zoom** (render): from another terminal, `$P --socket /tmp/manual-mux
  resize-pane -t %1 -Z` collapses the view to one pane; `-Z` again restores
  the split with both panes intact.
- **Detach cleanliness**: prefix `d` in both modes; the prompt returns intact,
  colors and cursor normal, no leftover alt-screen or hidden cursor.
- **SSH**: from another machine, `ssh <host>` then run the same attach
  commands against the daemon's socket; everything above must hold identically.

## Results

| Terminal | Mode | Fidelity | Resize | Mouse | Scroll | Status bar | Zoom | Detach |
|----------|------|----------|--------|-------|--------|------------|------|--------|
| Ghostty | passthrough | | | | | n/a | n/a | |
| Ghostty | render | | | | | | | |
| iTerm2 | passthrough | | | | | n/a | n/a | |
| iTerm2 | render | | | | | | | |
| Terminal.app | passthrough | | | | | n/a | n/a | |
| Terminal.app | render | | | | | | | |
| SSH (note the host terminal) | render | | | | | | | |

Mark cells pass / fail. Under the table, note every failure: terminal, mode,
what broke, and the exact sequence.
