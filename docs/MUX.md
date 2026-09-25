# par-mux — Terminal Multiplexer Daemon

`par-mux` is a tmux-control-mode multiplexer daemon: it owns PTY-backed panes arranged in a session/window/pane tree and serves the tmux control-mode protocol over a local socket, so control-mode clients (including `par-term-tmux` and the `MuxClient` in this crate) can attach to it in place of tmux. It also carries an agent layer: panes can report agent state over the same socket, and agent sessions survive daemon restarts.

The daemon is feature-gated (Rust `mux` feature), optional, and independent of the Python bindings and the streaming server. The wire format's conformance oracle is the `TmuxControlParser` in `src/tmux_control.rs` — the daemon emits what that parser decodes.

> **Design decisions:** code comments cite D-numbered decisions (`D3.3`, `D5`, …) from the design document [`docs/par-mux.md`](par-mux.md), which points at the authoritative plan in the `par-agent-os` repository.

## Table of Contents

- [Building and Running](#building-and-running)
- [Command Line](#command-line)
- [Client Mode](#client-mode)
- [Socket and State Paths](#socket-and-state-paths)
- [Protocol Overview](#protocol-overview)
- [Command Reference](#command-reference)
- [Notifications](#notifications)
- [Agent Hook Reports](#agent-hook-reports)
- [Agent Scrape Tier](#agent-scrape-tier)
- [Persistence and Restart](#persistence-and-restart)
- [Pane Reaping](#pane-reaping)
- [Shutdown Semantics](#shutdown-semantics)
- [Streaming Panes to the Web](#streaming-panes-to-the-web)
- [Embedding from Rust](#embedding-from-rust)
- [Testing](#testing)
- [Module Map](#module-map)

## Building and Running

The daemon binary is not part of the default build. Build it with the `mux` feature:

```bash
cargo build --bin par-mux --no-default-features --features mux
```

Run it (it prints its socket path and serves until killed):

```bash
cargo run --bin par-mux --no-default-features --features mux
```

The Python wheel and the default `make dev` build do not include the daemon.

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
```

`--socket <path>` is what `MuxClient::connect_or_spawn_at` passes when it starts a daemon. A second daemon on a path a live server already owns is refused with "another server owns <path>"; a stale socket remnant (dead socket file, Windows marker file, or a stray regular file at the path) is reclaimed.

`--stop` and `--restart` are flags rather than subcommands — the positional `NAME` would otherwise be ambiguous with a session literally named `stop`. `--stop` sends `kill-server` to the daemon on that socket and waits (30 s bound) for the socket to stop accepting connections; "no daemon running" is reported but is not an error. `--restart` does the same stop, then serves the same socket from a detached process — the state save the stop just completed is what it restores. The invocation returns as soon as the daemon detaches (fork + `setsid`, stdio to `/dev/null`), so no `&` is needed and closing the terminal it was typed into leaves the daemon and its panes running. This is the routine fix after rebuilding par-mux, since clients attach to whatever daemon owns the socket and an old daemon keeps serving old code until restarted.

### Nested daemons

Serve mode refuses to start inside a par-mux pane: when `PAR_MUX_ENV` is set (the pane env contract marks every pane), `par-mux <name>` / `par-mux --socket <path>` exits non-zero with `refusing to start a nested daemon: PAR_MUX_ENV is set … set PAR_MUX_ALLOW_NESTED=1 to override` and binds no socket — a nested daemon would shadow the outer server's identity for every PTY under it. `MuxClient::connect_or_spawn` applies the same rule: from inside a pane it attaches to a daemon that is already running but refuses to auto-spawn one, failing fast instead of retrying a socket the guard keeps unbound.

Exempt from the guard, because a pane must keep operating on its own daemon: `--cmd` (client mode never starts a daemon), `--stop`, and `--restart` — tmux likewise allows `kill-server` from inside a session. `PAR_MUX_ALLOW_NESTED=1` (or unsetting `PAR_MUX_ENV`) starts a nested daemon anyway.

Panes never inherit a stale outer identity: a PTY spawned by any process inside a mux pane drops every `PAR_MUX_*` variable from its inherited environment and re-adds only its own via the env contract below — so a par-term started in a pane gets clean local tabs instead of reporting its agents to the outer daemon under the wrong pane id.

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

# A named daemon, or an explicit socket
par-mux work --cmd list-panes
par-mux --socket /tmp/par-mux-test.sock --cmd version
```

| Outcome | stdout | stderr | Exit code |
|---------|--------|--------|-----------|
| Success (`%end`) | The reply body, one line per reply line, framing stripped; nothing for an empty reply | — | 0 |
| Command failed (`%error`) | — | The daemon's error text | 1 |
| No daemon owns the socket | — | `no daemon running on <path>` | 1 |
| Transport failure after connecting (no reply block, connection closed) | — | The I/O error | 2 |

Client mode never starts a daemon: a bare `par-mux --cmd list-sessions` with nothing running fails immediately instead of booting one. The command string follows the daemon's grammar (see the quoting rules under [Command Reference](#command-reference)); wrap it in single quotes in the shell so its inner double quotes reach the daemon intact. One command per invocation — there is no `;` sequencing, interactive attach, or follow mode. Pushed notifications that arrive while the reply is pending are discarded. Stdout writes stop quietly on a closed pipe, so `par-mux --cmd '...' | head -1` does not panic.

## Socket and State Paths

| Item | Unix | Windows |
|------|------|---------|
| Transport | Unix domain socket at `<base>/par-mux-<name>.sock` | Named pipe; a marker file is written at the same path so staleness checks work uniformly |
| Default socket base | `$XDG_RUNTIME_DIR`, falling back to the temp dir | Per-user temp dir |
| Access control | Socket file mode `0600` (owner only) | Owner-only security descriptor (system + creating user, nothing for anyone else) |

The on-disk state file (see [Persistence and Restart](#persistence-and-restart)) never lives next to the socket:

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

A failed command closes with `%error` instead of `%end`, the body carrying the error message. Reply bodies are written raw, so a body line can itself start with `%end` (pane output captured verbatim) — a client therefore closes a block only on the `%end`/`%error` carrying the command number of its `%begin`, as tmux's own clients do. Pushed pane output arrives as `%output %N <data>` lines, with every non-printable byte octal-escaped (`\033`) and the backslash itself escaped (`\134`) so decoding is unambiguous.

### Client backpressure and eviction

Each registered client has a bounded broadcast queue (4096 lines). A client that stops reading its socket long enough for its queue to fill — 4096 undelivered lines, worst case ~128 MiB when every line carries a full PTY read — is **evicted**: the daemon logs a warning naming the reason (queue full — stalled, or draining slower than the burst), flags the connection, and its own reader/writer threads tear it down within a fraction of a second, closing the socket the client observes. A wedged client is disconnected rather than allowed to grow the daemon without bound. A client that keeps draining loses nothing. An evicted client simply reconnects and re-queries; pane output it missed is not replayed to it.

Eviction is by **queue depth only** — intended, and deliberately unlike tmux, whose control clients are evicted by output *age* (300 s) while buffering without bound in between. The depth policy bounds daemon memory at ~128 MiB per client; its accepted consequence is that a burst whose backlog passes 4096 lines evicts even a *healthy* client that drains continuously but slower than the producer — the backlog, not the client, is what hit the cap. The client reconnects and re-queries. Pinned by `a_slow_draining_client_is_evicted_once_its_backlog_passes_the_cap` (`src/mux/server.rs`); an age-based policy would replace that test with a drains-slowly-survives one.

## Command Reference

The parser is deliberately minimal: whitespace-split with a flag scan. tmux's full argument grammar (`--`, per-command option tables, command sequences) is not implemented. Quoting is honored in a fixed set of places, all sharing one bounded grammar (single or double quotes, backslash escapes outside quotes, the `'\''` close-escape-reopen idiom, no interpolation): the `send-keys` payload, the `new-session -s NAME` / `new-window -n NAME` names, the `select-pane -T TITLE` title, environment values (`new-session -e NAME=VALUE`, the `set-environment` name and value), and the `-t`/`-s` targets, so a name, title, value, or target may contain spaces. Every other flag is whitespace-split, and `rename-window` / `set-buffer` take the rest of the line verbatim. List replies have fixed shapes with no `-F` support — push notifications cover what `-F` polling existed for.

A target placeholder in the table below (`<pane>`, `<window>`, `<session>`) is either the typed id (`%N`, `@N`, `$N`) or a **name**, resolved daemon-side against the tree:

- **`<pane>`** matches the pane's sticky user title (`select-pane -T`) exactly. The pane program's live OSC 0/2 title never matches — it changes with the running program and would make name targets flaky.
- **`<window>`** matches the window's name (`new-window -n`, `rename-window`) exactly, across every session.
- **`<session>`** matches the session's name (`new-session -s`) exactly.

Ids always win over names: a value starting with the target kind's own sigil is the id, so a pane titled `%3` can never shadow pane `%3` (and a sigil-prefixed value that is not a valid id, like `%abc`, is a parse error). A name matching more than one pane/window/session is an **error listing the sorted candidate ids** (`ambiguous pane target: dup (matching: %0, %2)`) — never a silent pick, and the command acts on nothing. An unknown name errors (`no such pane: <name>`). One limitation: `send-keys` takes a single-token target, because its raw payload split cannot carry a spaced name next to free text — address such a pane by id.

| Command | Arguments | Reply body | Broadcasts |
|---------|-----------|------------|------------|
| `new-session` | `[-s name] [-e NAME=VALUE]…` | The session id (`$N`) | `%window-add` per window; `%session-changed` to the issuer |
| `new-window` | `[-t <session>] [-n name]` | The window id (`@N`) | `%window-add` |
| `select-window` | `-t <window>` | empty | `%window-pane-changed` |
| `kill-window` | `-t <window>` | empty | `%window-close` |
| `rename-window` | `-t <window> <name>` | empty | `%window-renamed` |
| `split-window` | `-t <pane> [-h\|-v] [-p 1-99]` | The new pane id (`%N`) | `%layout-change`, `%window-pane-changed` |
| `select-pane` | `-t <pane> [-T 'title']` | empty | `%layout-change`, `%window-pane-changed`; `%pane-title-changed` when `-T` changed the title |
| `pane-title` | `-t <pane>` | The pane's effective title as the body; an empty body (no lines) = no title set | — |
| `pane-info` | `-t %N` | One line, `%N @W COLSxROWS` for the resolved pane: its window and current grid size (read-only; lets a mirroring client seed at the pane's size instead of resizing it) | — |
| `resize-pane` | `-t <pane> (-L\|-R\|-U\|-D [cells] \| -x COLS [-y ROWS])` | empty | `%layout-change` |
| `swap-pane` | `-t <pane> -s <pane>` | empty | `%layout-change` |
| `kill-pane` | `-t <pane>` | empty | `%layout-change`, `%window-pane-changed` |
| `list-panes` | — | One `%N` line per pane, globally | — |
| `list-windows` | — | One `@N: name` line per window, globally | — |
| `list-sessions` | — | One `$N: name` line per session | — |
| `list-agents` | — | Roster: one `%N <agent> <state> <source>` line per state-carrying pane (see below) | — |
| `capture-pane` | `-t <pane> [-S start] [-E end] [-e]` | The captured lines | — |
| `send-keys` | `-t <pane> <keys>` | empty | — |
| `refresh-client` | `-t <pane> [-C WxH]` | Pane's screen-restore replay (no `-C`); empty with `-C` | `%layout-change` (with `-C`) |
| `set-buffer` | `<content>` | empty | — |
| `set-environment` | `-t <session> NAME VALUE` or `-t <session> -u NAME` | empty | — |
| `show-buffer` | — | The buffer content | — |
| `paste-buffer` | `-t <pane>` | empty | — |
| `version` | — | The daemon's build stamp, one line: `<crate version>+<git sha[-dirty]>` (`+unknown` when built outside a repository) | — |
| `kill-server` | — | empty | `%exit` to every client, then the daemon exits |

Details worth knowing:

- **Bare `new-window`** targets the most-recently-created session (ids are monotonic). par-mux has no client-session attachment, so "newest" is the documented stand-in for tmux's attached-session resolution.
- **`version` exists for stale-daemon detection**: the daemon outlives its clients, so an old daemon silently serves new clients. A client compares the reply against its own linked core's `mux::build_stamp()`; differing stamps mean the daemon predates the client's build. When either side's sha is `unknown` (crates.io builds), only the version prefix is comparable — a same-version mismatch is then unprovable and clients stay quiet rather than cry wolf.
- **`split-window` flags name the arrangement, not the divider**: `-h` puts the new pane beside the target, `-v`/default below it. `-p` is the percent of the split area given to the **new** pane (default 50; the target keeps the remainder).
- **`resize-pane` relative form** moves the bordering divider; the first of `-L -R -U -D` wins, and a flag without a number means 5 cells (tmux's default). The absolute form `-x COLS` and/or `-y ROWS` sets exact extents and cannot combine with the direction flags. Both forms re-fit the affected panes' terminals and PTYs to the layout geometry.
- **`capture-pane -S/-E`** use tmux's offset convention: `0` is the first visible line, negative numbers count history lines back from the screen top. Without flags, the visible screen is returned.
- **`capture-pane -e`** returns the pane's visible screen (the active grid — an alt-screen TUI captures its TUI screen) as one line per grid row with SGR escape bytes inline, tmux's `-e` contract: a styled run is emitted as `\x1b[0;<fg>;<bg>[;<attrs>]m` before its text (the export walker's fixed reset-fg-bg order), a row that used any SGR ends with `\x1b[0m` before its newline, and a row that used none is plain text. Empty rows are empty lines, so the reply always holds exactly one line per grid row. Styled trailing blanks survive (plain text trims them). Without `-e` the reply stays the plain logical-lines capture, byte-identical to the pre-`-e` reply; `-e` composes with `-S/-E`, which then trim the styled scrollback+screen composition. The framing is line-per-row, not cursor addressing — replaying it into an emulator requires per-row addressing on the consumer side (a bare LF staircases).
- **`send-keys`** speaks the tmux contract: key names (`C-a`…`C-z`, `C-Space`, `Escape`, `BSpace`, `Space`, arrows, `Enter`), `-l` literal payloads, `-H` hex bytes, and bare `0xNN` tokens. No trailing newline is appended — `Enter` is an expressible key.
- **`refresh-client -C WxH`** reports the client's renderer size; the window resizes to it (latest report wins) and every pane re-fits. Without `-C`, the command replays the pane's state to the requesting client — the reattach resync. The reply is `Terminal::export_screen_restore_sequence()`: the main screen's scrollback first when the pane has any (`\x1b[H` anchor, each history line CR-led and LF-terminated so it replays into the client emulator's own scrollback in order, then the still-on-screen lines pushed off the bottom row with plain line feeds), alt-screen selection next (`\x1b[?1049h` when active), the scroll region, then the styled screen content with absolute row addressing (`\x1b[H` anchor, per-row `\x1b[R;1H`, SGR-diffed runs, reset per row), then the cursor position (`\x1b[R;CH`), visibility (`\x1b[?25l`) and style (DECSCUSR), the input modes (DECCKM, bracketed paste, focus tracking, mouse tracking/encoding), origin mode restored last with a region-relative re-position, and a final `\x1b[0m`. The scrollback block does carry `\n` bytes (one per replayed history line) — despite that, raw ESC bytes still survive the `%begin`/`%end` line framing untouched, since the reader consumes the reply as an ordinary sequence of framed lines regardless of how many lines the body spans; a client feeds the whole body verbatim into its pane emulator and the pane's subsequent `%output` deltas land on the restored state. A pane with no scrollback yields the pre-scrollback-fix shape: no `\n` at all, purely CUP-addressed.
- **Session environment (`set-environment`, `new-session -e`)** is a per-session variable map applied on top of the daemon's own environment for every pane spawned into that session afterwards. Panes already running are untouched (tmux semantics), and the `PAR_MUX_*` contract vars always win over a same-named session var. It exists because the daemon outlives its clients: panes otherwise inherit whatever environment the daemon was started with, so a client that reconnects over SSH or with a changed `SSH_AUTH_SOCK`, `DISPLAY`, or `PATH` sends its current values here (tmux's `update-environment`). `-t` is required, `-e` repeats, quoting follows the bounded grammar above (`-e 'A=two words'`, `NAME 'a value'`), and a name that is empty or contains `=` or NUL is refused. Values can be secrets: they are never logged, and they persist in the owner-only state file.
- **Buffers** are a single slot named `default` — no numbered stack, and `-b` is not implemented.
- **`kill-pane`** is refused for a window's last pane. A pane whose child process exits on its own (rather than via `kill-pane`) is instead reaped automatically — see [Pane Reaping](#pane-reaping) below.
- **`kill-server`** raises the same shutdown flag SIGTERM does: the reply goes out, the accept loop notices on its next tick, `%exit` reaches every client, and the final state save runs before the process exits — see [Shutdown Semantics](#shutdown-semantics). Refused with an error for an embedded `MuxServer` that has no shutdown handle (`ctx.shutdown` is `None`). `par-mux --stop`/`--restart` send this command under the hood.
- **`list-agents`** returns one line per pane a hook has claimed or a scrape pattern has matched, sorted by pane id: `%N <agent> <state> <source>` with `source` `hook` or `scrape`, plus the blocked reason as the rest of the line when the agent reported one. Panes without state are absent — `unknown` is never reported, and `idle` is never guessed.
- **Pane titles (`select-pane -T`)** set a user title on the pane; `-T ''` clears it (quoting is what makes an empty value expressible). Precedence is a deliberate divergence from tmux: a user title is **sticky** — the pane program's OSC 0/2 title never overwrites it — while with no user title the pane reports the program's live OSC title. The effective title (user when set, else OSC) is what `pane-title -t %N` replies — an empty reply body means neither is set; a broadcast carries only the *user* title's changes, so a client composing a display title falls back to its own OSC tracking on the empty form. Setting the same title again is a no-op and broadcasts nothing.

## Notifications

Broadcast lines every connected (registered) client receives, emitted in this order per mutating dispatch: `%layout-change` first (it carries the geometry the pane-changed notification is read against), then lifecycle broadcasts, then issuer-only notifications.

| Line | Meaning |
|------|---------|
| `%output %N <data>` | Pane output, octal-escaped, pushed as bytes arrive — no polling |
| `%layout-change @N <layout> <visible-layout> <flags>` | A window's geometry changed |
| `%window-add @N` | A window was created |
| `%window-close @N` | A window was killed |
| `%window-renamed @N <name>` | A window was renamed |
| `%window-pane-changed @N %N` | A window's active pane changed |
| `%session-changed $N <name>` | Sent to the issuing client after `new-session` |
| `%agent-state-changed %N <agent> <state> [source=hook\|scrape]` | An agent's state changed, with provenance |
| `%agent-released %N <agent>` | The pane's agent released its claim (`pane.release_agent`) — state, hook authority, and session identity cleared; the pane left the roster |
| `%pane-title-changed %N [title]` | A pane's user title changed — `select-pane -T` set it (title follows the pane id, spaces included) or cleared it (no title token). The user title is sticky over the program's OSC title; the notification carries the user title only |
| `%exit` | Graceful shutdown — the daemon is ending deliberately, not dying |

tmux control-mode clients ignore `%` lines they do not recognize, so a client that never learned `%agent-state-changed` (or any future variant) still sees the raw line rather than an error.

## Agent Hook Reports

A pane's process can report agent state over the control socket by sending one JSON line and reading one JSON reply. herdr's integration scripts port unchanged except the env-var rename (`HERDR_*` → `PAR_MUX_*`).

Every pane process is seeded with the env contract (`src/mux/pane.rs`):

| Variable | Value |
|----------|-------|
| `PAR_MUX_PANE_ID` | The pane's own id (`%N`) |
| `PAR_MUX_SOCKET` | The control socket path |
| `PAR_MUX_ENV` | `1` — marks the contract as present |
| `PAR_MUX_SESSION_ID` | The owning session's id (`$N`) |
| `PAR_MUX_SESSION` | The owning session's name |
| `PAR_MUX_WINDOW_ID` | The owning window's id (`@N`) |
| `PAR_MUX_BIN` | The daemon executable, so a pane script can run client mode without `par-mux` on `PATH`: `"$PAR_MUX_BIN" --socket "$PAR_MUX_SOCKET" --cmd list-sessions`. Set by the `par-mux` binary; an embedded `MuxServer` leaves it unset unless its factory supplies `bin_path` |

These are set for every spawn path: `new-session`, `new-window`, `split-window`, and restore. They are **fixed at spawn**, as tmux's `TMUX`/`TMUX_PANE` are: a later `rename-session` leaves `PAR_MUX_SESSION` stale, and a `swap-pane` across windows leaves `PAR_MUX_WINDOW_ID` stale. The ids stay valid; the name is advisory. `PAR_MUX_SOCKET`, `PAR_MUX_ENV`, and `PAR_MUX_BIN` are absent when the server has no socket path or binary path to export.

Two methods (`src/mux/hooks.rs`):

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

- Common params: `pane_id`, `agent`, `seq` (monotonic per pane **per source**; a report at or below the last accepted `seq` from its own source is dropped with no write and no broadcast — sources stamp different clocks, e.g. `time.time_ns()` vs `Date.now()*1000`, so freshness is tracked per `source`), and optional `source`.
- `pane.report_agent` additionally carries `state` (`working`/`blocked`/`idle`; `unknown` is accepted but never written), optional identity (`agent_session_id`/`agent_session_path`), and optional `message` — the blocked reason, stored whitespace-collapsed and cleared when a later report omits it.
- Validation is at the door: `state` outside the fixed set, an `agent` label containing whitespace or control characters, or a `source` containing control characters is error-replied, nothing written, nothing broadcast. These fields are interpolated verbatim into the space-split `%agent-state-changed` and roster lines, and any process in any pane can reach this endpoint — a newline in a field would forge a control-mode line delivered to every attached client.
- `pane.report_agent_session` identifies the session by **id or transcript path** (either alone is enough) and may carry `session_resume_argv` — an array of non-empty strings, malformed values error-replied — stored verbatim as the pane's resume invocation. `session_start_source` records startup vs resume provenance. A session report that moves the pane to a **different agent** clears the previous agent's state, hook authority, and blocked reason rather than rebroadcasting them under the new label.
- `pane.release_agent` is the agent's exit announcement (herdr's SessionEnd shape): it clears the pane's whole claim — label, state, hook authority, blocked reason, sequence stamps, and session identity — broadcasts `%agent-released`, and thereby returns the pane to the roster-less majority (a later restart respawns the original command, not a resume invocation for a dead session). Guards: the releasing `agent` must match the pane's current label, and the report must clear the same per-source `seq` rule; a failing guard is a silent ok no-op, like a stale report.
- Replies are one JSON line: `{"id":…,"result":"ok"}` or an error object.
- Authorization is the socket's own owner-only boundary: a hook claiming another pane's id is same-user by construction — the same trust model tmux control mode has. There is no per-connection authentication beyond the socket.

## Agent Scrape Tier

Panes whose agent reports no state hook (claude, codex, grok) get state read from rendered content — OSC title and screen text matched against per-agent pattern rules bundled from herdr (Apache-2.0, credited per file). The scrape runs on the accept loop's idle poll once per second; there is no dedicated thread.

Structural rules (`src/mux/scrape.rs`):

- **Hooks stay primary.** A pane that has ever accepted a hook state report is hook-authoritative until it dies; the tick skips it outright, and a quiet hook agent holds its last claim.
- **A scrape never invents state.** An unmatched scrape clears its own earlier guess and yields nothing — never `idle`.
- **Provenance rides every surface**: the `source=` token on `%agent-state-changed`, the `<source>` column in `list-agents`, and the `agent_state_rule` metadata (which pattern fired).
- **`contains` matches case-insensitively** (herdr parity): needles lowercase at compile and the region text at match, so real-cased agent chrome ("Do you want to proceed?") hits rules authored in lowercase. `regex`/`line_regex` stay case-sensitive as written — make them `(?i)` explicitly when they must fold.

Patterns ship bundled (`include_str!` from `src/mux/patterns/{claude,codex,grok}.toml`). A local override file shadows the bundled set for its agent:

```text
<state_dir>/par-mux/agent-patterns/<agent>.toml
```

An override that fails to parse or validate falls back to the bundled set with a warning; a file for an agent with no bundled set adds it. Pattern files document which herdr rule regions are implemented (`osc_title`, `whole`, `bottom_non_empty_lines(N)`, `top_non_empty_lines(N)`).

## Persistence and Restart

State saving is crash-safe by construction (`src/mux/persist.rs`): serialize to `<file>.tmp`, fsync, rename over the target. The file is created `0600` on Unix (set at open, so there is no readable window between create and chmod) — the same owner-only posture as the socket. On Windows it inherits the ACL of the per-user state directory.

- **What persists:** the full tree (sessions, windows, panes, layout), each session's environment (`env`; a file written before the field existed loads with an empty one), each pane's screen and scrollback content, the paste buffer, each pane's user title (`user_title`, from `select-pane -T` — a save file written before the field existed loads with no title), and each agent pane's session identity (`agent_session`: agent label, session id and/or transcript path, source tag, and the hook-reported resume argv). Format version is **2**. Persisted scrollback is capped per pane to the newest 100 000 cells (~1 250 lines at 80 cols) — a persistence bound only: the running pane keeps its full in-memory history, and the cap is what keeps a scrollback-maxed shutdown save from stalling daemon exit (measured 2026-09-22: uncapped, one flooded pane's final save was a 136 MB write holding SIGTERM exit for two minutes).
- **What deliberately does not persist:** agent *state*, its provenance, the ordering `seq`, the blocked reason, and `session_start_source`. A restored pane reports state anew or holds none, so the roster is empty after a restart by design.
- **When it saves:** after every *successful mutating* command (the structural set: `new-session`, `set-environment`, `new-window`, `kill-window`, `rename-window`, `select-window`, `split-window`, `select-pane`, `resize-pane`, `swap-pane`, `kill-pane`, `set-buffer`, and `refresh-client -C`). Content commands (`send-keys`, `capture-pane`, `paste-buffer`, the lists) do not save — their staleness window is bounded by the next structural save and the clean-shutdown save.
- **Quarantine:** a corrupt or unknown-version state file is renamed aside (`<file>.quarantine-<timestamp>`) and the daemon starts fresh — unreadable state never blocks startup, and the evidence survives for inspection. A v1 state file is quarantined rather than partially read.
- **Last-good snapshot:** every pane-bearing save also copies the state file to `<file>.lastgood`. Reap-driven saves (a pane's child exiting on its own) never touch the snapshot, and an empty final shutdown save leaves it alone — so the reboot/logout race, where the panes are killed before the daemon's own stop (shells turn SIGHUP into exit code 1, so the deaths look like ordinary child exits), cannot wipe the layout: the next start finds a pane-less state file and restores the snapshot instead. The accepted tradeoff: a daemon whose last panes died by child exit (typing `exit`, not `kill-pane`) also resurrects on the next start — an explicit `kill-pane` of the last pane writes a deliberately-empty state that clears the snapshot, so the next start is fresh. A corrupt snapshot is ignored, not quarantined; it is a derived cache, not evidence.
- **Restore:** layout and content are rebuilt, and every pane's process is new — the original processes died with the previous server. A pane's terminal is rebuilt via `restore_for_new_process`, which keeps the saved screen content and scrollback but drops the state the *old* process left behind — as if the new process inherited a bare terminal, not the old one's TUI mode: it lands on the main screen (alternate grid cleared) regardless of what was active at save time, the scroll region and margins are reset, and cursor-key/keypad/mouse/focus/keyboard-protocol modes are off. This matters for a pane saved mid full-screen-app (htop, top): the pane returns to the shell's screen with its history intact, instead of resuming on a frozen, history-less alternate screen under a shell that never asked for it. Each pane's process respawns in its persisted cwd — captured at save from the shell's OSC 7 report, else the child's live process cwd — so a resumed pane (and a resumed agent, whose transcript lookup is cwd-sensitive) re-lands where it left off rather than in the daemon's start directory. A persisted cwd that no longer exists never fails the restore: the pane spawns in `$HOME` and a `par-mux: <dir> is gone` line is written into the restored pane. Agent panes respawn through their resume invocation when one can be built: the hook-reported `session_resume_argv` verbatim, else a per-agent table shipped in the binary (claude/codex/grok/pi/omp, each in its own CLI's argument shape — `claude --resume <id>`, `codex resume <id>`, `grok --resume <id>`, `pi --session <id-or-path>`, `omp --resume=<id-or-path>`). The resume invocation is never persisted — a stored argv would freeze a vendor CLI shape into a state file newer code later restores. Failure splits by where it happens. A chain that cannot *build* an invocation (missing table entry, a session ref the table cannot use) falls back structurally to the pane's original spawn command — the pane spawns as it was. A chain that builds but *runs* into failure (uninstalled binary, a session id the CLI rejects) does not delete the pane: on Unix the invocation spawns with a fallback tail (`|| { printf …; exec "$SHELL" }`), so a non-zero exit drops the pane onto a live shell carrying the restored screen, scrollback, and agent identity — an explanatory line is printed in the pane, the reaper never sees a dead pane, and the identity survives for a retry or an explicit `kill-pane`. On Windows the invocation stays bare (cmd.exe cannot parse the POSIX tail; Windows resume quoting is a known open gap), so a failed resume there still exits into the reaper.

## Pane Reaping

A pane whose child process exits on its own (as opposed to via `kill-pane`) is detected and cleaned up automatically rather than left frozen in the tree. The accept loop's idle tick runs a reaper every 250 ms (`REAP_INTERVAL`, `src/mux/server.rs`): a dead pane (the PTY reader's `is_running` flipped false on EOF) is removed from its window with `%layout-change` + `%window-pane-changed` broadcasts, or the window itself is closed with `%window-close` when the dead pane was its last one — the same notifications a structural `kill-pane`/`kill-window` sends, so existing tmux consumers need no new handling. An emptied session is left in place rather than removed; create-or-attach refills it. Before this existed, a pane whose shell exited (typing `exit` at the prompt, for example) sat dead in the tree indefinitely and clients saw a permanently frozen pane.

## Shutdown Semantics

- **SIGTERM**: the handler makes one atomic store on the server's per-instance shutdown flag. The accept loop notices on its idle tick, broadcasts `%exit` to every client, and a final save captures content that arrived since the last structural save.
- **`kill-server`**: the client-initiated equivalent of SIGTERM — raises the same shutdown flag, so it takes the identical path (reply out, accept loop exits, final save, `%exit` to clients). `par-mux --stop`/`--restart` (see [Command Line](#command-line)) send this and wait for the socket to stop accepting.
- **Ignored signals** (Unix): SIGHUP, SIGINT, SIGQUIT, SIGTSTP, and SIGPIPE are installed as ignored while serving, so a terminal hangup or stray Ctrl-C/Ctrl-Z cannot stop a daemon whose panes outlive any one terminal (tmux's server does the same). The auto-spawned daemon additionally starts in its own session with no controlling tty (`setsid` in `spawn_daemon`), so terminal-generated signals never reach it in the first place.
- **Accept faults**: EMFILE, ENFILE, and ECONNABORTED on accept back off for a second and keep serving (tmux's server pauses on ENFILE/EMFILE the same way), so a burst of connections cannot end the daemon. Any other listener fault is logged and takes the same final-save path as SIGTERM before exiting — an accept error never silently discards unsaved work. The fault raises the shutdown flag just as SIGTERM does, so the exit waits only for the in-flight save to drain (bounded by the persist worker's poll) — never on connected clients, whose handler threads each hold a persist-sender clone a silent client would never drop. Both the fault exit and the SIGTERM exit drain in-flight saves before the process ends.
- **SIGKILL**: skips all of this and loses the last window's worth of unsaved content — an accepted trade (the design's D3.3), not a bug.
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
- Pane creation goes through a `PaneFactory`, whose `create_pane` receives a `SpawnContext` (owning session, window, and session environment); the default `ShellPaneFactory` spawns plain commands, and `AgentPaneFactory` pre-tags agent panes for embedders and the resume path.

## Testing

The mux test suites must run serialized — several spawn daemons on default-style paths that collide under parallel execution:

```bash
cargo test --no-default-features --features rust-only,mux,serde -- --test-threads=1
```

The integration suites live in `tests/mux_*.rs` (`mux_daemon`, `mux_restart`, `mux_reattach`, `mux_hooks`, `mux_agents`, `mux_feature_isolation`, and others).

## Module Map

| File | Role |
|------|------|
| `src/mux/mod.rs` | Module root and re-exports |
| `src/mux/server.rs` | Socket server: accept loop, client threads, broadcast sinks, scrape heartbeat |
| `src/mux/dispatch.rs` | Per-command handlers (`cmd_<name>`) plus the shared emit-and-save tail |
| `src/mux/command.rs` | Control-command parsing: the `COMMANDS` table, one `parse_<cmd>` per command, `MuxCommand`, and the `mutates()` persistence rule |
| `src/mux/emit.rs` | `TmuxNotification` → wire lines, reply blocks, octal output escaping |
| `src/mux/tree.rs` | The session/window/pane tree and its operations |
| `src/mux/layout.rs` | The binary `LayoutTree` split geometry and tmux layout strings |
| `src/mux/pane.rs` | `MuxPane` (PTY + `Terminal`), `PaneFactory`, `ShellPaneFactory`, the env contract |
| `src/mux/ids.rs` | `SessionId`/`WindowId`/`PaneId` (`$N`/`@N`/`%N`) and id allocation |
| `src/mux/ipc.rs` | Cross-platform local socket transport (Unix socket / Windows named pipe), `0600`/DACL binding, stale-path reclamation |
| `src/mux/persist.rs` | Save format (version 2), atomic writes, quarantine, restore incl. the agent resume path |
| `src/mux/hooks.rs` | The JSON hook-report grammar: `pane.report_agent`, `pane.report_agent_session`, `pane.release_agent` |
| `src/mux/scrape.rs` | The scrape tier engine and its 1 s tick |
| `src/mux/agent_resume.rs` | The per-agent resume invocation table |
| `src/mux/client.rs` | `MuxClient`: connect/attach, `send`, notifications, spawn/kill daemon |
| `src/mux/patterns/` | Bundled scrape patterns: `claude.toml`, `codex.toml`, `grok.toml` |
| `src/bin/par_mux/main.rs` | The daemon binary: CLI, state load/quarantine, signal handlers, `--restart` detach |
