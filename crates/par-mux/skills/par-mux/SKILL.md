---
name: par-mux
description: Use when managing par-mux (the terminal multiplexer bundled with par-term) from a script or agent — starting/stopping/restarting the par-mux daemon, creating workspaces, sessions, windows, and panes, spawning panes for dev servers or long-running commands, sending keys or text to a pane, capturing pane output, reading the agent roster (list-agents), reporting agent state via hook reports, resizing/renaming/titling panes, or debugging "no daemon running" and stale-daemon errors. Covers the `par-mux` CLI (client mode) and the pane env contract.
---

<!-- PAR_MUX_INTEGRATION_VERSION=6 -->

# par-mux

## What it is

par-mux is the tmux-control-mode multiplexer bundled with par-term (also
installable standalone from crates.io): a daemon (`par-mux`) owns PTY-backed
panes in a `workspace → session → window → pane` tree and serves a local Unix
socket. The **`par-mux --cmd <command>`** client mode is how an agent drives
it — one command per invocation, reply printed, exit code 0 on success and 1
on a daemon-reported error (`%error` prints to stderr). It **never starts a
daemon**: a `--cmd` with nothing running fails immediately with `no daemon
running on <path>`.

Ids are typed and stable: workspaces `+N`, sessions `$N`, windows `@N`,
panes `%N`. Prefer ids over names — a name that matches two targets is an
error listing the candidates, never a silent pick. Ids are monotonic and
never reused, even across daemon restarts (a restored tree keeps its old
ids) — always re-query, never assume `%0` or `%1`.

Every client call targets a socket: `--socket PATH` overrides it, and the
default is `$XDG_RUNTIME_DIR/par-mux-<name>.sock` (a per-user fallback dir
below the temp dir on Linux/macOS). The `no daemon running on <path>` error
names the exact path it tried — that is your first debug signal.

## Daemon lifecycle

```sh
par-mux default                 # start detached AND attach this terminal
                                #   (named; "default" may be omitted);
                                #   add --no-attach for daemon-only, or
                                #   --foreground to serve in-process
par-mux --restart               # stop + start detached on the same socket — the
                                #   routine fix after rebuilding par-mux (an old
                                #   daemon keeps serving old code until restarted)
par-mux --stop                  # stop cleanly and wait
par-mux --list-servers          # every live server this user can reach
par-mux <name> --cmd version    # build stamp — compare against the client build
                                #   to detect a stale daemon
# Verb forms exist alongside the flags (same handlers):
#   par-mux start|stop|restart [name|--socket P], par-mux status,
#   par-mux ls (list-sessions), par-mux config [--force]
```

`--restart` works when nothing is running (prints a notice, still exits 0).
The daemon detaches itself; no `&` needed. A fresh launch lands in a working
tree: when the daemon it started has no sessions, the auto-attach creates
the default one (workspace and session `default`, window `1`, one pane)
instead of failing. It ignores terminal hangup, exits
once its tree has been empty for 5 s — zero sessions, or only dead panes;
attached clients do NOT keep it alive (tmux's `exit-empty`; set
`[daemon] exit-empty = false` to keep it running) — and persists the
tree (state, scrollback, paste buffer) across restarts — panes respawn with
fresh processes in their saved cwd. Panes it spawns are marked with
`PAR_MUX_ENV=1` and starting a daemon from inside a pane is refused
(nested-daemon guard) unless `PAR_MUX_ALLOW_NESTED=1`. A config file
(`~/.config/par-mux/config.toml`, `[daemon]` and `[client]` sections,
`par-mux --gen-config` to seed it) carries socket/state-dir defaults and is
hot-reloaded by `reload-config`.

`par-mux attach` (built in only when installed with `--features mux-bin,attach`
— the par-term bundle is; the crate's default feature set is empty, so a
bare `cargo install par-mux` builds no binary at all) opens the interactive
terminal client — render mode (panes, status bar, mouse, chords; the
default) or `--mode passthrough`; `-t` targets a session/window/pane and the
prefix chord defaults to `C-b`. Target-less attach follows a fixed rule: a
daemon with no sessions gets a default tree created (workspace and session
`default`, a window named `1`, one unnamed pane) and attaches to it;
exactly one existing session attaches as-is; several sessions are a refusal
that lists them with the `-t` hint — pass `-t <session>` to pick one. That
one is for humans; agents stay on `--cmd`.

## Orient → act → confirm

