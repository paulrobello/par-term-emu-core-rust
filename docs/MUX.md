# par-mux — Terminal Multiplexer Daemon

`par-mux` is a tmux-control-mode multiplexer daemon: it owns PTY-backed panes arranged in a session/window/pane tree and serves the tmux control-mode protocol over a local socket, so control-mode clients (including `par-term-tmux` and the `MuxClient` in this crate) can attach to it in place of tmux. It also carries an agent layer: panes can report agent state over the same socket, and agent sessions survive daemon restarts.

The daemon is feature-gated (Rust `mux` feature), optional, and independent of the Python bindings and the streaming server. The wire format's conformance oracle is the `TmuxControlParser` in `src/tmux_control.rs` — the daemon emits what that parser decodes.

> **Design decisions:** code comments cite D-numbered decisions (`D3.3`, `D5`, …) from the design document [`docs/par-mux.md`](par-mux.md), which points at the authoritative plan in the `par-agent-os` repository.

## Table of Contents

- [Building and Running](#building-and-running)
- [Command Line](#command-line)
- [Socket and State Paths](#socket-and-state-paths)
- [Protocol Overview](#protocol-overview)
- [Command Reference](#command-reference)
- [Notifications](#notifications)
- [Agent Hook Reports](#agent-hook-reports)
- [Agent Scrape Tier](#agent-scrape-tier)
- [Persistence and Restart](#persistence-and-restart)
- [Shutdown Semantics](#shutdown-semantics)
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

The daemon hand-parses its arguments (there is no `--help`):

```text
par-mux <name>            Bind the default socket path for <name>
par-mux --socket <path>   Bind an explicit socket path
par-mux                   Same as par-mux default
```

`--socket <path>` is what `MuxClient::connect_or_spawn_at` passes when it starts a daemon. A second daemon on a path a live server already owns is refused with "another server owns <path>"; a stale socket remnant (dead socket file, Windows marker file, or a stray regular file at the path) is reclaimed.

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

A failed command closes with `%error` instead of `%end`, the body carrying the error message. Pushed pane output arrives as `%output %N <data>` lines, with every non-printable byte octal-escaped (`\033`) and the backslash itself escaped (`\134`) so decoding is unambiguous.

### Client backpressure and eviction

Each registered client has a bounded broadcast queue (4096 lines). A client that stops reading its socket long enough for its queue to fill — 4096 undelivered lines, worst case ~128 MiB when every line carries a full PTY read — is **evicted**: the daemon logs a warning, flags the connection, and its own reader/writer threads tear it down within a fraction of a second, closing the socket the client observes. This is tmux's policy for a control client that stops draining: a wedged client is disconnected rather than allowed to grow the daemon without bound. A client that keeps draining loses nothing. An evicted client simply reconnects and re-queries; pane output it missed is not replayed to it.

## Command Reference

The parser is deliberately minimal: whitespace-split with a flag scan. tmux's full argument grammar (`--`, per-command option tables, command sequences) is not implemented. Quoting is honored in exactly four places, all sharing one bounded grammar (single or double quotes, backslash escapes outside quotes, the `'\''` close-escape-reopen idiom, no interpolation): the `send-keys` payload, the `new-session -s NAME` / `new-window -n NAME` names, and the `select-pane -T TITLE` title, so a name or title may contain spaces. Every other flag is whitespace-split — the `-t`/`-s` targets elsewhere are typed `$N`/`@N`/`%N` ids that cannot contain whitespace, and `rename-window` / `set-buffer` take the rest of the line verbatim. List replies have fixed shapes with no `-F` support — push notifications cover what `-F` polling existed for.

| Command | Arguments | Reply body | Broadcasts |
|---------|-----------|------------|------------|
| `new-session` | `[-s name]` | The session id (`$N`) | `%window-add` per window; `%session-changed` to the issuer |
| `new-window` | `[-t $N] [-n name]` | The window id (`@N`) | `%window-add` |
| `select-window` | `-t @N` | empty | `%window-pane-changed` |
| `kill-window` | `-t @N` | empty | `%window-close` |
| `rename-window` | `-t @N <name>` | empty | `%window-renamed` |
| `split-window` | `-t %N [-h\|-v] [-p 1-99]` | The new pane id (`%N`) | `%layout-change`, `%window-pane-changed` |
| `select-pane` | `-t %N [-T 'title']` | empty | `%layout-change`, `%window-pane-changed`; `%pane-title-changed` when `-T` changed the title |
| `pane-title` | `-t %N` | The pane's effective title as the body; an empty body (no lines) = no title set | — |
| `resize-pane` | `-t %N (-L\|-R\|-U\|-D [cells] \| -x COLS [-y ROWS])` | empty | `%layout-change` |
| `swap-pane` | `-t %N -s %N` | empty | `%layout-change` |
| `kill-pane` | `-t %N` | empty | `%layout-change`, `%window-pane-changed` |
| `list-panes` | — | One `%N` line per pane, globally | — |
| `list-windows` | — | One `@N: name` line per window, globally | — |
| `list-sessions` | — | One `$N: name` line per session | — |
| `list-agents` | — | Roster: one `%N <agent> <state> <source>` line per state-carrying pane (see below) | — |
| `capture-pane` | `-t %N [-S start] [-E end] [-e]` | The captured lines | — |
| `send-keys` | `-t %N <keys>` | empty | — |
| `refresh-client` | `-t %N [-C WxH]` | Pane's screen-restore replay (no `-C`); empty with `-C` | `%layout-change` (with `-C`) |
| `set-buffer` | `<content>` | empty | — |
| `show-buffer` | — | The buffer content | — |
| `paste-buffer` | `-t %N` | empty | — |

Details worth knowing:

- **Bare `new-window`** targets the most-recently-created session (ids are monotonic). par-mux has no client-session attachment, so "newest" is the documented stand-in for tmux's attached-session resolution.
- **`split-window` flags name the arrangement, not the divider**: `-h` puts the new pane beside the target, `-v`/default below it. `-p` is the percent of the split area given to the **new** pane (default 50; the target keeps the remainder).
- **`resize-pane` relative form** moves the bordering divider; the first of `-L -R -U -D` wins, and a flag without a number means 5 cells (tmux's default). The absolute form `-x COLS` and/or `-y ROWS` sets exact extents and cannot combine with the direction flags. Both forms re-fit the affected panes' terminals and PTYs to the layout geometry.
- **`capture-pane -S/-E`** use tmux's offset convention: `0` is the first visible line, negative numbers count history lines back from the screen top. Without flags, the visible screen is returned.
- **`capture-pane -e`** returns the pane's visible screen (the active grid — an alt-screen TUI captures its TUI screen) as one line per grid row with SGR escape bytes inline, tmux's `-e` contract: a styled run is emitted as `\x1b[0;<fg>;<bg>[;<attrs>]m` before its text (the export walker's fixed reset-fg-bg order), a row that used any SGR ends with `\x1b[0m` before its newline, and a row that used none is plain text. Empty rows are empty lines, so the reply always holds exactly one line per grid row. Styled trailing blanks survive (plain text trims them). Without `-e` the reply stays the plain logical-lines capture, byte-identical to the pre-`-e` reply; `-e` composes with `-S/-E`, which then trim the styled scrollback+screen composition. The framing is line-per-row, not cursor addressing — replaying it into an emulator requires per-row addressing on the consumer side (a bare LF staircases).
- **`send-keys`** speaks the tmux contract: key names (`C-a`…`C-z`, `C-Space`, `Escape`, `BSpace`, `Space`, arrows, `Enter`), `-l` literal payloads, `-H` hex bytes, and bare `0xNN` tokens. No trailing newline is appended — `Enter` is an expressible key.
- **`refresh-client -C WxH`** reports the client's renderer size; the window resizes to it (latest report wins) and every pane re-fits. Without `-C`, the command replays the pane's state to the requesting client — the reattach resync. The reply is `Terminal::export_screen_restore_sequence()`: alt-screen selection first (`\x1b[?1049h` when active), the scroll region, then the styled screen content with absolute row addressing (`\x1b[H` anchor, per-row `\x1b[R;1H`, SGR-diffed runs, reset per row), then the cursor position (`\x1b[R;CH`), visibility (`\x1b[?25l`) and style (DECSCUSR), the input modes (DECCKM, bracketed paste, focus tracking, mouse tracking/encoding), origin mode restored last with a region-relative re-position, and a final `\x1b[0m`. The reply body contains no `\n` (row placement is CUP-addressed), so raw ESC bytes survive the `%begin`/`%end` line framing untouched; a client feeds the bytes verbatim into its pane emulator and the pane's subsequent `%output` deltas land on the restored state.
- **Buffers** are a single slot named `default` — no numbered stack, and `-b` is not implemented.
- **`kill-pane`** is refused for a window's last pane.
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

- Common params: `pane_id`, `agent`, `seq` (monotonic per pane; a report at or below the last accepted `seq` is dropped with no write and no broadcast), and optional `source`.
- `pane.report_agent` additionally carries `state` (`working`/`blocked`/`idle`; `unknown` is accepted but never written), optional identity (`agent_session_id`/`agent_session_path`), and optional `message` — the blocked reason, stored whitespace-collapsed and cleared when a later report omits it.
- `pane.report_agent_session` identifies the session by **id or transcript path** (either alone is enough) and may carry `session_resume_argv` — an array of non-empty strings, malformed values error-replied — stored verbatim as the pane's resume invocation. `session_start_source` records startup vs resume provenance.
- Replies are one JSON line: `{"id":…,"result":"ok"}` or an error object.
- Authorization is the socket's own owner-only boundary: a hook claiming another pane's id is same-user by construction — the same trust model tmux control mode has. There is no per-connection authentication beyond the socket.

## Agent Scrape Tier

Panes whose agent reports no state hook (claude, codex, grok) get state read from rendered content — OSC title and screen text matched against per-agent pattern rules bundled from herdr (Apache-2.0, credited per file). The scrape runs on the accept loop's idle poll once per second; there is no dedicated thread.

Structural rules (`src/mux/scrape.rs`):

- **Hooks stay primary.** A pane that has ever accepted a hook state report is hook-authoritative until it dies; the tick skips it outright, and a quiet hook agent holds its last claim.
- **A scrape never invents state.** An unmatched scrape clears its own earlier guess and yields nothing — never `idle`.
- **Provenance rides every surface**: the `source=` token on `%agent-state-changed`, the `<source>` column in `list-agents`, and the `agent_state_rule` metadata (which pattern fired).

Patterns ship bundled (`include_str!` from `src/mux/patterns/{claude,codex,grok}.toml`). A local override file shadows the bundled set for its agent:

```text
<state_dir>/par-mux/agent-patterns/<agent>.toml
```

An override that fails to parse or validate falls back to the bundled set with a warning; a file for an agent with no bundled set adds it. Pattern files document which herdr rule regions are implemented (`osc_title`, `whole`, `bottom_non_empty_lines(N)`, `top_non_empty_lines(N)`).

## Persistence and Restart

State saving is crash-safe by construction (`src/mux/persist.rs`): serialize to `<file>.tmp`, fsync, rename over the target. The file is created `0600` on Unix — the same owner-only posture as the socket.

- **What persists:** the full tree (sessions, windows, panes, layout), each pane's screen and scrollback content, the paste buffer, each pane's user title (`user_title`, from `select-pane -T` — a save file written before the field existed loads with no title), and each agent pane's session identity (`agent_session`: agent label, session id and/or transcript path, source tag, and the hook-reported resume argv). Format version is **2**. Persisted scrollback is capped per pane to the newest 100 000 cells (~1 250 lines at 80 cols) — a persistence bound only: the running pane keeps its full in-memory history, and the cap is what keeps a scrollback-maxed shutdown save from stalling daemon exit (measured 2026-09-22: uncapped, one flooded pane's final save was a 136 MB write holding SIGTERM exit for two minutes).
- **What deliberately does not persist:** agent *state*, its provenance, the ordering `seq`, the blocked reason, and `session_start_source`. A restored pane reports state anew or holds none, so the roster is empty after a restart by design.
- **When it saves:** after every *successful mutating* command (the structural set: `new-session`, `new-window`, `kill-window`, `rename-window`, `select-window`, `split-window`, `select-pane`, `resize-pane`, `swap-pane`, `kill-pane`, `set-buffer`, and `refresh-client -C`). Content commands (`send-keys`, `capture-pane`, `paste-buffer`, the lists) do not save — their staleness window is bounded by the next structural save and the clean-shutdown save.
- **Quarantine:** a corrupt or unknown-version state file is renamed aside (`<file>.quarantine-<timestamp>`) and the daemon starts fresh — unreadable state never blocks startup, and the evidence survives for inspection. A v1 state file is quarantined rather than partially read.
- **Restore:** layout and content are rebuilt, and every pane's process is new — the original processes died with the previous server. Agent panes respawn through their resume invocation when one can be built: the hook-reported `session_resume_argv` verbatim, else a per-agent table shipped in the binary (claude/codex/grok/pi/omp, each in its own CLI's argument shape — `claude --resume <id>`, `codex resume <id>`, `grok --resume <id>`, `pi --session <id-or-path>`, `omp --resume=<id-or-path>`). The resume invocation is never persisted — a stored argv would freeze a vendor CLI shape into a state file newer code later restores. Every failure mode of the chain falls back structurally to the pane's original spawn command: a missing table entry or an uninstalled binary just spawns the pane as it was.

## Shutdown Semantics

- **SIGTERM**: the handler makes one atomic store on the server's per-instance shutdown flag. The accept loop notices on its idle tick, broadcasts `%exit` to every client, and a final save captures content that arrived since the last structural save.
- **SIGKILL**: skips all of this and loses the last window's worth of unsaved content — an accepted trade (the design's D3.3), not a bug.
- Each `MuxServer` instance has its own shutdown flag; stopping one does not affect another server in the same process.

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
- `MuxClient::connect` / `connect_or_spawn` / `connect_or_spawn_at` / `send` (one command, returns the reply body lines) / `notifications` (pushed `TmuxNotification`s) / `kill_spawned_daemon` — deliberately not a `Drop` impl, so a disconnecting client never kills the server other clients use.
- Pane creation goes through a `PaneFactory`; the default `ShellPaneFactory` spawns plain commands, and `AgentPaneFactory` pre-tags agent panes for embedders and the resume path.

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
| `src/mux/hooks.rs` | The JSON hook-report grammar: `pane.report_agent`, `pane.report_agent_session` |
| `src/mux/scrape.rs` | The scrape tier engine and its 1 s tick |
| `src/mux/agent_resume.rs` | The per-agent resume invocation table |
| `src/mux/client.rs` | `MuxClient`: connect/attach, `send`, notifications, spawn/kill daemon |
| `src/mux/patterns/` | Bundled scrape patterns: `claude.toml`, `codex.toml`, `grok.toml` |
| `src/bin/par_mux/main.rs` | The daemon binary: CLI, state load/quarantine, SIGTERM handler |
