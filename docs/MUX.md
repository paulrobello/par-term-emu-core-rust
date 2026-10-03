# par-mux — Terminal Multiplexer Daemon

`par-mux` is a tmux-control-mode multiplexer daemon: it owns PTY-backed panes arranged in a session/window/pane tree and serves the tmux control-mode protocol over a local socket, so control-mode clients (including `par-term-tmux` and the `MuxClient` in this crate) can attach to it in place of tmux. It also carries an agent layer: panes can report agent state over the same socket, and agent sessions survive daemon restarts.

The daemon is feature-gated (Rust `mux` feature for the library, `mux-bin` for the `par-mux` binary), optional, and independent of the Python bindings and the streaming server. The wire format's conformance oracle is the `TmuxControlParser` in `src/tmux_control.rs` — the daemon emits what that parser decodes.

> **Design decisions:** code comments cite D-numbered decisions (`D3.3`, `D5`, …) summarized one line each in [MUX_DECISIONS.md](MUX_DECISIONS.md); the full plan lives in the `par-agent-os` repository (see [par-mux.md](par-mux.md)).

## Table of Contents

- [Building and Running](#building-and-running)
- [Command Line](#command-line)
- [Client Mode](#client-mode)
- [Discovering servers](#discovering-servers)
- [Attaching from a terminal](#attaching-from-a-terminal)
- [Socket and State Paths](#socket-and-state-paths)
- [Protocol Overview](#protocol-overview)
- [Command Reference](#command-reference)
- [Notifications](#notifications)
- [Agent Hook Reports](#agent-hook-reports)
- [Host Telemetry Probe](#host-telemetry-probe)
- [Agent Scrape Tier](#agent-scrape-tier)
- [Persistence and Restart](#persistence-and-restart)
- [Pane Reaping](#pane-reaping)
- [Shutdown Semantics](#shutdown-semantics)
- [Streaming Panes to the Web](#streaming-panes-to-the-web)
- [Embedding from Rust](#embedding-from-rust)
- [Testing](#testing)
- [Module Map](#module-map)

## Building and Running

The daemon binary is not part of the default build. Build it with the `mux-bin` feature (`mux` plus the `clap` CLI parser):

```bash
cargo build --bin par-mux --no-default-features --features mux-bin
```

Run it (it prints its socket path and serves until killed — or until it empties: no sessions and no clients for 5 s, see [Shutdown Semantics](#shutdown-semantics)):

```bash
cargo run --bin par-mux --no-default-features --features mux-bin
```

The Python wheel and the default `make dev` build do not include the daemon.

At startup the daemon raises its `RLIMIT_NOFILE` soft limit toward the hard limit (Unix), logging the old and new values — an inherited launchd-style limit of 256 descriptors caps the daemon near 60 panes (~4 descriptors each), which the raise removes. An unbounded hard limit is treated as 8192.

## Command Line

The daemon parses its arguments with `clap` (`--help` and `--version` both work):

```text
par-mux <name>              Bind the default socket path for <name>
par-mux --socket <path>     Bind an explicit socket path
par-mux                     Same as par-mux default
par-mux --state-dir <dir>   Override the platform state directory the tree is persisted under
par-mux [<name>] --stop     Stop the daemon on this socket cleanly and wait for it to exit
par-mux [<name>] --restart  Stop, then serve the same socket from a detached process
par-mux [<name>] --cmd CMD  Send one control command to the running daemon and print the reply
par-mux --pane-endpoints          Give each pane its own hook-only socket (see Agent Hook Reports)
par-mux --expose-control-socket   With --pane-endpoints: also export PAR_MUX_CONTROL_SOCKET in panes
par-mux --gen-config [--force]    Write the config file with the current effective settings
```

The `[daemon]` section of `<config dir>/par-mux/config.toml` carries
defaults for the socket, state dir, and the two endpoint switches — see
[Configuration file](#configuration-file).

`--socket <path>` is what `MuxClient::connect_or_spawn_at` passes when it starts a daemon. A second daemon on a path a live server already owns is refused with "another server owns <path>"; a stale socket remnant (dead socket file, Windows marker file, or a stray regular file at the path) is reclaimed.

`--stop` and `--restart` are flags rather than subcommands — the positional `NAME` would otherwise be ambiguous with a session literally named `stop`. `--stop` sends `kill-server` to the daemon on that socket and waits (30 s bound) for the socket to stop accepting connections; "no daemon running" is reported but is not an error. The daemon unlinks the socket only AFTER its final state save is on disk, so the wait's return is also the guarantee that the save has landed — a restart racing its predecessor's save read a missing or stale state file before this ordering held. `--restart` does the same stop, then serves the same socket from a detached process — the state save the stop just completed is what it restores. Before the fork, the invocation reports on the terminal what the fresh daemon will restore: nothing (with the reason — no saved state, or the previous daemon saved an empty tree) when the restore will be empty, and where the details live. The fresh daemon's stderr is routed to `<state file>.log` beside the state file (created `0600`, appended across restarts) instead of discarded, so a startup failure — a bind error, a restore failure, an exit-when-empty — leaves evidence there after the terminal is gone. The invocation returns as soon as the daemon detaches (fork + `setsid`; stdin/stdout to `/dev/null`), so no `&` is needed and closing the terminal it was typed into leaves the daemon and its panes running. This is the routine fix after rebuilding par-mux, since clients attach to whatever daemon owns the socket and an old daemon keeps serving old code until restarted.

`--pane-endpoints` (opt-in, default off) gives every pane its own socket endpoint that accepts only hook reports — see [Agent Hook Reports](#agent-hook-reports). `--expose-control-socket` accompanies it: panes then also get `PAR_MUX_CONTROL_SOCKET` naming the full control socket, and a session env of `PAR_MUX_CONTROL=1` grants the same per session.

### Nested daemons

Serve mode refuses to start inside a par-mux pane: when `PAR_MUX_ENV` is set (the pane env contract marks every pane), `par-mux <name>` / `par-mux --socket <path>` exits non-zero with `refusing to start a nested daemon: PAR_MUX_ENV is set … set PAR_MUX_ALLOW_NESTED=1 to override` and binds no socket — a nested daemon would shadow the outer server's identity for every PTY under it. `MuxClient::connect_or_spawn` applies the same rule: from inside a pane it attaches to a daemon that is already running but refuses to auto-spawn one, failing fast instead of retrying a socket the guard keeps unbound.

Exempt from the guard, because a pane must keep operating on its own daemon: `--cmd` (client mode never starts a daemon), `--stop`, and `--restart` — tmux likewise allows `kill-server` from inside a session. `PAR_MUX_ALLOW_NESTED=1` (or unsetting `PAR_MUX_ENV`) starts a nested daemon anyway.

Panes never inherit a stale outer identity: a PTY spawned by any process inside a mux pane drops every `PAR_MUX_*` variable from its inherited environment and re-adds only its own via the env contract below — so a par-term started in a pane gets clean local tabs instead of reporting its agents to the outer daemon under the wrong pane id. The same drop removes the outer agent session's identity vars (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_MESSAGING_TOKEN`, `OMPCODE`, `CODEX_THREAD_ID` — herdr parity): a pane is not a child agent of whatever started the daemon, and nested-session detection keyed on these vars (omp treats `OMPCODE=1` as nested and never reports) would hide the pane's own agents from rosters. `set-environment`/`new-session -e` (or any explicit `set_env`) opts back in for an intentional child session.

## Client Mode

`par-mux --cmd '<command>'` (short form `-c`) connects to the daemon that owns the socket, sends one control command, prints the reply, and exits. It drives panes from a shell or script without linking `MuxClient`. The target socket resolves exactly as the daemon's does: the positional `<name>` (default `default`) maps to the named default path, and `--socket <path>` overrides it. Like `--stop`, it is a flag rather than a subcommand so the positional name stays unambiguous.

```bash
# Type a command into pane %3 and press Enter (send-keys appends nothing; Enter is a key)
par-mux --cmd 'send-keys -t %3 "ls -la" Enter'

# Capture a pane's visible screen, or 200 lines of history plus the screen
par-mux --cmd 'capture-pane -t %3'
par-mux --cmd 'capture-pane -t %3 -S -200'

# The agent roster, and a split of pane %3
par-mux --cmd list-agents
par-mux --cmd 'split-window -h -t %3'

# A named daemon, an explicit socket, or (inside a pane) the socket the pane spawned with
par-mux work --cmd list-panes
par-mux --socket /tmp/par-mux-test.sock --cmd version
par-mux --cmd list-sessions   # resolves $PAR_MUX_SOCKET before the default socket
```

| Outcome | stdout | stderr | Exit code |
|---------|--------|--------|-----------|
| Success (`%end`) | The reply body, one line per reply line, framing stripped; nothing for an empty reply | — | 0 |
| Command failed (`%error`) | — | The daemon's error text | 1 |
| No daemon owns the socket | — | `no daemon running on <path>` | 1 |
| Transport failure after connecting (no reply block, connection closed) | — | The I/O error | 2 |
| Target is a pane endpoint (`--pane-endpoints` daemon) | — | `this pane has hook-only access; start the daemon with --expose-control-socket or pass --socket` | 1 |

Client mode never starts a daemon: a bare `par-mux --cmd list-sessions` with nothing running fails immediately instead of booting one. The command string follows the daemon's grammar (see the quoting rules under [Command Reference](#command-reference)); wrap it in single quotes in the shell so its inner double quotes reach the daemon intact. One command per invocation — there is no `;` sequencing, interactive attach, or follow mode. Pushed notifications that arrive while the reply is pending are discarded. Stdout writes stop quietly on a closed pipe, so `par-mux --cmd '...' | head -1` does not panic.

### Discovering servers

`par-mux --list-servers` enumerates every par-mux daemon this user can reach and prints one line per LIVE server (`<name-or-path>  sessions=N  <build stamp>`), so `attach` has something to enumerate:

```bash
par-mux --list-servers
# work  sessions=2  0.58.1+a1b2c3d
# /tmp/scratch.sock  sessions=0  0.58.1+a1b2c3d
```

Each daemon writes one pointer file into its state directory at bind (`<state-base>/servers/<socket-stem>.json`: socket path, pid, start time, build stamp) and removes it on the clean shutdown path (`kill-server`/`--stop`). A crash leaves the file behind; the next list-servers run probes every registered socket with `version` + `list-servers` through the same transport a client uses, prunes the entries whose probe fails (the pruning is reported on stderr), and — when the registry base is the platform default — also sweeps the named-default socket directory, so named daemons an older binary never registered still appear. `--list-servers` honors `--state-dir`, which scopes both the registry and the sweep to that base (what tests and sandboxes use). Registry failure never blocks a daemon from serving; discovery then simply falls back to the directory sweep.

`par-mux attach` with no NAME and no `--socket` while MORE THAN ONE server is live refuses with the same list plus a one-line hint (`attach to one with par-mux attach <name|path>`, exit 1) instead of silently picking one; exactly one live server attaches to it directly.

## Attaching from a terminal

`par-mux attach` (feature `attach`; `par-mux attach [-t TARGET] [--prefix KEY] [NAME | --socket PATH]`) is the built-in client: one pane fullscreen in your terminal, the host terminal acting as the pane's VT emulator — a byte pump plus key pump plus a one-row status line, tmux's attach model minus the window chrome. Build with `--features mux-bin,attach`.

```bash
par-mux attach                      # newest session's newest pane
par-mux attach -t %3                # that pane
par-mux attach -t @1                # that window's active pane
par-mux attach -t work              # pane title / window name / session name, daemon-resolved
par-mux attach work --prefix C-a    # named daemon, custom prefix
```

**Startup:** the handshake runs in the client-contract order (`version`, `list-commands`, `set-client-colors`, the `refresh-client -C WxH -p WxH` size report), the target resolves to a pane (`%N` directly; a window/session target narrows through the targeted `list-panes`/`list-windows` queries to the marked pane; no `-t` takes the highest pane id (ids are monotonic, so the max is the newest pane)), and the pane's screen-restore replay goes to your terminal verbatim. The daemon then pushes `%output` for that pane as it arrives, octal-decoded to raw bytes — mouse tracking, bracketed paste, and every graphics protocol work for free in passthrough: the pane's escape bytes flow to the host emulator, and the host's reports flow back on stdin.

**Keys:** stdin bytes are forwarded to the pane in chunked `send-keys -H` (~512 bytes per command). The prefix (default `C-b`, tmux's; `[client] prefix` in the config file) is intercepted: `d` detaches; `o` and the arrow keys cycle panes; `n`/`p` move to the next/previous window; `(`/`)` move to the previous/next session; `%` splits the focused pane right and `"` splits it down (`split-window -t <focused> [-h]` — the client lands on the fresh split through the same resync every switch uses); `x` kills the focused pane (`kill-pane -t <focused>` — the client follows the window's surviving pane, and killing the last pane shows the exit cue instead); `c` opens a new window in the focused pane's session and lands on it; `{`/`}` swap the focused pane with its layout-order neighbor (`swap-pane -s <focused> -t <neighbor>`); `R` enters a sticky resize mode — arrows move the focused pane's edges (`resize-pane -t <focused> -L|-R|-U|-D <step>`, the wire's relative form) until Enter, Escape, or `q` exits, and any other key leaves the mode and is reprocessed normally; `?` prints the bindings panel as plain text (every chord at its effective binding — see the render-mode help bullet); `r` respawns the pane — but only when it is held dead (a live pane restart from a mistyped chord would be destructive, and the daemon refuses `respawn-pane` of a running pane anyway). `C-b C-b` sends a literal `C-b` to the pane. The reload chord (default `C-b C-r`, `[client] reload`) re-reads the config file mid-session: prefix and reload rebind live, `config reloaded` (or the error) flashes on the status row, and `reload-config` goes to the daemon — see [Configuration file](#configuration-file). When the focused pane is held dead, plain stdin bytes are dropped instead of forwarded — the daemon answers every `send-keys` to a dead pane with an error and the bytes would only echo into the pane's frozen screen — so the client stays quiet and the status cue explains why; prefix chords keep working. Every switch is daemon-side `select-*` followed by a `refresh-client` resync, so the redraw always shows the daemon's authoritative screen. The management chords (`%`, `"`, `x`, `c`) are settable per chord in the config file (see [Configuration file](#configuration-file)).

**Status line:** the bottom terminal row is reserved with DECSTBM (the scroll region shrinks to rows 1..N-1, so pane output scrolls above it), showing session name, pane title, the pane's agent count, and, over a held-dead pane, an `(exited N — C-b r respawns)` cue naming the restart chord. The whole view runs in the host's alternate screen (`ESC[?1049h` at startup, left on every exit path), so the replay never overwrites the content the terminal was already showing. The draw also keeps a shadow emulator of the pane (the same core `Terminal` the render mode feeds, seeded by the replay and every `%output` chunk), and every draw closes by placing the cursor ABSOLUTELY at the emulator's tracked cell (`ESC[<row>;<col>H`) instead of ending on the save/restore wrap alone — pane output flowing while a draw runs once raced the restore (a scroll landing between save and restore put the cursor one line off, so later output painted over the wrong row), and a fresh absolute position cannot be made wrong by a scroll that lands after it.

**Exit paths:** prefix `d` detaches and exits 0; the daemon's `%exit` (graceful shutdown, `--stop`) reports "daemon exited" and exits 0; a closed socket (the daemon died, or the eviction policy disconnected the client) triggers one reconnect-and-requery — a fresh handshake plus a resync of the same pane — and exits 0 when that succeeds or the daemon is truly gone. The terminal is restored (raw mode off, scroll region reset) on every path, including errors: attach never leaves a terminal wedged.

**Phase B foundation (library API, feature `attach`):** alongside the passthrough client, the attach module now carries the pieces the full TUI later phases build on. The tmux layout-string parser (`mux::attach::layout::parse_layout` / `parse_layout_triple`) maps a `%layout-change` triple to absolute per-pane rects, honoring the `Z` flag by rendering the *visible* layout — the zoomed pane alone — and pinning the same leaf order `list-panes -t` reports and `LayoutTree::render` emits (golden tests over real-tmux strings plus round-trips against the daemon's own renderer). The pane renderer (`mux::attach::render::PaneRenderer`) mirrors every visible pane in its own core `Terminal` fed by the `refresh-client -t` replays and `%output` bytes, maps cells to a ratatui `Buffer` (truecolor passthrough, wide-char spacers, combining marks), draws dividers with a focused-pane highlight (UTF-8 box drawing, ACS fallback), and damage-diffs frames at a 16 ms cadence so an output flood between two frames collapses into one diff. Passthrough stays the default (`run`); `run_with_mode(AttachMode::Render)` selects the renderer seam. The CLI still attaches in passthrough; the TUI wiring is the next Phase B card.

**Phase B render mode (`--mode render`):** `par-mux attach --mode render` selects the pane renderer instead of passthrough (the default stays passthrough; an unknown `--mode` value falls back to it). With no `-t`, the target resolves to the newest session's (highest id — ids are monotonic) active window's active pane — the same default passthrough documents. The client enters the alternate screen, mirrors every visible pane of the window in its own core emulator, and paints the layout at 16 ms frame cadence. The host terminal captures the mouse (SGR + button-event tracking), and stdin is routed rather than forwarded:

- **Keys:** an incremental parser tokenizes the raw stdin stream; plain byte runs forward verbatim behind the prefix scan (prefix `d` detaches, `C-b C-b` sends the literal), while escape-sequence keys (arrows, Home/End, tilde function keys, SS3 forms, modifier chords) are decoded and re-encoded against the FOCUSED pane's tracked input state — DECCKM application cursor keys, kitty keyboard flags, modifyOtherKeys — exactly what the pane's replay and `%output` set (the shared `keyboard::encode_key` encoder, the same one the daemon's `send-keys` uses). A pane running vim gets `ESC O A` for Up; a plain shell gets `ESC [ A`. Full kitty keyboard protocol forwarding (negotiating flags on a pane's behalf, CSI u re-encoding) is a later Phase B card.
- **Mouse:** SGR reports from the host are located in the layout rects. A click on the TAB STRIP (top row) switches to that window (see below); a click in the content area focuses the pane under the pointer — locally (divider highlight) and daemon-side (`select-pane`) — and forwards a pane-relative SGR report when that pane owns mouse tracking (its emulator tracked DECSET 1000/1002/1003 from the pane's own bytes). Drag and release forward the same path; a pane without mouse ownership consumes nothing and its wheel scrolls the client.
- **Wheel:** over a pane that does NOT own mouse tracking, the wheel scrolls the CLIENT's view of that pane into its scrollback (3 lines per notch, clamped to history; the rect's top rows show the newest history lines and the live screen shifts down; any forwarded keystroke snaps back to live). Over a pane that DOES own it, the wheel forwards pane-relative like any other mouse event.
- **Status bar (bottom row):** the window renders into the rows between the tab strip and a reserved bottom row carrying `sessions | windows | pane title | agent chips` — every session as `$N:name` (the shown one bold), the shown session's windows as `N:name` (the active one bold and `*`-marked), the focused pane's title, and one `agent:state` chip per roster entry from `list-agents`. The state is the throttled re-query the notifications teach: `%agent-state-changed`, `%agent-telemetry-changed`, and `%sessions-changed` (plus renames) mark it stale, and one re-query per event burst repaints the row — a burst of agent churn costs one `list-sessions`/`list-windows`/`pane-title`/`list-agents` round. When the shown window's session is gone (`%sessions-changed` after a kill), the view ends cleanly — the daemon contract's rule. While the scroll viewport is up, a bold `[scroll +N]` cue reports the offset. A host resize repaints the row with the content (the same redraw discipline the pane frames follow, so a repaint flood cannot leave it blank).
- **Tab strip (top row):** one tab per window of the shown session — `N:name` in window order, the active tab in the theme accent (indexed 14, bold), the inactive tabs dim. The strip reads the SAME queried state as the status bar (the `list-windows -t` half of the throttled re-query), so window add/close/rename refreshes it with the status row. Truncation keeps the active tab always visible (the par-term tab-bar rule): full names when they fit, otherwise every name truncated to the widest shared budget with an ellipsis, otherwise a contiguous run of tabs containing the active one with `…` markers at the hidden edges. A click on a tab switches to that window through the same daemon-side `select-window` + resync every switch uses — a strip click never focuses or forwards into a pane, and drags on the strip row do nothing. The content grid (what `refresh-client -C` reports and what clicks/wheels/cursor placement map through) is the host grid minus the strip row minus the status row.
- **prefix [ (scroll mode):** a keyboard viewport over the FOCUSED pane's client scrollback. The viewport starts one window up from live (clamped to the history extent), holds against pane output (new lines append below without snapping the view), and moves with Up/Down (a line), PgUp/PgDn (a page), Home (the top of history); q, Enter, or End exit and snap back to live. Keys never reach the pane while the viewport is up. This is NOT tmux copy-mode: no search, no copy, no selections.
- **Management chords (`%`, `"`, `x`, `c`, `{`/`}`):** the same daemon commands the passthrough chords send, mirrored onto the render view: `%`/`"` split the FOCUSED pane (`split-window -t <focused> [-h]`), focus the fresh split (the reply body is its id), and re-seed the window from the layout the split broadcast — the new pane renders at its rect immediately; `x` kills the focused pane and lands the view on the window's surviving active pane (killing the last pane ends the view through the session-gone contract); `c` opens a window in the shown session and re-seeds onto it; `{`/`}` swap the focused pane with its layout-order neighbor (`swap-pane -s <focused> -t <neighbor>`; the swap's `%layout-change` re-seeds the window). All rebind live on the reload chord, settable as `[client] split-right` / `split-down` / `kill-pane` / `new-window` / `swap-prev` / `swap-next`.
- **Pane resize:** prefix `R` (settable as `[client] resize`) enters a sticky resize mode — arrows send `resize-pane -t <focused> -L|-R|-U|-D <step>` at the configured step (`[client] resize-step`, default 1) until Enter/Escape/`q` exits and any other key leaves the mode. Render mode adds the mouse: a press within one cell of a divider starts a drag (it never focuses or forwards — a divider press is not a click-through), motion maps the pointer's cell delta to the same relative `resize-pane` aimed at the boundary's left/top pane, applied one cell at a time at the frame cadence, with the dragged divider drawn reversed while the drag lasts; a release that never moved falls through as a click (the pane under the pointer still focuses, and the release forwards when it owns the mouse). The prefix chords work in passthrough too (the wire command only; the daemon's repaint rides the pane's `%output` stream).
- **Help panel (`?`):** prefix `?` (settable as `[client] help`) opens the bindings modal — a themed modal (every cell in the resolved host background, rounded box-drawing ring in the accent color) titled `keybinds` with an `esc close` badge embedded in the top border and a footer line `search / · scroll j/k/arrows/pgup/pgdn · close esc/enter`. Entries group into category headers (`global`, `panes`, `windows/sessions`, `navigation`, `mouse`), each row showing the chord at its EFFECTIVE binding (a remapped chord shows its remapped key, from the live config). the filter line is ALWAYS visible (placeholder when inactive, a live ` /query▌` cursor the moment `/` opens it; Backspace edits, Enter commits the filter), j/k and the arrows and PgUp/PgDn scroll the content window when it overflows, and esc/Enter/q close — the prior frame repaints. Keys AND the mouse never reach the pane while the panel is up: wheels scroll the panel. Passthrough prints the same category rows as plain text (no overlay surface; the pane's next output redraws over them).
- **Session/window switching:** prefix `o` and the arrows cycle panes; prefix `n`/`p` move to the next/previous window of the shown session; prefix `(`/`)` move to the previous/next session — every switch is the passthrough dispatch semantics (daemon-side select, then the renderer re-seeds from a fresh layout report and per-pane replays; the status bar re-queries for the new view).
- **Cursor:** each frame places the host cursor at the FOCUSED pane's tracked cell — the pane emulator's cursor position mapped through the pane's rect origin and the client scroll offset — with the pane's tracked DECSCUSR shape (`CSI Ps SP q`) and DECTCEM visibility honored: a pane that hid its cursor (`CSI ?25l`) keeps it hidden, and a scrolled-off-live view (wheel or prefix-[) hides the host cursor entirely, since the live cell is not on screen. A quiet pump re-emits nothing; a flushed frame always repositions (the diff's per-cell CUPs moved the host cursor).
- **Focus indicator:** each divider boundary names its side: bright cyan (indexed 14) when the focused pane is the boundary's LEFT/TOP pane, bright magenta (indexed 13) when it is the RIGHT/BOTTOM pane, dim when the boundary does not touch the focus. In a two-pane split both panes share one divider, so a single accent read identically from either side — the side-named colors make a focus switch flip the shared divider's color visibly.
- **Background fill:** every frame cell carries the host terminal's resolved background before painting, so unwritten cells (rows below the layout, short history lines, wide-char spacers) render as background instead of the terminal default (the light-grey bands a dark-theme host showed). The color comes from an OSC 11 probe (`ESC ] 11 ; ? ST`, 150 ms deadline, poll(2)-based) run right after raw mode comes up and before the pump's stdin reader starts; black stays the fallback (Windows and non-xtermish hosts), and a corrected `set-client-colors` rides the probe so the daemon's theme record follows the host.
- **Limitations:** forward-only mouse re-encoding (always SGR spelling regardless of the pane's negotiated legacy encoding) and wheel scrollback is view-only (no pane resize probe). A host resize re-fits the render client end to end (the new grid is reported with the content height, the daemon re-divides, panes re-seed, the status row repaints at the new bottom row — pinned by `render_mode_host_resize_refits_window_layout_and_status_row`), and a daemon-side zoom (`resize-pane -Z`) re-renders the visible layout — the zoomed pane alone — with `-Z` again restoring the split (`render_mode_daemon_zoom_shows_single_pane_and_unzoom_restores_split`); both in `tests/mux_attach.rs`. MANUAL-PASS.md at the repo root carries the human checklist for Ghostty / iTerm2 / Terminal.app / SSH.

## Socket and State Paths

### Configuration file

`par-mux` reads `<config dir>/par-mux/config.toml` for defaults the CLI
flags then override — resolution precedence everywhere is **flags > env >
file > built-in defaults**. The location is `$PAR_MUX_CONFIG` when set,
else `dirs::config_dir()/par-mux/config.toml` (on macOS
`~/Library/Application Support/par-mux/config.toml`, on Linux
`~/.config/par-mux/config.toml`, on Windows `%APPDATA%\par-mux\config.toml`).
A file that exists but does not parse is a startup WARNING with defaults
applied (unreadable config never blocks the daemon, the same rule the
state file follows), and an `%error` on `reload-config` (see below). The
file is optional — with none, everything behaves exactly as before.

```toml
[client]
prefix = "C-b"        # the attach detach prefix (tmux spelling)
mode = "passthrough"  # default attach mode: passthrough | render
reload = "C-b C-r"    # the chord that reloads this file mid-attach
split-right = "%"     # split the focused pane right
split-down = "\""    # split the focused pane down
kill-pane = "x"       # kill the focused pane
new-window = "c"      # new window in the focused pane's session
swap-prev = "{"       # swap the focused pane with the previous pane
swap-next = "}"       # swap the focused pane with the next pane
resize = "R"          # enter the sticky resize mode (arrows move the edge)
resize-step = 1       # cells per resize step (arrows and divider drag)
help = "?"            # the bindings help panel
pane-borders = false       # render mode: full ring per pane (focused accent) instead of shared dividers
show-label-in-border = false  # with pane-borders: the pane's title embedded in its top edge

[daemon]
socket = "default"    # named default socket, or an absolute path
state-dir = ""        # empty = the OS default state directory
pane-endpoints = false
expose-control-socket = false
```

Every setting maps to an existing flag or resolution rule: `[client]` keys
feed `attach` (`--prefix`, `--mode` — a config `mode` applies only when
`--mode` is absent, and an unknown value falls back to passthrough); the
`[daemon] socket` accepts a NAME (as `par-mux <name>` spells it) or an
absolute path, and sits in the same precedence chain as the env/flag tiers
of `resolve_socket_path` (`$PAR_MUX_SOCKET` still beats the file; an
explicit `--socket`/positional still beats the env). Reload semantics per
setting:

| Setting | Reload behavior |
|---------|-----------------|
| `client.prefix` | **Live** — the running attach rebinds its prefix immediately (the old prefix stops intercepting the same keystroke). |
| `client.reload` | **Live** — it rebinds itself: the NEXT reload follows the new chord. The chord's key is matched by byte, so the default `C-r` (0x12) never collides with prefix `r` (respawn, the literal 0x72). |
| `client.split-right` / `split-down` / `kill-pane` / `new-window` / `swap-prev` / `swap-next` / `resize` / `help` | **Live** — the running attach rebinds these chords immediately (parsed with the same prefix grammar as `prefix`; defaults `%`, `"`, `x`, `c`, `{`, `}`, `R`, `?`). |
| `client.resize-step` | **Live** — cells per resize step; a file value below 1 clamps to 1. |
| `client.mode` | Startup-only — changing it needs a re-attach. |
| `daemon.socket` | Startup-only by definition (the socket is bound at start). |
| `daemon.pane-endpoints` / `daemon.expose-control-socket` | Startup-only in v1 (reported as restart-required). |
| `daemon.state-dir` | Startup-only — the daemon resolves the persist path once at startup and the persist worker writes to that fixed path. |

There are three reload entry points:

- **Control command `reload-config`** (no arguments — it always re-reads
  the canonical path): the daemon re-reads the file and replies one line
  per `[daemon]` setting, `unchanged: <name>` or
  `restart-required: <name>`. The file speaks only for settings it
  actually states — a setting absent from the file is `unchanged`, which
  is what keeps a daemon started with one-shot `--socket`/`--state-dir`
  flags quiet. A PRESENT file that fails to parse is a `%error` (the
  daemon never silently ignores a config edit). A server without an
  applied config (embedded) answers `%error` too.
- **Client chord** (default `C-b C-r`, settable as `[client] reload`):
  the attach client (passthrough AND render mode) re-reads the file,
  rebinds prefix/reload live, flashes `config reloaded` (or the error) on
  the status row, and sends `reload-config` to the daemon so the
  daemon-side report rides the same keypress.
- **`par-mux --gen-config`** writes the config file with the CURRENT
  EFFECTIVE values (file if present > env > built-in defaults; startup
  CLI flags are not introspectable after parse, so they do not appear —
  except where the generating invocation's flag tier is visible to the
  resolution, i.e. the socket and the daemon bools of the generating
  run). Never overwrites an existing file without `--force`. The
  location follows `$PAR_MUX_CONFIG` / the platform config dir.

`reload-config` mutates nothing — it is the report of what WOULD change
plus the client-side live rebinds above; a restart (`par-mux --restart`)
applies restart-required daemon settings.

| Item | Unix | Windows |
|------|------|---------|
| Transport | Unix domain socket at `<base>/par-mux-<name>.sock` | Named pipe; a marker file is written at the same path so staleness checks work uniformly |
| Default socket base | `$XDG_RUNTIME_DIR`, falling back to `<tmp>/par-mux-<uid>/` | Per-user temp dir |
| Access control | Socket file mode `0600` (owner only); the fallback directory is mode `0700` and its owner and mode are verified before bind and connect; accepted connections are refused unless the peer runs as the daemon's user | Owner-only security descriptor (system + creating user, nothing for anyone else) |

The fallback-directory guard is tmux's `/tmp/tmux-<uid>` defense: `/tmp` is world-writable and shared by every local user, so a socket named directly under it could be pre-bound by another user, whose server would then receive a client's keystrokes and clipboard. A fallback directory that exists but is owned by another user, grants group/other access, or is not a directory fails closed with `PermissionDenied` on both the bind and the connect side; remove or fix the directory to proceed. Explicit `--socket` paths are not guarded — whoever named the path chose its location. `$XDG_RUNTIME_DIR` needs no guard: the OS provisions it per-user.

Which socket one `par-mux` invocation targets follows one precedence (`resolve_socket_path` in `src/mux/ipc.rs`): an explicit `--socket PATH`, then the positional `NAME`, then `$PAR_MUX_SOCKET`, then the unnamed `default`. The env tier is what makes the client flags work from inside a pane — every pane spawns with `PAR_MUX_SOCKET` naming its own daemon, so `par-mux --cmd list-sessions`, `--stop`, and `--restart` typed there target that daemon instead of failing against `par-mux-default.sock`. An empty value counts as unset. Note that `--stop`/`--restart` resolved this way kill the daemon that owns the very pane the command was typed into — the panes die with it, which is the point of a restart but worth knowing before typing it.

The on-disk state file (see [Persistence and Restart](#persistence-and-restart)) never lives next to the socket. With `--pane-endpoints`, each pane also gets a sibling socket beside the control socket, `<control stem>.pane-<pane id>.sock` in the same runtime directory — same `0600` mode and directory guard, one accept thread per pane, the socket file removed when the pane dies (crash remnants are reclaimed at the next daemon start). A path over the platform socket-address limit, or past the 256-endpoint-per-daemon cap, leaves the pane with no socket at all rather than a fallback to the control socket:

```text
<state_dir>/par-mux/<socket-stem>.state.json
```

`<state_dir>` is the platform state directory (`dirs::state_dir()`, falling back to the data directory; on macOS that is `~/Library/Application Support`). The socket is ephemeral by design; pane content is as private as the terminal it came from.

Two servers on different sockets never share state — the state file name derives from the socket stem.

## Protocol Overview

One line-based socket carrying two grammars, classified by the first non-whitespace byte of each line:

- A line starting with `{` is a JSON **agent hook report** — answered in place with one JSON reply line on the same connection. Hook connections never join the broadcast set, matching the send-one-line/read-one-reply/close pattern herdr's integration scripts use.
- Anything else is a **tmux control command**. The first control command a connection sends (a parse error counts) registers it for broadcasts.

Every command reply is bracketed, and the command number ties the reply to the request:

```text
%begin <timestamp> <command-number> 1
...body lines...
%end <timestamp> <command-number> 1
```

A failed command closes with `%error` instead of `%end`, the body carrying the error message. Reply bodies are written raw, so a body line can itself start with `%end` (pane output captured verbatim) — a client therefore closes a block only on the `%end`/`%error` carrying the command number of its `%begin`, as tmux's own clients do. Pushed pane output arrives as `%output %N <data>` lines, with every non-printable byte octal-escaped (`\033`) and the backslash itself escaped (`\134`) so decoding is unambiguous. A client line that is not valid UTF-8 is answered the same way — a numbered `%error` block ("line is not valid UTF-8") — and the connection stays open: the bytes through the newline were consumed, so the stream remains line-framed, and one bad `send-keys -l` payload must not cost the client its whole session.

### Client backpressure and eviction

Each registered client has a bounded broadcast queue (4096 lines). A client that stops reading its socket long enough for its queue to fill — 4096 undelivered lines, worst case ~128 MiB when every line carries a full PTY read — is **evicted**: the daemon logs a warning naming the reason (queue full — stalled, or draining slower than the burst), flags the connection, and its own reader/writer threads tear it down within a fraction of a second, closing the socket the client observes. A wedged client is disconnected rather than allowed to grow the daemon without bound. A client that keeps draining loses nothing. An evicted client simply reconnects and re-queries; pane output it missed is not replayed to it.

Eviction is by **queue depth only** — intended, and deliberately unlike tmux, whose control clients are evicted by output *age* (300 s) while buffering without bound in between. The depth policy bounds daemon memory at ~128 MiB per client; its accepted consequence is that a burst whose backlog passes 4096 lines evicts even a *healthy* client that drains continuously but slower than the producer — the backlog, not the client, is what hit the cap. The client reconnects and re-queries. Pinned by `a_slow_draining_client_is_evicted_once_its_backlog_passes_the_cap` (`src/mux/server.rs`); an age-based policy would replace that test with a drains-slowly-survives one.

## Command Reference

The parser is deliberately minimal: whitespace-split with a flag scan. tmux's full argument grammar (`--`, per-command option tables, command sequences) is not implemented. Quoting is honored in a fixed set of places, all sharing one bounded grammar (single or double quotes, backslash escapes outside quotes, the `'\''` close-escape-reopen idiom, no interpolation): the `send-keys` payload, the `new-session -s NAME` / `new-window -n NAME` names, the `split-window -c DIR` / `new-window -c DIR` start directories, the `select-pane -T TITLE` title, environment values (`new-session -e NAME=VALUE`, the `set-environment` name and value, the `rename-session` name), and the `-t`/`-s` targets, so a name, directory, title, value, or target may contain spaces. Every other flag is whitespace-split, and `rename-window` / `set-buffer` take the rest of the line verbatim. List replies have fixed shapes with no `-F` support — push notifications cover what `-F` polling existed for.

A target placeholder in the table below (`<pane>`, `<window>`, `<session>`) is either the typed id (`%N`, `@N`, `$N`) or a **name**, resolved daemon-side against the tree:

- **`<pane>`** matches the pane's sticky user title (`select-pane -T`) exactly. The pane program's live OSC 0/2 title never matches — it changes with the running program and would make name targets flaky.
- **`<window>`** matches the window's name (`new-window -n`, `rename-window`) exactly, across every session.
- **`<session>`** matches the session's name (`new-session -s`) exactly.

Ids always win over names: a value starting with the target kind's own sigil is the id, so a pane titled `%3` can never shadow pane `%3` (and a sigil-prefixed value that is not a valid id, like `%abc`, is a parse error). A name matching more than one pane/window/session is an **error listing the sorted candidate ids** (`ambiguous pane target: dup (matching: %0, %2)`) — never a silent pick, and the command acts on nothing. An unknown name errors (`no such pane: <name>`). One limitation: `send-keys` takes a single-token target, because its raw payload split cannot carry a spaced name next to free text — address such a pane by id.

| Command | Arguments | Reply body | Broadcasts |
|---------|-----------|------------|------------|
| `new-session` | `[-s name] [-e NAME=VALUE]…` (no start command) | The session id (`$N`) | `%window-add` per window, `%sessions-changed`; `%session-changed` to the issuer |
| `new-window` | `[-t <session>] [-n name] [-c dir]` (no start command) | The window id (`@N`) | `%window-add` |
| `select-window` | `-t <window>` | empty | `%window-pane-changed` |
| `kill-window` | `-t <window>` | empty | `%window-close`; `%sessions-changed` when it emptied the session |
| `rename-window` | `-t <window> <name>` | empty | `%window-renamed` |
| `rename-session` | `-t <session> <name>` | empty | `%session-renamed` |
| `kill-session` | `-t <session>` | empty | `%window-close` per killed window, then `%sessions-changed` |
| `split-window` | `-t <pane> [-h\|-v] [-b] [-p 1-99] [-c dir]` | The new pane id (`%N`) | `%layout-change`, `%window-pane-changed` |
| `select-pane` | `-t <pane> [-T 'title']` | empty | `%layout-change`, `%window-pane-changed`; `%pane-title-changed` when `-T` changed the title |
| `pane-title` | `-t <pane>` | The pane's effective title as the body; an empty body (no lines) = no title set | — |
| `pane-info` | `-t %N` | One line, `%N @W COLSxROWS [cmd=<base64>] [exited=<code\|?>]` for the resolved pane: its window, current grid size (read-only; lets a mirroring client seed at the pane's size instead of resizing it), and — when knowable — the foreground command name (standard base64 of the basename of the deepest descendant of the pane's child process; absent on Windows and when the argv is unreadable, so older clients keep parsing the fixed prefix). The name is a hint set by the program itself (argv[0]); names with control characters or over 128 bytes are served absent. tmux's `#{pane_current_command}` equivalent, for close-confirmation prompts. `exited=` appears only for a held-dead pane: its exit code, or `?` when the code was unreadable (a dead pane has no `cmd=`, since its child is reaped) | — |
| `pane-exited-replay` | — | empty; the held panes' `%pane-exited %N [code]` lines (one per held pane, sorted by id) are pushed to the issuing client only, ahead of the reply block, exactly as registration replay frames them (ENH-042). No held panes = no lines | — |
| `clear-history` | `-t <pane>` | empty | — |
| `resize-pane` | `-t <pane> (-L\|-R\|-U\|-D [cells] \| -x COLS [-y ROWS] \| -Z)` | empty | `%layout-change` |
| `swap-pane` | `-t <pane> -s <pane>` | empty | `%layout-change` |
| `break-pane` | `-s <pane> [-n name]` | The new window id (`@N`) | `%window-add`; `%layout-change` for the source window (`%window-close` instead when the pane was its last) |
| `join-pane` | `-s <pane> -t <pane> [-h\|-v] [-p 1-99]` | empty | `%layout-change` for both windows (`%window-close` instead of the source's when it emptied; `%sessions-changed` when its session died) |
| `move-window` | `-s <window> -t <position>` | empty | `%sessions-changed` |
| `swap-window` | `-s <window> -t <window>` | empty | `%sessions-changed` |
| `respawn-pane` | `-t <pane> [-k] [-c dir] [--] [command]` (flags before the command) | empty | `%pane-respawned` |
| `kill-pane` | `-t <pane>` | empty | `%layout-change`, `%window-pane-changed`; `%window-close` instead when it was the window's last pane, then `%sessions-changed` when that emptied the session |
| `list-panes` | `-t <window>` | One `%N <leaf> <marker>` line per pane of that window in layout leaf order (`leaf` is the deterministic index mapping tmux layout-string leaves to pane ids; `*` marks the window's active pane). Bare form: one `%N` line per pane, globally | — |
| `list-windows` | `-t <session>` | One `@N <marker> <name>` line per window of that session in window order (`*` marks the session's active window; the name is the line remainder, so spaced names survive). Bare form: one `@N: name` line per window, globally | — |
| `list-sessions` | — | One `$N: name` line per session | — |
| `list-agents` | — | Roster: one `%N <agent> <state> <source>` line per state-carrying pane (see below) | — |
| `list-commands` | — | One line per dispatchable command, sorted: `name [feature …]`, plus a trailing daemon-level `features replay-held-state` line (see the capability-discovery bullet below) | — |
| `capture-pane` | `-t <pane> [-S start] [-E end] [-e]` | The captured lines | — |
| `send-keys` | `-t <pane> <keys>` | empty | — (space lives inside a quoted run; see below) |
| `refresh-client` | `-t <pane> [-C WxH] [-p WxH]` or `[-C WxH] [-p WxH]` | Pane's screen-restore replay (with `-t`, no `-C`); empty otherwise | `%layout-change` (with `-C`) |
| `set-buffer` | `<content>` | empty | — |
| `set-client-colors` | `-f rrggbb` and/or `-b rrggbb` | empty | — |
| `set-environment` | `-t <session> NAME VALUE` or `-t <session> -u NAME` | empty | — |
| `show-buffer` | — | The buffer content | — |
| `paste-buffer` | `-t <pane>` | empty | — |
| `version` | — | The daemon's build stamp, one line: `<crate version>+<git sha[-dirty]>` (a `src-<16hex>` content digest when built outside a repository, e.g. from a crates.io tarball) | — |
| `reload-config` | — | One line per `[daemon]` setting: `unchanged: <name>` or `restart-required: <name>`, diffing the re-read config file against the daemon's applied settings (see [Configuration file](#configuration-file)) | — |
| `kill-server` | — | empty | `%exit` to every client, then the daemon exits |

Details worth knowing:

- **Bare `new-window`** targets the most-recently-created session (ids are monotonic). par-mux has no client-session attachment, so "newest" is the documented stand-in for tmux's attached-session resolution.
- **`new-session` and `new-window` take no start command**: a trailing word (`new-window sleep 5`) is an error, not silently dropped. Start the pane's program with `respawn-pane -t %N [-k] <command>`. tmux's other value flags (`-x 80`, `-y 24`, `-F fmt`, …) and bare flags (`-d`, `-a`, …) are tolerated and ignored.
- **`version` exists for stale-daemon detection**: the daemon outlives its clients, so an old daemon silently serves new clients. A client compares the reply against its own linked core's `mux::build_stamp()`; differing stamps mean the daemon predates the client's build. Outside a repository both sides carry a content digest of the crate source, so same-version drift is still comparable; only a build with neither identity (`+unknown`) degrades to version-only comparison — a same-version mismatch is then unprovable and clients stay quiet rather than cry wolf.
- **`split-window` flags name the arrangement, not the divider**: `-h` puts the new pane beside the target, `-v`/default below it. `-p` is the percent of the split area given to the **new** pane (default 50; the target keeps the remainder). `-b` places the new pane **before** the target — left of it under `-h`, above it in the default direction — keeping exactly its `-p` share.
- **`split-window -c dir` and `new-window -c dir`** start the new pane in `dir` instead of the daemon-wide default. A `dir` that does not exist degrades to home rather than failing the command — the same rule a restore applies to a gone persisted cwd — and the new pane's screen says so (`par-mux: <dir> is gone; pane started in <home>`).
- **`resize-pane` relative form** moves the bordering divider; the first of `-L -R -U -D` wins, and a flag without a number means 5 cells (tmux's default). The absolute form `-x COLS` and/or `-y ROWS` sets exact extents and cannot combine with the direction flags. Both forms re-fit the affected panes' terminals and PTYs to the layout geometry.
- **`resize-pane -Z`** toggles zoom (tmux semantics): the pane's terminal and PTY take the full window grid while the layout tree is untouched, so `-Z` again restores the exact prior geometry. On the wire, `window_layout` keeps the true tree (that is what makes unzoom exact), `window_visible_layout` is the zoomed pane alone at full extent, and `window_raw_flags` carries `Z`. Zooming a different pane moves the zoom to it; `split-window`, `kill-pane`, `swap-pane`, `break-pane`, `join-pane`, and `select-pane` to another pane unzoom first (selecting the zoomed pane itself keeps the zoom). A successful `resize-pane -x/-y/-L/-R/-U/-D` also ends the zoom before it moves a divider, as tmux does, so the hidden layout never changes under a zoom; a rejected resize (no divider on that axis) changes nothing, including the zoom. `-x` and `-y` together are all-or-nothing: if either axis cannot move, neither does. Zoom is session state, not layout — a restored window starts unzoomed. `-Z` cannot combine with the other adjustment forms.
- **`break-pane` and `join-pane` move panes between windows without re-spawning them** — the pane's process, terminal, and title travel with it; only layout membership changes. `break-pane -s %N` puts the pane in a new window appended to its session (the session's new active window, inheriting the source window's grid size); the source window closes when the pane was its last, but the session cannot die — the new window exists before the source drops. `join-pane -s %N -t %M` moves a pane next to the target with `split-window`'s arrangement rule (`-h` beside, `-v`/default below; `-p` is the moved pane's share, default 50) and makes it the destination's active pane; a same-window join is a within-window move. The mirror image of break-pane's guarantee: joining the last pane out of a session's only window **closes that session** (the destination belongs to the target's window). Both end any zoom in the windows they touch.
- **`move-window -s @N -t <position>` and `swap-window -s @A -t @B`** reorder the session's window list (the order `list-windows` returns and persistence saves). `move-window` clamps out-of-range positions to the ends; `swap-window` requires both windows in the same session. The active window is tracked by identity, not position — it stays active through the reorder.
- **`capture-pane -S/-E`** use tmux's offset convention: `0` is the first visible line, negative numbers count history lines back from the screen top. Without flags, the visible screen is returned.
- **`capture-pane -e`** returns the pane's visible screen (the active grid — an alt-screen TUI captures its TUI screen) as one line per grid row with SGR escape bytes inline, tmux's `-e` contract: a styled run is emitted as `\x1b[0;<fg>;<bg>[;<attrs>]m` before its text (the export walker's fixed reset-fg-bg order), a row that used any SGR ends with `\x1b[0m` before its newline, and a row that used none is plain text. Empty rows are empty lines, so the reply always holds exactly one line per grid row. Styled trailing blanks survive (plain text trims them). Without `-e` the reply stays the plain logical-lines capture, byte-identical to the pre-`-e` reply; `-e` composes with `-S/-E`, which then trim the styled scrollback+screen composition. The framing is line-per-row, not cursor addressing — replaying it into an emulator requires per-row addressing on the consumer side (a bare LF staircases).
- **`send-keys`** speaks the tmux contract: key names, `-l` literal payloads, `-H` hex bytes, and bare `0xNN` tokens. No trailing newline is appended — `Enter` is an expressible key. **Whitespace-separated payload tokens join with NOTHING between them in every mode, exactly as real tmux (3.7c, measured)**: `send-keys -t %0 -l echo LEFT` types `echoLEFT` (the separator between `-l` tokens is dropped — a common surprise), while the quoted single argument `send-keys -t %0 -l 'echo LEFT'` types `echo LEFT`. A space must live inside a quoted run or be an explicit `Space` key name; for bytes a shell line cannot express (including arbitrary spacing and control bytes), `send-keys -H 20 0a …` is the lossless path. Key names come in two classes. The control-byte names (`C-a`…`C-z`, `C-Space`, `Escape`/`Esc`, `BSpace`, `Space`, `Enter`, `Tab`) are always the raw byte, because par-term sends already-encoded keystrokes in this form. The navigation and function keys (`Up`/`Down`/`Left`/`Right`, `Home`, `End`, `PageUp`/`PgUp`/`PPage`, `PageDown`/`PgDn`/`NPage`, `Insert`/`IC`, `Delete`/`DC`, `F1`…`F12`, `BTab`) go through the shared key encoder against the target pane's application-cursor (DECCKM) and kitty keyboard state, as tmux does: `Up` is `ESC [ A`, or `ESC O A` once the app enables application cursor keys. A key name matches inside a quoted run too (`'Home'` is the Home key, as in tmux), so use `-l` to send such a word as text. Any other token is written literally.
- **`refresh-client -C WxH`** reports the client's renderer size; the window resizes to it (latest report wins) and every pane re-fits. The `-t`-less form is the size report the attach handshake sends before it has resolved a pane: `-p` still applies daemon-wide and `-C` resizes the newest session's active window (the same stand-in bare `new-window` uses) — never a replay, since replay is what the `-t` form exists for. With `-t` and without `-C`, the command replays the pane's state to the requesting client — the reattach resync. The reply is `Terminal::export_screen_restore_sequence()`: the main screen's scrollback first when the pane has any (`\x1b[H` anchor, each history line CR-led and LF-terminated so it replays into the client emulator's own scrollback in order, then the still-on-screen lines pushed off the bottom row with plain line feeds), alt-screen selection next (`\x1b[?1049h` when active), the scroll region, then the styled screen content with absolute row addressing (`\x1b[H` anchor, per-row `\x1b[R;1H`, SGR-diffed runs, reset per row), then the cursor position (`\x1b[R;CH`), visibility (`\x1b[?25l`) and style (DECSCUSR), the input modes (DECCKM, bracketed paste, focus tracking, mouse tracking/encoding), origin mode restored last with a region-relative re-position, and a final `\x1b[0m`. The scrollback block does carry `\n` bytes (one per replayed history line) — despite that, raw ESC bytes still survive the `%begin`/`%end` line framing untouched, since the reader consumes the reply as an ordinary sequence of framed lines regardless of how many lines the body spans; a client feeds the whole body verbatim into its pane emulator and the pane's subsequent `%output` deltas land on the restored state. A pane with no scrollback yields the pre-scrollback-fix shape: no `\n` at all, purely CUP-addressed.
- **`refresh-client -p WxH`** reports the client's per-CELL pixel size (font metrics, e.g. `-p 10x20`) — the one renderer measurement every pane shares, while grid extents differ per pane. It is held daemon-wide with the same latest-wins policy as `-C`, applied to every pane through `resize_with_cell_pixels`, and re-derived on every later re-fit, so `TIOCGWINSZ` (image tools read it), XTWINOPS pixel reports (`CSI 14 t`/`CSI 16 t`), and image cell-span math (how many rows the cursor advances after a kitty/iTerm2/Sixel image) all carry the size cells actually render at instead of the 10×20 construction default. Independently optional from `-C`: pixels without `-C` re-fit at the current grid, `-C` without pixels keeps the last reported cells. **Client contract:** send `-p` with the cell size on attach and whenever the font or zoom changes, alongside the `-C` report a resize already sends. Not persisted — a reconnecting client re-reports.
- **`set-client-colors -f/-b rrggbb`** reports the client's theme foreground/background so OSC 10/11 queries in panes answer with what the client actually renders (dark/light detection) instead of the core's built-in theme. Each flag is independent; at least one is required; six hex digits, no `#`. Applied to every pane terminal and inherited by panes created later. **Client contract:** send it on attach and on theme change. Not persisted.
- **Kitty `t=t` (temp-file) images** render in mux panes because the daemon-side pane terminal *retains* the temp file: client mirrors rebuild their grid from the raw `%output` bytes, and the t=t contract has the reading terminal delete the file — the daemon reads first, so its read must not delete, or no client could ever load the graphic. The client that actually renders the graphic deletes it with its own read. Streamed (`m=1`) and direct transfers carry their data inline and never had the problem; `t=f` files are never deleted by anyone. Two attached clients remain first-reader-wins for t=t (the second mirror finds the file gone) — same as before, no worse. A t=t file whose client never processes it lingers in the sender's temp dir until OS cleanup, the one accepted leak.
- **Session environment (`set-environment`, `new-session -e`)** is a per-session variable map applied on top of the daemon's own environment for every pane spawned into that session afterwards. Panes already running are untouched (tmux semantics), and the `PAR_MUX_*` contract vars always win over a same-named session var. It exists because the daemon outlives its clients: panes otherwise inherit whatever environment the daemon was started with, so a client that reconnects over SSH or with a changed `SSH_AUTH_SOCK`, `DISPLAY`, or `PATH` sends its current values here (tmux's `update-environment`). `-t` is required, `-e` repeats, quoting follows the bounded grammar above (`-e 'A=two words'`, `NAME 'a value'`), and a name that is empty or contains `=` or NUL is refused. Values can be secrets: they are never logged, and they persist in the owner-only state file.
- **Buffers** are a single slot named `default` — no numbered stack, and `-b` is not implemented.
- **`kill-pane`** of a window's last pane closes the window (`%window-close`), and of a session's last window closes the session (`%sessions-changed`). A pane whose child process exits on its own (rather than via `kill-pane`) is instead HELD with its exit code (remain-on-exit) and announced — see [Pane Reaping](#pane-reaping) below; `respawn-pane` restarts it in place.
- **`kill-server`** raises the same shutdown flag SIGTERM does: the reply goes out, the accept loop notices on its next tick, `%exit` reaches every client, and the final state save runs before the process exits — see [Shutdown Semantics](#shutdown-semantics). Refused with an error for an embedded `MuxServer` that has no shutdown handle (`ctx.shutdown` is `None`). `par-mux --stop`/`--restart` send this command under the hood.
- **`list-agents`** returns one line per pane a hook has claimed or a scrape pattern has matched, sorted by pane id: `%N <agent> <state> <source>` with `source` `hook` or `scrape`, then zero or more whitespace-free `key=value` tokens: `reason=<base64>` (the blocked reason — standard base64 of the whitespace-collapsed message, so a reason reading `hook` or containing `telemetry=` cannot be mistaken for the grammar's own tokens; ARC-060), one `telemetry=<base64>` token when the pane holds a **fresh** telemetry sample (base64 of the canonical JSON, so string values with spaces ride as one whitespace-free token), and a sibling `host_telemetry=<base64>` token when the daemon's host probe has fresh fields for the pane's cwd. **Consumers parse positions 1-4 positionally, then split each remaining token on its first `=`; unknown keys are ignored.** Stale or absent telemetry adds nothing — the row keeps its exact pre-telemetry shape — and a token leaves with its sample's age, no write needed. Panes without state are absent — `unknown` is never reported, and `idle` is never guessed.
- **`list-commands` is capability discovery** (ENH-037). The reply is one line per dispatchable command, sorted: `name [feature …]`, where feature tokens name flag-level abilities a client cannot probe by sending — `resize-pane zoom absolute`, `split-window before start-dir`, `respawn-pane kill start-dir`, `pane-info cmd`, `refresh-client cell-pixels`, `capture-pane escape` — plus one daemon-level trailing line, `features replay-held-state`, announcing registration-time replay. Tokens are `[a-z-]+`; a client must ignore unknown tokens and lines, and a daemon never removes a token without a CHANGELOG "Removed" entry. **Client contract:** send `list-commands` once after `version`; an `unknown command: list-commands` error means a pre-feature daemon — assume the older feature set known from its `version` stamp, or none. The tokens are the third column of the `COMMANDS` table itself (`src/mux/command.rs`), one declarative row per command (ARC-094b).
- **Pane titles (`select-pane -T`)** set a user title on the pane; `-T ''` clears it (quoting is what makes an empty value expressible). Precedence is a deliberate divergence from tmux: a user title is **sticky** — the pane program's OSC 0/2 title never overwrites it — while with no user title the pane reports the program's live OSC title. The effective title (user when set, else OSC) is what `pane-title -t %N` replies — an empty reply body means neither is set; a broadcast carries only the *user* title's changes, so a client composing a display title falls back to its own OSC tracking on the empty form. Setting the same title again is a no-op and broadcasts nothing.

## Notifications

Broadcast lines every connected (registered) client receives, emitted in this order per mutating dispatch: `%layout-change` first (it carries the geometry the pane-changed notification is read against), then lifecycle broadcasts, then issuer-only notifications.

| Line | Meaning |
|------|---------|
| `%output %N <data>` | Pane output, octal-escaped, pushed as bytes arrive — no polling |
| `%layout-change @N <layout> <visible-layout> <flags>` | A window's geometry changed |
| `%window-add @N [layout visible-layout flags]` | A window was created; the layout triple mirrors `%layout-change` and is empty on a bare-id line (real tmux's shape, or an older daemon) |
| `%window-close @N` | A window was killed |
| `%window-renamed @N <name>` | A window was renamed |
| `%window-pane-changed @N %N` | A window's active pane changed |
| `%pane-exited %N [code]` | A pane's process exited and the pane is HELD (remain-on-exit): the pane, its window, and its frozen screen stay in the tree. The exit code token is present whenever the OS reported an exit status (a death by signal reports `1`, portable-pty's mapping) and absent only when none was available as the pass observed the death (the PTY closed before the child was reapable). The cue clients show "Process exited (code N)" over — never instead of — the frozen screen |
| `%pane-respawned %N` | A pane's process was restarted in place by `respawn-pane`: same id, window, and layout, fresh terminal — the cue clients clear their exited-state chrome and re-render from the new screen on |
| `%session-changed $N <name>` | Sent to the issuing client after `new-session` |
| `%session-renamed $N <name>` | A session was renamed (`rename-session`). The name is the rest of the line, spaces included |
| `%sessions-changed` | The session set changed — a session was created (`new-session`), destroyed (`kill-session`, or a `kill-window`/`kill-pane`/`join-pane` that emptied it), or its windows were reordered (`move-window`/`swap-window`). Argument-less, tmux's shape: clients re-query `list-sessions` rather than parsing the line. **Client contract:** on receiving it, re-run `list-sessions` (and `list-windows` for the new order after a reorder); if the session a view is showing is gone, end that view (close the tab / show an empty state) — the daemon does not send anything more specific, and if this emptied the daemon it exits shortly after (see [Shutdown Semantics](#shutdown-semantics)), closing the transport |
| `%agent-state-changed %N <agent> <state> [source=hook\|scrape]` | An agent's state changed, with provenance |
| `%agent-released %N <agent>` | The pane's agent released its claim (`pane.release_agent`) — state, hook authority, session identity, and telemetry cleared; the pane left the roster |
| `%agent-telemetry-changed %N <agent>` | An agent's telemetry changed (`pane.report_agent_telemetry` accepted a fresh sample). Identity only, no values — clients re-query `list-agents` for the fresh blob, the same shape `%sessions-changed` teaches |
| `%pane-title-changed %N [title]` | A pane's user title changed — `select-pane -T` set it (title follows the pane id, spaces included) or cleared it (no title token). The user title is sticky over the program's OSC title; the notification carries the user title only |
| `%exit` | Graceful shutdown — the daemon is ending deliberately, not dying |

tmux control-mode clients ignore `%` lines they do not recognize, so a client that never learned `%agent-state-changed` (or any future variant) still sees the raw line rather than an error.

**Late clients** (ENH-037): on registration a client receives `%pane-exited` for each held pane and `%layout-change` for each zoomed window, queued ahead of its first command's reply. A held pane's `%pane-exited` may repeat: the registration snapshot and an in-flight reap broadcast can overlap, and the line is idempotent (both say "held with code N"). A client that registered before the death or the zoom receives no replay line. At any other time a client can pull the held panes' lines on demand with the `pane-exited-replay` command (ENH-042), delivered to the issuing client only.

## Agent Hook Reports

A pane's process can report agent state over the control socket by sending one JSON line and reading one JSON reply. herdr's integration scripts port unchanged except the env-var rename (`HERDR_*` → `PAR_MUX_*`).

Every pane process is seeded with the env contract (`src/mux/pane.rs`):

| Variable | Value |
|----------|-------|
| `PAR_MUX_PANE_ID` | The pane's own id (`%N`) |
| `PAR_MUX_SOCKET` | The control socket path — or, on a daemon started with `--pane-endpoints`, the pane's own hook-only socket (see below). Also the CLI's fallback target: a `par-mux` invocation with no `--socket` and no positional NAME uses it before the unnamed default socket, so `par-mux --cmd …` / `--stop` / `--restart` typed inside a pane reach their own daemon |
| `PAR_MUX_CONTROL_SOCKET` | The full control socket — exported only when the daemon runs `--pane-endpoints --expose-control-socket`, or the pane's session env carries `PAR_MUX_CONTROL=1`; absent otherwise |
| `PAR_MUX_ENV` | `1` — marks the contract as present |
| `PAR_MUX_SESSION_ID` | The owning session's id (`$N`) |
| `PAR_MUX_SESSION` | The owning session's name |
| `PAR_MUX_WINDOW_ID` | The owning window's id (`@N`) |
| `PAR_MUX_BIN` | The daemon executable, so a pane script can run client mode without `par-mux` on `PATH`: `"$PAR_MUX_BIN" --socket "$PAR_MUX_SOCKET" --cmd list-sessions`. Set by the `par-mux` binary; an embedded `MuxServer` leaves it unset unless its factory supplies `bin_path` |

These are set for every spawn path: `new-session`, `new-window`, `split-window`, `respawn-pane`, and restore. They are **fixed at spawn**, as tmux's `TMUX`/`TMUX_PANE` are: a later `rename-session` leaves `PAR_MUX_SESSION` stale, and a `swap-pane` across windows or a `break-pane`/`join-pane` that moves the pane to another window leaves `PAR_MUX_WINDOW_ID` stale (and the session variables too when the move crosses sessions). `respawn-pane` re-seeds them with the pane's current ids. The ids stay valid; the name is advisory. `PAR_MUX_SOCKET`, `PAR_MUX_ENV`, and `PAR_MUX_BIN` are absent when the server has no socket path or binary path to export.

### Pane endpoints (opt-in)

`par-mux --pane-endpoints` changes what `PAR_MUX_SOCKET` names inside a pane: a per-pane socket that accepts exactly the four hook methods below, **bound to that pane** — a report whose `pane_id` names another pane is refused (`pane_id does not match this endpoint`), and a report that omits `pane_id` is filed for the bound pane. Any other line — a control command (`capture-pane`, `send-keys`, `kill-server`, …) — is answered with `{"error":"hook-only endpoint"}` and the connection closes. A pane's child processes can therefore report their agent state but cannot read other panes, type into them, or stop the server. The socket carries the same owner-only boundary as the control socket; it is not a defense against same-uid code, which can always reach the control socket directly (see SECURITY.md).

The mode is **off by default**, and the default stays off: an in-pane `$PAR_MUX_SOCKET` naming the FULL control socket is a core feature — an agent in one pane must be able to spawn and drive agents in other panes via `par-mux --cmd`, and the par-mux skill depends on that in-pane fallback. **Default-flip gate** — the criteria that would ever have to be met to turn the mode on by default: an explicit decision by the repository owner (declined 2026-09-29), acceptance of the break it causes to the in-pane `--cmd` fallback for daemons started without `--expose-control-socket`, and a migration story for the hook shims and agent extensions that consume `PAR_MUX_SOCKET` today. Absent all three, pass `--pane-endpoints` per daemon to opt in, and add `--expose-control-socket` when agent-driven pane control should keep working through `PAR_MUX_CONTROL_SOCKET`.

Four methods (`src/mux/hooks/`):

```json
{"id":1,"method":"pane.report_agent","params":{
  "pane_id":"%0","agent":"claude","seq":4,
  "state":"working",
  "message":"running tests",
  "agent_session_id":"abc-123"
}}
```

```json
{"id":2,"method":"pane.report_agent_session","params":{
  "pane_id":"%0","agent":"pi","seq":7,
  "agent_session_path":"/tmp/pi/session.jsonl",
  "session_resume_argv":["pi","--session","/tmp/pi/session.jsonl"],
  "session_start_source":"startup"
}}
```

```json
{"id":3,"method":"pane.report_agent_telemetry","params":{
  "pane_id":"%0","agent":"claude","seq":9,
  "telemetry":{
    "version":1,
    "source":"claude_code",
    "sampled_at_unix_ms":1790532000000,
    "model":"GLM-5.3",
    "effort":"high",
    "thinking_enabled":true,
    "context_used_percent":63,
    "context_remaining_percent":37,
    "five_hour_remaining_percent":80,
    "seven_day_remaining_percent":95,
    "five_hour_resets_at_unix_ms":1790553600000,
    "seven_day_resets_at_unix_ms":1791110400000
  }
}}
```

- Common params: `pane_id`, `agent`, `seq` (monotonic per pane **per source**; a report at or below the last accepted `seq` from its own source is dropped with no write and no broadcast — sources stamp different clocks, e.g. `time.time_ns()` vs `Date.now()*1000`, so freshness is tracked per `source`), and optional `source`.
- `pane.report_agent` additionally carries `state` (`working`/`blocked`/`idle`; `unknown` is accepted but never written), optional identity (`agent_session_id`/`agent_session_path`), and optional `message` — the blocked reason, stored whitespace-collapsed and cleared when a later report omits it.
- Validation is at the door: `state` outside the fixed set, an `agent` label containing whitespace or control characters, or a `source` containing control characters is error-replied, nothing written, nothing broadcast. These fields are interpolated verbatim into the space-split `%agent-state-changed` and roster lines, and any process in any pane can reach this endpoint — a newline in a field would forge a control-mode line delivered to every attached client.
- `pane.report_agent_session` identifies the session by **id or transcript path** (either alone is enough) and may carry `session_resume_argv` — an array of non-empty strings, malformed values error-replied — stored verbatim as the pane's resume invocation. `session_start_source` records startup vs resume provenance. A session report that moves the pane to a **different agent** clears the previous agent's state, hook authority, and blocked reason rather than rebroadcasting them under the new label.
- `pane.report_agent_telemetry` carries one `telemetry` object — versioned (`version`, currently 1), bounded (strings capped: `model` 128, `effort` 32; percents 0-100 rounded; no control characters), and stamped (`sampled_at_unix_ms` plus its own `source`, so clients can tell hook-reported from daemon-probed data). The shape mirrors the hub's normalized telemetry (par-remote-herd `status_telemetry.py`), and the producer is a hook tailing the agent harness's own status file — par-mux never parses transcripts. Malformed payloads are error-replied per the rules above, and a `sampled_at_unix_ms` in the future is error-replied the same way (it would saturate the freshness check to "maximally fresh" forever, QA-156). Four silent drops, each an ok no-op with nothing written: a sample older than the 55-minute freshness window (the hub's `STATUSLINE_FRESHNESS_SECONDS`, so daemon and hub age data out together — absent beats stale), a report at or below the last accepted `seq`, a sample older than the one already stored (a backward step, whatever its `seq`), and a report whose `agent` is not the pane's current label — telemetry attaches to a claim, it never takes one over. Telemetry is **display-only and ephemeral**: it never touches pane state or roster membership, never persists (the save format copies named identity keys only), and is cleared with the claim by `pane.release_agent` and the liveness sweep. An accepted write broadcasts `%agent-telemetry-changed` (identity only) and serves from the next `list-agents` as the row's trailing `telemetry=<base64>` token.
- `pane.release_agent` is the agent's exit announcement (herdr's SessionEnd shape): it clears the pane's whole claim — label, state, hook authority, blocked reason, sequence stamps, session identity, and telemetry — broadcasts `%agent-released`, and thereby returns the pane to the roster-less majority (a later restart respawns the original command, not a resume invocation for a dead session). Guards: the releasing `agent` must match the pane's current label, and the report must clear the same per-source `seq` rule; a failing guard is a silent ok no-op, like a stale report.
- Replies are one JSON line: `{"id":…,"result":"ok"}` or an error object.
- Authorization is the socket's own owner-only boundary: a hook claiming another pane's id is same-user by construction — the same trust model tmux control mode has. There is no per-connection authentication beyond the socket.

## Host Telemetry Probe

What the daemon can measure about a rostered pane's cwd that no hook can (card 01a0e3f1205371619309073eb5f803d6): disk-free percent (`statvfs` on Unix; Windows serves nothing for it — a field that cannot be measured is absent, never zero) and git state (`symbolic-ref` for the branch, `diff-index` + `ls-files --others` for dirty; a failing repo serves no verdict, never "dirty"). The shapes mirror HerdDeck's hub probe (par-remote-herd `status_telemetry.py` `_probe_cwd`).

A dedicated thread sweeps every 30 s (`src/mux/host_probe.rs`), whole-sweep deadline 10 s, one git deadline 5 s — never on the roster poll, so `list-agents` costs zero probe syscalls and reads only what the sweep already wrote (the invariant the hub keeps by snapshot cadence). Results land in pane metadata as one canonical object with per-field `sampled_at_unix_ms` and `source: "host-probe"`, served as the roster row's `host_telemetry=<base64>` sibling token, aged per field against the same 55-minute window as hook telemetry — a stopped sweep fades the token out field by field. Like hook telemetry it is display-only (never persisted) and clears with the claim. It broadcasts nothing: `%agent-telemetry-changed` stays the hook's signal, and host fields refresh on the roster poll's next read.

## Agent Scrape Tier

Panes whose agent reports no state hook (claude, codex, grok) get state read from rendered content — OSC title and screen text matched against per-agent pattern rules bundled from herdr (Apache-2.0, credited per file). The scrape runs on the accept loop's idle poll once per second; there is no dedicated thread.

Structural rules (`src/mux/scrape.rs`):

- **Hooks stay primary.** A pane that has ever accepted a hook state report is hook-authoritative until it dies; the tick skips it outright, and a quiet hook agent holds its last claim — except the liveness sweep below, which clears the one thing a quiet hook can never correct: a dead agent.
- **A scrape never invents state.** An unmatched scrape clears its own earlier guess and yields nothing — never `idle`.
- **Provenance rides every surface**: the `source=` token on `%agent-state-changed`, the `<source>` column in `list-agents`, and the `agent_state_rule` metadata (which pattern fired).
- **`contains` matches case-insensitively** (herdr parity): needles lowercase at compile and the region text at match, so real-cased agent chrome ("Do you want to proceed?") hits rules authored in lowercase. `regex`/`line_regex` stay case-sensitive as written — make them `(?i)` explicitly when they must fold.
- **Every pattern list is ALL-of** (herdr parity): a `contains`, `regex`, or `line_regex` list holds when every entry matches (`line_regex` per pattern needs at least one matching line, not one line matching all). A rule carrying two patterns where only one is present does not fire.

Patterns ship bundled (`include_str!` from `src/mux/patterns/{claude,codex,grok}.toml`). A local override file shadows the bundled set for its agent:

```text
<state_dir>/par-mux/agent-patterns/<agent>.toml
```

An override that fails to parse or validate falls back to the bundled set with a warning; a file for an agent with no bundled set adds it. Pattern files document which herdr rule regions are implemented (`osc_title`, `whole`, `bottom_non_empty_lines(N)`, `top_non_empty_lines(N)`).

### Claim liveness sweep

`pane.release_agent` only fires when the agent's hook is alive to send it. An agent that crashes or is killed would hold its roster entry and resume identity until relabel or pane death, so the same once-per-second tick runs a liveness sweep over hook-authoritative panes (`src/mux/foreground.rs`): a claim is stale when **no process descending from the pane's child matches the claimed agent's CLI** — the descendant tree, not the foreground process, because an agent backgrounded with Ctrl+Z or hidden behind an editor is still alive. The CLI match accepts the label as a path component or a `-`/`_`/`.`-suffixed variant (`pi`, `…/bin/pi`, `…/claude-cli/cli.js`, `pi.js`), which covers npm/bun-installed agents whose argv is an interpreter plus a package script. <!-- doc-path: example -->

Two mismatching ticks (not interrupted by a proven match) clear the whole claim and broadcast `%agent-released`, exactly as a release report would. The verdict is conservative in every ambiguous direction: a probe that goes blind (unreadable argv — the fresh-spawn window where macOS cannot read a mid-`exec` argv) keeps its recorded misses and never adds one, and Windows has no portable process-tree/argv access, so the sweep is inert there and hook release remains the only claim-clearing path.

## Persistence and Restart

State saving is crash-safe by construction (`src/mux/persist.rs`): serialize to `<file>.tmp`, fsync, rename over the target. The file is created `0600` on Unix (set at open, so there is no readable window between create and chmod) — the same owner-only posture as the socket. On Windows it inherits the ACL of the per-user state directory.

- **What persists:** the full tree (sessions, windows, panes, layout), each session's environment (`env`; a file written before the field existed loads with an empty one), each pane's screen and scrollback content, the paste buffer, each pane's user title (`user_title`, from `select-pane -T` — a save file written before the field existed loads with no title), each agent pane's session identity (`agent_session`: agent label, session id and/or transcript path, source tag, and the hook-reported resume argv), and whether a pane was held dead at save time (`dead`, with its `exit_code` when one was recorded, ARC-114; both are skipped when unset, and a save file without them loads every pane alive, so older files restore exactly as before and older daemons ignore the fields). Format version is **2**. Persisted scrollback is capped per pane to the newest 100 000 cells (~1 250 lines at 80 cols) — a persistence bound only: the running pane keeps its full in-memory history, and the cap is what keeps a scrollback-maxed shutdown save from stalling daemon exit (measured 2026-09-22: uncapped, one flooded pane's final save was a 136 MB write holding SIGTERM exit for two minutes).
- **What deliberately does not persist:** agent *state*, its provenance, the ordering `seq`, the blocked reason, and `session_start_source`. A restored pane reports state anew or holds none, so the roster is empty after a restart by design.
- **When it saves:** after every *successful mutating* command (the structural set: `new-session`, `rename-session`, `kill-session`, `set-environment`, `new-window`, `kill-window`, `rename-window`, `select-window`, `split-window`, `select-pane`, `resize-pane`, `swap-pane`, `break-pane`, `join-pane`, `move-window`, `swap-window`, `respawn-pane`, `kill-pane`, `set-buffer`, `clear-history`, and `refresh-client -C`). Content commands (`send-keys`, `capture-pane`, `paste-buffer`, the lists) do not save — their staleness window is bounded by the next structural save and the clean-shutdown save.
- **Quarantine:** a corrupt or unknown-version state file is renamed aside (`<file>.quarantine-<timestamp>`) and the daemon starts fresh — unreadable state never blocks startup, and the evidence survives for inspection. A v1 state file is quarantined rather than partially read.
- **Last-good snapshot:** every pane-bearing save also copies the state file to `<file>.lastgood`. Reap-driven saves (a pane's child exiting on its own) never touch the snapshot, and an empty final shutdown save leaves it alone — so the reboot/logout race, where the panes are killed before the daemon's own stop (shells turn SIGHUP into exit code 1, so the deaths look like ordinary child exits), cannot wipe the layout: the next start finds a pane-less state file and restores the snapshot instead. Two emptinesses are deliberate instead and clear the snapshot so the next start is fresh: an explicit `kill-pane` of the last pane (a Command save of an empty tree), and the daemon's exit-when-empty final save (`ShutdownEmpty` origin — every pane closed, no clients left, no shutdown signal; see [Shutdown Semantics](#shutdown-semantics)). The accepted tradeoff that remains: a daemon stopped by SIGTERM whose last panes died by child exit just before the signal resurrects on the next start — the emptiness may be the logout race, and the snapshot must not be wiped on a guess. A corrupt snapshot is ignored, not quarantined; it is a derived cache, not evidence.
- **Restore:** layout and content are rebuilt, and every live pane's process is new — the original processes died with the previous server. A pane that was **held dead at save time** (`dead: true`, ARC-114) is the exception: it is restored held dead, with no process spawned for it. It comes back as it stood: its exit code, its frozen screen and scrollback exactly as the client last saw them (the saved alternate screen and input modes are kept, not reset, because there is no new process to protect), its spawn command and agent identity. So `pane-info` reports `exited=<code>`, registration replays its `%pane-exited`, and `respawn-pane` (no `-k` needed) restarts it in place. Restore runs no agent resume invocation and no cwd spawn for a dead pane, and its persisted cwd is kept for the later `respawn-pane`. The OSC 7 hostname that came with that cwd (`cwd_host`, recorded only when the cwd was an OSC 7 report, skipped when absent, so older files load without it) is re-seeded beside it, so a remote report from an SSH session is still remote after the restart and never picks the respawn directory (SEC-128). The rest of this bullet describes a **live** pane's restore. A pane's terminal is rebuilt via `restore_for_new_process`, which keeps the saved screen content and scrollback but drops the state the *old* process left behind — as if the new process inherited a bare terminal, not the old one's TUI mode: it lands on the main screen (alternate grid cleared) regardless of what was active at save time, the scroll region and margins are reset, and cursor-key/keypad/mouse/focus/keyboard-protocol modes are off. This matters for a pane saved mid full-screen-app (htop, top): the pane returns to the shell's screen with its history intact, instead of resuming on a frozen, history-less alternate screen under a shell that never asked for it. Each pane's process respawns in its persisted cwd — captured at save from the shell's OSC 7 report, else the child's live process cwd — so a resumed pane (and a resumed agent, whose transcript lookup is cwd-sensitive) re-lands where it left off rather than in the daemon's start directory. A persisted cwd that no longer exists never fails the restore: the pane spawns in `$HOME` and a `par-mux: <dir> is gone` line is written into the restored pane. Agent panes respawn through their resume invocation when one can be built: the hook-reported `session_resume_argv` verbatim, else a per-agent table shipped in the binary (claude/codex/grok/pi/omp, each in its own CLI's argument shape — `claude --resume <id>`, `codex resume <id>`, `grok --resume <id>`, `pi --session <id-or-path>`, `omp --resume=<id-or-path>`). The resume invocation is never persisted — a stored argv would freeze a vendor CLI shape into a state file newer code later restores. Failure splits by where it happens. A chain that cannot *build* an invocation (missing table entry, a session ref the table cannot use) falls back structurally to the pane's original spawn command — the pane spawns as it was. A chain that builds but *runs* into failure (uninstalled binary, a session id the CLI rejects) does not delete the pane: on Unix the invocation spawns with a fallback tail (`|| { printf …; exec "$SHELL" }`), so a non-zero exit drops the pane onto a live shell carrying the restored screen, scrollback, and agent identity — an explanatory line is printed in the pane, the reaper never sees a dead pane, and the identity survives for a retry or an explicit `kill-pane`. On Windows the resume argv never crosses a shell string: an `argv[0]` that resolves to a PE image (`.exe`/`.com`) spawns directly with CreateProcess-exact arguments, while `.cmd`/`.bat` shims (the npm-installed agent CLI shape — npm also drops an extensionless POSIX sh shim beside them, which CreateProcess cannot run and is never picked) and unresolved names run through a self-deleting bridge batch under `%COMSPEC% /d /c`, where double-quoted arguments carry spaces and `&` verbatim (`src/mux/win_resume.rs`; `%VAR%` expansion and embedded double quotes remain unrepresentable on a cmd line). A failed Windows resume still has no fallback shell — a non-zero exit still exits into the reaper, the one asymmetry that remains.
- **State is keyed by socket basename:** the state file is `<state_dir>/par-mux/<socket-stem>.state.json`, so a daemon started on a socket path with the same basename restores that stem's tree — this is the resurrection feature, and it spans daemon generations. Ephemeral and CI harnesses that start a fresh daemon per run on a stable socket path therefore inherit every prior run's panes, and the state file grows across runs (measured 2026-09-28 on a test harness: 94 panes, a 262 MB state file, daemon startup >10 s). Pass a distinct `--state-dir` per run (or remove the state file between runs) so each daemon starts fresh.

## Pane Reaping

A pane whose child process exits on its own (as opposed to via `kill-pane`) is detected automatically and HELD rather than removed — remain-on-exit, always on. The accept loop's idle tick runs a pass every 250 ms (`REAP_INTERVAL`, `src/mux/server.rs`): a pane whose process died (the PTY reader's `is_running` flipped false on EOF, confirmed against the OS child handle — on Windows ConPTY that flag never flips after an exit, so the OS check is what observes the death there) is marked dead, its exit code recorded while the child handle can still be asked, and `%pane-exited %N [code]` broadcast. The pane keeps its id, window, layout, and frozen screen — the state a client needs to show "Process exited (code N)" and offer a restart. The code token is absent only when the OS had no exit status to report as the pass observed the death (the PTY closed before the child was reapable); a death by signal reports `1`. Nothing auto-closes: no `%layout-change`, no `%window-close`, no cascade — removal is explicit (`kill-pane`/`kill-window`) or a restart (`respawn-pane`). `%pane-exited` is pushed once; a client that attaches after the exit reads it from `pane-info`'s `exited=` token. At registration each control client is sent one `%pane-exited %N [code]` line per pane already held dead (with held-state replay, ENH-037), ahead of the registering command's reply block. A death the reaper observes after the registration arrives only through the ordinary broadcast, so each client still learns of every death exactly once. A client that wants the held panes' lines outside registration or a live death pulls them on demand with `pane-exited-replay` (ENH-042), delivered to the issuing client only.

`respawn-pane -t %N [-k] [-c dir] [--] [command]` restarts the pane's process in place: same pane id, window, and layout; a fresh terminal at the pane's current grid size; the old pane's user title carried over. The flags (`-t`, `-c`, `-k`, in any order) must precede the command: flag parsing stops at the first word that is not one of them, or at `--`, so a `-k` or `-c` inside the command belongs to the command (`respawn-pane -t %0 sh -c 'sort -k 1 f'` neither kills nor changes directory). A command that itself starts with `-` must follow `--`, and an unknown leading flag is an error. The command text is passed verbatim to the default shell (`$SHELL -c <command>`, or `cmd.exe /C <command>` on Windows), quoting and whitespace included. The command and cwd come from the trailing text and `-c` when given, else the pane's stored spawn command and a default cwd: the shell's OSC 7 report only when its host is this machine (absent, `localhost`, or the local hostname, full or short form) and the directory exists here, else the live child's kernel-reported cwd (the `-k` case; a held dead pane has none), else the daemon's default pane cwd (its `$HOME` when none is configured). A remote host's OSC 7 (an SSH session) never picks the directory (SEC-128). A still-running pane refuses without `-k` ("still running") and restarts with it, killing the old process first; a dead pane restarts with no flag. `%pane-respawned %N` is the cue clients clear their exited-state chrome on. A respawned pane persists as the new process. A pane still held dead when the tree is saved persists as dead (ARC-114): a save/restore cycle restores it held dead, with its exit code and frozen screen, rather than respawning it (see [Persistence and Restart](#persistence-and-restart)).

Held panes are for clients that might come back. A persisting daemon where every pane is dead AND no client is connected collects itself through the same exit-when-empty path as an empty tree (see [Shutdown Semantics](#shutdown-semantics)) — with nobody watching and nothing alive, the daemon exits and its final save restores on the next start with every pane still held dead, as it stood (ARC-114), where `respawn-pane` restarts any of them.

A structurally killed pane is reaped at kill time instead: `PtySession::kill` follows its signal escalation (SIGHUP → SIGKILL inside portable-pty) with a bounded wait, so a child that traps SIGHUP cannot linger as a zombie until the daemon exits. The kill also runs off the tree lock on a short-lived thread once the pane is removed from the tree — the ~200 ms SIGHUP-grace poll and the reap wait would otherwise stall every command for their duration (tmux kills from its child reaper, off the layout lock, for the same reason).

## Shutdown Semantics

- **SIGTERM**: the handler makes one atomic store on the server's per-instance shutdown flag. The accept loop notices on its idle tick, broadcasts `%exit` to every client, and a final save captures content that arrived since the last structural save.
- **Exit-when-empty** (tmux's `exit-empty`, persisting daemon only): a daemon with zero clients and either zero sessions or only dead panes, continuously for a 5 s grace, exits through the ordinary shutdown path — `%exit`, final save — with no one asking. The grace is what keeps the two races it could lose won instead: a client that reconnects (or creates a session) resets the clock, and a logout's SIGTERM, which follows pane deaths within moments, requests the exit first so the final save keeps `Shutdown` semantics (last-good snapshot preserved for the reboot-race resurrection). An exit-when-empty save carries the `ShutdownEmpty` origin instead. An empty tree clears the snapshot: everything was closed deliberately, so the next start is fresh rather than a resurrection of panes whose processes the user watched exit. An all-dead tree still holds its panes, so the save refreshes the snapshot and the next start restores them held dead (see [Pane Reaping](#pane-reaping)). A restored all-dead tree with no client exits again after the same grace. Consequences of the semantics: a bare `par-mux <name>` started with nothing to restore exits after the grace, and a one-shot `--cmd` client that finds an empty daemon ends it a grace after disconnecting. An embedded `MuxServer::run()` (no persistence) never exits-when-empty — it serves until stopped.
- **`kill-server`**: the client-initiated equivalent of SIGTERM — raises the same shutdown flag, so it takes the identical path (reply out, accept loop exits, final save, `%exit` to clients). `par-mux --stop`/`--restart` (see [Command Line](#command-line)) send this and wait for the socket to stop accepting.
- **Ignored signals** (Unix): SIGHUP, SIGINT, SIGQUIT, SIGTSTP, and SIGPIPE are installed as ignored while serving, so a terminal hangup or stray Ctrl-C/Ctrl-Z cannot stop a daemon whose panes outlive any one terminal (tmux's server does the same). The auto-spawned daemon additionally starts in its own session with no controlling tty (`setsid` in `spawn_daemon`), so terminal-generated signals never reach it in the first place.
- **Accept faults**: EMFILE, ENFILE, and ECONNABORTED on accept back off for a second and keep serving (tmux's server pauses on ENFILE/EMFILE the same way), so a burst of connections cannot end the daemon. Any other listener fault is logged and takes the same final-save path as SIGTERM before exiting — an accept error never silently discards unsaved work. The fault raises the shutdown flag just as SIGTERM does, so the exit waits only for the in-flight save to drain (bounded by the persist worker's poll) — never on connected clients, whose handler threads each hold a persist-sender clone a silent client would never drop. Both the fault exit and the SIGTERM exit drain in-flight saves before the process ends.
- **SIGKILL**: skips all of this and loses the last window's worth of unsaved content — an accepted trade (the design's [D3.3](MUX_DECISIONS.md)), not a bug.
- **Socket removal**: every exit path — SIGTERM, `kill-server`, exit-when-empty, accept fault — removes the socket file after the final save (the marker file on Windows), so `ls par-mux-*.sock` lists only live daemons. SIGKILL and a crash skip it, leaving a stale remnant the next bind reclaims.
- Each `MuxServer` instance has its own shutdown flag; stopping one does not affect another server in the same process.

## Streaming Panes to the Web

`par-term-streamer --mux-socket <path>` (built with `--features streaming-bin,mux`) serves the daemon's panes to the web and mobile frontend: each streaming session mirrors one pane (`?session=pane-N`), keys and mouse reach it through `send-keys -H`, and a viewer's resize resizes the pane (latest resize wins; opening a viewer never resizes it). The library form is `streaming::MuxSessionFactory`. Full behavior: [STREAMING.md — Mux-Backed Sessions](STREAMING.md#mux-backed-sessions).

## Embedding from Rust

Everything above is available as a library API under the `mux` feature:

```rust
use par_term_emu_core_rust::mux::{MuxServer, MuxClient};

// Serve with persistence (what the par-mux binary runs):
let server = MuxServer::bind(&socket_path)?;
server.run_persisting(state_path);

// Or in-process without persistence (tests and embedders):
let server = MuxServer::bind(&socket_path)?;
let shutdown = server.shutdown_handle();
std::thread::spawn(move || server.run());
// shutdown.store(true, std::sync::atomic::Ordering::Relaxed); // stop it later

// Client side — attach, spawning a daemon if none is running:
let mut client = MuxClient::connect_or_spawn("default")?;
let panes = client.send("list-panes")?;
for notification in client.notifications().try_recv() { /* … */ }
```

- `MuxServer::bind` / `bind_with_tree` / `run` (no persistence) / `run_persisting` / `shutdown_handle`.
- `MuxClient::connect` / `connect_or_spawn` / `connect_or_spawn_at` / `send` (one command, returns the reply body lines — a `%error` block's body too) / `send_checked` (the same, as a `Reply { body, ok }` that tells `%end` from `%error`) / `notifications` (pushed `TmuxNotification`s) / `kill_spawned_daemon` — deliberately not a `Drop` impl, so a disconnecting client never kills the server other clients use.
- Pane creation goes through a `PaneFactory`, whose `create_pane` receives a `SpawnContext` (owning session, window, and session environment); a second method, `create_argv_pane`, takes structured argv — the seam the Windows resume path uses to spawn without a cmd.exe string re-parse (its default renders through the string path, exact on POSIX `sh`); the default `ShellPaneFactory` spawns plain commands, and `AgentPaneFactory` pre-tags agent panes for embedders and the resume path.

## Testing

The mux test suites must run serialized — several spawn daemons on default-style paths that collide under parallel execution. The integration tests that exec the daemon fail to compile without `mux-bin`:

```bash
cargo test --no-default-features --features rust-only,mux-bin,serde -- --test-threads=1
```

The integration suites live in `tests/mux_*.rs` (`mux_daemon`, `mux_restart`, `mux_reattach`, `mux_hooks`, `mux_agents`, `mux_feature_isolation`, and others).

### Fuzzing the control protocol

The control-socket parser is an adversarial-input surface (it executes commands, spawns panes, and persists state), so its two grammars have cargo-fuzz targets in the detached `fuzz/` workspace (see [Fuzzing](../CONTRIBUTING.md#fuzzing) for toolchain setup and the crash-to-regression policy):

- `mux_parse_command` — `parse_line`/`parse_command` over arbitrary lines, including multi-line inputs and `%`-notification-looking shapes.
- `mux_hook_report` — the `{`-shaped hook-report JSON grammar through `handle_report` against an empty tree (JSON parse, header validation, SEC-105 value caps; no panes exist, so nothing downstream can be corrupted).

Run them locally from the repository root (nightly toolchain, first build is slow — the sanitizer instruments the whole dependency tree). Each runs for `FUZZ_SECONDS` (default 60) under `-rss_limit_mb=512`:

```bash
make fuzz-mux_parse_command
make fuzz-mux_hook_report
```

Seed corpora live in `fuzz/corpus/<target>/`; add a corpus file for every new command or report shape.

## Module Map

| File | Role |
|------|------|
| `src/mux/mod.rs` | Module root and re-exports |
| `src/mux/server.rs` | Socket server: accept loop, client threads, broadcast sinks, scrape heartbeat |
| `src/mux/dispatch.rs` | Per-command handlers (`cmd_<name>`) plus the shared emit-and-save tail |
| `src/mux/command.rs` | Control-command parsing: the `COMMANDS` table, one `parse_<cmd>` per command, `MuxCommand`, and the `mutates()` persistence rule |
| `src/mux/emit.rs` | `TmuxNotification` → wire lines, reply blocks, octal output escaping |
| `src/mux/tree/` | The session/window/pane tree and its operations (`mod.rs` types, accessors, and target resolvers; `lifecycle.rs` create/respawn/kill; `layout_ops.rs` splits, moves, resizes, and the `mutate_layout` choke point) |
| `src/mux/layout.rs` | The binary `LayoutTree` split geometry and tmux layout strings |
| `src/mux/pane.rs` | `MuxPane` (PTY + `Terminal`), `PaneFactory`, `ShellPaneFactory`, the env contract |
| `src/mux/ids.rs` | `SessionId`/`WindowId`/`PaneId` (`$N`/`@N`/`%N`) and id allocation |
| `src/mux/ipc.rs` | Cross-platform local socket transport (Unix socket / Windows named pipe), `0600`/DACL binding, stale-path reclamation |
| `src/mux/discovery.rs` | Server registry (`<state-base>/servers/<stem>.json`) and `--list-servers` enumeration: register/unregister, the named-default directory sweep, and the `version`/`list-sessions` probe |
| `src/mux/persist.rs` | Save format (version 2), atomic writes, quarantine, restore incl. the agent resume path |
| `src/mux/host_probe.rs` | The host telemetry probe: disk + git per pane cwd, its 30 s cadence thread, and per-field freshness serving |
| `src/mux/hooks/` | The JSON hook-report grammar: `pane.report_agent`, `pane.report_agent_session`, `pane.report_agent_telemetry`, `pane.release_agent` (`mod.rs` dispatch + shared header/seq/reply helpers, `report.rs` state and session, `telemetry.rs`, `release.rs`) |
| `src/mux/scrape.rs` | The scrape tier engine and its 1 s tick, incl. the claim liveness sweep |
| `src/mux/foreground.rs` | Process-table snapshot + descendant-tree liveness probe for hook claims (macOS `sysctl KERN_PROC_ALL` + `KERN_PROCARGS2` — libproc is ancestry-gated; Linux `/proc`; inert on Windows) |
| `src/mux/agent_resume.rs` | The per-agent resume invocation table |
| `src/mux/win_resume.rs` | Windows resume transport: PE resolution and the cmd bridge batch |
| `src/mux/client.rs` | `MuxClient`: connect/attach, `send`, notifications, spawn/kill daemon |
| `src/mux/patterns/` | Bundled scrape patterns: `claude.toml`, `codex.toml`, `grok.toml` |
| `src/bin/par_mux/main.rs` | The daemon binary: CLI, state load/quarantine, signal handlers, `--restart` detach |