```sh
par-mux status                  # the whole orientation in ONE call: stamp,
                                #   workspaces, sessions, displayed session,
                                #   pane count (exit 1 + start hint if no daemon)
par-mux --cmd list-workspaces   # "+N: name" lines; " active" marks the daemon one
par-mux --cmd list-sessions     # "+W: wname: $N: name" lines, every workspace when bare
par-mux --cmd list-windows      # "@N: name" lines
par-mux --cmd list-panes        # "%N" lines, global
par-mux --cmd list-agents       # agent roster: "%N <agent> <state> <source> [key=value…]"
                                #   extras: reason=<base64> telemetry=<base64>
                                #   host_telemetry=<base64> ws=+N — parse fields
                                #   1-4 by position, split the rest on the first
                                #   "=", ignore unknown keys
par-mux --cmd list-commands     # every dispatchable command with feature tokens —
                                #   the discovery call for what THIS daemon supports
```

Then act on ids captured from those listings, and confirm the effect by
capturing the pane (below). For stable field output use `-F`:
`list-sessions -F '#{session_id}=#{session_name}'` (also `list-windows`,
`list-panes`; unknown `#{names}` expand to nothing). For a pure existence
check use the exit-code probe: `has-session -t work` (0 = live, 1 = absent).

## Command map

| Goal | Command |
| --- | --- |
| New workspace | `new-workspace [-n name]` → replies `+N`, becomes active, lands with its first session |
| Switch workspace | `select-workspace -t +N` — resumes its active session |
| New session | `new-session [-s name] [-e KEY=VALUE]… [-n window] [-c dir] [-x W -y H] [--] [command]` → replies `$N` (lands in the active workspace) |
| New window | `new-window -t $N [-n name] [-c dir] [--] [command]` → replies `@N` |
| Split a pane | `split-window -t %N [-h\|-v] [-p 1-99] [-c dir] [--] [command]` → replies `%N` |
| Run a program | every create takes a trailing start command, tmux form: `new-session -s logs -- tail -f x`; `respawn-pane -t %N [-k] [-c dir] -- <command>` replaces a live pane's |
| Send keys | `send-keys -t %N <keys…>` — key names (`Enter`, `C-c`, `BSpace`, arrows), `-l` literal, `-H` hex bytes |
| Type a command | `send-keys -t %N -l "make test" --enter` — the `--enter` flag appends Enter (one call); the two-step `… "make test" Enter` also works |
| Capture screen | `capture-pane -t %N` — visible screen; `-S -200` adds 200 history lines |
| Capture styled | `capture-pane -t %N -e` — one line per grid row with SGR escapes inline |
| Wait for output | `wait-output -t %N (-m TEXT \| -r REGEX) [-T ms]` — blocks until a line matches; exit 0 + the line, exit 1 on timeout/exit/close |
| Stream raw bytes | `par-mux observe -t %N [--no-seed]` — the pane's raw PTY bytes to stdout, read-only; `par-mux control -t %N [--size WxH]` also forwards stdin. For scripts/embedders, not interactive use (that is `par-mux attach -t %N`) |
| Launch an agent | `agent-start -t %N [-T ms] <kind> [args…]` — types the agent into a pane at its shell prompt; exit 0 with `%N <agent> <state> <source>` once detected ready (idle), exit 1 on timeout or blocked startup |
| Prompt an agent | `agent-prompt -t %N -w [-T ms] -- <text>` — submits the prompt and blocks until the agent is idle/done/blocked; exit 1 with `stalled` if it never started working within 5 s. `agent-explain -t %N` shows how its state is detected |
| Read a title | `pane-title -t %N` (empty reply = none set) |
| Set a sticky title | `select-pane -t %N -T 'dev server'` (``-T ''`` clears) |
| Pane geometry | `pane-info -t %N` → `%N @W COLSxROWS`; `resize-pane -t %N -x 80 -y 24` (or `-L\|-R\|-U\|-D [cells]`) |
| Rename | `rename-workspace -t +N <name>` / `rename-session -t $N <name>` / `rename-window -t @N <name>` |
| Kill | `kill-pane -t %N` (the last pane of a window closes the window too), `kill-window -t @N`, `kill-session -t $N`, `kill-workspace -t +N [-g]` (`-g` closes a repo workspace together with its linked worktree workspaces; see below) |
| Git worktree workspace | `worktree-create -c <repo> -b <branch> [--base base] [-p path] [-s name]` → replies `+N` (checks out or creates the branch in a new worktree, opens it as a workspace grouped under the repo's); `worktree-open -c <checkout>` (focuses it if already open); `worktree-remove -t +N [-f]`; `list-worktrees` |
| Paste buffer | `set-buffer <text>` / `show-buffer` / `paste-buffer -t %N` |
| Notify the user | `show-notification [-b body] [--] <title>` — delivered by every attached TUI through its own notify-delivery channel (toast, OSC 9/99, or OS notifier) |
| Session env | `set-environment -t $N KEY VALUE` (applies to panes spawned *after*) |
| Live settings | `show-options` (applied `key=value` lines) / `set-option exit-empty off` (live keys: `exit-empty`, `remain-on-exit`; everything else is restart-required) |
| Stop everything | `par-mux --stop` (or `kill-server`) — clean save, then exit |

`-h` places the new pane beside the target, `-v` (default) below; `-p` is the
percent given to the **new** pane. `new-window` without `-t` targets the
most-recently-created session. Quoted values (names, titles, `-c` dirs, env
values) may contain spaces; everything else is whitespace-split. A pane whose
child exited on its own is HELD with its exit code (when `[daemon]
remain-on-exit` is on) — `respawn-pane` restarts it in place.

## Git worktree workspaces

Give each parallel task its own git worktree and workspace. The daemon runs
git itself, so paths must be absolute or `~/`-relative (the daemon's cwd is
not yours):

```sh
par-mux --cmd "worktree-create -c ~/Repos/app -b feat/login"   # +3; opens repo workspace "app" (+2) if needed
par-mux --cmd list-worktrees
# +2 repo - main /Users/me/Repos/app
# +3 worktree +2 feat/login /Users/me/Repos/app-worktrees/feat-login
par-mux --cmd "worktree-remove -t +3"      # git worktree remove + close the workspace
par-mux --cmd "kill-workspace -t +2 -g"    # close app and every linked worktree workspace
```

- `list-worktrees` rows are `+N <repo|worktree> <parent|-> <branch|-> <checkout>`.
  `parent` is the open repo workspace a worktree is grouped under. The
  checkout path is the rest of the line, so it may contain spaces.
- New checkouts default to `<repo-parent>/<repo>-worktrees/<branch-slug>`;
  `-p` overrides. An existing local branch is checked out, otherwise it is
  created from `--base` (default `HEAD`).
- `kill-workspace -t <repo workspace>` with linked worktrees open is refused:
  `workspace +2 has linked worktree workspaces (+3); use -g to close the group`.
  Add `-g` to close them all. A linked worktree workspace closes alone with no
  flag. Closing never deletes a checkout. Only `worktree-remove` does.
- `worktree-remove` refuses a dirty checkout (modified or untracked files) or a
  workspace with any agent pane (any `list-agents` row, `idle` included)
  unless `-f`. It refuses a repo workspace. git runs first, so a refusal leaves
  everything open.
- `--cmd` allows `worktree-create`/`worktree-remove` 10 minutes to reply.

## The two gotchas

**1. `split-window -t` takes a pane id, not a window id.** You split *from
an existing pane. `split-window -t @1` fails with `no such pane: @1`; use the
pane in that window (`%1`), then capture the new pane id from the reply.

**2. The wire is line-based — never send a bare newline inside a payload.**
`send-keys` appends nothing: `Enter` is an expressible key you send
explicitly. To type multi-line text, send one line per call, or send raw
bytes with `-H`, which takes **one hex byte pair per token**
(`send-keys -t %N -H 63 61 74 0a` types `cat\n`; a longer hex string like
`636174` is a parse error). Bare `0xNN` tokens (e.g. `0x0a`) send single raw
bytes in an otherwise-literal payload. `set-buffer <content>` takes the rest
of the line verbatim, so it carries one line at a time; for multi-line
content, send line-wise or set the buffer once per line before each
`paste-buffer`.

## Reading pane output

`capture-pane` returns what is on screen right now. To wait for a
long-running command (dev server, test run) to print something, use
`wait-output` instead of polling:

```sh
par-mux --cmd "wait-output -t %3 -r 'listening on|error' -T 60000"
```

It blocks until a line of the pane's capture (the last 100 history lines
plus the screen; `-S/-E` as for `capture-pane`) matches `-m TEXT` or
`-r REGEX`, prints that line, and exits 0; a timeout, pane exit, or pane
close exits 1 with the reason on stderr. Output already on screen matches
at once, so wait for a marker unique to this run. To wait for an agent
verdict instead of watching its screen, read the roster:

```sh
par-mux --cmd list-agents       # %3 claude blocked hook reason=<base64> ws=+0
```

The blocked reason is standard base64 of the whitespace-collapsed message
(`echo <b64> | base64 -d`), so a reason can never be mistaken for a grammar
token.

A finished agent reads `idle` OR `done`: `done` means it finished while no
attached client was viewing its pane, and it stays `done` until one does. With
no render client attached every finish reads `done`, so a script waiting for
an agent to finish must accept both.

When the rendered screen is not enough — escapes, graphics, or modes must
survive — stream the pane's raw bytes instead of capturing it:

```sh
par-mux observe -t %3              # screen seed + live raw PTY bytes, read-only
par-mux observe -t %3 --no-seed    # live bytes only
printf 'make test\r' | par-mux control -t %3 --no-seed   # the same stream, plus stdin
```

The seed reproduces the screen in a fresh emulator of the reported size;
apply the following `%output` bytes to stay live. `observe` is for scripts
and embedders running their own terminal model (piping a pane into a logger
or an emulator); `control` adds input for driving a program that must see
raw bytes. Both exit 0 on the daemon's `%exit`, 1 when refused, 2 on a
mid-stream drop. For interactive single-pane use, run `par-mux attach -t %N`.

Agent panes spawn with an env contract a script inside the pane can use:

```sh
echo "pane=$PAR_MUX_PANE_ID session=$PAR_MUX_SESSION_ID window=$PAR_MUX_WINDOW_ID socket=$PAR_MUX_SOCKET bin=$PAR_MUX_BIN"
"$PAR_MUX_BIN" --socket "$PAR_MUX_SOCKET" --cmd list-panes   # client mode without PATH
```

`PAR_MUX_SOCKET` names the FULL control socket by default — an agent in one
pane can drive every other pane through it. Only on a daemon started with
`--pane-endpoints` does it name a hook-only socket bound to that pane alone;
such daemons also export `PAR_MUX_CONTROL_SOCKET` (started with
`--expose-control-socket`) for full control. The vars are fixed at spawn — a
rename or a pane move leaves the name fields stale (the id fields stay
valid), and `respawn-pane` re-seeds them.

## Agent-state integrations

par-mux installs the hooks and extensions that feed the roster itself:
`par-mux --install-agent-hooks` (claude, codex, grok, muse, droid, copilot,
cursor), `--install-agent-extensions`
(pi, omp), with `--uninstall-agent-hooks`, `--uninstall-agent-extensions` and
`--agent-hooks-status` counterparts. par-term runs these after installing
par-mux. The hooks are inert and instant outside a par-mux pane, and silent on
every failure. `par-mux --integration-status` reports every bundled asset at
once — hook, extension, and skill lines, each `<kind> <name> installed|absent`
followed by the asset's version currency (`v2`, `v1 → v2 outdated`, `asset
missing`) — so an agent (or the TUI) can see an out-of-date install after a
par-mux upgrade and re-run the install flags to refresh it.

## Agent hook reports

A pane process reports agent state by sending **one JSON line** to the
socket and reading **one JSON reply** (`{"id":…,"result":"ok"}`). No `--cmd`
support — this is a raw socket write:

```sh
printf '%s\n' "{\"id\":1,\"method\":\"pane.report_agent\",\"params\":{
  \"pane_id\":\"$PAR_MUX_PANE_ID\",\"agent\":\"claude\",\"seq\":$(date +%s),
  \"state\":\"blocked\",\"message\":\"waiting for review\"}}" | nc -U "$PAR_MUX_SOCKET"
```

- `state` is `working` / `blocked` / `question` / `done` / `idle` (`unknown` accepted, never
  stored; the daemon stores an `idle` after work as `done` until a client views
  the pane, so a reporter need not send `done` itself);
  `seq` must increase per pane **per source** — stale reports are dropped
  silently. Use an epoch clock (`date +%s`); a shell-start counter like
  `$SECONDS` restarts near 0 for every invocation and gets dropped.
- `pane.report_agent_session` (id/transcript path + optional resume argv) and
  `pane.report_agent_telemetry` (versioned, display-only) exist too;
  `pane.release_agent` is the exit announcement and clears the whole claim.
- A hook report makes the pane **hook-authoritative** — the automatic
  screen-scrape (claude/codex/grok/muse/droid/copilot/cursor) never
  overrides it. Panes whose agents
  never report a hook get scraped state instead (roster `source=scrape`).
- `nc -U` needs a netcat with Unix-socket support (macOS built-in works).

## Requirements

- The `par-mux` binary: ships with par-term (`make install` and `make bundle`
  place it next to par-term), or standalone — `cargo install par-mux
  --features mux-bin,attach` from crates.io, plus GitHub Releases binaries.
- This skill installs itself: `par-mux --install-skill` writes it to
  `~/.claude/skills/par-mux/` (overwriting any older copy), and par-term runs
  that same command after installing par-mux — so the skill always documents
  the binary you actually have.
- Full command reference (list-commands feature tokens, client-snapshot,
  attach chords, notifications, persistence semantics): `MUX.md` in the
  par-term-emu-core-rust repository (github.com/paulrobello/par-term-emu-core-rust).
  par-term's `docs/features/MUX.md` is a different document: its user-facing <!-- doc-path: example -->
  integration guide (attaching par-term windows to sessions), not a copy.
